"""Compare the Rust core against Pillow and the original Python project.

usage: python3 -I tools/verify_against_pillow.py HARNESS_BINARY ORIGINAL_PROJECT_ROOT

HARNESS_BINARY  : tools/harness.rs compiled against the ddn_dither library
ORIGINAL_PROJECT_ROOT : the Python repo (uses its bayer_dither_4tone unchanged)
"""
import math, os, subprocess, sys, tempfile
import numpy as np
from PIL import Image, ImageOps

HARNESS, ORIG = sys.argv[1], sys.argv[2]
sys.path.insert(0, os.path.join(ORIG, "src"))
from ddn_dither.dithering import bayer_dither_4tone  # original implementation, unchanged

TMP = tempfile.mkdtemp()
rng = np.random.default_rng(42)
FRAME_BIN = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "assets", "frame_default.bin")


def photo(w, h, seed):
    """Photo-like test image: smooth gradients, blobs, edges and a little noise."""
    r = np.random.default_rng(seed)
    y, x = np.mgrid[0:h, 0:w].astype(float)
    img = np.zeros((h, w, 3))
    for c in range(3):
        img[..., c] = 80 + 60 * np.sin(x / (30 + 20 * c) + r.random() * 6) + 50 * np.cos(y / (40 + 15 * c))
    for _ in range(6):
        cx, cy, rad = r.random() * w, r.random() * h, 20 + r.random() * min(w, h) / 3
        mask = (x - cx) ** 2 + (y - cy) ** 2 < rad ** 2
        img[mask] = r.random(3) * 255
    img += r.normal(0, 6, img.shape)
    return Image.fromarray(np.clip(img, 0, 255).astype(np.uint8), "RGB")


def write_pnm(path, im):
    a = np.array(im)
    magic = b"P5" if a.ndim == 2 else b"P6"
    with open(path, "wb") as f:
        f.write(magic + f"\n{a.shape[1]} {a.shape[0]}\n255\n".encode())
        f.write(a.tobytes())


def read_pgm(path):
    b = open(path, "rb").read()
    parts = b.split(b"\n", 3)
    w, h = map(int, parts[1].split())
    return np.frombuffer(parts[3], np.uint8).reshape(h, w)


def run(*args):
    return subprocess.run([HARNESS, *map(str, args)], check=True, capture_output=True, text=True).stdout


fails = 0


def check(name, ok, extra=""):
    global fails
    print(("PASS " if ok else "FAIL ") + name + (" " + extra if extra else ""))
    fails += 0 if ok else 1


# 1. Lanczos resize: bit-exact against Pillow (including fractional boxes)
cases = [
    ((1200, 800), (400, 175), None), ((4000, 3000), (400, 175), (300.5, 400.25, 3900.75, 1800.0)),
    ((640, 480), (1000, 700), None), ((333, 517), (128, 112), (10.0, 20.0, 300.0, 480.0)),
    ((50, 40), (25, 40), None), ((50, 40), (50, 20), None), ((777, 555), (777, 555), (1.5, 0.0, 777.0, 555.0)),
    ((2000, 1500), (4, 3), None), ((100, 100), (1, 1), None),
]
for (sw, sh), (dw, dh), box in cases:
    im = photo(sw, sh, sw + sh).convert("L")
    p = os.path.join(TMP, "in.pgm"); o = os.path.join(TMP, "out.pgm")
    write_pnm(p, im)
    b = box or (0, 0, sw, sh)
    run("resize", p, dw, dh, *b, o)
    ref = np.array(im.resize((dw, dh), Image.LANCZOS, box=box))
    got = read_pgm(o)
    d = np.abs(ref.astype(int) - got.astype(int))
    check(f"resize {sw}x{sh} -> {dw}x{dh} box={box}", d.max() == 0, f"(max diff {d.max()}, {int((d > 0).sum())} px)")

# 2. autocontrast cutoff (0,10): identical to Pillow
for seed in range(4):
    im = photo(300, 200, seed).convert("L")
    p = os.path.join(TMP, "in.pgm"); o = os.path.join(TMP, "out.pgm")
    write_pnm(p, im); run("autocontrast", p, o)
    ref = np.array(ImageOps.autocontrast(im, cutoff=(0, 10)))
    check(f"autocontrast seed {seed}", np.array_equal(ref, read_pgm(o)))

