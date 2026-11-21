mod io;

#[cfg(feature = "gui")]
mod gui;

use ddn_dither::cli::{self, Cli, CropMode, FrameSel};
use ddn_dither::cropbox::CropBox;
use ddn_dither::frame::Frame;
use ddn_dither::palette;
use ddn_dither::pipeline::{self, Fit, Params};
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cli = match cli::parse(&args) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}\n\nTry 'ddn-dither --help'.");
            return ExitCode::from(2);
        }
    };
    if cli.help {
        print!("{}", cli::USAGE);
        return ExitCode::SUCCESS;
    }
    if cli.version {
        println!("ddn-dither {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }
    for w in &cli.warnings {
        eprintln!("warning: {w}");
    }

    let wants_window = cli.gui || cli.crop_mode == CropMode::Interactive || cli.inputs.is_empty();
    let result = if wants_window { run_window(cli) } else { run_batch(&cli) };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(feature = "gui")]
fn run_window(cli: Cli) -> Result<(), String> {
    gui::run(cli)
}

#[cfg(not(feature = "gui"))]
fn run_window(_cli: Cli) -> Result<(), String> {
    Err("this build has no window (built with --no-default-features); give an input file and use --crop-mode cover or contain".into())
}

fn load_selected_frame(sel: &FrameSel) -> Result<Option<Frame>, String> {
    match sel {
        FrameSel::None => Ok(None),
        FrameSel::Builtin => Ok(Some(Frame::builtin())),
        FrameSel::Name(id) => match Frame::builtin_named(id) {
            Some(f) => Ok(Some(f)),
            None => Err(format!(
                "unknown built-in frame '{id}' (compiled in: {})",
                if Frame::builtin_ids().is_empty() { "none".to_string() } else { Frame::builtin_ids().join(", ") }
            )),
        },
        FrameSel::Path(p) => io::load_frame(p, "Custom").map(Some),
    }
}

fn run_batch(cli: &Cli) -> Result<(), String> {
    let pal = palette::resolve(&cli.palette)?;
    let frame = load_selected_frame(&cli.frame)?;
    if let Some(f) = &frame {
        if cli.debug {
            eprintln!(
                "debug: frame '{}' is {}x{}, window {}x{} at +{},+{}, {}",
                f.name, f.w, f.h, f.hole.w, f.hole.h, f.hole.x, f.hole.y,
                if f.hole_is_rect { "plain rectangle" } else { "art overlaps the window (photo shows through only the transparent pixels)" }
            );
        }
        if f.source_grey_values > 4 {
            eprintln!("note: frame '{}' used {} grey values; converted to 4 tones", f.name, f.source_grey_values);
        }
        if !f.hole_is_rect {
            eprintln!("note: frame '{}' has a non-rectangular window; the photo fills its bounding box", f.name);
        }
        if cli.output_size.is_some() || cli.output_width.is_some() {
            eprintln!("note: with a frame the output size is set by the frame; use --scale instead of --output-size");
        }
    }

    let mut failed = 0;
    for input in &cli.inputs {
        match convert_one(cli, input, &pal, frame.as_ref()) {
            Ok(out) => println!("{input} -> {out}"),
            Err(e) => {
                eprintln!("error: {input}: {e}");
                failed += 1;
            }
        }
    }
    if failed > 0 {
        Err(format!("{failed} of {} file(s) failed", cli.inputs.len()))
    } else {
        Ok(())
    }
}

fn convert_one(cli: &Cli, input: &str, pal: &palette::Palette, frame: Option<&Frame>) -> Result<String, String> {
    let img = io::load_rgb(input)?;
    let gray = img.to_gray();
    let (out_w, out_h) = cli::resolve_size(cli, gray.w, gray.h);

    let mut params = Params {
        fit: Fit::Contain,
        out_w,
        out_h,
        pixel_size: cli.pixel_size,
        frame,
        frame_scale: cli.scale,
        autocontrast: cli.autocontrast,
        gamma: cli.gamma,
        mask: None,
        mask_gamma: 1.0,
    };
    params.fit = match cli.crop_mode {
        CropMode::Cover => {
            let aspect = pipeline::crop_aspect(&params);
            Fit::Cover(CropBox::max_fit(gray.w as f64, gray.h as f64, aspect).to_box())
        }
        CropMode::Contain => Fit::Contain,
        CropMode::PadRepeat => Fit::EdgeRepeat,
        CropMode::Interactive => Fit::Contain,
    };
    let rendered = pipeline::render(&gray, &params);
    let out = cli.output.clone().unwrap_or_else(|| cli::default_output(input, cli.format));
    io::save(&out, &rendered, pal)?;
    if cli.debug {
        eprintln!(
            "debug: {}x{} source -> grid {}x{} x{} -> {}x{}",
            gray.w, gray.h, rendered.grid_w, rendered.grid_h, rendered.scale, rendered.out_w, rendered.out_h
        );
    }
    Ok(out)
}
