//! Command-line parsing (std only). The old Python options keep working:
//! `ddn-dither --output-size 800x350 --crop-mode interactive input.jpg`.

use std::path::Path;

pub const USAGE: &str = "\
ddn-dither - Game Boy Camera style 4-tone dithering

USAGE:
    ddn-dither [OPTIONS] [INPUT...]

With no INPUT, or with --crop-mode interactive / --gui, the window opens (drop an image
onto it, or use \"Open image\"). Otherwise each INPUT is converted and saved.

OPTIONS:
    -o, --output-size WxH     Output size, e.g. 800x350 (a single number sets the width)
        --output-width W      Output width; height follows --aspect or the image
    -a, --aspect RATIO        W:H, W/H or a number (21:9, 16/9, 2.33)
        --crop-mode MODE      cover (centre crop, default), contain (pad), interactive (window)
    -p, --palette SPEC        ddn (default), green, bw, pocket, color, or four hex colours
                              darkest first, e.g. '#0f0f0f,#4a4a4a,#868686,#c1c1c1'
        --pixel-size N        Output pixels per dithered pixel (default: auto, about 384 across)
        --frame SPEC          none (default), default (built-in), or the path of a frame PNG
                              (transparent window; four grey levels). Sets the output size.
        --scale N             Integer upscale for framed output (default 4 -> 640x576)
        --no-autocontrast     Skip the contrast stretch before dithering
    -f, --format png|webp     Format when the output name is generated (default png)
    -O, --output PATH         Output file (default: INPUT_dithered.png; one input only)
        --gui                 Open the window
    -d, --debug               Print extra information
    -h, --help                Show this help
    -V, --version             Show the version

Old Python options: -s/--internal-scale is accepted but ignored (use --pixel-size).
";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CropMode {
    Cover,
    Contain,
    Interactive,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Png,
    Webp,
}

