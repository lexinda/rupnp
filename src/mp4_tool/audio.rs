//! 音频提取与转换。
//!
//! 两个常用操作：
//!
//! - [`extract_mp3`]：从视频里把音轨抽出来存成 MP3
//! - [`to_mp3`]：把任意音频（比如 WAV）转成 MP3
//!
//! 两者都不碰视频流，只处理音频，所以比整体转码快得多。如果源音轨本身就是
//! MP3、只是换个容器，会直接 `-c copy`。

use anyhow::{Context, Result, bail};
use ffmpeg_sidecar::command::FfmpegCommand;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// MP3 编码参数。
#[derive(Debug, Clone)]
pub struct Mp3Options {
    /// 码率，例如 `128k` / `192k` / `320k`。默认 `192k`。
    pub bitrate: String,
    /// 采样率，例如 `44100`。为 `None` 时保持原样。
    pub sample_rate: Option<u32>,
    /// 声道数，`1` 为单声道、`2` 为立体声。为 `None` 时保持原样。
    pub channels: Option<u16>,
}

impl Default for Mp3Options {
    fn default() -> Self {
        Self {
            bitrate: "192k".to_string(),
            sample_rate: None,
            channels: None,
        }
    }
}

impl Mp3Options {
    /// 高码率：320k，接近无损。
    pub fn high() -> Self {
        Self {
            bitrate: "320k".to_string(),
            ..Self::default()
        }
    }

    /// 低码率：适合语音、播客、有声书。
    pub fn voice() -> Self {
        Self {
            bitrate: "64k".to_string(),
            sample_rate: Some(44_100),
            channels: Some(1),
        }
    }
}

/// 从视频里提取音轨，存成 MP3。
///
/// `output` 为 `None` 时写到视频同目录的 `<名字>.mp3`。
///
/// ```no_run
/// use std::path::Path;
/// use rupnp::mp4_tool::audio::{self, Mp3Options};
///
/// # fn main() -> anyhow::Result<()> {
/// let ffmpeg = Path::new("ffmpeg");
/// let mp3 = audio::extract_mp3(ffmpeg, Path::new("movie.mp4"), None, &Mp3Options::default())?;
/// println!("{}", mp3.display());
/// # Ok(())
/// # }
/// ```
pub fn extract_mp3(
    ffmpeg: &Path,
    input: &Path,
    output: Option<&Path>,
    options: &Mp3Options,
) -> Result<PathBuf> {
    let output = match output {
        Some(output) => output.to_path_buf(),
        None => derived_mp3_path(input),
    };

    // 没有音轨的文件，ffmpeg 会直接报错；提前给出更清楚的提示
    if !has_audio_stream(ffmpeg, input)? {
        bail!("{} 里没有音轨", input.display());
    }

    encode_mp3(ffmpeg, input, &output, options, true)?;

    Ok(output)
}

/// 把任意音频转成 MP3，比如 WAV → MP3。
///
/// `output` 为 `None` 时写到同目录的 `<名字>.mp3`。
///
/// ```no_run
/// use std::path::Path;
/// use rupnp::mp4_tool::audio::{self, Mp3Options};
///
/// # fn main() -> anyhow::Result<()> {
/// let ffmpeg = Path::new("ffmpeg");
/// let mp3 = audio::to_mp3(ffmpeg, Path::new("song.wav"), None, &Mp3Options::default())?;
/// println!("{}", mp3.display());
/// # Ok(())
/// # }
/// ```
pub fn to_mp3(
    ffmpeg: &Path,
    input: &Path,
    output: Option<&Path>,
    options: &Mp3Options,
) -> Result<PathBuf> {
    let output = match output {
        Some(output) => output.to_path_buf(),
        None => derived_mp3_path(input),
    };

    encode_mp3(ffmpeg, input, &output, options, false)?;

    Ok(output)
}

/// 批量把音频转成 MP3。单个失败不影响其它，失败项以 `Err` 返回。
pub fn to_mp3_batch(
    ffmpeg: &Path,
    inputs: &[PathBuf],
    options: &Mp3Options,
) -> Vec<Result<PathBuf>> {
    inputs
        .iter()
        .map(|input| {
            eprintln!("转换 {} -> MP3", input.display());

            to_mp3(ffmpeg, input, None, options)
        })
        .collect()
}

