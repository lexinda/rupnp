//! `mp4-tool` —— 检查与修复 MP4 的 atom 布局，使其适合 HTTP Range 播放。
//!
//! 渲染器通过 HTTP Range 拉流时，如果 MP4 的 `moov` atom 在 `mdat` 之后，
//! 渲染器必须把整个文件下载完才知道文件多长、编码是什么，大文件就会一直
//! 转圈。这个工具检查并修复这个问题。
//!
//! ```text
//! mp4-tool check <文件>...                  检查布局，不需要 ffmpeg
//! mp4-tool fix   [--ffmpeg-url <地址>] <文件>...
//! ```

mod mp4;

use anyhow::{bail, Context, Result};
use ffmpeg_sidecar::command::FfmpegCommand;
use ffmpeg_sidecar::download::FfmpegDownloadProgressEvent;
use std::cell::Cell;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

/// 环境变量名，用它可以指定 ffmpeg 的下载地址（镜像）。
const FFMPEG_URL_ENV: &str = "FFMPEG_SIDECAR_URL";

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(err) => {
            eprintln!("错误: {err:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<ExitCode> {
    let args: Vec<String> = std::env::args().skip(1).collect();

    let command = args.first().map(String::as_str).unwrap_or_default();

    match command {
        "check" => {
            let options = parse_args(&args[1..])?;
            check(&options.files)
        }
        "fix" => {
            let options = parse_args(&args[1..])?;
            fix(&options)
        }
        "" | "-h" | "--help" | "help" => {
            print_help();
            Ok(ExitCode::SUCCESS)
        }
        other => bail!("未知子命令 `{other}`，用 --help 看用法"),
    }
}

/// 命令行选项。
struct Options {
    /// ffmpeg 的下载地址，覆盖内置地址。
    ffmpeg_url: Option<String>,
    /// 要处理的文件。
    files: Vec<PathBuf>,
}

fn parse_args(args: &[String]) -> Result<Options> {
    let mut ffmpeg_url = std::env::var(FFMPEG_URL_ENV)
        .ok()
        .filter(|url| !url.trim().is_empty());
    let mut files = Vec::new();

    let mut index = 0;
    while index < args.len() {
        let arg = args[index].as_str();

        if arg == "--ffmpeg-url" {
            index += 1;
            let url = args
                .get(index)
                .context("--ffmpeg-url 后面需要一个下载地址")?;
            ffmpeg_url = Some(url.clone());
        } else if let Some(url) = arg.strip_prefix("--ffmpeg-url=") {
            ffmpeg_url = Some(url.to_string());
        } else {
            files.push(PathBuf::from(arg));
        }

        index += 1;
    }

    Ok(Options { ffmpeg_url, files })
}

fn print_help() {
    println!(
        "\
mp4-tool —— 检查与修复 MP4 的 atom 布局

用法:
    mp4-tool check <文件>...                    检查 moov 是否在 mdat 之前
    mp4-tool fix   [--ffmpeg-url <地址>] <文件>...
                                              重新封装，把 moov 移到文件头

选项:
    --ffmpeg-url <地址>   从指定地址下载 ffmpeg
                          也可以用环境变量 {FFMPEG_URL_ENV} 设置

说明:
    fix 会把 ffmpeg 需要的元数据（moov）搬到文件开头，这样渲染器通过
    HTTP Range 拉流时可以立刻开始播放，而不必下载整个文件。
    如果本机没有 ffmpeg，会自动下载一个 —— 下载走 Rust 的 HTTP 客户端，
    不依赖系统里有没有 curl。

    国内网络直连境外下载源可能很慢（甚至卡住），这时可以用 --ffmpeg-url
    指向镜像，例如 GitHub 加速代理：
        mp4-tool fix --ffmpeg-url \\
          https://gh-proxy.com/https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl.zip \\
          movie.mp4

退出码:
    0  全部文件都正常
    1  有文件需要修复
    2  用法错误
"
    );
}

/// 检查文件的 atom 布局。
fn check(files: &[PathBuf]) -> Result<ExitCode> {
    if files.is_empty() {
        bail!("请至少指定一个文件");
    }

    let mut all_fast_start = true;

    for path in files {
        let atoms = mp4::top_level_atoms(path)?;
        let layout = mp4::layout(&atoms);

        println!("=== {} ===", path.display());
        println!("{:>12}  {:<6} {:>14}", "偏移", "atom", "大小");
        println!("{}", "-".repeat(40));

        for atom in &atoms {
            println!("{:>12}  {:<6} {:>14}", atom.offset, atom.kind, atom.size);
        }

        println!();

        match layout {
            mp4::Layout::FastStart => {
                println!("结论: moov 在 mdat 之前 —— fast start 已优化，可以秒开");
            }
            mp4::Layout::MoovAtEnd => {
                println!("结论: moov 在 mdat 之后 —— 渲染器必须下完整个文件才能播！");
                println!("      修复: mp4-tool fix {}", path.display());
                all_fast_start = false;
            }
            mp4::Layout::Unknown => {
                println!("结论: 找不到 moov/mdat，这不是标准 MP4");
            }
        }

        println!();
    }

    Ok(if all_fast_start {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

/// 重新封装文件，把 moov 移到文件头。
fn fix(options: &Options) -> Result<ExitCode> {
    if options.files.is_empty() {
        bail!("请至少指定一个文件");
    }

    ensure_ffmpeg(options.ffmpeg_url.as_deref())?;

    for path in &options.files {
        let output = faststart_path(path);

        eprintln!("{} -> {}", path.display(), output.display());

        remux(path, &output)?;

        // 校验结果
        let atoms = mp4::top_level_atoms(&output)?;
        if !mp4::layout(&atoms).is_fast_start() {
            bail!("{} 修复后布局仍不正确", output.display());
        }

        println!("{} 已修复", output.display());
    }

    Ok(ExitCode::SUCCESS)
}

/// 准备 ffmpeg。
///
/// 本机没有的话会自动下载 —— 下载走 Rust 的 HTTP 客户端，不依赖系统里有没有
/// curl。下载过程会显示进度，否则上百兆的下载看起来就像卡住了。
fn ensure_ffmpeg(url_override: Option<&str>) -> Result<()> {
    match url_override {
        Some(url) => download_ffmpeg_from(url),
        None => {
            eprintln!("准备 ffmpeg（本机没有的话会自动下载）...");

            let last_printed = Cell::new(0u64);
            let started = Instant::now();

            ffmpeg_sidecar::download::auto_download_with_progress(|event| {
                report_progress(&last_printed, started, event)
            })
            .context("准备 ffmpeg 失败")
        }
    }
}

/// 从指定地址下载并安装 ffmpeg。
fn download_ffmpeg_from(url: &str) -> Result<()> {
    eprintln!("准备 ffmpeg（从 {url} 下载）...");

    let directory = ffmpeg_sidecar::paths::sidecar_dir()?;

    let last_printed = Cell::new(0u64);
    let started = Instant::now();

    let archive =
        ffmpeg_sidecar::download::download_ffmpeg_package_with_progress(url, &directory, |event| {
            report_progress(&last_printed, started, event)
        })
        .context("下载 ffmpeg 失败")?;

    eprintln!("\r  解压中...                          ");
    let _ = std::io::stderr().flush();

    ffmpeg_sidecar::download::unpack_ffmpeg_without_extras(&archive, &directory)
        .context("解压 ffmpeg 失败")?;

    eprintln!("\r  ffmpeg 准备完成                   ");
    let _ = std::io::stderr().flush();

    Ok(())
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
            let previous = last_printed.get();

            if downloaded_bytes.saturating_sub(previous) < step && downloaded_bytes != total_bytes {
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
                "\r  下载中 {percent:5.1}%  {downloaded_bytes}/{total_bytes} 字节  {speed:.0} KB/s    "
            );
            let _ = std::io::stderr().flush();
        }

        FfmpegDownloadProgressEvent::UnpackingArchive => {
            eprintln!("\r  解压中...                          ");
            let _ = std::io::stderr().flush();
        }

        FfmpegDownloadProgressEvent::Done => {
            eprintln!("\r  ffmpeg 准备完成                   ");
            let _ = std::io::stderr().flush();
        }
    }
}

/// 用 ffmpeg 重新封装，`-c copy` 不重编码，只把 moov 搬到文件头。
fn remux(input: &Path, output: &Path) -> Result<()> {
    let mut command = FfmpegCommand::new();
    command
        .arg("-y")
        .arg("-i")
        .arg(input)
        .arg("-c")
        .arg("copy")
        .arg("-movflags")
        .arg("+faststart")
        .arg(output);

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

/// 修复后的输出路径：`<名字>.faststart.mp4`。
fn faststart_path(input: &Path) -> PathBuf {
    let stem = input
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "output".to_string());

    input.with_file_name(format!("{stem}.faststart.mp4"))
}
