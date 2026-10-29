//! 转码：把任意常见格式转成 MP4。
//!
//! 和 [`media`](super::media) 里的 `split` / `concat` 不同，这里的输入容器和
//! 编码都是未知的，所以必须重编码。
//!
//! 性能上有三个关键点：
//!
//! 1. **容器兼容时直接 remux** —— 已经是 MP4 且编码已经符合目标时，用
//!    `-c copy` 只换容器，几秒钟完成，画质零损失
//! 2. **只拷能拷的流** —— 视频已经是 h264 就 `copy`，只有音频需要转的时候
//!    只重编码音频，反之亦然
//! 3. **单遍编码** —— 不做两遍分析，用 `-preset` 调节速度/画质，默认 `veryfast`
//!
//! 默认输出参数偏向"能被 DLNA 渲染器直接播放"：H.264 + yuv420p + AAC + faststart。

use anyhow::{Context, Result, bail};
use ffmpeg_sidecar::command::FfmpegCommand;
use ffmpeg_sidecar::event::FfmpegEvent;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// 转码选项。
#[derive(Debug, Clone)]
pub struct TranscodeOptions {
    /// `libx264` 的 preset：`ultrafast` … `veryslow`。
    ///
    /// 默认 `veryfast`。转码速度大致是 `ultrafast` 的 1/1.5 到 `veryslow` 的
    /// 1/20，体积则相反。批量处理用 `veryfast`，最终交付可以上 `slow`。
    pub preset: String,
    /// CRF，越小画质越好体积越大。默认 23（x264 的默认值）。
    pub crf: u32,
    /// 音频码率，例如 `128k`。默认 `128k`。
    pub audio_bitrate: String,
    /// 目标像素格式，默认 `yuv420p`（兼容性最好）。
    pub pixel_format: String,
    /// 是否把 moov 前置（faststart）。默认开启，投屏必须。
    pub faststart: bool,
    /// 限制输出高度。给 `Some(720)` 时等比缩到 720p 以内，用于减小体积。
    pub max_height: Option<u32>,
}

impl Default for TranscodeOptions {
    fn default() -> Self {
        Self {
            preset: "veryfast".to_string(),
            crf: 23,
            audio_bitrate: "128k".to_string(),
            pixel_format: "yuv420p".to_string(),
            faststart: true,
            max_height: None,
        }
    }
}

impl TranscodeOptions {
    /// 快速模式：速度优先，体积略大。适合批量转码。
    pub fn fast() -> Self {
        Self {
            preset: "ultrafast".to_string(),
            crf: 28,
            ..Self::default()
        }
    }

    /// 高质量模式：速度慢，但体积小、画质好。适合最终交付。
    pub fn high_quality() -> Self {
        Self {
            preset: "slow".to_string(),
            crf: 20,
            audio_bitrate: "192k".to_string(),
            ..Self::default()
        }
    }
}

/// 输入文件的基本信息，用来决定是 remux 还是重编码。
#[derive(Debug, Clone, Default)]
pub struct MediaInfo {
    /// 容器格式名，例如 `avi` / `flv` / `mp4`。
    pub format: String,
    /// 视频编码，例如 `h264` / `wmv3` / `flv1`。
    pub video_codec: Option<String>,
    /// 音频编码，例如 `aac` / `mp3` / `wmav2`。
    pub audio_codec: Option<String>,
}

impl MediaInfo {
    /// 视频是否已经是 H.264（可以直接 copy）。
    pub fn video_is_h264(&self) -> bool {
        matches!(self.video_codec.as_deref(), Some("h264"))
    }

    /// 音频是否已经是 AAC（可以直接 copy）。
    pub fn audio_is_aac(&self) -> bool {
        matches!(self.audio_codec.as_deref(), Some("aac"))
    }

    /// 是否完全不需要重编码：MP4 容器 + H.264 + AAC。
    pub fn needs_no_reencode(&self) -> bool {
        self.format.contains("mp4") && self.video_is_h264() && self.audio_is_aac()
    }

    /// 是否已经是 MP4 容器。
    ///
    /// ffmpeg 把 MP4 家族报成 `mov,mp4,m4a,3gp,3g2,mj2`，所以要看整串，
    /// 不能只看第一个字段。
    pub fn is_mp4(&self) -> bool {
        self.format.contains("mp4")
    }
}