/// 调 ffmpeg 编码 MP3。
///
/// `video_input` 为 true 时从视频里取音轨（`-vn` 丢掉视频），否则按纯音频处理。
fn encode_mp3(
    ffmpeg: &Path,
    input: &Path,
    output: &Path,
    options: &Mp3Options,
    video_input: bool,
) -> Result<()> {
    let mut command = command(ffmpeg);

    command.arg("-i").arg(input);

    if video_input {
        // 只取第一条音轨，丢掉视频
        command.arg("-map").arg("0:a:0").arg("-vn");
    }

    command
        .arg("-c:a")
        .arg("libmp3lame")
        .arg("-b:a")
        .arg(&options.bitrate);

    if let Some(rate) = options.sample_rate {
        command.arg("-ar").arg(rate.to_string());
    }

    if let Some(channels) = options.channels {
        command.arg("-ac").arg(channels.to_string());
    }

    command.arg(output);

    run(command, "编码 MP3")
}

/// 判断文件里有没有音轨。
fn has_audio_stream(ffmpeg: &Path, input: &Path) -> Result<bool> {
    use std::process::{Command, Stdio};

    let mut command = Command::new(ffmpeg);

    command
        .arg("-hide_banner")
        .arg("-i")
        .arg(input)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    let output = command
        .output()
        .with_context(|| format!("无法探测 {}", input.display()))?;

    let text = String::from_utf8_lossy(&output.stderr);

    Ok(text.lines().any(|line| line.contains("Audio:")))
}

/// 构造基础命令。
fn command(ffmpeg: &Path) -> FfmpegCommand {
    let mut command = FfmpegCommand::new_with_path(ffmpeg);

    if cfg!(windows) {
        command.create_no_window();
    }

    command.arg("-y").arg("-hide_banner");

    command
}

/// 跑完命令，失败时把 ffmpeg 报错打出来。
fn run(mut command: FfmpegCommand, what: &str) -> Result<()> {
    let mut child = command
        .spawn()
        .with_context(|| format!("启动 ffmpeg 失败（{what}）"))?;

    let errors: Vec<String> = {
        let events = child.iter().context("读取 ffmpeg 输出失败")?;
        events.filter_errors().collect()
    };

    let status = child.wait().context("等待 ffmpeg 结束失败")?;

    if !status.success() {
        for error in &errors {
            eprintln!("[ffmpeg] {error}");
        }
        bail!("ffmpeg 处理失败（{what}）");
    }

    Ok(())
}

/// 淡入淡出的曲线形状。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FadeCurve {
    /// 线性，听感最"平"。
    #[default]
    Linear,
    /// 正弦，起伏更自然，适合背景音乐。
    Sine,
    /// 指数，变化更剧烈。
    Exponential,
}

impl FadeCurve {
    /// ffmpeg `afade` 的 `curve` 取值。
    fn as_ffmpeg(self) -> &'static str {
        match self {
            FadeCurve::Linear => "tri",
            FadeCurve::Sine => "qsin",
            FadeCurve::Exponential => "exp",
        }
    }
}

/// MP3 拼接的选项。
#[derive(Debug, Clone, Default)]
pub struct ConcatMp3Options {
    /// 段与段之间的静音间隔。默认没有间隔。
    pub gap: Duration,
    /// 每段首尾的淡入淡出时长。默认不淡。
    pub fade: Duration,
    /// 淡入淡出的曲线形状。
    pub curve: FadeCurve,
    /// 输出的 MP3 参数。
    pub mp3: Mp3Options,
}

impl ConcatMp3Options {
    /// 无缝拼接，不加间隔也不淡入淡出。
    pub fn seamless() -> Self {
        Self::default()
    }

    /// 带间隔和淡入淡出，像节目单那样一段段接起来。
    pub fn with_gap_and_fade(gap: Duration, fade: Duration) -> Self {
        Self {
            gap,
            fade,
            curve: FadeCurve::Sine,
            ..Self::default()
        }
    }
}

