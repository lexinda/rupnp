//! MP4 顶层 atom 的解析。
//!
//! MP4 文件由若干 atom 组成，其中：
//!
//! - `moov` 存放时长、轨道信息、编码参数、sample 索引、关键帧表
//! - `mdat` 存放实际的音视频数据
//!
//! 渲染器通过 HTTP Range 拉流。`moov` 在 `mdat` 之前（所谓的 fast start）时，
//! 渲染器第一次 Range 请求就能读到元数据，可以立刻开始播放；在之后则必须把
//! 整个文件下载完才知道文件多长、编码是什么，大文件就会一直转圈。

use anyhow::{Context, Result};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// 一个顶层 atom。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Atom {
    /// 在文件中的偏移量。
    pub offset: u64,
    /// atom 的类型，比如 `ftyp` / `moov` / `mdat`。
    pub kind: String,
    /// atom 的长度，包含 8 字节的头。
    pub size: u64,
}

/// 文件的 atom 布局。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    /// `moov` 在 `mdat` 之前，渲染器可以秒开。
    FastStart,
    /// `moov` 在 `mdat` 之后，渲染器必须把整个文件下载完。
    MoovAtEnd,
    /// 不是标准的 MP4。
    Unknown,
}

impl Layout {
    /// 是否是适合 HTTP Range 播放的布局。
    pub fn is_fast_start(self) -> bool {
        matches!(self, Layout::FastStart)
    }
}

/// 读取文件的顶层 atom。
pub fn top_level_atoms(path: &Path) -> Result<Vec<Atom>> {
    let mut file = File::open(path).with_context(|| format!("无法打开 {}", path.display()))?;

    let file_size = file
        .metadata()
        .with_context(|| format!("无法读取 {} 的大小", path.display()))?
        .len();

    let mut atoms = Vec::new();
    let mut offset = 0u64;

    while offset < file_size {
        file.seek(SeekFrom::Start(offset))?;

        let mut header = [0u8; 8];
        if file.read_exact(&mut header).is_err() {
            break;
        }

        let size32 = u32::from_be_bytes([header[0], header[1], header[2], header[3]]);
        let kind = String::from_utf8_lossy(&header[4..8]).into_owned();

        let (size, header_size) = match size32 {
            // 长度是 64 位，紧跟在类型后面
            1 => {
                let mut extended = [0u8; 8];
                file.read_exact(&mut extended)
                    .with_context(|| format!("{} 的 atom 头不完整", path.display()))?;
                (u64::from_be_bytes(extended), 16)
            }
            // 长度延伸到文件末尾
            0 => (file_size - offset, 8),
            _ => (size32 as u64, 8),
        };

        atoms.push(Atom { offset, kind, size });

        if size < header_size {
            break;
        }
        offset += size;
    }

    Ok(atoms)
}

/// 判断 atom 布局。
pub fn layout(atoms: &[Atom]) -> Layout {
    let moov = atoms.iter().position(|atom| atom.kind == "moov");
    let mdat = atoms.iter().position(|atom| atom.kind == "mdat");

    match (moov, mdat) {
        (Some(moov), Some(mdat)) if moov < mdat => Layout::FastStart,
        (Some(_), Some(_)) => Layout::MoovAtEnd,
        _ => Layout::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn atom(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&((payload.len() + 8) as u32).to_be_bytes());
        out.extend_from_slice(kind);
        out.extend_from_slice(payload);
        out
    }

    fn temp_file(name: &str, contents: &[u8]) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(name);
        let mut file = File::create(&path).unwrap();
        file.write_all(contents).unwrap();
        path
    }

    #[test]
    fn detects_fast_start() {
        let mut contents = atom(b"ftyp", &[0u8; 12]);
        contents.extend(atom(b"moov", &[0u8; 64]));
        contents.extend(atom(b"mdat", &[0u8; 1024]));

        let path = temp_file("mp4_tool_faststart.mp4", &contents);
        let atoms = top_level_atoms(&path).unwrap();

        assert_eq!(atoms.len(), 3);
        assert_eq!(atoms[0].kind, "ftyp");
        assert_eq!(atoms[1].kind, "moov");
        assert_eq!(atoms[2].kind, "mdat");
        assert_eq!(layout(&atoms), Layout::FastStart);

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn detects_moov_at_end() {
        let mut contents = atom(b"ftyp", &[0u8; 12]);
        contents.extend(atom(b"mdat", &[0u8; 1024]));
        contents.extend(atom(b"moov", &[0u8; 64]));

        let path = temp_file("mp4_tool_moov_at_end.mp4", &contents);
        let atoms = top_level_atoms(&path).unwrap();

        assert_eq!(layout(&atoms), Layout::MoovAtEnd);

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn detects_non_mp4() {
        let path = temp_file("mp4_tool_not_mp4.mp4", b"not an mp4 at all");
        let atoms = top_level_atoms(&path).unwrap();

        assert_eq!(layout(&atoms), Layout::Unknown);

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn handles_64_bit_sizes() {
        // size == 1 表示后面跟着 64 位长度
        let mut contents = Vec::new();
        contents.extend_from_slice(&1u32.to_be_bytes());
        contents.extend_from_slice(b"free");
        contents.extend_from_slice(&24u64.to_be_bytes());
        contents.extend_from_slice(&[0u8; 8]);

        let path = temp_file("mp4_tool_64bit.mp4", &contents);
        let atoms = top_level_atoms(&path).unwrap();

        assert_eq!(atoms.len(), 1);
        assert_eq!(atoms[0].kind, "free");
        assert_eq!(atoms[0].size, 24);

        let _ = std::fs::remove_file(path);
    }
}
