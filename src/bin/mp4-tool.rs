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
use rupnp::mp4_tool::{
    self, ConcatMp3Options, FadeCurve, FfmpegOptions, MediaKind, Mp3Options, SplitAudioOptions,
    TranscodeOptions,
};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

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
        "convert" => convert(&options),
        "extract-mp3" => extract_mp3(&options),
        "to-mp3" => to_mp3(&options),
        "concat-mp3" => concat_mp3(&options),
        "split-audio" => split_audio(&options),
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
    /// `convert` 的画质档位。
    quality: Option<String>,
    /// `convert` 的 x264 preset。
    preset: Option<String>,
    /// `convert` 的 CRF。
    crf: Option<u32>,
    /// `convert` 的最大高度。
    max_height: Option<u32>,
    /// MP3 码率，例如 `192k`。
    bitrate: Option<String>,
    /// MP3 采样率。
    sample_rate: Option<u32>,
    /// MP3 声道数。
    channels: Option<u16>,
    /// 拼接时段间的静音间隔（秒）。
    gap: Option<f64>,
    /// 拼接时每段的淡入淡出时长（秒）。
    fade: Option<f64>,
    /// 淡入淡出曲线。
    curve: Option<FadeCurve>,
    /// 拼接输出路径。
    output: Option<PathBuf>,
    /// 分割的结束时间。
    end: Option<String>,
    /// 分割时重新编码（精确到帧）。
    reencode: bool,
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
            "--quality" | "-q" => options.quality = Some(take_value(&mut index)?),
            "--preset" => options.preset = Some(take_value(&mut index)?),
            "--crf" => {
                let value = take_value(&mut index)?;
                options.crf = Some(
                    value
                        .parse()
                        .with_context(|| format!("--crf 不是数字: {value}"))?,
                );
            }
            "--max-height" => {
                let value = take_value(&mut index)?;
                options.max_height = Some(
                    value
                        .parse()
                        .with_context(|| format!("--max-height 不是数字: {value}"))?,
                );
            }
            "--bitrate" | "-b" => options.bitrate = Some(take_value(&mut index)?),
            "--sample-rate" => {
                let value = take_value(&mut index)?;
                options.sample_rate = Some(
                    value
                        .parse()
                        .with_context(|| format!("--sample-rate 不是数字: {value}"))?,
                );
            }
            "--channels" => {
                let value = take_value(&mut index)?;
                options.channels = Some(
                    value
                        .parse()
                        .with_context(|| format!("--channels 不是数字: {value}"))?,
                );
            }
            "--gap" => {
                let value = take_value(&mut index)?;
                options.gap = Some(
                    value
                        .parse()
                        .with_context(|| format!("--gap 不是数字: {value}"))?,
                );
            }
            "--fade" => {
                let value = take_value(&mut index)?;
                options.fade = Some(
                    value
                        .parse()
                        .with_context(|| format!("--fade 不是数字: {value}"))?,
                );
            }
            "--curve" => {
                let value = take_value(&mut index)?;
                options.curve = Some(match value.as_str() {
                    "linear" | "tri" => FadeCurve::Linear,
                    "sine" | "qsin" => FadeCurve::Sine,
                    "exp" | "exponential" => FadeCurve::Exponential,
                    other => {
                        bail!("--curve 只能是 linear / sine / exp，收到 `{other}`")
                    }
                });
            }
            "--output" | "-o" => options.output = Some(PathBuf::from(take_value(&mut index)?)),
            "--end" => options.end = Some(take_value(&mut index)?),
            "--reencode" => options.reencode = true,
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
    mp4-tool convert [选项] <文件>...                     把任意格式转成 MP4
    mp4-tool extract-mp3 [选项] <视频>...                 从视频里提取音轨存成 MP3
    mp4-tool to-mp3 [选项] <音频>...                      把音频（如 WAV）转成 MP3
    mp4-tool concat-mp3 [选项] <音频>...                   拼接 MP3，可加间隔和淡入淡出
    mp4-tool split-audio [选项] <音频> --start <时间> --end <时间>
                                                         截取音频的 [开始, 结束) 段

选项:
    --ffmpeg <路径>        指定已有的 ffmpeg 可执行文件，不会下载
    --ffmpeg-url <地址>    ffmpeg 的下载地址
    --ffmpeg-dir <目录>    ffmpeg 下载到哪个目录