/// 音频分割的选项。
#[derive(Debug, Clone, Default)]
pub struct SplitAudioOptions {
    /// 重新编码以获得精确到帧的切点。
    ///
    /// 默认只复制（`-c copy`），速度快且对 MP3 无损。MP3 每帧都能独立解码，
    /// 所以切点本来就是准的；换成 AAC 之类有帧间依赖的格式时可能需要重编码。
    pub reencode: bool,
    /// 输出的 MP3 参数（`reencode` 时用）。
    pub mp3: Mp3Options,
}

/// 从音频里截取 `[start, end)` 这一段，存成 MP3。
///
/// `start` 和 `end` 用 ffmpeg 的时间格式，见 [`parse_time`](crate::mp4_tool::parse_time)：
/// `90`、`1:30`、`00:01:30`、`00:01:30.500`、`1h30m` 都能解析。
///
/// `output` 为 `None` 时写到同目录的 `<名字>.split.mp3`。
///
/// ```no_run
/// use std::path::Path;
/// use rupnp::mp4_tool::audio::{self, SplitAudioOptions};
///
/// # fn main() -> anyhow::Result<()> {
/// let ffmpeg = Path::new("ffmpeg");
/// // 取第 30 秒到第 90 秒
/// let out = audio::split_audio(
///     ffmpeg,
///     Path::new("song.mp3"),
///     "00:00:30",
///     "00:01:30",
///     None,
///     &SplitAudioOptions::default(),
/// )?;
/// println!("{}", out.display());
/// # Ok(())
/// # }
/// ```
pub fn split_audio(
    ffmpeg: &Path,
    input: &Path,
    start: &str,
    end: &str,
    output: Option<&Path>,
    options: &SplitAudioOptions,
) -> Result<PathBuf> {
    let start = crate::mp4_tool::parse_time(start)
        .with_context(|| format!("无法解析开始时间 `{start}`"))?;
    let end =
        crate::mp4_tool::parse_time(end).with_context(|| format!("无法解析结束时间 `{end}`"))?;

    if end <= start {
        bail!("结束时间必须晚于开始时间（{start:?} -> {end:?}）");
    }

    let duration = end - start;

    let output = match output {
        Some(output) => output.to_path_buf(),
        None => derived_split_path(input),
    };

    let mut command = command(ffmpeg);

    // -ss 放在 -i 之前是快速定位，配合 -c copy 很快
    command
        .arg("-ss")
        .arg(format!("{:.3}", start.as_secs_f64()))
        .arg("-i")
        .arg(input)
        .arg("-t")
        .arg(format!("{:.3}", duration.as_secs_f64()));

    if options.reencode {
        command
            .arg("-c:a")
            .arg("libmp3lame")
            .arg("-b:a")
            .arg(&options.mp3.bitrate);
    } else {
        command.arg("-c:a").arg("copy");
    }

    command.arg(&output);

    run(command, "分割音频")?;

    Ok(output)
}

/// 同目录下的 `<名字>.split.mp3`。
pub fn derived_split_path(input: &Path) -> PathBuf {
    let stem = input
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "output".to_string());

    let directory = input.parent().unwrap_or_else(|| Path::new("."));

    directory.join(format!("{stem}.split.mp3"))
}

