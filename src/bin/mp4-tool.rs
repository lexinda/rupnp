//! `mp4-tool` —— MP4 / MP3 的检查、修复与常用处理。
//!
//! ```text
//! mp4-tool check  <文件>...                             检查 moov 是否在 mdat 之前
//! mp4-tool fix    [选项] <文件>...                      重新封装，把 moov 移到文件头
//! mp4-tool split  [选项] <文件> --start <时间> --duration <时长>
//! mp4-tool concat [选项] <文件>...
//! mp4-tool insert [选项] <目标> --insert <文件> --at <时间>
//! mp4-tool blank  [选项] --kind <video|audio> --duration <时长>
//!
//! 选项:
//!     --ffmpeg <路径>      指定已有的 ffmpeg 可执行文件
//!     --ffmpeg-url <地址>  ffmpeg 的下载地址
//!     --ffmpeg-dir <目录>  ffmpeg 下载到哪个目录
//! ```

use anyhow::{Context, Result, bail};
use rupnp::mp4_tool::{self, FfmpegOptions, MediaKind};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

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

    let rest = if args.is_empty() {
        &args[..]
    } else {
        &args[1..]
    };
    let options = parse_args(rest)?;

    match command {
        "check" => check(&options.files),
        "fix" => fix(&options),
        "split" => split(&options),
        "concat" => concat(&options),
        "insert" => insert(&options),
        "blank" => blank(&options),
        "" | "-h" | "--help" | "help" => {
            print_help();
            Ok(ExitCode::SUCCESS)
        }
        other => bail!("未知子命令 `{other}`，用 --help 看用法"),
    }
}

/// 命令行选项。
#[derive(Default)]
struct Options {
    ffmpeg: FfmpegOptions,
    /// 位置参数（输入文件）。
    files: Vec<PathBuf>,
    /// `split` 的起点。
    start: Option<String>,
    /// `split` / `blank` 的时长。
    duration: Option<String>,
    /// `insert` 要插入的文件。
    insert: Option<PathBuf>,
    /// `insert` 的插入位置。
    at: Option<String>,
    /// `blank` 的媒体类型。
    kind: Option<MediaKind>,
}

fn parse_args(args: &[String]) -> Result<Options> {
    let mut options = Options::default();

    let mut index = 0;
    while index < args.len() {
        let arg = args[index].as_str();

        // 支持 --key value 和 --key=value 两种写法
        let (key, inline) = match arg.split_once('=') {
            Some((key, value)) if key.starts_with("--") => (key, Some(value.to_string())),
            _ => (arg, None),
        };

        let take_value = |index: &mut usize| -> Result<String> {
            if let Some(value) = &inline {
                return Ok(value.clone());
            }
            *index += 1;
            args.get(*index)
                .cloned()
                .with_context(|| format!("{key} 后面需要一个值"))
        };

        match key {
            "--ffmpeg" => options.ffmpeg.executable = Some(PathBuf::from(take_value(&mut index)?)),
            "--ffmpeg-url" => options.ffmpeg.url = Some(take_value(&mut index)?),
            "--ffmpeg-dir" => {
                options.ffmpeg.directory = Some(PathBuf::from(take_value(&mut index)?))
            }
            "--start" => options.start = Some(take_value(&mut index)?),
            "--duration" | "-t" => options.duration = Some(take_value(&mut index)?),
            "--insert" => options.insert = Some(PathBuf::from(take_value(&mut index)?)),
            "--at" => options.at = Some(take_value(&mut index)?),
            "--kind" => {
                let value = take_value(&mut index)?;
                options.kind = Some(match value.as_str() {
                    "video" | "mp4" => MediaKind::Video,
                    "audio" | "mp3" => MediaKind::Audio,
                    other => bail!("--kind 只能是 video 或 audio，收到 `{other}`"),
                });
            }
            other => options.files.push(PathBuf::from(other)),
        }

        index += 1;
    }

    Ok(options)
}

