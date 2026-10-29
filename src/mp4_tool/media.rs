//! 常用的媒体处理：分割、拼接、插入、生成空白片。
//!
//! 所有操作都尽量用 `-c copy` 完成，不重编码，速度快、画质无损。
//!
//! 时间参数用 ffmpeg 的时长格式，例如 `90`、`00:01:30`、`00:01:30.500`。

use anyhow::{Context, Result, bail};
use ffmpeg_sidecar::command::FfmpegCommand;
use std::path::{Path, PathBuf};

/// 媒体类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    /// 视频，输出 `.mp4`
    Video,
    /// 音频，输出 `.mp3`
    Audio,
}

impl MediaKind {
    /// 根据文件扩展名推断媒体类型。
    pub fn from_path(path: &Path) -> Option<Self> {
        match path
            .extension()
            .map(|ext| ext.to_string_lossy().to_ascii_lowercase())
            .as_deref()
        {
            Some("mp4" | "m4v" | "mov" | "mkv") => Some(MediaKind::Video),
            Some("mp3" | "m4a" | "aac" | "wav" | "flac") => Some(MediaKind::Audio),
            _ => None,
        }
    }

    /// 输出文件的扩展名。
    pub fn extension(self) -> &'static str {
        match self {
            MediaKind::Video => "mp4",
            MediaKind::Audio => "mp3",
        }
    }
}

/// 分割：从 `start` 开始截取 `duration` 秒。
///
/// 不重编码。输出写到 `<名字>.cut.<扩展名>`。
///
/// ```no_run
/// use std::path::Path;
/// use rupnp::mp4_tool::media;
///
/// # fn main() -> anyhow::Result<()> {
/// // 从 30 秒处开始，取 60 秒
/// media::split(Path::new("ffmpeg"), Path::new("in.mp4"), "00:00:30", "60")?;
/// # Ok(())
/// # }
/// ```
pub fn split(ffmpeg: &Path, input: &Path, start: &str, duration: &str) -> Result<PathBuf> {
    let output = derived_path(input, "cut", None);
    segment(ffmpeg, input, start, Some(duration), &output)?;
    Ok(output)
}

/// 拼接：把多个文件按顺序拼成一个，输出写到 `<第一个文件>.concat.<扩展名>`。
///
/// 要求所有输入是**同一种容器和编码**，否则 `-c copy` 会失败。
pub fn concat(ffmpeg: &Path, inputs: &[PathBuf]) -> Result<PathBuf> {
    if inputs.len() < 2 {
        bail!("拼接至少需要两个输入文件");
    }

    let kind = MediaKind::from_path(&inputs[0]).context("无法从第一个输入推断媒体类型")?;

    for input in inputs {
        if MediaKind::from_path(input) != Some(kind) {
            bail!("拼接要求所有输入是同一种媒体类型");
        }
    }

    let output = derived_path(&inputs[0], "concat", Some(kind));
    concat_into(ffmpeg, inputs, &output)?;

    Ok(output)
}

/// 插入：把 `insert` 接到 `input` 的第 `at` 秒之后，输出写到
/// `<输入>.inserted.<扩展名>`。
///
/// 实现是「切两段 → 拼接」，所以同样要求同一种容器和编码。
pub fn insert(ffmpeg: &Path, input: &Path, insert: &Path, at: &str) -> Result<PathBuf> {
    let kind = MediaKind::from_path(input).context("无法推断输入文件的媒体类型")?;

    if MediaKind::from_path(insert) != Some(kind) {
        bail!("插入的文件必须和输入是同一种媒体类型");
    }

    let head = temp_output(input, "head", kind);
    let tail = temp_output(input, "tail", kind);
    let output = derived_path(input, "inserted", Some(kind));

    // 前半段 0..at，后半段 at..结尾
    segment(ffmpeg, input, "0", Some(at), &head)?;

    let tail_result = segment(ffmpeg, input, at, None, &tail);

    let result = tail_result.and_then(|()| {
        concat_into(
            ffmpeg,
            &[head.clone(), insert.to_path_buf(), tail.clone()],
            &output,
        )
    });

    // 清理中间产物，不管成功与否
    let _ = std::fs::remove_file(&head);
    let _ = std::fs::remove_file(&tail);

    result?;

    Ok(output)
}

/// 生成一段黑屏视频或静音音频，时长 `duration` 秒。
///
/// 视频是 1280x720 的纯黑画面 + 静音音轨，音频则是纯静音。
/// 输出写到系统临时目录。
///
/// ```no_run
/// use std::path::Path;
/// use rupnp::mp4_tool::media::{self, MediaKind};
///
/// # fn main() -> anyhow::Result<()> {
/// media::blank(Path::new("ffmpeg"), MediaKind::Video, "5")?;
/// # Ok(())
/// # }
/// ```
pub fn blank(ffmpeg: &Path, kind: MediaKind, duration: &str) -> Result<PathBuf> {
    let output = std::env::temp_dir().join(format!(
        "rupnp-blank-{}.{}",
        unique_suffix(),
        kind.extension()
    ));

    let mut command = command(ffmpeg);

    match kind {
        MediaKind::Video => {
            command
                .arg("-f")
                .arg("lavfi")
                .arg("-i")
                .arg("color=c=black:s=1280x720:r=25")
                .arg("-f")
                .arg("lavfi")
                .arg("-i")
                .arg("anullsrc=channel_layout=stereo:sample_rate=44100")
                .arg("-t")
                .arg(duration)
                .arg("-c:v")
                .arg("libx264")
                .arg("-pix_fmt")
                .arg("yuv420p")
                .arg("-c:a")
                .arg("aac")
                .arg("-shortest")
                .arg(&output);
        }
        MediaKind::Audio => {
            command
                .arg("-f")
                .arg("lavfi")
                .arg("-i")
                .arg("anullsrc=channel_layout=stereo:sample_rate=44100")
                .arg("-t")
                .arg(duration)
                .arg("-c:a")
                .arg("libmp3lame")
                .arg("-b:a")
                .arg("128k")
                .arg(&output);
        }
    }

    run(command, "生成空白片")?;

    Ok(output)
}

