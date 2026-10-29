#!/usr/bin/env python3
"""检查 MP4 的 atom 布局，判断是否适合 HTTP Range 播放。

MP4 的顶层结构是若干 atom，其中：

    moov  存放时长、轨道信息、编码参数、sample 索引、关键帧表
    mdat  存放实际的音视频数据

渲染器通过 HTTP Range 拉流。如果 moov 在 mdat 之前（所谓的 fast start），
渲染器第一次 Range 请求就能读到元数据，可以立刻开始播放；如果 moov 在
mdat 之后，渲染器必须把整个文件下载完才知道文件多长、编码是什么，
大文件就会表现为"一直转圈"或直接超时。

用法：
    python mp4_atoms.py <file.mp4> [更多文件...]

退出码：
    0  所有文件都是 fast start 布局
    1  至少有一个文件 moov 在后，需要重新封装
    2  用法错误或文件无法解析
"""

import struct
import sys


def read_atoms(file):
    """按顺序产出 (偏移, atom 名, 大小)。"""
    offset = 0
    file.seek(0, 2)
    file_size = file.tell()

    while offset < file_size:
        file.seek(offset)
        header = file.read(8)
        if len(header) < 8:
            break

        size = struct.unpack(">I", header[:4])[0]
        kind = header[4:8].decode("latin-1")
        header_size = 8

        if size == 1:
            # 64 位长度
            extended = file.read(8)
            if len(extended) < 8:
                break
            size = struct.unpack(">Q", extended)[0]
            header_size = 16
        elif size == 0:
            # 长度延伸到文件末尾
            size = file_size - offset

        yield offset, kind, size

        if size < header_size:
            break
        offset += size


def check(path):
    """检查单个文件，返回 True 表示布局良好。"""
    try:
        with open(path, "rb") as file:
            print(f"=== {path} ===")
            print(f"{'偏移':>12}  {'atom':<6} {'大小':>14}")
            print("-" * 40)

            order = []
            for offset, kind, size in read_atoms(file):
                print(f"{offset:12d}  {kind:<6} {size:14d}")
                order.append(kind)
    except OSError as err:
        print(f"=== {path} ===\n无法读取: {err}", file=sys.stderr)
        return True

    if "moov" not in order or "mdat" not in order:
        print("\n结论: 找不到 moov/mdat，这不是标准 MP4")
        return True

    if order.index("moov") < order.index("mdat"):
        print("\n结论: moov 在 mdat 之前 —— fast start 已优化，可以秒开")
        return True

    print("\n结论: moov 在 mdat 之后 —— 渲染器必须下完整个文件才能播！")
    print("      修复: ffmpeg -i 输入.mp4 -c copy -movflags +faststart 输出.mp4")
    return False


def main(argv):
    if len(argv) < 2:
        print(__doc__)
        return 2

    ok = True
    for path in argv[1:]:
        if not check(path):
            ok = False
        print()

    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv))
