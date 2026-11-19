#!/usr/bin/env python3
# Convert a frame PNG into the built-in .bin format embedded by src/frame.rs:
#   2 bytes little-endian width, 2 bytes height, then w*h bytes,
#   one per pixel: 0..3 = grey level (darkest..lightest), >= 4 = transparent.
#
# The thresholds match dither.rs: luma >= 64/128/192 -> level 1/2/3,
# luma uses Pillow's RGB->L weights (as in gray.rs).
#
# Usage: tools/genframe.py FRAME.png assets/frame_<id>.bin   (needs Pillow)
#
# The part after `frame_` in the output file name is the frame id, which selects
# the frame (`--frame <id>`, window dropdown). build.rs collects every
# assets/frame_*.bin automatically when the project is built.
import struct
import sys
from pathlib import Path

from PIL import Image


def main() -> None:
    if len(sys.argv) != 3:
        sys.exit(f"usage: {sys.argv[0]} FRAME.png OUT.bin")
    src, dst = sys.argv[1], sys.argv[2]
    im = Image.open(src).convert("RGBA")
    w, h = im.size
    if w == 0 or h == 0:
        sys.exit("frame is empty")
    px = im.load()
    vals = bytearray()
    transparent = 0
    for y in range(h):
        for x in range(w):
            r, g, b, a = px[x, y]
            if a < 128:
                vals.append(4)
                transparent += 1
                continue
            luma = (19595 * r + 38470 * g + 7471 * b + 0x8000) >> 16
            level = 0
            for t in (64, 128, 192):
                if luma >= t:
                    level += 1
            vals.append(level)
    if transparent == 0:
        sys.exit("frame has no transparent window (alpha channel needed)")
    Path(dst).write_bytes(struct.pack("<HH", w, h) + bytes(vals))
    print(f"{src} ({w}x{h}, window through {transparent} transparent px) -> {dst}")


if __name__ == "__main__":
    main()