/// 探测输入文件的信息。
///
/// 用 `ffmpeg -i` 解析 stderr，避免依赖 ffprobe（`ffmpeg-sidecar` 可能只下载了
/// `ffmpeg` 本体）。
pub fn probe(ffmpeg: &Path, input: &Path) -> Result<MediaInfo> {
    let output = Command::new(ffmpeg)
        .arg("-hide_banner")
        .arg("-i")
        .arg(input)
        .stdin(Stdio::null())
        .stderr(Stdio::piped())
        .stdout(Stdio::null())
        .creation_flags_no_window()
        .output()
        .with_context(|| format!("无法运行 ffmpeg 探测 {}", input.display()))?;

    if !output.status.success() && output.stderr.is_empty() {
        bail!("探测 {} 失败", input.display());
    }

    let text = String::from_utf8_lossy(&output.stderr);
    Ok(parse_probe_output(&text))
}

/// 从 `ffmpeg -i` 的输出里解析容器和编码。
fn parse_probe_output(text: &str) -> MediaInfo {
    let mut info = MediaInfo::default();

    for line in text.lines() {
        let line = line.trim();

        if let Some(rest) = line.strip_prefix("Input #0, ") {
            // 形如 "Input #0, avi, from 'a.avi':"
            // MP4 家族会报成 "mov,mp4,m4a,3gp,3g2,mj2, from 'a.mp4':"
            // 所以取到 " from " 之前为止，而不是只取第一个逗号段。
            let format = rest.split(", from ").next().unwrap_or(rest);
            info.format = format.trim().to_ascii_lowercase();
            continue;
        }

        if let Some(rest) = line.strip_prefix("Stream #") {
            if rest.contains("Video:") {
                info.video_codec = extract_codec(rest);
            } else if rest.contains("Audio:") {
                info.audio_codec = extract_codec(rest);
            }
        }
    }

    info
}

/// 从 "0:0(und): Video: h264 (High), yuv420p, 1280x720, ..." 里取编码名。
fn extract_codec(stream: &str) -> Option<String> {
    let after = stream
        .split_once("Video:")
        .or_else(|| stream.split_once("Audio:"))?;

    let rest = after.1.trim_start();

    let codec = rest
        .split(|c: char| c == ',' || c == '(' || c.is_whitespace())
        .next()?
        .trim();

    if codec.is_empty() {
        None
    } else {
        Some(codec.to_ascii_lowercase())
    }
}

/// 转码结果。
#[derive(Debug, Clone)]
pub struct TranscodeOutcome {
    /// 输出文件路径。
    pub output: PathBuf,
    /// true 表示只换了容器，没有重编码。
    pub remuxed: bool,
    /// 实际重编码了哪些流。
    pub reencoded_video: bool,
    pub reencoded_audio: bool,
}

/// 把任意常见格式转成 MP4。
///
/// 会自动先探测输入，能 copy 的流就 copy，只重编码必要的部分。
///
/// ```no_run
/// use std::path::Path;
/// use rupnp::mp4_tool::transcode::{self, TranscodeOptions};
///
/// # fn main() -> anyhow::Result<()> {
/// let ffmpeg = Path::new("ffmpeg");
/// let outcome = transcode::to_mp4(
///     ffmpeg,
///     Path::new("movie.avi"),
///     Path::new("movie.converted.mp4"),
///     &TranscodeOptions::default(),
/// )?;
/// println!("remuxed: {}", outcome.remuxed);
/// # Ok(())
/// # }
/// ```
pub fn to_mp4(
    ffmpeg: &Path,
    input: &Path,
    output: &Path,
    options: &TranscodeOptions,
) -> Result<TranscodeOutcome> {
    let info = probe(ffmpeg, input)?;

    // 已经是 mp4 + h264 + aac：只换容器，几秒钟的事
    if info.needs_no_reencode() {
        remux_to_mp4(ffmpeg, input, output, options.faststart)?;

        return Ok(TranscodeOutcome {
            output: output.to_path_buf(),
            remuxed: true,
            reencoded_video: false,
            reencoded_audio: false,
        });
    }

    let reencode_video = !info.video_is_h264();
    let reencode_audio = !info.audio_is_aac();

    let mut command = FfmpegCommand::new_with_path(ffmpeg);

    if cfg!(windows) {
        command.create_no_window();
    }

    command
        .arg("-y")
        .arg("-hide_banner")
        .arg("-i")
        .arg(input)
        // 音轨可选：源文件可能没有音频
        .arg("-map")
        .arg("0:v:0?")
        .arg("-map")
        .arg("0:a:0?");

    if reencode_video {
        command
            .arg("-c:v")
            .arg("libx264")
            .arg("-preset")
            .arg(&options.preset)
            .arg("-crf")
            .arg(options.crf.to_string())
            .arg("-pix_fmt")
            .arg(&options.pixel_format);

        if let Some(height) = options.max_height {
            // 只缩小不放大，保持等比
            command
                .arg("-vf")
                .arg(format!("scale=-2:'min({height},ih)':flags=bicubic"));
        }
    } else {
        command.arg("-c:v").arg("copy");
    }

    if reencode_audio {
        command
            .arg("-c:a")
            .arg("aac")
            .arg("-b:a")
            .arg(&options.audio_bitrate);
    } else {
        command.arg("-c:a").arg("copy");
    }

    if options.faststart {
        command.arg("-movflags").arg("+faststart");
    }

    command.arg(output);

    run_with_progress(command, "转码")?;

    Ok(TranscodeOutcome {
        output: output.to_path_buf(),
        remuxed: false,
        reencoded_video: reencode_video,
        reencoded_audio: reencode_audio,
    })
}