fn print_help() {
    println!(
        "\
mp4-tool —— MP4 / MP3 的检查、修复与常用处理

用法:
    mp4-tool check  <文件>...                             检查 moov 是否在 mdat 之前
    mp4-tool fix    [选项] <文件>...                      重新封装，把 moov 移到文件头
    mp4-tool split  [选项] <文件> --start <时间> --duration <时长>
                                                         分割
    mp4-tool concat [选项] <文件>...                      拼接（按命令行顺序）
    mp4-tool insert [选项] <目标> --insert <文件> --at <时间>
                                                         在指定时间插入一段
    mp4-tool blank  [选项] --kind <video|audio> --duration <时长>
                                                         生成黑屏视频 / 静音音频

选项:
    --ffmpeg <路径>        指定已有的 ffmpeg 可执行文件，不会下载
    --ffmpeg-url <地址>    ffmpeg 的下载地址
    --ffmpeg-dir <目录>    ffmpeg 下载到哪个目录

时间格式:
    ffmpeg 的时长写法都支持，例如 90、1:30、00:01:30、00:01:30.500

说明:
    check 只读文件，不需要 ffmpeg。其余子命令都需要 ffmpeg：
    没有指定 --ffmpeg 时会自动准备一个 —— 创建下载目录，目录里没有就下载，
    失败会重试。

    split / concat / insert 都尽量用 -c copy，不重编码。
    concat 和 insert 要求输入是同一种容器和编码。

示例:
    mp4-tool split  movie.mp4 --start 00:00:30 --duration 60
    mp4-tool concat part1.mp4 part2.mp4 part3.mp4
    mp4-tool insert movie.mp4 --insert ad.mp4 --at 00:05:00
    mp4-tool blank  --kind video --duration 5

退出码:
    0  成功
    1  有文件需要修复（仅 check）
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
        let atoms = mp4_tool::top_level_atoms(path)?;
        let layout = mp4_tool::layout(&atoms);

        println!("=== {} ===", path.display());
        println!("{:>12}  {:<6} {:>14}", "偏移", "atom", "大小");
        println!("{}", "-".repeat(40));

        for atom in &atoms {
            println!("{:>12}  {:<6} {:>14}", atom.offset, atom.kind, atom.size);
        }

        println!();

        match layout {
            mp4_tool::Layout::FastStart => {
                println!("结论: moov 在 mdat 之前 —— fast start 已优化，可以秒开");
            }
            mp4_tool::Layout::MoovAtEnd => {
                println!("结论: moov 在 mdat 之后 —— 渲染器必须下完整个文件才能播！");
                println!("      修复: mp4-tool fix {}", path.display());
                all_fast_start = false;
            }
            mp4_tool::Layout::Unknown => {
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

    let ffmpeg = mp4_tool::ensure_ffmpeg(&options.ffmpeg)?;

    for path in &options.files {
        let output = faststart_path(path);

        eprintln!("{} -> {}", path.display(), output.display());

        mp4_tool::remux(&ffmpeg, path, &output)?;

        let atoms = mp4_tool::top_level_atoms(&output)?;
        if !mp4_tool::layout(&atoms).is_fast_start() {
            bail!("{} 修复后布局仍不正确", output.display());
        }

        println!("{} 已修复", output.display());
    }

    Ok(ExitCode::SUCCESS)
}

/// 分割。
fn split(options: &Options) -> Result<ExitCode> {
    let input = single_input(options, "split")?;

    let start = options.start.as_deref().unwrap_or("0");
    let duration = options
        .duration
        .as_deref()
        .context("split 需要 --duration <时长>")?;

    let ffmpeg = mp4_tool::ensure_ffmpeg(&options.ffmpeg)?;

    eprintln!("分割 {}：从 {start} 开始，取 {duration}", input.display());

    let output = mp4_tool::split(&ffmpeg, input, start, duration)?;
    println!("{} 已生成", output.display());

    Ok(ExitCode::SUCCESS)
}

/// 拼接。
fn concat(options: &Options) -> Result<ExitCode> {
    if options.files.len() < 2 {
        bail!("concat 至少需要两个输入文件");
    }

    let ffmpeg = mp4_tool::ensure_ffmpeg(&options.ffmpeg)?;

    eprintln!("拼接 {} 个文件", options.files.len());

    let output = mp4_tool::concat(&ffmpeg, &options.files)?;
    println!("{} 已生成", output.display());

    Ok(ExitCode::SUCCESS)
}

/// 插入。
fn insert(options: &Options) -> Result<ExitCode> {
    let input = single_input(options, "insert")?;

    let insert = options
        .insert
        .as_deref()
        .context("insert 需要 --insert <文件>")?;
    let at = options.at.as_deref().context("insert 需要 --at <时间>")?;

    let ffmpeg = mp4_tool::ensure_ffmpeg(&options.ffmpeg)?;

    eprintln!(
        "在 {} 的第 {at} 秒处插入 {}",
        input.display(),
        insert.display()
    );

    let output = mp4_tool::insert(&ffmpeg, input, insert, at)?;
    println!("{} 已生成", output.display());

    Ok(ExitCode::SUCCESS)
}

/// 生成黑屏视频 / 静音音频。
fn blank(options: &Options) -> Result<ExitCode> {
    let kind = options.kind.context("blank 需要 --kind <video|audio>")?;
    let duration = options
        .duration
        .as_deref()
        .context("blank 需要 --duration <时长>")?;

    let ffmpeg = mp4_tool::ensure_ffmpeg(&options.ffmpeg)?;

    eprintln!(
        "生成 {} 秒的{}",
        duration,
        match kind {
            MediaKind::Video => "黑屏视频",
            MediaKind::Audio => "静音音频",
        }
    );

    let output = mp4_tool::blank(&ffmpeg, kind, duration)?;
    println!("{} 已生成", output.display());

    Ok(ExitCode::SUCCESS)
}

/// 只允许一个输入文件。
fn single_input<'a>(options: &'a Options, command: &str) -> Result<&'a Path> {
    match options.files.as_slice() {
        [input] => Ok(input),
        [] => bail!("{command} 需要一个输入文件"),
        _ => bail!("{command} 只接受一个输入文件"),
    }
}

/// 修复后的输出路径：`<名字>.faststart.mp4`。
fn faststart_path(input: &Path) -> PathBuf {
    let stem = input
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "output".to_string());

    input.with_file_name(format!("{stem}.faststart.mp4"))
}
