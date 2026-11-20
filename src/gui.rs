//! The window (eframe/egui): open or drop an image, drag the crop box, pick size / aspect /
//! palette / frame, see the dithered result live, save.
//!
//! All geometry and image processing lives in the `ddn_dither` library and is unit-tested
//! there; this file only wires it to egui widgets.

use std::path::Path;

use eframe::egui::{self, pos2, vec2, Color32, Rect, Sense, Stroke, TextureHandle, TextureOptions};

use ddn_dither::cli::{self, Cli, CropMode, Format, FrameSel};
use ddn_dither::cropbox::{CropBox, Handle};
use ddn_dither::frame::Frame;
use ddn_dither::gray::Plane;
use ddn_dither::palette::{self, Palette};
use ddn_dither::pipeline::{self, Fit, Params, Rendered};
use ddn_dither::resample::{resize_plane, resize_rgb, BoxF};

use crate::io;

/// While dragging, render from a copy of the photo this large (longest side) for speed.
const PROXY_MAX: usize = 1200;
/// Longest side of the texture used to show the source photo.
const DISPLAY_MAX: usize = 2048;

/// Overlay lines drawn on top of the crop box to help alignment.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Guides {
    Off,
    Thirds,
    Cross,
    Both,
}

impl Guides {
    const ALL: [Guides; 4] = [Guides::Off, Guides::Thirds, Guides::Cross, Guides::Both];

    fn next(self) -> Guides {
        let i = Guides::ALL.iter().position(|&g| g == self).unwrap_or(0);
        Guides::ALL[(i + 1) % Guides::ALL.len()]
    }

    fn name(self) -> &'static str {
        match self {
            Guides::Off => "Off",
            Guides::Thirds => "Thirds",
            Guides::Cross => "Cross",
            Guides::Both => "Thirds + cross",
        }
    }
}

const ASPECT_PRESETS: [(&str, f64); 8] = [
    ("Custom", 0.0),
    ("16:7", 16.0 / 7.0),
    ("21:9", 21.0 / 9.0),
    ("16:9", 16.0 / 9.0),
    ("3:2", 3.0 / 2.0),
    ("4:3", 4.0 / 3.0),
    ("1:1", 1.0),
    ("8:7", 8.0 / 7.0),
];

struct Source {
    path: String,
    w: usize,
    h: usize,
    gray: Plane,
    /// Smaller grey copy for fast previews while dragging: (plane, x scale, y scale).
    proxy: Option<(Plane, f64, f64)>,
    tex: TextureHandle,
}

struct DragState {
    handle: Handle,
    start: CropBox,
    ptr: (f64, f64),
}

/// What the preview render depends on, except the palette: switching palettes only
/// needs a recolour of the cached level grid, not a new Lanczos+dither pass.
#[derive(Clone, PartialEq)]
struct RenderKey {
    crop: [f64; 4],
    out: (usize, usize),
    pixel_size: usize,
    frame: Option<usize>,
    scale: usize,
    autocontrast: bool,
    contrast: f64,
    fit: usize,
    proxy: bool,
    source_id: u64,
}

struct Preview {
    tex: TextureHandle,
    grid: (usize, usize),
    levels: Vec<u8>,
    palette: Palette,
    /// Output pixels per dither pixel (rendered scale), so 100% shows the file 1:1.
    scale: usize,
    out: (usize, usize),
    /// Exact encoded output size; None while a fast proxy render is shown.
    size: Option<u64>,
}

struct App {
    cli: Cli,
    source: Option<Source>,
    source_id: u64,
    first_load: bool,

    crop: CropBox,
    last_aspect: f64,
    drag: Option<DragState>,

    aspect_idx: usize,
    out_w: usize,
    out_h: usize,
    pixel_size: usize,
    fit_mode: usize,
    autocontrast: bool,
    contrast: f64,
    snap_pixels: bool,
    guides: Guides,
    format: Format,

    frames: Vec<Frame>,
    frame_idx: Option<usize>,
    frame_scale: usize,

    pal_idx: usize,
    custom_pal: [String; 4],
    palette: Palette,
    pal_error: Option<String>,

    preview: Option<Preview>,
    last_key: Option<RenderKey>,
    /// None = fit into the pane; Some(z) = z screen pixels per output pixel (1.0 = 100%).
    preview_zoom: Option<f32>,
    pv_pan: (f32, f32),
    /// Effective fit scale of the preview, for the toolbar indicator (updated each frame).
    pv_fit: f32,

    output_override: Option<String>,
    exit_on_save: bool,
    status: String,
    /// Native Wayland gets no file-drop events from winit 0.29 (X11/Windows/macOS do).
    dnd_native_wayland: bool,
}

fn wayland_native() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some()
        && !std::env::var("WINIT_UNIX_BACKEND").map(|v| v.eq_ignore_ascii_case("x11")).unwrap_or(false)
}