/// 批量转码，返回每一项的结果。
///
/// 单个文件失败不会中断整批，失败项的错误会以 `Err` 的形式出现在返回的 `Vec` 里。
pub fn to_mp4_batch(
    ffmpeg: &Path,
    inputs: &[PathBuf],
    options: &TranscodeOptions,
) -> Vec<Result<TranscodeOutcome>> {
    inputs
        .iter()
        .map(|input| {
            let output = converted_path(input);

            eprintln!("转换 {} -> {}", input.display(), output.display());

            to_mp4(ffmpeg, input, &output, options)
        })
        .collect()
}

/// 只换容器：`-c copy -movflags +faststart`。
fn remux_to_mp4(ffmpeg: &Path, input: &Path, output: &Path, faststart: bool) -> Result<()> {
    let mut command = FfmpegCommand::new_with_path(ffmpeg);

    if cfg!(windows) {
        command.create_no_window();
    }

    command
        .arg("-y")
        .arg("-hide_banner")
        .arg("-i")
        .arg(input)
        .arg("-c")
        .arg("copy");

    if faststart {
        command.arg("-movflags").arg("+faststart");
    }

    command.arg(output);

    run_with_progress(command, "换容器")
}

/// 跑命令，转码时把进度打到 stderr。
fn run_with_progress(mut command: FfmpegCommand, what: &str) -> Result<()> {
    let mut child = command
        .spawn()
        .with_context(|| format!("启动 ffmpeg 失败（{what}）"))?;

    let events = child
        .iter()
        .with_context(|| format!("读取 ffmpeg 输出失败（{what}）"))?;

    let mut errors = Vec::new();

    for event in events {
        match event {
            FfmpegEvent::Progress(progress) => {
                eprint!("\r    {what} 中 {:<12}", progress.time);
                use std::io::Write;
                let _ = std::io::stderr().flush();
            }
            FfmpegEvent::Log(level, message) => {
                if matches!(level, ffmpeg_sidecar::event::LogLevel::Error) {
                    errors.push(message);
                }
            }
            FfmpegEvent::Error(message) => errors.push(message),
            _ => {}
        }
    }

    let status = child
        .wait()
        .with_context(|| format!("等待 ffmpeg 结束失败（{what}）"))?;

    eprintln!();

    if !status.success() {
        for error in &errors {
            eprintln!("[ffmpeg] {error}");
        }
        bail!("ffmpeg 处理失败（{what}）");
    }

    Ok(())
}

/// 转换后的默认输出路径：`<名字>.converted.mp4`。
pub fn converted_path(input: &Path) -> PathBuf {
    let stem = input
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "output".to_string());

    let directory = input.parent().unwrap_or_else(|| Path::new("."));

    directory.join(format!("{stem}.converted.mp4"))
}

/// 给 `std::process::Command` 加"不弹窗"标志。
trait NoWindow {
    fn creation_flags_no_window(&mut self) -> &mut Self;
}