/// 把多个音频拼成一个 MP3，段与段之间可以有静音间隔，每段首尾可以淡入淡出。
///
/// 实现是「每段单独处理 → 再拼接」两遍：
///
/// 1. 每段按需加上淡入淡出，编码成临时 MP3（不需要淡入淡出时直接 `-c copy`）
/// 2. 用 concat demuxer 把临时文件和间隔静音拼起来
///
/// 之所以不一趟 `filter_complex` 做完，是因为淡出的起点需要每段的时长，
/// 而且间隔静音的采样率/声道必须和音轨一致 —— 探测出来更稳妥。
///
/// ```no_run
/// use std::path::{Path, PathBuf};
/// use std::time::Duration;
/// use rupnp::mp4_tool::audio::{self, ConcatMp3Options};
///
/// # fn main() -> anyhow::Result<()> {
/// let ffmpeg = Path::new("ffmpeg");
/// let inputs = vec![PathBuf::from("a.mp3"), PathBuf::from("b.mp3")];
///
/// let options = ConcatMp3Options::with_gap_and_fade(
///     Duration::from_secs(1),  // 段间静音 1 秒
///     Duration::from_millis(500), // 每段 0.5 秒淡入淡出
/// );
///
/// audio::concat_mp3(&ffmpeg, &inputs, Path::new("out.mp3"), &options)?;
/// # Ok(())
/// # }
/// ```
pub fn concat_mp3(
    ffmpeg: &Path,
    inputs: &[PathBuf],
    output: &Path,
    options: &ConcatMp3Options,
) -> Result<()> {
    if inputs.len() < 2 {
        bail!("拼接至少需要两个输入文件");
    }

    // 探测每段的时长，以及统一的采样率/声道
    let infos = inputs
        .iter()
        .map(|input| probe(ffmpeg, input))
        .collect::<Result<Vec<_>>>()?;

    let sample_rate = infos
        .iter()
        .find_map(|info| info.sample_rate)
        .unwrap_or(44_100);
    let channels = infos
        .iter()
        .find_map(|info| info.channels.clone())
        .unwrap_or_else(|| "stereo".to_string());

    let mut parts: Vec<PathBuf> = Vec::new();
    let mut cleanup: Vec<PathBuf> = Vec::new();

    let result = (|| -> Result<()> {
        for (input, info) in inputs.iter().zip(&infos) {
            let part = temp_path(input, "part");

            let filter = fade_filter(info.duration, options.fade, options.curve);

            encode_segment(
                ffmpeg,
                input,
                &part,
                filter.as_deref(),
                options,
                sample_rate,
                &channels,
            )?;

            cleanup.push(part.clone());
            parts.push(part);
        }

        // 段间静音
        if !options.gap.is_zero() {
            let gap = temp_path(output, "gap");

            generate_silence(
                ffmpeg,
                &gap,
                options.gap,
                sample_rate,
                &channels,
                &options.mp3,
            )?;

            cleanup.push(gap.clone());

            // 静音要插在每一对片段之间
            let mut with_gaps = Vec::with_capacity(parts.len() * 2);
            for (index, part) in parts.iter().enumerate() {
                if index > 0 {
                    with_gaps.push(gap.clone());
                }
                with_gaps.push(part.clone());
            }
            parts = with_gaps;
        }

        concat_files(ffmpeg, &parts, output)
    })();

    for path in cleanup {
        let _ = std::fs::remove_file(path);
    }

    result
}

/// 生成一段静音。
fn generate_silence(
    ffmpeg: &Path,
    output: &Path,
    duration: Duration,
    sample_rate: u32,
    channels: &str,
    mp3: &Mp3Options,
) -> Result<()> {
    let mut command = command(ffmpeg);

    command
        .arg("-f")
        .arg("lavfi")
        .arg("-i")
        .arg(format!("anullsrc=r={sample_rate}:cl={channels}"))
        .arg("-t")
        .arg(format!("{:.3}", duration.as_secs_f64()))
        .arg("-c:a")
        .arg("libmp3lame")
        .arg("-b:a")
        .arg(&mp3.bitrate)
        .arg(output);

    run(command, "生成静音")
}

/// 处理一段：带淡入淡出时重编码，否则直接复制。
fn encode_segment(
    ffmpeg: &Path,
    input: &Path,
    output: &Path,
    filter: Option<&str>,
    options: &ConcatMp3Options,
    sample_rate: u32,
    channels: &str,
) -> Result<()> {
    let mut command = command(ffmpeg);

    command.arg("-i").arg(input);

    match filter {
        Some(filter) => {
            command
                .arg("-filter:a")
                .arg(filter)
                .arg("-c:a")
                .arg("libmp3lame")
                .arg("-b:a")
                .arg(&options.mp3.bitrate);
        }
        // 不需要重采样就不动它，保持原样最省事
        None => {
            command.arg("-c:a").arg("copy");
        }
    }

    command.arg("-ar").arg(sample_rate.to_string());
    command.arg("-ac").arg(channels_arg(channels));
    command.arg(output);

    run(command, "处理片段")
}