pub fn run(cli: Cli) -> Result<(), String> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("DDN Dither")
            .with_inner_size([1180.0, 740.0])
            .with_min_inner_size([820.0, 520.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };
    eframe::run_native(
        "DDN Dither",
        options,
        Box::new(move |cc| Ok(Box::new(App::new(cc, cli)))),
    )
    .map_err(|e| e.to_string())
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>, cli: Cli) -> App {
        let mut status = String::new();

        // Frames: built-ins first (all assets/frame_<id>.bin), then the frames folders.
        let mut frames: Vec<Frame> = Frame::builtins().collect();
        let (found, warnings) = io::discover_frames();
        frames.extend(found);
        if !warnings.is_empty() {
            status = format!("Skipped frame(s): {}", warnings.join("; "));
        }
        let mut frame_idx = None;
        match &cli.frame {
            FrameSel::None => {}
            FrameSel::Builtin => frame_idx = Some(0),
            FrameSel::Name(id) => match Frame::builtin_named(id) {
                Some(f) => frame_idx = frames.iter().position(|g| g.name == f.name),
                None => status = format!("unknown built-in frame '{id}'"),
            },
            FrameSel::Path(p) => match io::load_frame(p, "Custom") {
                Ok(f) => {
                    frames.push(f);
                    frame_idx = Some(frames.len() - 1);
                }
                Err(e) => status = e,
            },
        }

        // Palette from --palette (name, or custom hex list).
        let (pal_idx, custom_pal, palette) = match palette::by_name(&cli.palette) {
            Some(p) => (
                palette::names().iter().position(|n| *n == cli.palette.to_ascii_lowercase()).unwrap_or(0),
                [(); 4].map(|()| String::new()),
                p,
            ),
            None => (palette::BUILTIN.len(), split_palette_spec(&cli.palette), palette::resolve(&cli.palette).unwrap_or(palette::BUILTIN[0].1)),
        };

        let mut app = App {
            source: None,
            source_id: 0,
            first_load: true,
            crop: CropBox { x: 0.0, y: 0.0, w: 1.0, h: 1.0 },
            last_aspect: 1.0,
            drag: None,
            aspect_idx: 0,
            out_w: 800,
            out_h: 350,
            pixel_size: cli.pixel_size,
            fit_mode: fit_mode_of(cli.crop_mode),
            autocontrast: cli.autocontrast,
            contrast: cli.contrast,
            snap_pixels: false,
            guides: Guides::Thirds,
            format: cli.format,
            frames,
            frame_idx,
            frame_scale: cli.scale,
            pal_idx,
            custom_pal,
            palette,
            pal_error: None,
            preview: None,
            last_key: None,
            preview_zoom: None,
            pv_pan: (0.0, 0.0),
            pv_fit: 1.0,
            output_override: cli.output.clone(),
            exit_on_save: !cli.inputs.is_empty(),
            status,
            cli,
            dnd_native_wayland: wayland_native(),
        };
        if let Some(first) = app.cli.inputs.first().cloned() {
            app.load(&cc.egui_ctx, &first);
        }
        app
    }

    // ---------------------------------------------------------------- state helpers

    fn frame(&self) -> Option<&Frame> {
        self.frame_idx.and_then(|i| self.frames.get(i))
    }

    /// Aspect ratio the crop must have right now.
    fn aspect(&self) -> f64 {
        match self.frame() {
            Some(f) => f.hole_aspect(),
            None => self.out_w.max(1) as f64 / self.out_h.max(1) as f64,
        }
    }

    fn load(&mut self, ctx: &egui::Context, path: &str) {
        let rgb = match io::load_rgb(path) {
            Ok(i) => i,
            Err(e) => {
                self.status = e;
                return;
            }
        };
        let (w, h) = (rgb.w, rgb.h);
        let gray = rgb.to_gray();
        let longest = w.max(h);

        let shown = if longest > DISPLAY_MAX {
            let s = DISPLAY_MAX as f64 / longest as f64;
            let (dw, dh) = (((w as f64 * s).round() as usize).max(1), ((h as f64 * s).round() as usize).max(1));
            resize_rgb(&rgb, dw, dh, BoxF::full(w, h))
        } else {
            rgb
        };
        let tex = ctx.load_texture(
            "source",
            egui::ColorImage::from_rgb([shown.w, shown.h], &shown.data),
            TextureOptions::LINEAR,
        );

        let proxy = if longest > PROXY_MAX {
            let s = PROXY_MAX as f64 / longest as f64;
            let (pw, ph) = (((w as f64 * s).round() as usize).max(1), ((h as f64 * s).round() as usize).max(1));
            Some((resize_plane(&gray, pw, ph, BoxF::full(w, h)), pw as f64 / w as f64, ph as f64 / h as f64))
        } else {
            None
        };

        if self.first_load {
            // Command-line size options (e.g. --output-size 800x350) apply to the first image.
            let (ow, oh) = cli::resolve_size(&self.cli, w, h);
            self.out_w = ow;
            self.out_h = oh;
            let a = ow as f64 / oh as f64;
            self.aspect_idx = ASPECT_PRESETS
                .iter()
                .position(|(_, p)| *p > 0.0 && (p - a).abs() < 0.002)
                .unwrap_or(0);
            self.first_load = false;
        }

        self.source_id += 1;
        self.source = Some(Source { path: path.to_string(), w, h, gray, proxy, tex });
        self.crop = CropBox::max_fit(w as f64, h as f64, self.aspect());
        self.last_aspect = self.aspect();
        self.drag = None;
        self.last_key = None;
        self.status = format!("Loaded {path} ({w}x{h})");
    }

    /// Keep the crop box valid when the target aspect ratio changes.
    fn sync_crop_aspect(&mut self) {
        let a = self.aspect();
        if (a - self.last_aspect).abs() > 1e-9 {
            if let Some(src) = &self.source {
                self.crop = self.crop.with_aspect(a, src.w as f64, src.h as f64);
            }
            self.last_aspect = a;
        }
    }

    fn params<'a>(&'a self, crop: BoxF) -> Params<'a> {
        Params {
            fit: match self.fit_mode {
                1 => Fit::Contain,
                2 => Fit::EdgeRepeat,
                _ => Fit::Cover(crop),
            },
            out_w: self.out_w,
            out_h: self.out_h,
            pixel_size: self.pixel_size,
            frame: self.frame(),
            frame_scale: self.frame_scale,
            autocontrast: self.autocontrast,
            contrast: self.contrast,
        }
    }

    /// Render from the proxy (fast, while dragging) or the full-resolution photo.
    fn render_levels(&self, use_proxy: bool) -> Option<Rendered> {
        let src = self.source.as_ref()?;
        let b = self.crop.to_box();
        match (&src.proxy, use_proxy) {
            (Some((plane, sx, sy)), true) => {
                let pb = BoxF { x0: b.x0 * sx, y0: b.y0 * sy, x1: b.x1 * sx, y1: b.y1 * sy };
                Some(pipeline::render(plane, &self.params(pb)))
            }
            _ => Some(pipeline::render(&src.gray, &self.params(b))),
        }
    }

    fn render_key(&self, use_proxy: bool) -> RenderKey {
        RenderKey {
            crop: [self.crop.x, self.crop.y, self.crop.w, self.crop.h],
            out: (self.out_w, self.out_h),
            pixel_size: self.pixel_size,
            frame: self.frame_idx,
            scale: self.frame_scale,
            autocontrast: self.autocontrast,
            contrast: self.contrast,
            fit: self.fit_mode,
            proxy: use_proxy,
            source_id: self.source_id,
        }
    }

    fn refresh_preview(&mut self, ctx: &egui::Context) {
        if self.source.is_none() {
            return;
        }
        let use_proxy = self.drag.is_some();
        let key = self.render_key(use_proxy);
        if self.last_key.as_ref() == Some(&key) {
            if let Some(pv) = &mut self.preview {
                if pv.palette != self.palette {
                    let img = egui::ColorImage::from_rgb(
                        [pv.grid.0, pv.grid.1],
                        &palette::colour_levels(&pv.levels, &self.palette),
                    );
                    pv.tex.set(img, TextureOptions::NEAREST);
                    pv.palette = self.palette;
                    let full = pipeline::Rendered {
                        grid_w: pv.grid.0,
                        grid_h: pv.grid.1,
                        levels: pv.levels.clone(),
                        scale: pv.scale,
                        out_w: pv.out.0,
                        out_h: pv.out.1,
                    };
                    pv.size = Some(io::encoded_size(self.format, &full, &self.palette));
                    ctx.request_repaint();
                }
            }
            return;
        }
        if let Some(r) = self.render_levels(use_proxy) {
            let rgb = r.rgb_native(&self.palette);
            let tex = ctx.load_texture(
                "preview",
                egui::ColorImage::from_rgb([r.grid_w, r.grid_h], &rgb),
                TextureOptions::NEAREST,
            );
            let out = (r.out_w, r.out_h);
            let size = (!use_proxy).then(|| io::encoded_size(self.format, &r, &self.palette));
            self.preview = Some(Preview {
                tex,
                grid: (r.grid_w, r.grid_h),
                levels: r.levels,
                palette: self.palette,
                scale: r.scale,
                out,
                size,
            });
            self.last_key = Some(key);
            ctx.request_repaint();
        }
    }

    // ---------------------------------------------------------------- saving

    fn default_output_path(&self) -> Option<String> {
        let src = self.source.as_ref()?;
        Some(self.output_override.clone().unwrap_or_else(|| cli::default_output(&src.path, self.format)))
    }

    fn save_to(&mut self, path: &str) -> bool {
        let Some(src) = self.source.as_ref() else { return false };
        let r = pipeline::render(&src.gray, &self.params(self.crop.to_box()));
        match io::save(path, &r, &self.palette) {
            Ok(()) => {
                self.status = format!("Saved {path} ({}x{})", r.out_w, r.out_h);
                true
            }
            Err(e) => {
                self.status = e;
                false
            }
        }
    }

    fn quick_save(&mut self, ctx: &egui::Context) {
        if let Some(path) = self.default_output_path() {
            if self.save_to(&path) && self.exit_on_save {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    fn save_as(&mut self) {
        let Some(src) = self.source.as_ref() else { return };
        let suggested = cli::default_output(&src.path, self.format);
        let p = Path::new(&suggested);
        let mut dialog = rfd::FileDialog::new().add_filter("PNG / WebP", &["png", "webp"]);
        if let Some(dir) = p.parent().filter(|d| !d.as_os_str().is_empty()) {
            dialog = dialog.set_directory(dir);
        }
        if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
            dialog = dialog.set_file_name(name);
        }
        if let Some(path) = dialog.save_file() {
            self.save_to(&path.to_string_lossy());
        }
    }

    fn reset_settings(&mut self) {
        self.frame_idx = match &self.cli.frame {
            FrameSel::None => None,
            FrameSel::Builtin => Some(0),
            FrameSel::Name(id) => Frame::builtin_named(id).and_then(|f| self.frames.iter().position(|g| g.name == f.name)),
            FrameSel::Path(p) => {
                let stem = Path::new(p).file_stem().and_then(|n| n.to_str()).unwrap_or(p);
                self.frames.iter().position(|g| g.name == stem)
            }
        };
        if let Some(pi) = palette::by_name(&self.cli.palette) {
            self.pal_idx = palette::names().iter().position(|n| *n == self.cli.palette.to_ascii_lowercase()).unwrap_or(0);
            self.custom_pal = [(); 4].map(|()| String::new());
            self.palette = pi;
        } else {
            self.pal_idx = palette::BUILTIN.len();
            self.custom_pal = split_palette_spec(&self.cli.palette);
            self.palette = palette::resolve(&self.cli.palette).unwrap_or(palette::BUILTIN[0].1);
        }
        self.pal_error = None;
        self.pixel_size = self.cli.pixel_size;
        self.fit_mode = fit_mode_of(self.cli.crop_mode);
        self.autocontrast = self.cli.autocontrast;
        self.contrast = self.cli.contrast;
        self.snap_pixels = false;
        self.frame_scale = self.cli.scale;
        self.format = self.cli.format;
        self.guides = Guides::Thirds;
        if let Some(src) = &self.source {
            let (ow, oh) = cli::resolve_size(&self.cli, src.w, src.h);
            self.out_w = ow;
            self.out_h = oh;
            let a = ow as f64 / oh as f64;
            self.aspect_idx = ASPECT_PRESETS.iter().position(|(_, p)| *p > 0.0 && (p - a).abs() < 0.002).unwrap_or(0);
            self.last_aspect = self.aspect();
            self.crop = CropBox::max_fit(src.w as f64, src.h as f64, self.aspect());
        } else {
            self.out_w = 800;
            self.out_h = 350;
            self.aspect_idx = 0;
        }
        self.drag = None;
        self.last_key = None;
        self.status = "Settings reset to startup values.".to_string();
    }

    /// With integer scaling on, keep the output size exactly as set and instead snap the
    /// pixel size to the nearest value that divides both sides, so every dither block is a
    /// whole, uncropped N x N and the resolution the user asked for is preserved.
    fn snap_pixels_to_output(&mut self) {
        let eff = if self.pixel_size == 0 { pipeline::auto_pixel_size(self.out_w) } else { self.pixel_size.max(1) };
        if self.out_w.is_multiple_of(eff) && self.out_h.is_multiple_of(eff) {
            return;
        }
        self.pixel_size = snap_pixel_size(self.out_w, self.out_h, eff);
    }

    fn open_dialog(&mut self, ctx: &egui::Context) {
        let picked = rfd::FileDialog::new()
            .add_filter("Images", &["png", "jpg", "jpeg", "webp", "bmp", "gif"])
            .pick_file();
        if let Some(p) = picked {
            self.load(ctx, &p.to_string_lossy());
        }
    }

    fn load_frame_dialog(&mut self) {
        let Some(p) = rfd::FileDialog::new().add_filter("Frame (PNG with transparent window)", &["png"]).pick_file()
        else {
            return;
        };
        match io::load_frame(&p.to_string_lossy(), "Custom") {
            Ok(f) => {
                self.frames.push(f);
                self.frame_idx = Some(self.frames.len() - 1);
            }
            Err(e) => self.status = e,
        }
    }

    // ---------------------------------------------------------------- UI

    fn options_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.add_space(4.0);
        if ui.button("Open image…").clicked() {
            self.open_dialog(ctx);
        }
        if let Some(src) = &self.source {
            ui.add(egui::Label::new(format!("{}  ({}×{})", file_name(&src.path), src.w, src.h)).truncate());
        }
        ui.separator();

        // ---- frame
        ui.strong("Frame");
        egui::ComboBox::from_id_salt("frame")
            .selected_text(self.frame().map(|f| pretty_frame_name(&f.name)).unwrap_or_else(|| "None".to_string()))
            .width(220.0)
            .show_ui(ui, |ui| {
                ui.set_min_width(200.0);
                ui.selectable_value(&mut self.frame_idx, None, "None");
                let group_of = |i: usize| -> String {
                    let g = &self.frames[i].group;
                    if g.is_empty() { "Built-in".to_string() } else { g.clone() }
                };
                let mut groups: Vec<String> = Vec::new();
                for i in 0..self.frames.len() {
                    let g = group_of(i);
                    if !groups.contains(&g) {
                        groups.push(g);
                    }
                }
                groups.sort_by(|a, b| group_rank(a).cmp(&group_rank(b)).then_with(|| a.cmp(b)));
                for g in &groups {
                    ui.separator();
                    ui.strong(pretty_frame_name(g));
                    for i in 0..self.frames.len() {
                        if &group_of(i) == g {
                            let name = pretty_frame_name(&self.frames[i].name);
                            ui.selectable_value(&mut self.frame_idx, Some(i), name);
                        }
                    }
                }
            })
            .response
            .on_hover_text(
                "A frame is four-tone art with a transparent window for the photo. It sets the output size and locks the crop to the window's aspect. Frames are grouped by the folder they came from: assets/ is Built-in, an assets/<sub>/ or frames/<sub>/ subfolder is its own group, frames/ top level is Custom.",
            );
        if ui
            .small_button("Load frame file…")
            .on_hover_text(
                "PNG with a transparent window. You can also put frame PNGs in frames/ next to the executable or ~/.config/ddn-dither/frames/.",
            )
            .clicked()
        {
            self.load_frame_dialog();
        }
        // Copy what we need out of the frame so `self` is free to be edited below.
        let info = self.frame().map(|f| (f.hole.w, f.hole.h, f.w, f.h, f.source_grey_values, f.hole_is_rect));
        let framed = info.is_some();
        if let Some((hole_w, hole_h, fw, fh, greys, hole_is_rect)) = info {
            ui.add(egui::Label::new(format!("Window {hole_w}×{hole_h} px, frame {fw}×{fh} px")).truncate());
            if greys > 4 {
                ui.add(egui::Label::new(egui::RichText::new(format!("Converted from {greys} grey values to 4 tones.")).color(Color32::from_rgb(200, 140, 0))).truncate());
            }
            if !hole_is_rect {
                ui.add(egui::Label::new(egui::RichText::new("Window is not a plain rectangle; the photo fills its bounding box.").color(Color32::from_rgb(200, 140, 0))).truncate());
            }
            ui.add(
                egui::Slider::new(&mut self.frame_scale, 1..=16)
                    .custom_formatter(|n, _| format!("×{n:.0}"))
                    .text("Scale"),
            )
            .on_hover_text("Integer upscale of the framed image. ×4 makes 640×576 for the built-in frame.");
            let s = self.frame_scale.max(1);
            ui.add(egui::Label::new(format!("Output {}×{} px (set by the frame)", fw * s, fh * s)).truncate());
        }
        ui.separator();

        // ---- size / aspect
        ui.add_enabled_ui(!framed, |ui| {
            ui.strong("Size & crop");
            let before_idx = self.aspect_idx;
            egui::ComboBox::from_id_salt("aspect")
                .selected_text(format!("Aspect: {}", ASPECT_PRESETS[self.aspect_idx].0))
                .show_ui(ui, |ui| {
                    for (i, (name, _)) in ASPECT_PRESETS.iter().enumerate() {
                        ui.selectable_value(&mut self.aspect_idx, i, *name);
                    }
                })
                .response
                .on_hover_text(
                    "The crop box always has exactly this aspect ratio. With a frame, the frame's window wins.",
                );
            if self.aspect_idx != before_idx && self.aspect_idx != 0 {
                let a = ASPECT_PRESETS[self.aspect_idx].1;
                self.out_h = ((self.out_w as f64 / a).round() as usize).max(1);
            }
            let (ow, oh, old_ps, snap_before) = (self.out_w, self.out_h, self.pixel_size, self.snap_pixels);
            ui.horizontal(|ui| {
                ui.label("W");
                ui.add(egui::DragValue::new(&mut self.out_w).range(1..=16384));
                ui.label("H");
                ui.add(egui::DragValue::new(&mut self.out_h).range(1..=16384));
                if ui.small_button("⇄").on_hover_text("Swap width and height").clicked() {
                    std::mem::swap(&mut self.out_w, &mut self.out_h);
                    let a = self.out_w.max(1) as f64 / self.out_h.max(1) as f64;
                    self.aspect_idx = ASPECT_PRESETS
                        .iter()
                        .position(|(_, p)| *p > 0.0 && (p - a).abs() < 0.002)
                        .unwrap_or(0);
                }
            })
            .response
            .on_hover_text("Output size in pixels. With an aspect preset, changing one side adjusts the other.");
            if self.aspect_idx != 0 {
                let a = ASPECT_PRESETS[self.aspect_idx].1;
                if self.out_w != ow {
                    self.out_h = ((self.out_w as f64 / a).round() as usize).max(1);
                } else if self.out_h != oh {
                    self.out_w = ((self.out_h as f64 * a).round() as usize).max(1);
                }
            }
            if self.snap_pixels {
                let eff = if self.pixel_size == 0 { pipeline::auto_pixel_size(self.out_w) } else { self.pixel_size.max(1) };
                ui.horizontal_wrapped(|ui| {
                    ui.label("Pixel size");
                    if ui.selectable_label(self.pixel_size == 0, "auto").clicked() {
                        self.pixel_size = 0;
                    }
                    for cand in 1..=32 {
                        if self.out_w.is_multiple_of(cand) && self.out_h.is_multiple_of(cand) {
                            let selected = self.pixel_size == cand;
                            let text = if eff == cand { format!("{cand} ✓") } else { cand.to_string() };
                            if ui.selectable_label(selected, text).clicked() {
                                self.pixel_size = cand;
                            }
                        }
                    }
                    ui.weak(format!("{gw}×{gh} grid", gw = self.out_w.div_ceil(eff), gh = self.out_h.div_ceil(eff)));
                })
                .response
                .on_hover_text(
                    "Output pixels per dithered pixel; Integer scale keeps only the sizes that divide the output exactly, so every dither block stays a whole, uncropped N×N and the check mark marks the effective one (auto falls back to it). The readout shows the resulting grid size (pixels across = grid width). The Game Boy Camera shot a 128×112 grid; bigger = blockier.",
                );
            } else {
                ui.horizontal(|ui| {
                    ui.label("Pixel size");
                    ui.add(
                        egui::Slider::new(&mut self.pixel_size, 0..=32)
                            .custom_formatter(|n, _| if n == 0.0 { "auto".to_string() } else { format!("{n:.0}") })
                            .custom_parser(|s| {
                                if s.trim().eq_ignore_ascii_case("auto") { Some(0.0) } else { s.trim().parse::<f64>().ok() }
                            }),
                    );
                    let ps = if self.pixel_size == 0 { pipeline::auto_pixel_size(self.out_w) } else { self.pixel_size.max(1) };
                    if self.pixel_size == 0 {
                        ui.weak(format!("→ {ps}"));
                    }
                    ui.weak(format!("{gw}×{gh} grid", gw = self.out_w.div_ceil(ps), gh = self.out_h.div_ceil(ps)));
                    let (cw, ch) = (self.out_w % ps, self.out_h % ps);
                    if cw != 0 || ch != 0 {
                        ui.weak(format!("(crops {cw}×{ch} px)"));
                    }
                })
                .response
                .on_hover_text(
                    "Output pixels per dithered pixel; the readout shows the resulting grid size (pixels across = grid width) and whether any row/column of the output would be cropped. The Game Boy Camera shot a 128×112 grid; bigger = blockier.",
                );
            }
            ui.checkbox(&mut self.snap_pixels, "Integer scale")
                .on_hover_text(
                    "Replaces the pixel-size slider while it is on: only sizes that divide your output exactly are offered, so every dither block stays a whole, uncropped N×N and the output size is never changed (turning it off brings the free slider back).",
                )
                .changed();
            if self.snap_pixels
                && (self.pixel_size != old_ps || self.out_w != ow || self.out_h != oh || self.snap_pixels != snap_before)
            {
                self.snap_pixels_to_output();
            }
        });
        if ui
            .button(format!("Guides: {}", self.guides.name()))
            .on_hover_text("Guide lines over the crop box to help alignment: rule-of-thirds lines and/or a full-size crosshair through the crop centre.")
            .clicked()
        {
            self.guides = self.guides.next();
        }
        ui.horizontal(|ui| {
            ui.label("Fit");
            for (i, name) in ["Crop", "Pad", "Edge pad"].iter().enumerate() {
                if ui.selectable_label(self.fit_mode == i, *name).clicked() {
                    self.fit_mode = i;
                }
            }
        })
        .response
        .on_hover_text("How the picture fills the output. Crop: centre-crop to the output aspect. Pad: scale the whole picture in and fill the rest with the darkest tone. Edge pad: fill the rest by repeating the picture's outermost rows and columns, so the image looks smeared out to the edges.");
        ui.separator();

        // ---- contrast
        ui.strong("Contrast");
        ui.checkbox(&mut self.autocontrast, "Auto contrast")
            .on_hover_text("Stretches the picture's histogram before dithering, like the original tool (top 10% clipped).");
        ui.horizontal(|ui| {
            ui.label("Contrast");
            ui.add(
                egui::Slider::new(&mut self.contrast, 0.25..=4.0)
                    .custom_formatter(|n, _| format!("{n:.2}"))
                    .custom_parser(|s| s.trim().parse::<f64>().ok()),
            );
            if ui.small_button("Reset").on_hover_text("Back to no extra contrast (1.00)").clicked() {
                self.contrast = 1.0;
            }
        })
        .response
        .on_hover_text("Manual contrast around mid-grey, applied after auto contrast. 1.00 = no change.");
        ui.separator();

        // ---- palette
        ui.strong("Palette");
        let names = palette::names();
        let custom_idx = names.len();
        let before_pal = self.pal_idx;
        egui::ComboBox::from_id_salt("palette")
            .selected_text(if self.pal_idx < custom_idx { names[self.pal_idx].to_string() } else { "custom".to_string() })
            .show_ui(ui, |ui| {
                ui.set_min_width(210.0);
                for (i, n) in names.iter().enumerate() {
                    ui.horizontal(|ui| {
                        swatch_strip(ui, &palette::BUILTIN[i].1);
                        ui.selectable_value(&mut self.pal_idx, i, *n);
                    });
                }
                ui.horizontal(|ui| {
                    swatch_strip(ui, &self.palette);
                    ui.selectable_value(&mut self.pal_idx, custom_idx, "custom");
                });
            })
            .response
            .on_hover_text("Recolour any time — changing the palette never re-dithers.");
        if self.pal_idx < custom_idx {
            self.palette = palette::BUILTIN[self.pal_idx].1;
            self.pal_error = None;
            if before_pal != self.pal_idx {
                self.custom_pal = palette::BUILTIN[self.pal_idx]
                    .1
                    .iter()
                    .map(|c| format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2]))
                    .collect::<Vec<_>>()
                    .try_into()
                    .unwrap_or_default();
            }
        } else {
            ui.label("Four hex colours, darkest first:")
                .on_hover_text("One box per colour, in the level order of the swatch strip below; 3-digit shorthand like #0f0 works.");
            let mut edited = before_pal != self.pal_idx;
            ui.horizontal_wrapped(|ui| {
                for text in self.custom_pal.iter_mut() {
                    let fill = palette::parse_hex(text).ok();
                    let (rect, _) = ui.allocate_exact_size(vec2(14.0, 17.0), Sense::hover());
                    let c = match fill {
                        Some(c) => Color32::from_rgb(c[0], c[1], c[2]),
                        None => Color32::from_gray(35),
                    };
                    ui.painter().rect_filled(rect, 2.0, c);
                    if fill.is_none() {
                        ui.painter().rect_stroke(rect, 2.0, Stroke::new(1.0_f32, Color32::from_rgb(200, 60, 60)));
                    }
                    edited |= ui
                        .add(egui::TextEdit::singleline(text).desired_width(66.0).font(egui::TextStyle::Monospace))
                        .changed();
                }
            });
            if edited {
                match palette::resolve(&self.custom_pal.join(",")) {
                    Ok(p) => {
                        self.palette = p;
                        self.pal_error = None;
                    }
                    Err(e) => self.pal_error = Some(e),
                }
            }
            if let Some(e) = &self.pal_error {
                ui.colored_label(Color32::from_rgb(200, 60, 60), e);
            }
        }
        ui.horizontal(|ui| {
            for c in self.palette {
                let (rect, _) = ui.allocate_exact_size(vec2(36.0, 18.0), Sense::hover());
                ui.painter().rect_filled(rect, 2.0, Color32::from_rgb(c[0], c[1], c[2]));
            }
        });
        ui.separator();

        // ---- output
        ui.strong("Output");
        egui::ComboBox::from_id_salt("format")
            .selected_text(match self.format {
                Format::Png => "PNG (indexed)",
                Format::Webp => "WebP (lossless)",
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut self.format, Format::Png, "PNG (indexed)");
                ui.selectable_value(&mut self.format, Format::Webp, "WebP (lossless)");
            })
            .response
            .on_hover_text("PNG output is 2-bit indexed (a few KB). WebP is lossless full-colour and bigger.");
        let have = self.source.is_some();
        ui.horizontal(|ui| {
            let label = if self.exit_on_save { "Save & quit" } else { "Save" };
            if ui
                .add_enabled(have, egui::Button::new(label))
                .on_hover_text("Saves to the destination shown below, replacing an earlier file of the same name.")
                .clicked()
            {
                self.quick_save(ctx);
            }
            if ui
                .add_enabled(have, egui::Button::new("Save as…"))
                .on_hover_text("Opens a file dialog to pick the destination.")
                .clicked()
            {
                self.save_as();
            }
        });
        if have {
            let dest = match &self.output_override {
                Some(p) => p.clone(),
                None => cli::default_output(&self.source.as_ref().map(|s| s.path.clone()).unwrap_or_default(), self.format),
            };
            ui.add(egui::Label::new(egui::RichText::new(format!("→ {}", file_name(&dest))).weak()).truncate());
        }
        ui.separator();
        ui.add(egui::Label::new(self.status.as_str()).truncate());
        ui.add_space(6.0);
        let hint = if self.dnd_native_wayland {
            "Drag inside the box to move it; drag a handle to resize; scroll the wheel over the box to grow or shrink it (opposite sides and the aspect stay put). Esc closes."
        } else {
            "Drag inside the box to move it; drag a handle to resize; scroll the wheel over the box to grow or shrink it (opposite sides and the aspect stay put). Drop another image anywhere to switch. Esc closes."
        };
        ui.weak(hint);
        ui.add_space(10.0);
        ui.vertical_centered_justified(|ui| {
            if ui
                .small_button("Reset all settings")
                .on_hover_text("Return every option to its startup value: size, pixel size, contrast, frame selection and scale, palette, crop box.")
                .clicked()
            {
                self.reset_settings();
            }
        });
    }

    fn main_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let hovering = ctx.input(|i| !i.raw.hovered_files.is_empty());
        let Some((tex_id, iw, ih)) = self.source.as_ref().map(|s| (s.tex.id(), s.w as f64, s.h as f64)) else {
            let prompt = if self.dnd_native_wayland {
                "Drag & drop does not work on Wayland yet — click “Open image…”"
            } else {
                "Drop an image here, or click “Open image…”"
            };
            ui.centered_and_justified(|ui| {
                ui.heading(if hovering { "Release to open" } else { prompt });
            });
            return;
        };

        let avail = ui.available_size();
        const IMG_PAD: f32 = 14.0;
        let source_h = (avail.y * 0.58).max(160.0 + 2.0 * IMG_PAD);
        let (resp, painter) = ui.allocate_painter(vec2(avail.x, source_h), Sense::click_and_drag());
        let area = resp.rect.shrink2(vec2(8.0, IMG_PAD));
        let scale = (area.width() as f64 / iw).min(area.height() as f64 / ih).max(1e-6);
        let img_rect = Rect::from_center_size(area.center(), vec2((iw * scale) as f32, (ih * scale) as f32));
        painter.image(tex_id, img_rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);

        let to_img = |p: egui::Pos2| ((p.x - img_rect.min.x) as f64 / scale, (p.y - img_rect.min.y) as f64 / scale);
        let to_screen = |x: f64, y: f64| pos2(img_rect.min.x + (x * scale) as f32, img_rect.min.y + (y * scale) as f32);

        if self.fit_mode == 0 {
            // ---- interaction
            let tol = 10.0 / scale;
            let aspect = self.aspect();
            let min_w = 24.0_f64.min(iw / 4.0);

            if self.drag.is_none() {
                if let Some(pos) = resp.hover_pos() {
                    let (ix, iy) = to_img(pos);
                    if let Some(h) = self.crop.hit(ix, iy, tol) {
                        ctx.set_cursor_icon(cursor_for(h, false));
                    }
                    let dz = ctx.input(|i| i.raw_scroll_delta.y);
                    if dz != 0.0 && self.crop.hit(ix, iy, 0.0).is_some() {
                        self.crop = self.crop.zoomed((dz as f64 / 250.0).exp(), aspect, iw, ih, min_w);
                    }
                }
            }
            if resp.drag_started() {
                if let Some(pos) = resp.interact_pointer_pos() {
                    let (ix, iy) = to_img(pos);
                    if let Some(h) = self.crop.hit(ix, iy, tol) {
                        self.drag = Some(DragState { handle: h, start: self.crop, ptr: (ix, iy) });
                    }
                }
            }
            if resp.dragged() {
                if let (Some(d), Some(pos)) = (self.drag.as_ref(), resp.interact_pointer_pos()) {
                    ctx.set_cursor_icon(cursor_for(d.handle, true));
                    let (ix, iy) = to_img(pos);
                    self.crop = CropBox::drag(&d.start, d.handle, d.ptr, (ix, iy), aspect, iw, ih, min_w);
                }
            }
            if resp.drag_stopped() {
                self.drag = None;
            }

            // ---- overlay
            let cb = self.crop;
            let crop_rect = Rect::from_min_max(to_screen(cb.x, cb.y), to_screen(cb.right(), cb.bottom()));
            let dim = Color32::from_black_alpha(140);
            painter.rect_filled(img_rect, 0.0, dim);
            painter
                .with_clip_rect(crop_rect)
                .image(tex_id, img_rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            painter.rect_stroke(crop_rect, 0.0, Stroke::new(2.0_f32, Color32::WHITE));
            let c = crop_rect.center();
            for p in [
                crop_rect.left_top(),
                crop_rect.right_top(),
                crop_rect.left_bottom(),
                crop_rect.right_bottom(),
                pos2(c.x, crop_rect.min.y),
                pos2(c.x, crop_rect.max.y),
                pos2(crop_rect.min.x, c.y),
                pos2(crop_rect.max.x, c.y),
            ] {
                let r = Rect::from_center_size(p, vec2(9.0, 9.0));
                painter.rect_filled(r, 0.0, Color32::WHITE);
                painter.rect_stroke(r, 0.0, Stroke::new(1.0_f32, Color32::BLACK));
            }
            if self.guides != Guides::Off {
                let thin = Stroke::new(1.0_f32, Color32::from_white_alpha(110));
                if matches!(self.guides, Guides::Thirds | Guides::Both) {
                    for i in 1..3 {
                        let fx = crop_rect.min.x + crop_rect.width() * i as f32 / 3.0;
                        painter.line_segment([pos2(fx, crop_rect.min.y), pos2(fx, crop_rect.max.y)], thin);
                        let fy = crop_rect.min.y + crop_rect.height() * i as f32 / 3.0;
                        painter.line_segment([pos2(crop_rect.min.x, fy), pos2(crop_rect.max.x, fy)], thin);
                    }
                }
                if matches!(self.guides, Guides::Cross | Guides::Both) {
                    let c = crop_rect.center();
                    painter.line_segment([pos2(img_rect.min.x, c.y), pos2(img_rect.max.x, c.y)], thin);
                    painter.line_segment([pos2(c.x, img_rect.min.y), pos2(c.x, img_rect.max.y)], thin);
                }
            }
            let sel = format!("Crop {}×{} px", cb.w.round(), cb.h.round());
            painter.text(
                img_rect.left_top() + vec2(6.0, 4.0),
                egui::Align2::LEFT_TOP,
                sel,
                egui::FontId::proportional(13.0),
                Color32::WHITE,
            );
        } else {
            painter.text(
                img_rect.left_top() + vec2(6.0, 4.0),
                egui::Align2::LEFT_TOP,
                "Whole image is used (padded)",
                egui::FontId::proportional(13.0),
                Color32::WHITE,
            );
        }
        if hovering {
            painter.rect_filled(area, 0.0, Color32::from_black_alpha(160));
            painter.text(area.center(), egui::Align2::CENTER_CENTER, "Release to open", egui::FontId::proportional(22.0), Color32::WHITE);
        }

        // ---- live preview
        ui.separator();
        let pv_bytes = self.preview.as_ref().and_then(|pv| pv.size);
        let pv_info = self
            .preview
            .as_ref()
            .map(|pv| format!("{0}×{1} px · {2}×{3} grid ×{4}", pv.out.0, pv.out.1, pv.grid.0, pv.grid.1, pv.scale));
        let mut header = "Preview".to_string();
        if let Some(info) = &pv_info {
            header.push_str(" · ");
            header.push_str(info);
        }
        if let Some(b) = pv_bytes {
            header.push_str(&format!(" · {}", human_size(b)));
        }
        ui.horizontal(|ui| {
            ui.weak(header.as_str());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .selectable_label(self.preview_zoom.is_none(), "Fit")
                    .on_hover_text("Scale the preview to fill the pane.")
                    .clicked()
                {
                    self.preview_zoom = None;
                    self.pv_pan = (0.0, 0.0);
                }
                if ui
                    .selectable_label(self.preview_zoom == Some(1.0), "100%")
                    .on_hover_text("One screen pixel per output pixel - pixel-for-pixel what the saved file contains.")
                    .clicked()
                {
                    self.preview_zoom = Some(1.0);
                }
                if let Some(z) = self.preview_zoom {
                    ui.weak(format!("{:.0}%", z * 100.0));
                } else {
                    ui.weak(format!("Fit · {:.0}%", self.pv_fit * 100.0));
                }
            });
        })
        .response
        .on_hover_text(
            "Zoom the preview: wheel zooms at the pointer, drag pans, double-click returns to Fit. Zoom and pan survive re-renders, so you can compare pixel sizes on the same spot. The header shows the output resolution, dither grid with the effective pixel size, and the exact size the output file will have.",
        );
        let pv_size = ui.available_size();
        let (pv_rect, pv_resp) = ui.allocate_exact_size(pv_size, Sense::click_and_drag());
        let pv_info = self
            .preview
            .as_ref()
            .map(|pv| (pv.tex.id(), pv.grid.0 as f32 * pv.scale as f32, pv.grid.1 as f32 * pv.scale as f32));
        if let Some((tex_id, nw, nh)) = pv_info {
            let fit = (pv_rect.width() / nw).min(pv_rect.height() / nh);
            let s = match self.preview_zoom {
                None => {
                    if fit >= 1.0 { fit.floor() } else { fit } // whole-number zoom keeps pixels crisp
                }
                Some(z) => z,
            };
            self.pv_fit = s;
            let ov = (
                ((nw * s - pv_rect.width()) / 2.0).max(0.0),
                ((nh * s - pv_rect.height()) / 2.0).max(0.0),
            );
            let cl = |v: f32, o: f32| v.clamp(-o, o);
            if pv_resp.dragged() {
                let d = pv_resp.drag_delta();
                self.pv_pan = (cl(self.pv_pan.0 + d.x, ov.0), cl(self.pv_pan.1 + d.y, ov.1));
            }
            if pv_resp.hovered() {
                let dz = ctx.input(|i| i.raw_scroll_delta.y);
                if dz != 0.0 {
                    let z0 = self.preview_zoom.unwrap_or(if fit >= 1.0 { fit.floor() } else { fit });
                    let z = (z0 * (dz / 250.0).exp()).clamp(0.1, 16.0);
                    if let Some(pos) = pv_resp.hover_pos() {
                        let cur = Rect::from_center_size(pv_rect.center() + vec2(self.pv_pan.0, self.pv_pan.1), vec2(nw, nh) * z0);
                        let u = ((pos.x - cur.min.x) / cur.width(), (pos.y - cur.min.y) / cur.height());
                        let next = Rect::from_min_size(
                            pos2(pos.x - u.0 * nw * z, pos.y - u.1 * nh * z),
                            vec2(nw * z, nh * z),
                        );
                        let c = next.center() - pv_rect.center();
                        self.pv_pan = (cl(c.x, ov.0), cl(c.y, ov.1));
                    }
                    self.preview_zoom = Some(z);
                }
            }
            if pv_resp.double_clicked() {
                self.preview_zoom = None;
                self.pv_pan = (0.0, 0.0);
            }
            if self.preview_zoom.is_some() && (pv_resp.hovered() || pv_resp.dragged()) {
                ctx.set_cursor_icon(if pv_resp.dragged() { egui::CursorIcon::Grabbing } else { egui::CursorIcon::Grab });
            }
            let r = Rect::from_center_size(
                pv_rect.center() + vec2(self.pv_pan.0, self.pv_pan.1),
                vec2(nw, nh) * s,
            );
            let p = ui.painter_at(pv_rect);
            p.image(tex_id, r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }
    }
}

