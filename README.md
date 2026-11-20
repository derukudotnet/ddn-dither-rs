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
(`--output-size`, `--output-width`, `--aspect`, `--crop-mode cover|contain|pad|interactive`,
`--format`, `--output`); `-s/--internal-scale` is accepted and ignored.

### Window

* Drop an image anywhere, or **Open image….** Drag inside the box to move it; drag a handle
  to resize (the opposite corner/side stays fixed, aspect ratio is kept); scroll the wheel
  over the box to grow or shrink it about its centre.
* There is no save-on-Enter shortcut; saving always happens through the **Save** buttons, so an
  accidental keypress can never silently overwrite a file. When started from the command line
  with a file, **Save & quit** saves and closes the window; Esc always closes it.
* Size: aspect presets + width/height (with a swap button). With a preset the other dimension
  follows; choose *Custom* to set both freely. The crop box always has exactly the output's
  aspect ratio.
* **Integer scale** (Size & crop) keeps the dither blocks whole and uncropped *without
  changing your output size*: while it is on, the pixel-size slider is replaced by one-click
  choices for exactly the values that divide width and height (800×350, for instance, allows
  1, 2, 5, 10, 25, 50, plus *auto*); the effective one is marked. Turning it off brings the
  free slider back, letting any pixel size be used with a partial row/column cut off (the
  pixel-size row warns).
* **Pixel size** slider = output pixels per dithered pixel (whole numbers → crisp, even blocks).
  *auto* aims for about 384 dithered pixels across; the row shows the resulting grid size if you
  want to match the Camera's authentic 128×112 photo grid.
* **Preview zoom**: Fit / 100% buttons above the preview, with the current zoom shown next to
  them (in Fit mode you see the effective scale, e.g. `Fit · 87%`). Mouse wheel zooms at the
  pointer, dragging pans (grab cursor), double-click returns to Fit. Zoom and pan survive
  re-renders, so you can change pixel size and compare on exactly the same spot. At 100% the
  preview is pixel-for-pixel what the saved file contains — no need to save-and-check to judge
  the result. The header also shows the exact size of the output file for the current
  format (computed with the same encoder settings as saving, so it is the real number). The
  header also shows the output resolution and the dither grid with the effective pixel size
  (e.g. `800×350 px · 400×175 grid · ×2`), so you can read the result without scanning the sidebar.
* **Reset all settings** (bottom of the sidebar) returns every option to its startup value —
  size, pixel size, contrast, frame selection and scale, palette, and the crop box — as if the
  app were restarted with the same image, without closing it.
* **Copy command line** puts a `ddn-dither …` command with the current size, pixel size, fit
  mode, contrast, format, palette and frame onto the clipboard, so a render can be reproduced
  (or batched) exactly in a script. The crop position itself is chosen in the window, so the
  command uses the default centre-crop.
* **Contrast** slider boosts or softens contrast around mid-grey, on top of auto contrast (1.00 = no change).
* **Guides** over the crop box: rule-of-thirds lines or a crosshair through the crop centre, to help alignment.
* **Fit** chooses how the picture fills the output: **Crop** centre-crops, **Pad** scales the
  whole picture in and fills the rest with the darkest tone, **Edge pad** fills the rest by
  repeating the picture's outermost rows and columns (a smeared-out extension of the image
  border; sometimes it hides dithering seams better than dark padding).
* Palette: ddn (default), green, bgb, pocket, bw, demichrome, icecream, kirokaze, color, or your own four hex colours (darkest first; 3-digit shorthand like `#0f0` works). The palette dropdown shows a mini swatch strip for each entry, the custom editor is one colour-filled text box per colour (boxes can be reordered by dragging their grip), and picking a built-in palette fills the boxes so you can fine-tune from there.
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
and the window tells you. Frame names are shown prettified in the dropdown (`film_roll`
becomes "Film Roll"); frame ids stay as-is for `--frame`. The dropdown groups frames by
size: 160×144 (Game Boy Camera), 160×224 (wild), everything else under "Other sizes".

* Built in: the Deruku frame (160×144, 128×112 window).

  **Adding built-in frames is automatic.** A *built-in* frame is a four-tone PNG with a
  transparent window converted to a compact `.bin` — no `include_bytes!`, no extra Rust
  function, nothing to register. Two steps:

  1. `tools/genframe.py myframe.png otherframe.png ...` (needs Pillow; the script writes
     `assets/frame_<id>.bin` for each, the id from the file stem, a leading `frame_`
     stripped; it also takes directories, and `-n` previews the names). It reproduces
     `assets/frame_deruku.bin` from `assets/frame_deruku_4tone.png` byte for byte.
     The old explicit form `genframe.py myframe.png assets/frame_myframe.bin` still works.
  2. Rebuild. `build.rs` collects every `assets/frame_*.bin` at compile time (top level and
     one subfolder level), so the frame appears in the window's frame list and can be
     selected with `--frame myframe`.
     (`--frame default`/`--frame builtin` keeps meaning the first one in file-name order.)

  **The frame dropdown is grouped by folder.** Top-level `assets/frame_*.bin` files form the
  *Built-in* section; frames in `assets/<sub>/` (e.g. `assets/wild/`) form one section per
  folder; runtime frames in `frames/` likewise, with top-level `frames/*.png` under
  *Custom*. Sections appear as Built-in, then folders alphabetically, then Custom.

  If you skip step 1 you don't even need a rebuild: drop the plain PNG in a `frames/`
  folder (next to the executable, or in `~/.config/ddn-dither/frames/`) and the window
  will load it at start-up. For a design source of truth, encoding it into `assets/`
  makes it part of the executable itself.
* More frames: put PNGs in `frames/` next to the executable or in `~/.config/ddn-dither/frames/`.

### Frames and copyright

Frame art you did not draw yourself — the original Game Boy Camera frames are
Nintendo's — should stay out of published git repositories. Everything described
above works without committing any of it:

* runtime frames live in `frames/` or `~/.config/ddn-dither/frames/` (never in the repo);
* locally built-in frames are genuinely embedded in your binary, and this repo's
  `.gitignore` keeps `assets/frame_*.bin` out of git except the original `frame_deruku.bin`.

This repository stays free of third-party art, and every user supplies whatever frames
they are entitled to use themselves.
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

`Cargo.lock` is committed: the executable should build with the exact dependency versions
it was tested against. (Regenerate deliberately with `cargo update`, not as a side effect.)

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