/// 把声道布局转成 ffmpeg 的 `-ac` 参数。
fn channels_arg(layout: &str) -> String {
    match layout {
        "mono" => "1".to_string(),
        "5.1" | "5.1(side)" => "6".to_string(),
        "7.1" | "7.1(side)" => "8".to_string(),
        _ => "2".to_string(),
    }
}

/// 构造淡入淡出的滤镜链。
///
/// 淡入淡出会被限制在片段时长的一半以内，否则整段会变成静音。
fn fade_filter(duration: Option<Duration>, fade: Duration, curve: FadeCurve) -> Option<String> {
    if fade.is_zero() {
        return None;
    }

    let duration = duration?;
    let fade = fade.as_secs_f64().min(duration.as_secs_f64() / 2.0);

    if fade <= 0.0 {
        return None;
    }

    let fade_out_start = (duration.as_secs_f64() - fade).max(0.0);
    let curve = curve.as_ffmpeg();

    Some(format!(
        "afade=t=in:st=0:d={fade:.3}:curve={curve},\
         afade=t=out:st={fade_out_start:.3}:d={fade:.3}:curve={curve}"
    ))
}

/// 用 concat demuxer 把若干 MP3 拼起来。
fn concat_files(ffmpeg: &Path, parts: &[PathBuf], output: &Path) -> Result<()> {
    let list = std::env::temp_dir().join(format!("rupnp-concat-{}.txt", unique_suffix()));

    let mut contents = String::new();
    for part in parts {
        let absolute = std::fs::canonicalize(part)
            .with_context(|| format!("无法解析路径 {}", part.display()))?;
        let escaped = absolute
            .to_string_lossy()
            .replace('\\', "/")
            .replace('\'', "'\\''");
        contents.push_str(&format!("file '{escaped}'\n"));
    }

    std::fs::write(&list, contents)
        .with_context(|| format!("无法写入拼接清单 {}", list.display()))?;

    let mut command = command(ffmpeg);
    command
        .arg("-f")
        .arg("concat")
        .arg("-safe")
        .arg("0")
        .arg("-i")
        .arg(&list)
        .arg("-c")
        .arg("copy")
        .arg(output);

    let result = run(command, "拼接");

    let _ = std::fs::remove_file(&list);

    result
}

/// 中间产物路径。
fn temp_path(base: &Path, label: &str) -> PathBuf {
    let stem = base
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "output".to_string());

    std::env::temp_dir().join(format!("rupnp-{stem}-{label}-{}.mp3", unique_suffix()))
}

/// 一个够用的唯一后缀，避免并发时撞名。
fn unique_suffix() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or_default();

    format!("{}-{nanos}", std::process::id())
}

/// 探测媒体信息。
fn probe(ffmpeg: &Path, input: &Path) -> Result<crate::mp4_tool::MediaInfo> {
    crate::mp4_tool::probe(ffmpeg, input)
}