fn cursor_for(h: Handle, dragging: bool) -> egui::CursorIcon {
    use egui::CursorIcon as C;
    match h {
        Handle::Move => if dragging { C::Grabbing } else { C::Grab },
        Handle::N => C::ResizeNorth,
        Handle::S => C::ResizeSouth,
        Handle::E => C::ResizeEast,
        Handle::W => C::ResizeWest,
        Handle::NE => C::ResizeNorthEast,
        Handle::NW => C::ResizeNorthWest,
        Handle::SE => C::ResizeSouthEast,
        Handle::SW => C::ResizeSouthWest,
    }
}

fn file_name(path: &str) -> String {
    Path::new(path).file_name().and_then(|n| n.to_str()).unwrap_or(path).to_string()
}

/// Compact file-size label for the preview header.
fn swatch_strip(ui: &mut egui::Ui, pal: &Palette) {
    for c in pal {
        let (rect, _) = ui.allocate_exact_size(vec2(9.0, 12.0), Sense::hover());
        ui.painter().rect_filled(rect, 2.0, Color32::from_rgb(c[0], c[1], c[2]));
    }
}

fn split_palette_spec(spec: &str) -> [String; 4] {
    let mut parts: Vec<String> = spec.split(',').map(|s| s.trim().to_string()).collect();
    parts.truncate(4);
    parts.resize(4, String::new());
    parts.try_into().unwrap_or_default()
}

