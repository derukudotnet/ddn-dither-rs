# ddn-dither (Rust)

Game Boy Camera–style 4-tone Bayer dithering. One executable: open or drop an image, drag a
fixed-aspect crop box, pick size / palette / frame, watch the dithered result update live, save.

> **Status:** the image-processing core and command-line parsing are unit-tested and verified
> bit-for-bit against Pillow and the original Python code (see *Verification*). The window
> (`src/gui.rs`) and file I/O (`src/io.rs`) were written without access to the crates.io
> registry, so they have only been type-checked against stand-ins. **The first
> `cargo build` may need small fixes** for crate API differences — see *First build*.

## Build

```sh
cargo build --release                       # window + command line  -> target/release/ddn-dither
cargo build --release --no-default-features # command line only (just `image` + `png`)
```

The release binary is a single file (no Python, Tk or venv). It needs a running Wayland or X11
session and the usual graphics stack at runtime, and glibc. To run on older distributions,
build on an older one (e.g. inside a Debian/Ubuntu LTS container). File dialogs use the
desktop's portal (`xdg-desktop-portal`); drag-and-drop works without it.

## Use

```sh
ddn-dither --output-size 800x350 --crop-mode interactive input.jpg   # your usual command
ddn-dither photo.jpg                      # no window: centre crop -> photo_dithered.png
ddn-dither *.jpg -p green --pixel-size 3 -a 21:9 -o 1280
ddn-dither photo.jpg --frame default --scale 4 -p pocket   # Game Boy frame, 640x576
ddn-dither                                # window, drop an image onto it
```

`ddn-dither --help` lists everything. Your old options keep working
(`--output-size`, `--output-width`, `--aspect`, `--crop-mode cover|contain|interactive`,
`--format`, `--output`); `-s/--internal-scale` is accepted and ignored.

### Window

* Drop an image anywhere, or **Open image…**. Drag inside the box to move it; drag a handle to
  resize (the opposite corner/side stays fixed, aspect ratio is kept).
* **Enter** saves. When started from the command line with a file, Enter saves and quits; Esc always closes the window.
* Size: aspect presets + width/height. With a preset the other dimension follows; choose
  *Custom* to set both freely. The crop box always has exactly the output's aspect ratio.
* **Pixel size** slider = output pixels per dithered pixel (whole numbers → crisp, even blocks).
  *auto* aims for about 384 dithered pixels across.
* **Contrast** slider boosts or softens contrast around mid-grey, on top of auto contrast (1.00 = no change).
* **Guides** over the crop box: rule-of-thirds lines or a crosshair through the crop centre, to help alignment.
* **Fit whole image** pads instead of cropping (padding uses the darkest tone).
* Palette: ddn, green, bw, pocket, color, or your own four hex colours (darkest first; 3-digit shorthand like `#0f0` works).
* Switching palette or recolouring happens instantly — the dithered grid is cached and only recoloured.
* Every control has a tooltip; hover (or long-press) it for details.
* Drag & drop from a file manager works on X11 and on Windows/macOS. On native Wayland the
  window cannot receive file drops yet (a limitation of the windowing library, not this tool);
  use “Open image…” — or run `WINIT_UNIX_BACKEND=x11 ddn-dither` (XWayland) and drop away.
  The window tells you which of the two applies to your session.

### Frames

A frame is a PNG with a transparent rectangular window. Like the real Game Boy Camera's frames
it uses **four grey levels**; the palette then recolours frame and photo together. A frame that
has more greys is converted to four tones on load (with the same thresholds as the dither step)
and the window tells you.

* Built in: the Deruku frame (160×144, 128×112 window).

  **Adding built-in frames is automatic.** A *built-in* frame is a four-tone PNG with a
  transparent window converted to a compact `.bin` — no `include_bytes!`, no extra Rust
  function, nothing to register. Two steps:

  1. `tools/genframe.py myframe.png assets/frame_myframe.bin` (needs Pillow; the script
     reproduces `assets/frame_deruku.bin` from `assets/frame_deruku_4tone.png` byte for byte).
     The part after `frame_` in the file name is the frame id (`myframe` above).
  2. Rebuild. `build.rs` collects every `assets/frame_*.bin` at compile time, so the frame
     appears in the window's frame list and can be selected with `--frame myframe`.
     (`--frame default`/`--frame builtin` keeps meaning the first one in file-name order.)

  If you skip step 1 you don't even need a rebuild: drop the plain PNG in a `frames/`
  folder (next to the executable, or in `~/.config/ddn-dither/frames/`) and the window
  will load it at start-up. For a design source of truth, encoding it into `assets/`
  makes it part of the executable itself.
* More frames: put PNGs in `frames/` next to the executable or in `~/.config/ddn-dither/frames/`.
* With a frame, the crop is locked to the window's aspect ratio and the output size is the frame
  size × **Scale** (default 4).

## What changed compared with the Python version

* Whole-number upscaling only (the Python version scaled 384 px → 800 px by 2.08×, giving uneven blocks).
* `contain` now fills the output instead of pasting a small picture into a big canvas.
* `--output-size 800` (width only) works instead of crashing.
* The crop is chosen in the source image's full resolution (fractional boxes supported).
* Photos are rotated by their EXIF orientation.
* Output PNGs are 2-bit indexed (a few KB); `--format webp` writes lossless WebP.
* Resize/dither/autocontrast produce the **same pixels** as the Python tool for the same settings.

## Verification

```sh
tools/verify.sh /path/to/the/python/repo    # needs only rustc + Python with Pillow/numpy
```

This runs the unit tests (36) and compares the Rust pipeline with Pillow's `resize(LANCZOS, box=…)`,
`ImageOps.autocontrast(cutoff=(0,10))` and the original `bayer_dither_4tone`, with and without the
frame: all identical pixel for pixel.

## First build

If `cargo build` reports errors, they are most likely in `src/gui.rs` or `src/io.rs` and about
crate API details (pinned: `eframe 0.29`, `image ≥0.25.6`, `png 0.17`, `rfd 0.15`). The
`image` EXIF call `decoder.orientation()` needs 0.25.6 or newer. Everything else
(`src/lib.rs` and the modules it lists) is plain `std` and does not depend on any crate.

## Layout

```
src/lib.rs + gray, resample, autocontrast, dither, palette, frame, cropbox, pipeline, pack, cli
                              std-only core (unit-tested)
src/io.rs, src/main.rs        file I/O and command line
src/gui.rs                    window (feature "gui", on by default)
assets/                       built-in frames (frame_*.bin, auto-registered by build.rs)
                              + frame_deruku_4tone.png, the source of frame_deruku.bin
tools/                        verification harness and scripts
```