impl NoWindow for Command {
    fn creation_flags_no_window(&mut self) -> &mut Self {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            self.creation_flags(CREATE_NO_WINDOW);
        }

        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROBE_AVI: &str = "\
Input #0, avi, from 'movie.avi':
  Duration: 00:01:00.00, start: 0.000000, bitrate: 1500 kb/s
  Stream #0:0: Video: mpeg4 (Simple Profile) (FMP4 / 0x34504D46), yuv420p, 1280x720, 25 fps
  Stream #0:1: Audio: mp3 (U[0][0][0] / 0x0055), 44100 Hz, stereo, fltp, 128 kb/s
";

    const PROBE_MP4_H264: &str = "\
Input #0, mov,mp4,m4a,3gp,3g2,mj2, from 'movie.mp4':
  Duration: 00:01:00.00, start: 0.000000, bitrate: 1500 kb/s
  Stream #0:0[0x1](und): Video: h264 (High) (avc1 / 0x31637661), yuv420p, 1280x720, 25 fps
  Stream #0:1[0x2](und): Audio: aac (LC) (mp4a / 0x6134706D), 44100 Hz, stereo, fltp, 128 kb/s
";

    const PROBE_MKV_H265: &str = "\
Input #0, matroska,webm, from 'movie.mkv':
  Duration: 00:01:00.00, start: 0.000000, bitrate: 1500 kb/s
  Stream #0:0: Video: hevc (Main), yuv420p(tv), 1920x1080, 25 fps
  Stream #0:1: Audio: ac3, 48000 Hz, 5.1(side), fltp, 384 kb/s
";

    #[test]
    fn parses_avi_probe_output() {
        let info = parse_probe_output(PROBE_AVI);

        assert_eq!(info.format, "avi");
        assert_eq!(info.video_codec.as_deref(), Some("mpeg4"));
        assert_eq!(info.audio_codec.as_deref(), Some("mp3"));
        assert!(!info.needs_no_reencode());
        assert!(!info.video_is_h264());
        assert!(!info.audio_is_aac());
    }

    #[test]
    fn parses_mp4_probe_output() {
        let info = parse_probe_output(PROBE_MP4_H264);

        assert!(info.is_mp4());
        assert_eq!(info.format, "mov,mp4,m4a,3gp,3g2,mj2");
        assert_eq!(info.video_codec.as_deref(), Some("h264"));
        assert_eq!(info.audio_codec.as_deref(), Some("aac"));
        assert!(info.needs_no_reencode());
    }

    #[test]
    fn parses_mkv_probe_output() {
        let info = parse_probe_output(PROBE_MKV_H265);

        assert_eq!(info.format, "matroska,webm");
        assert!(!info.is_mp4());
        assert_eq!(info.video_codec.as_deref(), Some("hevc"));
        assert_eq!(info.audio_codec.as_deref(), Some("ac3"));
        assert!(!info.needs_no_reencode());
    }

    #[test]
    fn detects_partial_reencode() {
        // 视频已经是 h264，但音频不是
        let text = "\
Input #0, mp4, from 'a.mp4':
  Stream #0:0: Video: h264 (High), yuv420p, 1280x720
  Stream #0:1: Audio: mp3, 44100 Hz, stereo
";
        let info = parse_probe_output(text);

        assert!(info.video_is_h264());
        assert!(!info.audio_is_aac());
        assert!(!info.needs_no_reencode());
    }

    #[test]
    fn handles_missing_audio_stream() {
        let text = "\
Input #0, flv, from 'a.flv':
  Stream #0:0: Video: flv1, yuv420p, 640x480
";
        let info = parse_probe_output(text);

        assert_eq!(info.format, "flv");
        assert_eq!(info.video_codec.as_deref(), Some("flv1"));
        assert_eq!(info.audio_codec, None);
    }

    #[test]
    fn derives_converted_path() {
        assert_eq!(
            converted_path(Path::new("/tmp/movie.avi")),
            Path::new("/tmp/movie.converted.mp4")
        );
        assert_eq!(
            converted_path(Path::new("/tmp/movie")),
            Path::new("/tmp/movie.converted.mp4")
        );
    }

    #[test]
    fn presets_differ() {
        let fast = TranscodeOptions::fast();
        let hq = TranscodeOptions::high_quality();

        assert_eq!(fast.preset, "ultrafast");
        assert!(fast.crf > hq.crf);
        assert!(hq.audio_bitrate.ends_with('k'));
    }
}