fn human_size(n: u64) -> String {
    if n >= 1 << 20 {
        format!("{:.1} MiB", n as f64 / (1 << 20) as f64)
    } else if n >= 1 << 10 {
        format!("{:.1} KiB", n as f64 / (1 << 10) as f64)
    } else {
        format!("{n} B")
    }
}

fn snap_pixel_size(w: usize, h: usize, want: usize) -> usize {
    (1..=32)
        .filter(|ps| w.is_multiple_of(*ps) && h.is_multiple_of(*ps))
        .min_by_key(|ps| (want as i64 - *ps as i64).abs())
        .unwrap_or(1)
}

fn fit_mode_of(mode: CropMode) -> usize {
    match mode {
        CropMode::Contain => 1,
        CropMode::PadRepeat => 2,
        CropMode::Cover | CropMode::Interactive => 0,
    }
}

/// Dropdown section order: built-ins first, folder groups in between, Custom last.
fn group_rank(g: &str) -> usize {
    match g {
        "Built-in" => 0,
        "Custom" => usize::MAX,
        _ => 1,
    }
}

/// Display name for a frame: "film_roll" -> "Film Roll".
fn pretty_frame_name(raw: &str) -> String {
    let (id, suffix) = match raw.find(" (") {
        Some(i) => (&raw[..i], &raw[i..]),
        None => (raw, ""),
    };
    let pretty = id
        .split(|c: char| c == '_' || c.is_whitespace())
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut cs = w.chars();
            match cs.next() {
                Some(f) => f.to_uppercase().collect::<String>() + cs.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    format!("{pretty}{suffix}")
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Dropped files (native: the path is available)
        let dropped = ctx.input(|i| i.raw.dropped_files.iter().find_map(|f| f.path.clone()));
        if let Some(p) = dropped {
            self.load(ctx, &p.to_string_lossy());
        }

        // Keyboard: Esc closes the window
        if !ctx.wants_keyboard_input() && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }

        egui::SidePanel::right("options")
            .width_range(300.0..=520.0)
            .default_width(320.0)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| self.options_ui(ui, ctx));
            });
        egui::CentralPanel::default().show(ctx, |ui| self.main_ui(ui, ctx));

        self.sync_crop_aspect();
        self.refresh_preview(ctx);
    }
}

#[cfg(test)]
mod tests {
    use super::snap_pixel_size;

    #[test]
    fn snapping_keeps_valid_pixel_sizes() {
        assert_eq!(snap_pixel_size(800, 350, 2), 2);
        assert_eq!(snap_pixel_size(800, 350, 4), 5);
        assert_eq!(snap_pixel_size(800, 350, 5), 5);
    }

    #[test]
    fn snapping_picks_the_nearest_divisor() {
        assert_eq!(snap_pixel_size(800, 350, 3), 2);
        assert_eq!(snap_pixel_size(800, 315, 4), 5);
    }

    #[test]
    fn snapping_falls_back_to_one_when_nothing_divides() {
        assert_eq!(snap_pixel_size(7, 5, 4), 1);
    }

    #[test]
    fn snapping_never_exceeds_the_slider_range() {
        assert_eq!(snap_pixel_size(800, 400, 60), 25);
    }
}
