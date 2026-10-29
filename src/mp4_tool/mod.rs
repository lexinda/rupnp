//! 检查与修复 MP4 的 atom 布局，使其适合 HTTP Range 播放。
//!
//! 渲染器通过 HTTP Range 拉流时，如果 MP4 的 `moov` atom 在 `mdat` 之后，
//! 渲染器必须把整个文件下载完才知道文件多长、编码是什么，大文件就会一直
//! 转圈。这个模块提供检查与修复的能力。
//!
//! ```no_run
//! use std::path::Path;
//! use rupnp::mp4_tool;
//!
//! # fn main() -> anyhow::Result<()> {
//! let atoms = mp4_tool::top_level_atoms(Path::new("movie.mp4"))?;
//! println!("{:?}", mp4_tool::layout(&atoms));
//! # Ok(())
//! # }
//! ```

mod atom;
pub mod audio;
pub mod ffmpeg;
pub mod media;
pub mod transcode;

pub use atom::{Atom, Layout, layout, top_level_atoms};
pub use audio::{Mp3Options, derived_mp3_path, extract_mp3, to_mp3, to_mp3_batch};
pub use ffmpeg::{FfmpegOptions, ensure_ffmpeg, remux};
pub use media::{MediaKind, blank, concat, insert, split};
pub use transcode::{
    MediaInfo, TranscodeOptions, TranscodeOutcome, converted_path, probe, to_mp4, to_mp4_batch,
};
