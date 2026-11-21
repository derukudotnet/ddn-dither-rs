#!/usr/bin/env python3
# Slice Game Boy Camera stamp sprite sheets into individual RGBA stamps.
#
# The original sheets (Japanese + international) are sectioned into groups of
# cells; every cell sits on its own light-purple backdrop and is separated
# from its neighbours by gutters of the page purple that run all the way
# through each section band. Both sheets share the same two purples, so the
# colour keys below are the only sheet-specific knowledge.
#
# The splitter recurses rows-first-then-columns over runs of fully-purple
# rows/cols, trims every leaf to its ink (= stamp) bounding box, sorts out
# leaves without ink (backdrop scraps) and leaves without backdrop (page
# labels, e.g. section captions), both of which are skipped, and turns both
# purples transparent. Exact duplicates across all input sheets are detected
# on the trimmed grey pattern; every unique stamp is written once, later
# occurrences are recorded in the manifest as duplicates.
#
# Usage:
#   tools/splitstamps.py SHEET.png [SHEET2.png ...]
#       Slices one or more sheets into <outdir>/st_<sheet>_<n>.png files plus
#       a manifest.json and, with --contact-sheet, per-sheet review boards
#       cs_<sheet>.png showing the numbered unique stamps. Sheets and output
#       are user assets: the default outdir assets/stamps/ is gitignored.
#   -O/--outdir DIR       output directory (default: assets/stamps)
#   --min-size N          drop leaves whose ink is smaller than N px (default 3)
#   --no-dedupe           write duplicates too instead of skipping them
#   --labels              also write the captured page labels as lb_*.png
#   --tol T               tolerate T stray non-purple px per row/col (default 0)
#   -n/--dry-run          preview counts and file names, write nothing
import argparse
import json
import sys
from pathlib import Path

from PIL import Image, ImageDraw

PAGE = (157, 141, 211)
BACK = (200, 191, 231)
BGTOL = 2
INK_LEVELS = ((230, 3), (150, 2), (60, 1), (0, 0))

def is_page(p):
    return (abs(p[0] - PAGE[0]) <= BGTOL and abs(p[1] - PAGE[1]) <= BGTOL
            and abs(p[2] - PAGE[2]) <= BGTOL)

def is_back(p):
    return (abs(p[0] - BACK[0]) <= BGTOL and abs(p[1] - BACK[1]) <= BGTOL
            and abs(p[2] - BACK[2]) <= BGTOL)

def level(p):
    for t, lvl in INK_LEVELS:
        if p[0] > t and p[1] > t:
            return lvl
    return 0

def spans(mask, start, end):
    out, s = [], None
    for i in range(start, end):
        if mask[i] and s is None:
            s = i
        elif not mask[i] and s is not None:
            out.append((s, i)); s = None
    if s is not None:
        out.append((s, end))
    return out

def slice_axis(px, x0, y0, x1, y1, tol, horizontal):
    def is_page_line(v):
        stray = 0
        if horizontal:
            for x in range(x0, x1):
                if not is_page(px[x, v]):
                    stray += 1
                    if stray > tol:
                        return False
        else:
            for y in range(y0, y1):
                if not is_page(px[v, y]):
                    stray += 1
                    if stray > tol:
                        return False
        return True
    if horizontal:
        bands = spans([not is_page_line(y) for y in range(y0, y1)], 0, y1 - y0)
        return [(x0, y0 + a, x1, y0 + b) for a, b in bands]
    cols = spans([not is_page_line(x) for x in range(x0, x1)], 0, x1 - x0)
    return [(x0 + a, y0, x0 + b, y1) for a, b in cols]

def split(px, x0, y0, x1, y1, tol, horizontal=True):
    boxes = slice_axis(px, x0, y0, x1, y1, tol, horizontal)
    if len(boxes) > 1:
        out = []
        for b in boxes:
            out += split(px, b[0], b[1], b[2], b[3], tol, not horizontal)
        return out
    return [(x0, y0, x1, y1)]

def trim_to_ink(px, x0, y0, x1, y1):
    ix0, iy0, ix1, iy1 = x1, y1, x0, y0
    has_back = False
    for y in range(y0, y1):
        for x in range(x0, x1):
            p = px[x, y]
            if is_back(p):
                has_back = True
            elif not is_page(p):
                ix0 = min(ix0, x); iy0 = min(iy0, y)
                ix1 = max(ix1, x); iy1 = max(iy1, y)
    if ix0 > ix1:
        return None, False
    return (ix0, iy0, ix1 + 1, iy1 + 1), has_back

def measure(px, box):
    x0, y0, x1, y1 = box
    levels = bytearray()
    for y in range(y0, y1):
        for x in range(x0, x1):
            p = px[x, y]
            levels.append(9 if is_page(p) else level(p))
    return bytes(levels)