# 3. Whole pipeline against a Pillow reference that uses the ORIGINAL bayer_dither_4tone
fb = open(FRAME_BIN, "rb").read()
fw, fh = fb[0] | fb[1] << 8, fb[2] | fb[3] << 8
fdata = np.frombuffer(fb[4:], np.uint8).reshape(fh, fw)
HOLE = (16, 16, 128, 112)


def rnd(x):
    return int(math.floor(x + 0.5))


def ref_render(img, fit, box, out_w, out_h, ps, framed, ac=True):
    L = img.convert("L")
    if framed:
        gw, gh = HOLE[2], HOLE[3]
    else:
        ps = ps or max(1, rnd(out_w / 384.0))
        gw, gh = -(-out_w // ps), -(-out_h // ps)
    if fit == "cover":
        small = L.resize((gw, gh), Image.LANCZOS, box=box)
        if ac: small = ImageOps.autocontrast(small, cutoff=(0, 10))
        lv = bayer_dither_4tone(np.array(small))
    else:
        s = min(gw / L.width, gh / L.height)
        fw_, fh_ = min(max(rnd(L.width * s), 1), gw), min(max(rnd(L.height * s), 1), gh)
        small = L.resize((fw_, fh_), Image.LANCZOS)
        if ac: small = ImageOps.autocontrast(small, cutoff=(0, 10))
        canvas = np.zeros((gh, gw), np.uint8)
        oy, ox = (gh - fh_) // 2, (gw - fw_) // 2
        canvas[oy:oy + fh_, ox:ox + fw_] = np.array(small)
        lv = bayer_dither_4tone(canvas)
    if framed:
        canvas = np.zeros((fh, fw), np.uint8)
        canvas[HOLE[1]:HOLE[1] + HOLE[3], HOLE[0]:HOLE[0] + HOLE[2]] = lv
        opaque = fdata < 4
        canvas[opaque] = fdata[opaque]
        lv = canvas
    return lv


scenarios = [
    ("frameless cover 800x350, fractional crop", photo(4000, 3000, 1), dict(fit="cover", box=(210.5, 600.25, 3790.5, 2182.0), out_w=800, out_h=350, ps=0, framed=False)),
    ("frameless cover, pixel size 3", photo(1200, 800, 2), dict(fit="cover", box=(0, 0, 1200, 800), out_w=640, out_h=427, ps=3, framed=False)),
    ("frameless cover, odd size (not divisible)", photo(900, 600, 3), dict(fit="cover", box=(0, 0, 900, 600), out_w=101, out_h=67, ps=4, framed=False)),
    ("frameless contain", photo(1000, 500, 4), dict(fit="contain", box=None, out_w=480, out_h=480, ps=2, framed=False)),
    ("framed cover (built-in frame)", photo(1500, 1300, 5), dict(fit="cover", box=(100.0, 50.5, 1400.0, 1295.5 - 0.0), out_w=0, out_h=0, ps=0, framed=True)),
    ("framed contain (built-in frame)", photo(1000, 500, 6), dict(fit="contain", box=None, out_w=0, out_h=0, ps=0, framed=True)),
]
for name, img, kw in scenarios:
    p = os.path.join(TMP, "in.ppm"); o = os.path.join(TMP, "out.pgm")
    write_pnm(p, img)
    args = [f"fit={kw['fit']}", f"out_w={kw['out_w']}", f"out_h={kw['out_h']}", f"ps={kw['ps']}", f"frame={'builtin' if kw['framed'] else 'none'}", "scale=4"]
    if kw["box"]: args.append("box=" + ",".join(str(v) for v in kw["box"]))
    meta = run("render", p, o, *args).split()
    ref = ref_render(img, kw["fit"], kw["box"], kw["out_w"], kw["out_h"], kw["ps"], kw["framed"])
    got = read_pgm(o)
    same_shape = ref.shape == got.shape
    diff = int((ref != got).sum()) if same_shape else -1
    check(name, same_shape and diff == 0, f"(grid {got.shape[1]}x{got.shape[0]}, differing px: {diff}, rust meta: {' '.join(meta)})")

print("\nALL PASSED" if fails == 0 else f"\n{fails} FAILED")
sys.exit(1 if fails else 0)