convert 专用选项:
    -q, --quality <档位>   fast | balanced | high，默认 balanced
    --preset <名字>        直接指定 x264 preset（ultrafast … veryslow）
    --crf <数字>           直接指定 CRF，越小画质越好
    --max-height <像素>    等比缩小到该高度以内，用于压体积

MP3 专用选项:
    -b, --bitrate <码率>   例如 128k / 192k / 320k，默认 192k
    --sample-rate <Hz>     采样率，例如 44100
    --channels <数量>      声道数，1 或 2

concat-mp3 专用选项:
    --gap <秒>             段与段之间的静音间隔
    --fade <秒>            每段首尾的淡入淡出时长
    --curve <形状>         linear | sine | exp，默认 sine
    -o, --output <路径>    输出路径，默认 <第一个输入名字>.concat.mp3

split-audio 专用选项:
    --start <时间>         开始的时刻
    --end <时间>           结束的时刻（不含）
    --reencode             重新编码以精确到帧，默认只复制
    -o, --output <路径>    输出路径，默认 <输入名字>.split.mp3

时间格式:
    90、1:30、00:01:30、00:01:30.500、1h30m 都能解析

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
    mp4-tool convert movie.avi
    mp4-tool convert a.flv b.wmv c.mkv --quality high
    mp4-tool extract-mp3 movie.mp4
    mp4-tool to-mp3 song.wav --bitrate 320k
    mp4-tool concat-mp3 a.mp3 b.mp3 c.mp3 --gap 1.5 --fade 0.5
    mp4-tool split-audio song.mp3 --start 00:00:30 --end 00:01:30

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

/// 从视频里提取音轨存成 MP3。
fn extract_mp3(options: &Options) -> Result<ExitCode> {
    if options.files.is_empty() {
        bail!("请至少指定一个视频文件");
    }

    let mp3 = mp3_options(options);
    let ffmpeg = mp4_tool::ensure_ffmpeg(&options.ffmpeg)?;

    let mut failed = 0;

    for input in &options.files {
        match mp4_tool::extract_mp3(&ffmpeg, input, None, &mp3) {
            Ok(output) => println!("{} 已生成", output.display()),
            Err(err) => {
                eprintln!("提取 {} 的音轨失败: {err:#}", input.display());
                failed += 1;
            }
        }
    }

    if failed > 0 {
        eprintln!("{failed} 个文件失败");
        return Ok(ExitCode::FAILURE);
    }

    Ok(ExitCode::SUCCESS)
}

/// 把音频（比如 WAV）转成 MP3。
fn to_mp3(options: &Options) -> Result<ExitCode> {
    if options.files.is_empty() {
        bail!("请至少指定一个音频文件");
    }

    let mp3 = mp3_options(options);
    let ffmpeg = mp4_tool::ensure_ffmpeg(&options.ffmpeg)?;

    let mut failed = 0;

    for input in &options.files {
        match mp4_tool::to_mp3(&ffmpeg, input, None, &mp3) {
            Ok(output) => println!("{} 已生成", output.display()),
            Err(err) => {
                eprintln!("转换 {} 失败: {err:#}", input.display());
                failed += 1;
            }
        }
    }

    if failed > 0 {
        eprintln!("{failed} 个文件失败");
        return Ok(ExitCode::FAILURE);
    }

    Ok(ExitCode::SUCCESS)
}

/// 分割音频：截取 [start, end) 这一段。
fn split_audio(options: &Options) -> Result<ExitCode> {
    let input = single_input(options, "split-audio")?;

    let start = options
        .start
        .as_deref()
        .context("split-audio 需要 --start <时间>")?;
    let end = options
        .end
        .as_deref()
        .context("split-audio 需要 --end <时间>")?;

    let mut split = SplitAudioOptions {
        reencode: options.reencode,
        ..SplitAudioOptions::default()
    };

    if let Some(bitrate) = &options.bitrate {
        split.mp3.bitrate = bitrate.clone();
    }

    let ffmpeg = mp4_tool::ensure_ffmpeg(&options.ffmpeg)?;

    eprintln!("分割 {}：{start} 到 {end}", input.display());

    let output = mp4_tool::split_audio(
        &ffmpeg,
        input,
        start,
        end,
        options.output.as_deref(),
        &split,
    )?;
    println!("{} 已生成", output.display());

    Ok(ExitCode::SUCCESS)
}