def emit(px, box, dst):
    x0, y0, x1, y1 = box
    im = Image.new("RGBA", (x1 - x0, y1 - y0), (0, 0, 0, 0))
    op = im.load()
    for y in range(y0, y1):
        for x in range(x0, x1):
            p = px[x, y]
            if not is_page(p) and not is_back(p):
                op[x - x0, y - y0] = (p[0], p[1], p[2], 255)
    dst.parent.mkdir(parents=True, exist_ok=True)
    im.save(dst)

GREYS = {0: (24, 24, 33), 1: (90, 90, 107), 2: (173, 173, 181), 3: (255, 255, 255)}

def contact_sheet(items, dst, cols=32, cell=44):
    if not items:
        return
    rows = (len(items) + cols - 1) // cols
    im = Image.new("RGB", (cols * cell, rows * cell), (40, 40, 48))
    dr = ImageDraw.Draw(im)
    for i, (name, w, h, lvs) in enumerate(items):
        cx, cy = (i % cols) * cell, (i // cols) * cell + 10
        step = max(1, -(-w // (cell - 6)), -(-h // (cell - 14)))
        tw, th = w // step, h // step
        sx = cx + (cell - tw) // 2
        sy = cy + (cell - 12 - th) // 2
        for y in range(0, h, step):
            for x in range(0, w, step):
                v = lvs[y * w + x]
                if v < 8:
                    g = GREYS.get(v, (24, 24, 33))
                    im.putpixel((sx + x // step, sy + y // step), g)
        dr.text((cx + 1, cy - 9), name.rsplit("_", 1)[1].split(".")[0].lstrip("0") or "0", fill=(255, 190, 90))
    dst.parent.mkdir(parents=True, exist_ok=True)
    im.save(dst)

def main():
    ap = argparse.ArgumentParser(description="slice GBC stamp sheets into RGBA stamps")
    ap.add_argument("sheets", nargs="+", type=Path)
    ap.add_argument("-O", "--outdir", type=Path, default=Path("assets/stamps"))
    ap.add_argument("--min-size", type=int, default=3)
    ap.add_argument("--no-dedupe", action="store_true")
    ap.add_argument("--labels", action="store_true")
    ap.add_argument("--contact-sheet", action="store_true")
    ap.add_argument("--tol", type=int, default=0)
    ap.add_argument("-n", "--dry-run", action="store_true")
    args = ap.parse_args()

    manifest, seen, cs_items = [], {}, []
    total = dups = labels_ = dropped = file_no = 0
    for sheet in args.sheets:
        sheet = sheet.resolve()
        if not sheet.is_file():
            sys.exit(f"{sheet}: not a file")
        im = Image.open(sheet).convert("RGB")
        w, h = im.size
        px = im.load()
        written = 0
        sheet_cs = []
        for (lx0, ly0, lx1, ly1) in split(px, 8, 8, w - 8, h - 8, args.tol):
            t, has_back = trim_to_ink(px, lx0, ly0, lx1, ly1)
            if t is None:
                continue
            tw, th = t[2] - t[0], t[3] - t[1]
            if tw < args.min_size or th < args.min_size:
                dropped += 1
                continue
            lvl = measure(px, t)
            if not has_back:
                labels_ += 1
                entry = {"kind": "label", "source": sheet.name,
                         "x": t[0], "y": t[1], "w": tw, "h": th}
                if args.labels:
                    name = f"lb_{sheet.stem}_{labels_:03d}.png"
                    entry["file"] = name
                    if not args.dry_run:
                        emit(px, t, args.outdir / name)
                manifest.append(entry)
                continue
            total += 1
            key = (tw, th, lvl)
            if key in seen and not args.no_dedupe:
                dups += 1
                manifest.append({"kind": "duplicate", "duplicate_of": seen[key],
                                 "source": sheet.name, "x": t[0], "y": t[1], "w": tw, "h": th})
                continue
            name = f"st_{sheet.stem}_{file_no:04d}.png"
            file_no += 1
            seen[key] = name
            sheet_cs.append((name, tw, th, lvl))
            manifest.append({"kind": "stamp", "file": name, "source": sheet.name,
                             "x": t[0], "y": t[1], "w": tw, "h": th})
            written += 1
            if not args.dry_run:
                emit(px, t, args.outdir / name)
        mode = " (dry run)" if args.dry_run else f", {written} new stamps written to {args.outdir}"
        print(f"{sheet.name} ({w}x{h}): {written} unique stamps{mode}")
        if args.contact_sheet and sheet_cs and not args.dry_run:
            contact_sheet(sheet_cs, args.outdir / f"cs_{sheet.stem}.png")
    print(f"\ntotal: {total} stamps ({len(seen)} unique, {dups} duplicates, "
          f"{labels_} labels skipped, {dropped} debris dropped)")
    if not args.dry_run:
        out = args.outdir / "manifest.json"
        out.parent.mkdir(parents=True, exist_ok=True)
        out.write_text(json.dumps(manifest, indent=1))
        print(f"manifest: {out}")

if __name__ == "__main__":
    main()
