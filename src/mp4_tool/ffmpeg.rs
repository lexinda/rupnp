//! 准备与调用 ffmpeg。
//!
//! ffmpeg 用来把 MP4 的 `moov` atom 搬到文件头（fast start），让渲染器通过
//! HTTP Range 拉流时可以立刻开始播放。
//!
//! ffmpeg 可以是：
//!
//! - 调用方已经有的可执行文件 —— 指定路径即可，不会下载
//! - 没有指定 —— **创建下载目录并下载**一个，下载失败会重试，每次都打日志
//!
//! 下载走 Rust 的 HTTP 客户端（`ffmpeg-sidecar` 用的 `ureq`），不依赖系统里
//! 有没有 `curl`。

use anyhow::{Context, Result, bail};
use ffmpeg_sidecar::command::FfmpegCommand;
use ffmpeg_sidecar::download::FfmpegDownloadProgressEvent;
use std::cell::Cell;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// 下载失败时的重试次数。
const DOWNLOAD_ATTEMPTS: u32 = 3;

/// 两次重试之间的等待时间，每次翻倍。
const RETRY_DELAY: Duration = Duration::from_secs(5);

/// ffmpeg 的位置与下载选项。
#[derive(Debug, Clone, Default)]
pub struct FfmpegOptions {
    /// 已有的 ffmpeg 可执行文件路径。
    ///
    /// 指定了就直接用它，**不会下载**。路径不存在时报错。
    pub executable: Option<PathBuf>,
    /// ffmpeg 的下载地址。
    ///
    /// 为空时用 `ffmpeg-sidecar` 内置的地址。
    pub url: Option<String>,
    /// ffmpeg 下载到哪个目录。
    ///
    /// 为空时用 `ffmpeg-sidecar` 的 sidecar 目录。
    pub directory: Option<PathBuf>,
}

/// 准备 ffmpeg，返回可执行文件的路径。
///
/// - 指定了 `executable` 就直接用它
/// - 否则**创建下载目录**，目录里已经有 ffmpeg 就直接用，没有就下载
pub fn ensure_ffmpeg(options: &FfmpegOptions) -> Result<PathBuf> {
    if let Some(executable) = &options.executable {
        if executable.is_file() {
            eprintln!("使用指定的 ffmpeg: {}", executable.display());
            return Ok(executable.clone());
        }

        bail!("指定的 ffmpeg 不存在: {}", executable.display());
    }

    let directory = match &options.directory {
        Some(directory) => directory.clone(),
        None => ffmpeg_sidecar::paths::sidecar_dir()?,
    };

    // 下载需要目录存在
    std::fs::create_dir_all(&directory)
        .with_context(|| format!("无法创建目录 {}", directory.display()))?;

    let binary = directory.join(binary_name());

    if binary.is_file() {
        eprintln!("使用已有的 ffmpeg: {}", binary.display());
        return Ok(binary);
    }

    let url = match &options.url {
        Some(url) => url.clone(),
        None => ffmpeg_sidecar::download::ffmpeg_download_url()?.to_string(),
    };

    download_with_retry(&url, &directory)?;

    if !binary.is_file() {
        bail!("ffmpeg 解压后仍然找不到: {}", binary.display());
    }

    Ok(binary)
}

/// 下载 ffmpeg，失败会重试，每次尝试都打日志。
fn download_with_retry(url: &str, directory: &Path) -> Result<()> {
    let mut delay = RETRY_DELAY;

    for attempt in 1..=DOWNLOAD_ATTEMPTS {
        eprintln!("  第 {attempt}/{DOWNLOAD_ATTEMPTS} 次尝试下载 ffmpeg");
        eprintln!("    地址: {url}");
        eprintln!("    目录: {}", directory.display());

        let started = Instant::now();
        let last_printed = Cell::new(0u64);

        let result = ffmpeg_sidecar::download::download_ffmpeg_package_with_progress(
            url,
            directory,
            |event| report_progress(&last_printed, started, event),
        );

        match result {
            Ok(archive) => {
                eprintln!(
                    "\r    下载完成，用时 {:.0} 秒     ",
                    started.elapsed().as_secs_f64()
                );
                eprintln!("    解压中...");

                ffmpeg_sidecar::download::unpack_ffmpeg_without_extras(&archive, directory)
                    .context("解压 ffmpeg 失败")?;

                eprintln!("    解压完成");
                return Ok(());
            }
            Err(err) => {
                // 擦掉进度行，免得和后面的日志挤在一起
                eprintln!("\r{:80}", "");
                eprintln!("    下载失败: {err:#}");

                if attempt < DOWNLOAD_ATTEMPTS {
                    eprintln!("    {delay:?} 后重试");
                    std::thread::sleep(delay);
                    delay *= 2;
                }
            }
        }
    }

    bail!("下载 ffmpeg 失败，已重试 {DOWNLOAD_ATTEMPTS} 次");
}

/// 打印下载进度。每 5% 打一次，避免刷屏。
fn report_progress(last_printed: &Cell<u64>, started: Instant, event: FfmpegDownloadProgressEvent) {
    match event {
        FfmpegDownloadProgressEvent::Starting => {}

        FfmpegDownloadProgressEvent::Downloading {
            total_bytes,
            downloaded_bytes,
        } => {
            let step = (total_bytes / 20).max(1);

            if downloaded_bytes.saturating_sub(last_printed.get()) < step
                && downloaded_bytes != total_bytes
            {
                return;
            }
            last_printed.set(downloaded_bytes);

            let percent = if total_bytes > 0 {
                downloaded_bytes as f64 * 100.0 / total_bytes as f64
            } else {
                0.0
            };
            let elapsed = started.elapsed().as_secs_f64();
            let speed = if elapsed > 0.0 {
                downloaded_bytes as f64 / elapsed / 1024.0
            } else {
                0.0
            };

            eprint!(
                "\r    下载中 {percent:5.1}%  {downloaded_bytes}/{total_bytes} 字节  {speed:.0} KB/s     "
            );
            let _ = std::io::stderr().flush();
        }

        FfmpegDownloadProgressEvent::UnpackingArchive => {
            eprintln!("\r    解压中...                          ");
            let _ = std::io::stderr().flush();
        }

        FfmpegDownloadProgressEvent::Done => {
            eprintln!("\r    下载完成                           ");
            let _ = std::io::stderr().flush();
        }
    }
}

/// 用 ffmpeg 重新封装，把 `moov` 搬到文件头。
///
/// `-c copy` 表示不重编码，只是搬运元数据，**画质无损**。
pub fn remux(ffmpeg: &Path, input: &Path, output: &Path) -> Result<()> {
    let mut command = FfmpegCommand::new_with_path(ffmpeg);
    command
        .arg("-y")
        .arg("-i")
        .arg(input)
        .arg("-c")
        .arg("copy")
        .arg("-movflags")
        .arg("+faststart")
        .arg(output);

    if cfg!(windows) {
        command.create_no_window();
    }

    let mut child = command.spawn().context("启动 ffmpeg 失败")?;

    // 读事件，把 ffmpeg 的报错收集起来
    let errors: Vec<String> = {
        let events = child.iter().context("读取 ffmpeg 输出失败")?;
        events.filter_errors().collect()
    };

    let status = child.wait().context("等待 ffmpeg 结束失败")?;

    if !status.success() {
        for error in &errors {
            eprintln!("[ffmpeg] {error}");
        }
        bail!("ffmpeg 处理 {} 失败", input.display());
    }

    Ok(())
}

/// ffmpeg 可执行文件的名字。
fn binary_name() -> &'static str {
    if cfg!(windows) {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    }
}