/// 拼接 MP3，可带间隔和淡入淡出。
fn concat_mp3(options: &Options) -> Result<ExitCode> {
    if options.files.len() < 2 {
        bail!("concat-mp3 至少需要两个输入文件");
    }

    let mut concat = ConcatMp3Options::default();

    if let Some(gap) = options.gap {
        concat.gap = Duration::from_secs_f64(gap);
    }
    if let Some(fade) = options.fade {
        concat.fade = Duration::from_secs_f64(fade);
    }
    if let Some(curve) = options.curve {
        concat.curve = curve;
    }
    if let Some(bitrate) = &options.bitrate {
        concat.mp3.bitrate = bitrate.clone();
    }

    let output = options.output.clone().unwrap_or_else(|| {
        let stem = options
            .files
            .first()
            .and_then(|path| path.file_stem())
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_else(|| "output".to_string());

        options
            .files
            .first()
            .and_then(|path| path.parent())
            .unwrap_or_else(|| Path::new("."))
            .join(format!("{stem}.concat.mp3"))
    });

    let ffmpeg = mp4_tool::ensure_ffmpeg(&options.ffmpeg)?;

    eprintln!(
        "拼接 {} 个文件：间隔 {:?}，淡入淡出 {:?}，曲线 {:?}",
        options.files.len(),
        concat.gap,
        concat.fade,
        concat.curve
    );

    mp4_tool::concat_mp3(&ffmpeg, &options.files, &output, &concat)?;

    println!("{} 已生成", output.display());

    Ok(ExitCode::SUCCESS)
}

/// 从命令行选项里取出 MP3 参数。
fn mp3_options(options: &Options) -> Mp3Options {
    let mut mp3 = Mp3Options::default();

    if let Some(bitrate) = &options.bitrate {
        mp3.bitrate = bitrate.clone();
    }
    if let Some(rate) = options.sample_rate {
        mp3.sample_rate = Some(rate);
    }
    if let Some(channels) = options.channels {
        mp3.channels = Some(channels);
    }

    mp3
}

/// 只允许一个输入文件。
fn single_input<'a>(options: &'a Options, command: &str) -> Result<&'a Path> {
    match options.files.as_slice() {
        [input] => Ok(input),
        [] => bail!("{command} 需要一个输入文件"),
        _ => bail!("{command} 只接受一个输入文件"),
    }
}

/// 把任意格式转成 MP4。
fn convert(options: &Options) -> Result<ExitCode> {
    if options.files.is_empty() {
        bail!("请至少指定一个输入文件");
    }

    let mut transcode = match options.quality.as_deref() {
        None | Some("balanced") => TranscodeOptions::default(),
        Some("fast") => TranscodeOptions::fast(),
        Some("high") => TranscodeOptions::high_quality(),
        Some(other) => bail!("--quality 只能是 fast / balanced / high，收到 `{other}`"),
    };

    if let Some(preset) = &options.preset {
        transcode.preset = preset.clone();
    }
    if let Some(crf) = options.crf {
        transcode.crf = crf;
    }
    if let Some(height) = options.max_height {
        transcode.max_height = Some(height);
    }

    let ffmpeg = mp4_tool::ensure_ffmpeg(&options.ffmpeg)?;

    let mut failed = 0;

    for input in &options.files {
        let output = mp4_tool::converted_path(input);

        eprintln!("转换 {} -> {}", input.display(), output.display());

        match mp4_tool::to_mp4(&ffmpeg, input, &output, &transcode) {
            Ok(outcome) if outcome.remuxed => {
                println!("{} 已生成（只换容器，未重编码）", outcome.output.display());
            }
            Ok(outcome) => {
                let mut what = Vec::new();
                if outcome.reencoded_video {
                    what.push("视频");
                }
                if outcome.reencoded_audio {
                    what.push("音频");
                }
                let what = if what.is_empty() {
                    "无需重编码".to_string()
                } else {
                    format!("重编码{}", what.join("+"))
                };

                println!("{} 已生成（{what}）", outcome.output.display());
            }
            Err(err) => {
                eprintln!("转换 {} 失败: {err:#}", input.display());
                failed += 1;
            }
        }
    }

    if failed > 0 {
        eprintln!("{failed} 个文件转换失败");
        return Ok(ExitCode::FAILURE);
    }

    Ok(ExitCode::SUCCESS)
}

/// 修复后的输出路径：`<名字>.faststart.mp4`。
fn faststart_path(input: &Path) -> PathBuf {
    let stem = input
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "output".to_string());

    input.with_file_name(format!("{stem}.faststart.mp4"))
}