/// 截取一段。`duration` 为 `None` 表示截到结尾。
fn segment(
    ffmpeg: &Path,
    input: &Path,
    start: &str,
    duration: Option<&str>,
    output: &Path,
) -> Result<()> {
    let mut command = command(ffmpeg);

    command.arg("-ss").arg(start).arg("-i").arg(input);

    if let Some(duration) = duration {
        command.arg("-t").arg(duration);
    }

    command
        .arg("-c")
        .arg("copy")
        .arg("-map")
        .arg("0")
        .arg(output);

    run(command, "截取")
}

/// 用 concat demuxer 把多个文件拼到 `output`。
fn concat_into(ffmpeg: &Path, inputs: &[PathBuf], output: &Path) -> Result<()> {
    let list = concat_list(inputs)?;

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

/// 生成 concat demuxer 需要的清单文件。
fn concat_list(inputs: &[PathBuf]) -> Result<PathBuf> {
    let path = std::env::temp_dir().join(format!("rupnp-concat-{}.txt", unique_suffix()));

    let mut contents = String::new();

    for input in inputs {
        let absolute = std::fs::canonicalize(input)
            .with_context(|| format!("无法解析路径 {}", input.display()))?;

        // concat demuxer 用正斜杠，单引号要转义
        let escaped = absolute
            .to_string_lossy()
            .replace('\\', "/")
            .replace('\'', "'\\''");

        contents.push_str(&format!("file '{escaped}'\n"));
    }

    std::fs::write(&path, contents)
        .with_context(|| format!("无法写入拼接清单 {}", path.display()))?;

    Ok(path)
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

/// 跑完命令，失败时把 ffmpeg 的报错打出来。
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

/// 根据输入文件派生输出路径，和目标文件放在同一目录。
fn derived_path(base: &Path, label: &str, kind: Option<MediaKind>) -> PathBuf {
    let stem = base
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "output".to_string());

    let extension = kind
        .map(MediaKind::extension)
        .or_else(|| base.extension().and_then(|ext| ext.to_str()))
        .unwrap_or("mp4");

    let directory = base.parent().unwrap_or_else(|| Path::new("."));

    directory.join(format!("{stem}.{label}.{extension}"))
}

/// 中间产物放在系统临时目录，避免污染输入文件所在目录。
fn temp_output(base: &Path, label: &str, kind: MediaKind) -> PathBuf {
    let stem = base
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "output".to_string());

    std::env::temp_dir().join(format!(
        "rupnp-{stem}-{label}-{}.{}",
        unique_suffix(),
        kind.extension()
    ))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn infers_media_kind_from_extension() {
        assert_eq!(
            MediaKind::from_path(Path::new("a.mp4")),
            Some(MediaKind::Video)
        );
        assert_eq!(
            MediaKind::from_path(Path::new("a.MP4")),
            Some(MediaKind::Video)
        );
        assert_eq!(
            MediaKind::from_path(Path::new("a.mov")),
            Some(MediaKind::Video)
        );
        assert_eq!(
            MediaKind::from_path(Path::new("a.mp3")),
            Some(MediaKind::Audio)
        );
        assert_eq!(
            MediaKind::from_path(Path::new("a.m4a")),
            Some(MediaKind::Audio)
        );
        assert_eq!(MediaKind::from_path(Path::new("a.txt")), None);
        assert_eq!(MediaKind::from_path(Path::new("a")), None);
    }

    #[test]
    fn derives_output_path_next_to_the_input() {
        let output = derived_path(Path::new("/tmp/movie.mp4"), "cut", None);
        assert_eq!(output, Path::new("/tmp/movie.cut.mp4"));

        let output = derived_path(Path::new("/tmp/song.mp3"), "concat", Some(MediaKind::Audio));
        assert_eq!(output, Path::new("/tmp/song.concat.mp3"));
    }

    #[test]
    fn concat_requires_at_least_two_inputs() {
        let result = concat(Path::new("ffmpeg"), &[PathBuf::from("only-one.mp4")]);
        assert!(result.is_err());
    }

    #[test]
    fn concat_rejects_mixed_media_kinds() {
        let result = concat(
            Path::new("ffmpeg"),
            &[PathBuf::from("a.mp4"), PathBuf::from("b.mp3")],
        );
        assert!(result.is_err());
    }

    #[test]
    fn insert_rejects_mismatched_kind() {
        let result = insert(
            Path::new("ffmpeg"),
            Path::new("a.mp4"),
            Path::new("b.mp3"),
            "10",
        );
        assert!(result.is_err());
    }
}
