#!/usr/bin/env python3
# Convert frame PNGs into the built-in .bin format embedded by src/frame.rs:
#   2 bytes little-endian width, 2 bytes height, then w*h bytes,
#   one per pixel: 0..3 = grey level (darkest..lightest), >= 4 = transparent.
#
# The thresholds match dither.rs: luma >= 64/128/192 -> level 1/2/3,
# luma uses Pillow's RGB->L weights (as in gray.rs).
#
# The part after `frame_` in the output file name is the frame id, which selects
# the frame (`--frame <id>`, window dropdown). build.rs collects every
# assets/frame_*.bin automatically when the project is built.
#
# Usage:
#   tools/genframe.py FRAME.png [FRAME2.png ...]     (needs Pillow)
#       Converts one or more PNGs and/or directories (every *.png in each) into
#       <outdir>/frame_<id>.bin files, the id taken from the file stem (a leading
#       `frame_` is stripped). Default outdir: the project's assets/; override
#       with --outdir DIR, -n/--dry-run previews the names. The explicit
#       two-argument form still works:
#   tools/genframe.py FRAME.png assets/frame_myframe.bin
import argparse
import struct
import sys
from pathlib import Path

from PIL import Image


def is_png(path: Path) -> bool:
    return path.suffix.lower() == ".png"


def convert(src: Path, dst: Path, report) -> None:
    im = Image.open(src).convert("RGBA")
    w, h = im.size
    if w == 0 or h == 0:
        sys.exit(f"{src}: frame is empty")
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
        sys.exit(f"{src}: frame has no transparent window (alpha channel needed)")
    dst.parent.mkdir(parents=True, exist_ok=True)
    dst.write_bytes(struct.pack("<HH", w, h) + bytes(vals))
    report(f"{src} ({w}x{h}, window through {transparent} px) -> {dst}")


def main() -> None:
    here = Path(__file__).resolve().parent
    ap = argparse.ArgumentParser(
        description="Convert frame PNGs into the compiled-in .bin frame format (assets/frame_<id>.bin)."
    )
    ap.add_argument("inputs", nargs="+", help="frame PNGs and/or directories containing *.png")
    ap.add_argument(
        "-O", "--outdir",
        default=here.parent / "assets",
        type=Path,
        help="directory for generated frame_<id>.bin files (default: the project's assets/)",
    )
    ap.add_argument("-n", "--dry-run", action="store_true", help="show what would be written, write nothing")
    args = ap.parse_args()

    a = args.inputs
    if len(a) == 2 and is_png(Path(a[0])) and Path(a[1]).suffix.lower() == ".bin":
        sources, explicit = [Path(a[0])], Path(a[1])
    else:
        sources, explicit = [], None
        for name in a:
            p = Path(name)
            if p.is_dir():
                sources.extend(sorted(q for q in p.iterdir() if is_png(q)))
            elif p.is_file() and is_png(p):
                sources.append(p)
            else:
                sys.exit(f"{p}: no such file or directory, or not a PNG")
    if not sources:
        sys.exit("no input PNGs found")

    def report(msg: str) -> None:
        print(("dry-run: " + msg) if args.dry_run else msg)

    for src in sources:
        if explicit is not None:
            dst = explicit
        else:
            stem = src.stem
            fid = stem[6:] if stem.lower().startswith("frame_") else stem
            dst = args.outdir / f"frame_{fid}.bin"
        if args.dry_run:
            report(f"{src} -> {dst}")
        else:
            convert(src, dst, report)


if __name__ == "__main__":
    main()