/// 同目录下的 `<名字>.mp3`。
pub fn derived_mp3_path(input: &Path) -> PathBuf {
    let stem = input
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "output".to_string());

    let directory = input.parent().unwrap_or_else(|| Path::new("."));

    directory.join(format!("{stem}.mp3"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_mp3_path_next_to_input() {
        assert_eq!(
            derived_mp3_path(Path::new("/tmp/movie.mp4")),
            Path::new("/tmp/movie.mp3")
        );
        assert_eq!(
            derived_mp3_path(Path::new("/tmp/song.wav")),
            Path::new("/tmp/song.mp3")
        );
        assert_eq!(
            derived_mp3_path(Path::new("/tmp/noext")),
            Path::new("/tmp/noext.mp3")
        );
    }

    #[test]
    fn presets_have_expected_bitrates() {
        assert_eq!(Mp3Options::default().bitrate, "192k");
        assert_eq!(Mp3Options::high().bitrate, "320k");

        let voice = Mp3Options::voice();
        assert_eq!(voice.bitrate, "64k");
        assert_eq!(voice.channels, Some(1));
        assert_eq!(voice.sample_rate, Some(44_100));
    }

    #[test]
    fn batch_returns_one_result_per_input() {
        // 不真正调用 ffmpeg：用一个不存在的可执行文件，只会得到错误
        let results = to_mp3_batch(
            Path::new("definitely-not-ffmpeg"),
            &[PathBuf::from("/tmp/a.wav"), PathBuf::from("/tmp/b.wav")],
            &Mp3Options::default(),
        );

        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|result| result.is_err()));
    }

    #[test]
    fn fade_filter_is_built_from_duration_and_fade() {
        let duration = Some(Duration::from_secs(10));
        let filter = fade_filter(duration, Duration::from_millis(500), FadeCurve::Sine)
            .expect("应当生成滤镜");

        assert!(filter.contains("afade=t=in:st=0:d=0.500:curve=qsin"));
        assert!(filter.contains("afade=t=out:st=9.500:d=0.500:curve=qsin"));
    }

    #[test]
    fn no_fade_means_no_filter() {
        assert!(
            fade_filter(
                Some(Duration::from_secs(10)),
                Duration::ZERO,
                FadeCurve::Linear
            )
            .is_none()
        );
    }

    #[test]
    fn fade_is_clamped_to_half_the_duration() {
        // 片段只有 1 秒，淡入淡出各 2 秒 -> 会被压到 0.5 秒
        let filter = fade_filter(
            Some(Duration::from_secs(1)),
            Duration::from_secs(2),
            FadeCurve::Linear,
        )
        .expect("应当生成滤镜");

        assert!(filter.contains("d=0.500"));
        assert!(filter.contains("st=0.500"));
    }

    #[test]
    fn fade_needs_a_known_duration() {
        // 探测不到时长就不敢做淡出
        assert!(fade_filter(None, Duration::from_secs(1), FadeCurve::Linear).is_none());
    }

    #[test]
    fn curve_maps_to_ffmpeg_names() {
        assert_eq!(FadeCurve::Linear.as_ffmpeg(), "tri");
        assert_eq!(FadeCurve::Sine.as_ffmpeg(), "qsin");
        assert_eq!(FadeCurve::Exponential.as_ffmpeg(), "exp");
    }

    #[test]
    fn channels_map_to_ffmpeg_counts() {
        assert_eq!(channels_arg("mono"), "1");
        assert_eq!(channels_arg("stereo"), "2");
        assert_eq!(channels_arg("5.1(side)"), "6");
        assert_eq!(channels_arg("7.1"), "8");
    }

    #[test]
    fn concat_presets() {
        let seamless = ConcatMp3Options::seamless();
        assert!(seamless.gap.is_zero());
        assert!(seamless.fade.is_zero());

        let styled =
            ConcatMp3Options::with_gap_and_fade(Duration::from_secs(1), Duration::from_millis(500));
        assert_eq!(styled.gap, Duration::from_secs(1));
        assert_eq!(styled.fade, Duration::from_millis(500));
        assert_eq!(styled.curve, FadeCurve::Sine);
    }

    #[test]
    fn concat_requires_two_inputs() {
        let result = concat_mp3(
            Path::new("definitely-not-ffmpeg"),
            &[PathBuf::from("only-one.mp3")],
            Path::new("out.mp3"),
            &ConcatMp3Options::default(),
        );
        assert!(result.is_err());
    }

    #[test]
    fn derives_split_path_next_to_input() {
        assert_eq!(
            derived_split_path(Path::new("/tmp/song.mp3")),
            Path::new("/tmp/song.split.mp3")
        );
    }

    #[test]
    fn split_rejects_end_before_start() {
        let result = split_audio(
            Path::new("definitely-not-ffmpeg"),
            Path::new("/tmp/song.mp3"),
            "00:01:00",
            "00:00:30",
            None,
            &SplitAudioOptions::default(),
        );
        assert!(result.is_err());
    }

    #[test]
    fn split_rejects_invalid_times() {
        for (start, end) in [("abc", "00:01:00"), ("00:00:30", "xyz")] {
            let result = split_audio(
                Path::new("definitely-not-ffmpeg"),
                Path::new("/tmp/song.mp3"),
                start,
                end,
                None,
                &SplitAudioOptions::default(),
            );
            assert!(result.is_err(), "{start} -> {end} 应当被拒绝");
        }
    }
}