impl Format {
    pub fn ext(self) -> &'static str {
        match self {
            Format::Png => "png",
            Format::Webp => "webp",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FrameSel {
    None,
    Builtin,
    Path(String),
}

#[derive(Clone, Debug)]
pub struct Cli {
    pub inputs: Vec<String>,
    pub output: Option<String>,
    /// `-o WxH` or `-o W`
    pub output_size: Option<(usize, Option<usize>)>,
    pub output_width: Option<usize>,
    pub aspect: Option<f64>,
    pub crop_mode: CropMode,
    pub palette: String,
    /// 0 = auto
    pub pixel_size: usize,
    pub format: Format,
    pub frame: FrameSel,
    pub scale: usize,
    pub autocontrast: bool,
    pub gui: bool,
    pub debug: bool,
    pub help: bool,
    pub version: bool,
    pub warnings: Vec<String>,
}

impl Default for Cli {
    fn default() -> Cli {
        Cli {
            inputs: vec![],
            output: None,
            output_size: None,
            output_width: None,
            aspect: None,
            crop_mode: CropMode::Cover,
            palette: crate::palette::DEFAULT_NAME.to_string(),
            pixel_size: 0,
            format: Format::Png,
            frame: FrameSel::None,
            scale: 4,
            autocontrast: true,
            gui: false,
            debug: false,
            help: false,
            version: false,
            warnings: vec![],
        }
    }
}

pub fn parse_size(s: &str) -> Result<(usize, Option<usize>), String> {
    let bad = || format!("size must be WIDTHxHEIGHT (e.g. 800x350) or a width, got '{s}'");
    let t = s.trim().to_ascii_lowercase();
    let pos = |v: &str| v.trim().parse::<usize>().ok().filter(|&n| n > 0);
    match t.split_once('x') {
        Some((w, h)) => Ok((pos(w).ok_or_else(bad)?, Some(pos(h).ok_or_else(bad)?))),
        None => Ok((pos(&t).ok_or_else(bad)?, None)),
    }
}

pub fn parse_aspect(s: &str) -> Result<f64, String> {
    let bad = || format!("aspect must be W:H, W/H or a number (e.g. 21:9 or 2.333), got '{s}'");
    let t = s.trim();
    let num = |v: &str| v.trim().parse::<f64>().ok().filter(|n| n.is_finite() && *n > 0.0);
    for sep in [':', '/'] {
        if let Some((a, b)) = t.split_once(sep) {
            return Ok(num(a).ok_or_else(bad)? / num(b).ok_or_else(bad)?);
        }
    }
    num(t).ok_or_else(bad)
}

fn parse_count(name: &str, s: &str) -> Result<usize, String> {
    s.trim().parse::<usize>().map_err(|_| format!("{name} needs a whole number, got '{s}'"))
}

pub fn parse(args: &[String]) -> Result<Cli, String> {
    let mut c = Cli::default();
    let mut i = 0;
    while i < args.len() {
        let raw = &args[i];
        i += 1;
        if raw == "--" {
            c.inputs.extend(args[i..].iter().cloned());
            break;
        }
        if !raw.starts_with('-') || raw == "-" {
            c.inputs.push(raw.clone());
            continue;
        }
        let (key, inline): (String, Option<String>) = match raw.split_once('=') {
            Some((k, v)) if raw.starts_with("--") => (k.to_string(), Some(v.to_string())),
            _ => (raw.clone(), None),
        };
        let mut value = |name: &str| -> Result<String, String> {
            if let Some(v) = inline.clone() {
                return Ok(v);
            }
            let v = args.get(i).cloned().ok_or_else(|| format!("{name} needs a value"))?;
            i += 1;
            Ok(v)
        };
        match key.as_str() {
            "-h" | "--help" => c.help = true,
            "-V" | "--version" => c.version = true,
            "-d" | "--debug" => c.debug = true,
            "--gui" => c.gui = true,
            "--no-autocontrast" => c.autocontrast = false,
            "-o" | "--output-size" => c.output_size = Some(parse_size(&value(&key)?)?),
            "--output-width" => {
                let v = value(&key)?;
                c.output_width = Some(parse_count(&key, &v).and_then(|n| {
                    if n == 0 { Err("--output-width must be at least 1".to_string()) } else { Ok(n) }
                })?);
            }
            "-a" | "--aspect" => c.aspect = Some(parse_aspect(&value(&key)?)?),
            "--crop-mode" => {
                c.crop_mode = match value(&key)?.as_str() {
                    "cover" => CropMode::Cover,
                    "contain" => CropMode::Contain,
                    "interactive" => CropMode::Interactive,
                    other => return Err(format!("unknown crop mode '{other}' (cover, contain, interactive)")),
                }
            }
            "-p" | "--palette" => {
                let v = value(&key)?;
                crate::palette::resolve(&v)?;
                c.palette = v;
            }
            "--pixel-size" => {
                let v = value(&key)?;
                c.pixel_size = if v == "auto" { 0 } else { parse_count(&key, &v)? };
            }
            "--frame" => {
                let v = value(&key)?;
                c.frame = match v.as_str() {
                    "none" => FrameSel::None,
                    "default" | "builtin" => FrameSel::Builtin,
                    _ => FrameSel::Path(v),
                };
            }
            "--scale" => {
                let n = parse_count(&key, &value(&key)?)?;
                if n == 0 {
                    return Err("--scale must be at least 1".into());
                }
                c.scale = n;
            }
            "-f" | "--format" => {
                c.format = match value(&key)?.to_ascii_lowercase().as_str() {
                    "png" => Format::Png,
                    "webp" => Format::Webp,
                    other => return Err(format!("unknown format '{other}' (png, webp)")),
                }
            }
            "-O" | "--output" => c.output = Some(value(&key)?),
            "-s" | "--internal-scale" => {
                let _ = value(&key)?;
                c.warnings.push("--internal-scale/-s is ignored; use --pixel-size".into());
            }
            _ => return Err(format!("unknown option '{raw}'")),
        }
    }
    if c.output.is_some() && c.inputs.len() > 1 {
        return Err("--output can only be used with a single input".into());
    }
    Ok(c)
}

/// Output size when no frame is used. Same rules as the Python tool, minus its crash on `-o 800`.
pub fn resolve_size(c: &Cli, src_w: usize, src_h: usize) -> (usize, usize) {
    const DEFAULT_WIDTH: usize = 800;
    let from_width = |w: usize| {
        let h = match c.aspect {
            Some(a) => w as f64 / a,
            None => w as f64 * src_h as f64 / src_w as f64,
        };
        (w, (h.round() as usize).max(1))
    };
    match (c.output_size, c.output_width, c.aspect) {
        (Some((w, Some(h))), _, _) => (w, h),
        (Some((w, None)), _, _) => from_width(w),
        (None, Some(w), _) => from_width(w),
        (None, None, Some(_)) => from_width(DEFAULT_WIDTH),
        (None, None, None) => from_width(DEFAULT_WIDTH),
    }
}

/// `photo.jpg` -> `photo_dithered.png` next to the input.
pub fn default_output(input: &str, fmt: Format) -> String {
    let p = Path::new(input);
    let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("output");
    p.with_file_name(format!("{stem}_dithered.{}", fmt.ext())).to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(s: &[&str]) -> Vec<String> {
        s.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn the_everyday_command_still_parses() {
        let c = parse(&a(&["--output-size", "800x350", "--crop-mode", "interactive", "input.jpg"])).unwrap();
        assert_eq!(c.output_size, Some((800, Some(350))));
        assert_eq!(c.crop_mode, CropMode::Interactive);
        assert_eq!(c.inputs, vec!["input.jpg"]);
    }

    #[test]
    fn equals_syntax_and_short_options() {
        let c = parse(&a(&["-p=bw", "-O", "out.png", "--aspect=21:9", "x.jpg"])).unwrap_err();
        assert!(c.contains("unknown option")); // short options do not take '='
        let c = parse(&a(&["--palette=bw", "-O", "out.png", "--aspect=21:9", "x.jpg"])).unwrap();
        assert_eq!(c.palette, "bw");
        assert!((c.aspect.unwrap() - 21.0 / 9.0).abs() < 1e-12);
    }

    #[test]
    fn errors_are_helpful() {
        assert!(parse(&a(&["--crop-mode", "weird"])).is_err());
        assert!(parse(&a(&["--palette", "nope"])).is_err());
        assert!(parse(&a(&["-o"])).is_err());
        assert!(parse(&a(&["--scale", "0"])).is_err());
        assert!(parse(&a(&["a.jpg", "b.jpg", "-O", "x.png"])).is_err());
    }

    #[test]
    fn width_only_size_no_longer_crashes() {
        let c = parse(&a(&["--output-size", "800", "x.jpg"])).unwrap();
        assert_eq!(resolve_size(&c, 4000, 3000), (800, 600));
    }

    #[test]
    fn size_rules() {
        let mut c = Cli::default();
        assert_eq!(resolve_size(&c, 1200, 800), (800, 533));
        c.aspect = Some(21.0 / 9.0);
        assert_eq!(resolve_size(&c, 1200, 800), (800, 343));
        c.output_width = Some(320);
        assert_eq!(resolve_size(&c, 1200, 800), (320, 137));
        c.output_size = Some((800, Some(350)));
        assert_eq!(resolve_size(&c, 1200, 800), (800, 350));
    }

    #[test]
    fn frame_and_defaults() {
        let c = parse(&a(&["--frame", "default"])).unwrap();
        assert_eq!(c.frame, FrameSel::Builtin);
        assert_eq!(default_output("/a/b/photo.jpg", Format::Png), "/a/b/photo_dithered.png");
        assert_eq!(default_output("photo.jpg", Format::Webp), "photo_dithered.webp");
    }

    #[test]
    fn legacy_internal_scale_is_ignored_with_a_warning() {
        let c = parse(&a(&["-s", "3", "x.jpg"])).unwrap();
        assert_eq!(c.warnings.len(), 1);
    }
}
