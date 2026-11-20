//! File input/output (uses the `image` and `png` crates).

use std::fs::File;
use std::io::BufWriter;
use std::path::Path;
#[cfg(feature = "gui")]
use std::path::PathBuf;

use ddn_dither::frame::Frame;
#[cfg(feature = "gui")]
use ddn_dither::cli::Format;
use ddn_dither::gray::RgbImage;
use ddn_dither::pack::pack_2bit;
use ddn_dither::palette::Palette;
use ddn_dither::pipeline::Rendered;
use image::ImageDecoder;

/// Load an image as RGB, honouring the EXIF orientation (phone photos).
pub fn load_rgb(path: &str) -> Result<RgbImage, String> {
    let reader = image::ImageReader::open(path)
        .map_err(|e| format!("cannot open {path}: {e}"))?
        .with_guessed_format()
        .map_err(|e| format!("cannot read {path}: {e}"))?;
    let mut decoder = reader.into_decoder().map_err(|e| format!("cannot decode {path}: {e}"))?;
    let orientation = decoder.orientation().unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut img = image::DynamicImage::from_decoder(decoder).map_err(|e| format!("cannot decode {path}: {e}"))?;
    img.apply_orientation(orientation);
    let rgb = img.to_rgb8();
    let (w, h) = rgb.dimensions();
    Ok(RgbImage::new(w as usize, h as usize, rgb.into_raw()))
}

/// Load a frame PNG (needs an alpha channel with a transparent window).
pub fn load_frame(path: &str) -> Result<Frame, String> {
    let img = image::open(path).map_err(|e| format!("cannot open frame {path}: {e}"))?.to_rgba8();
    let (w, h) = img.dimensions();
    let name = Path::new(path).file_stem().and_then(|s| s.to_str()).unwrap_or("frame");
    Frame::from_rgba(name, w as usize, h as usize, img.as_raw())
}

/// Folders searched for extra frames: `frames/` next to the executable and the user config dir.
/// (Only the window browses frame folders; the CLI takes an explicit path.)
#[cfg(feature = "gui")]
pub fn frame_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            dirs.push(parent.join("frames"));
        }
    }
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")));
    if let Some(c) = config {
        dirs.push(c.join("ddn-dither").join("frames"));
    }
    dirs
}

/// All frame PNGs found in `frame_dirs()`, with a warning for each that cannot be used.
#[cfg(feature = "gui")]
pub fn discover_frames() -> (Vec<Frame>, Vec<String>) {
    let mut frames = Vec::new();
    let mut warnings = Vec::new();
    for dir in frame_dirs() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        let mut paths: Vec<PathBuf> = rd
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().and_then(|e| e.to_str()).map(|e| e.eq_ignore_ascii_case("png")).unwrap_or(false))
            .collect();
        paths.sort();
        for p in paths {
            match load_frame(&p.to_string_lossy()) {
                Ok(f) => frames.push(f),
                Err(e) => warnings.push(format!("{}: {e}", p.display())),
            }
        }
    }
    (frames, warnings)
}

/// Save a rendered image. The extension decides the format: `.webp` -> lossless WebP,
/// anything else -> 2-bit indexed PNG (tiny files, exactly the four palette colours).
pub fn save(path: &str, r: &Rendered, pal: &Palette) -> Result<(), String> {
    let is_webp = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("webp"))
        .unwrap_or(false);
    if let Some(parent) = Path::new(path).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
        }
    }
    let file = File::create(path).map_err(|e| format!("cannot write {path}: {e}"))?;
    let out = BufWriter::new(file);

    if is_webp {
        let rgb = r.rgb(pal);
        image::codecs::webp::WebPEncoder::new_lossless(out)
            .encode(&rgb, r.out_w as u32, r.out_h as u32, image::ExtendedColorType::Rgb8)
            .map_err(|e| format!("cannot write {path}: {e}"))
    } else {
        let levels = r.levels_upscaled();
        let packed = pack_2bit(&levels, r.out_w, r.out_h);
        let palette: Vec<u8> = pal.iter().flatten().copied().collect();
        let mut enc = png::Encoder::new(out, r.out_w as u32, r.out_h as u32);
        enc.set_color(png::ColorType::Indexed);
        enc.set_depth(png::BitDepth::Two);
        enc.set_palette(palette);
        enc.set_compression(png::Compression::Best);
        let mut writer = enc.write_header().map_err(|e| format!("cannot write {path}: {e}"))?;
        writer.write_image_data(&packed).map_err(|e| format!("cannot write {path}: {e}"))?;
        writer.finish().map_err(|e| format!("cannot write {path}: {e}"))
    }
}

/// Exact byte length the rendered image would have when saved in `fmt`, without touching the disk.
/// Uses the identical encoders/settings as `save`, so the number matches the real file.
#[cfg(feature = "gui")]
pub fn encoded_size(fmt: Format, r: &Rendered, pal: &Palette) -> u64 {
    let mut buf = Vec::new();
    match fmt {
        Format::Webp => {
            let rgb = r.rgb(pal);
            if image::codecs::webp::WebPEncoder::new_lossless(&mut buf)
                .encode(&rgb, r.out_w as u32, r.out_h as u32, image::ExtendedColorType::Rgb8)
                .is_err()
            {
                return 0;
            }
        }
        Format::Png => {
            let levels = r.levels_upscaled();
            let packed = pack_2bit(&levels, r.out_w, r.out_h);
            let palette: Vec<u8> = pal.iter().flatten().copied().collect();
            let mut enc = png::Encoder::new(&mut buf, r.out_w as u32, r.out_h as u32);
            enc.set_color(png::ColorType::Indexed);
            enc.set_depth(png::BitDepth::Two);
            enc.set_palette(palette);
            enc.set_compression(png::Compression::Best);
            match enc.write_header() {
                Ok(mut writer) => {
                    if writer.write_image_data(&packed).is_ok() {
                        let _ = writer.finish();
                    }
                }
                Err(_) => return 0,
            }
        }
    }
    buf.len() as u64
}
