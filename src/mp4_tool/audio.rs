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
}
