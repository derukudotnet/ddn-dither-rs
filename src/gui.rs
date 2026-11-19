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

#[derive(Clone, PartialEq)]
struct RenderKey {
    crop: [f64; 4],
    out: (usize, usize),
    pixel_size: usize,
    frame: Option<usize>,
    scale: usize,
    autocontrast: bool,
    contain: bool,
    palette: Palette,
    proxy: bool,
    source_id: u64,
}

struct Preview {
    tex: TextureHandle,
    grid: (usize, usize),
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
    fit_contain: bool,
    autocontrast: bool,
    format: Format,

    frames: Vec<Frame>,
    frame_idx: Option<usize>,
    frame_scale: usize,

    pal_idx: usize,
    custom_pal_text: String,
    palette: Palette,
    pal_error: Option<String>,

    preview: Option<Preview>,
    last_key: Option<RenderKey>,

    output_override: Option<String>,
    exit_on_save: bool,
    status: String,
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

        // Frames: built-in first, then any found in the frames folders.
        let mut frames = vec![Frame::builtin()];
        let (found, warnings) = io::discover_frames();
        frames.extend(found);
        if !warnings.is_empty() {
            status = format!("Skipped frame(s): {}", warnings.join("; "));
        }
        let mut frame_idx = None;
        match &cli.frame {
            FrameSel::None => {}
            FrameSel::Builtin => frame_idx = Some(0),
            FrameSel::Path(p) => match io::load_frame(p) {
                Ok(f) => {
                    frames.push(f);
                    frame_idx = Some(frames.len() - 1);
                }
                Err(e) => status = e,
            },
        }

        // Palette from --palette (name, or custom hex list).
        let (pal_idx, custom_text, palette) = match palette::by_name(&cli.palette) {
            Some(p) => (palette::names().iter().position(|n| *n == cli.palette.to_ascii_lowercase()).unwrap_or(0), String::new(), p),
            None => (
                palette::BUILTIN.len(),
                cli.palette.clone(),
                palette::resolve(&cli.palette).unwrap_or(palette::BUILTIN[0].1),
            ),
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
            fit_contain: cli.crop_mode == CropMode::Contain,
            autocontrast: cli.autocontrast,
            format: cli.format,
            frames,
            frame_idx,
            frame_scale: cli.scale,
            pal_idx,
            custom_pal_text: custom_text,
            palette,
            pal_error: None,
            preview: None,
            last_key: None,
            output_override: cli.output.clone(),
            exit_on_save: !cli.inputs.is_empty(),
            status,
            cli,
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
            fit: if self.fit_contain { Fit::Contain } else { Fit::Cover(crop) },
            out_w: self.out_w,
            out_h: self.out_h,
            pixel_size: self.pixel_size,
            frame: self.frame(),
            frame_scale: self.frame_scale,
            autocontrast: self.autocontrast,
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
            contain: self.fit_contain,
            palette: self.palette,
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
            return;
        }
        if let Some(r) = self.render_levels(use_proxy) {
            let rgb = r.rgb_native(&self.palette);
            let tex = ctx.load_texture(
                "preview",
                egui::ColorImage::from_rgb([r.grid_w, r.grid_h], &rgb),
                TextureOptions::NEAREST,
            );
            self.preview = Some(Preview { tex, grid: (r.grid_w, r.grid_h) });
            self.last_key = Some(key);
            ctx.request_repaint();
        }
    }

    // ---------------------------------------------------------------- saving

    fn default_output_path(&mut self) -> Option<String> {
        let src = self.source.as_ref()?;
        Some(self.output_override.take().unwrap_or_else(|| cli::default_output(&src.path, self.format)))
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
        match io::load_frame(&p.to_string_lossy()) {
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
            ui.label(format!("{}  ({}×{})", file_name(&src.path), src.w, src.h));
        }
        ui.separator();

        // ---- frame
        ui.strong("Frame");
        egui::ComboBox::from_id_salt("frame")
            .selected_text(self.frame().map(|f| f.name.clone()).unwrap_or_else(|| "None".to_string()))
            .width(220.0)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut self.frame_idx, None, "None");
                for i in 0..self.frames.len() {
                    let name = self.frames[i].name.clone();
                    ui.selectable_value(&mut self.frame_idx, Some(i), name);
                }
            });
        if ui.small_button("Load frame file…").clicked() {
            self.load_frame_dialog();
        }
        // Copy what we need out of the frame so `self` is free to be edited below.
        let info = self.frame().map(|f| (f.hole.w, f.hole.h, f.w, f.h, f.source_grey_values, f.hole_is_rect));
        let framed = info.is_some();
        if let Some((hole_w, hole_h, fw, fh, greys, hole_is_rect)) = info {
            ui.label(format!("Window {hole_w}×{hole_h} px, frame {fw}×{fh} px"));
            if greys > 4 {
                ui.colored_label(Color32::from_rgb(200, 140, 0), format!("Converted from {greys} grey values to 4 tones."));
            }
            if !hole_is_rect {
                ui.colored_label(Color32::from_rgb(200, 140, 0), "Window is not a plain rectangle; the photo fills its bounding box.");
            }
            ui.horizontal(|ui| {
                ui.label("Scale");
                ui.add(egui::DragValue::new(&mut self.frame_scale).range(1..=16).prefix("×"));
            });
            let s = self.frame_scale.max(1);
            ui.label(format!("Output {}×{} px (set by the frame)", fw * s, fh * s));
        }
        ui.separator();

        // ---- size / aspect
        ui.add_enabled_ui(!framed, |ui| {
            ui.strong("Size");
            let before_idx = self.aspect_idx;
            egui::ComboBox::from_id_salt("aspect")
                .selected_text(format!("Aspect: {}", ASPECT_PRESETS[self.aspect_idx].0))
                .show_ui(ui, |ui| {
                    for (i, (name, _)) in ASPECT_PRESETS.iter().enumerate() {
                        ui.selectable_value(&mut self.aspect_idx, i, *name);
                    }
                });
            if self.aspect_idx != before_idx && self.aspect_idx != 0 {
                let a = ASPECT_PRESETS[self.aspect_idx].1;
                self.out_h = ((self.out_w as f64 / a).round() as usize).max(1);
            }
            let (ow, oh) = (self.out_w, self.out_h);
            ui.horizontal(|ui| {
                ui.label("W");
                ui.add(egui::DragValue::new(&mut self.out_w).range(1..=16384));
                ui.label("H");
                ui.add(egui::DragValue::new(&mut self.out_h).range(1..=16384));
            });
            if self.aspect_idx != 0 {
                let a = ASPECT_PRESETS[self.aspect_idx].1;
                if self.out_w != ow {
                    self.out_h = ((self.out_w as f64 / a).round() as usize).max(1);
                } else if self.out_h != oh {
                    self.out_w = ((self.out_h as f64 * a).round() as usize).max(1);
                }
            }
            ui.horizontal(|ui| {
                ui.label("Pixel size");
                ui.add(
                    egui::DragValue::new(&mut self.pixel_size)
                        .range(0..=64)
                        .custom_formatter(|n, _| if n == 0.0 { "auto".to_string() } else { format!("{n:.0}") })
                        .custom_parser(|s| {
                            if s.trim().eq_ignore_ascii_case("auto") { Some(0.0) } else { s.trim().parse::<f64>().ok() }
                        }),
                );
                if self.pixel_size == 0 {
                    ui.weak(format!("→ {}", pipeline::auto_pixel_size(self.out_w)));
                }
            });
        });
        ui.checkbox(&mut self.fit_contain, "Fit whole image (pad instead of crop)");
        ui.checkbox(&mut self.autocontrast, "Auto contrast");
        ui.separator();

        // ---- palette
        ui.strong("Palette");
        let names = palette::names();
        let custom_idx = names.len();
        let before_pal = self.pal_idx;
        egui::ComboBox::from_id_salt("palette")
            .selected_text(if self.pal_idx < custom_idx { names[self.pal_idx].to_string() } else { "custom".to_string() })
            .show_ui(ui, |ui| {
                for (i, n) in names.iter().enumerate() {
                    ui.selectable_value(&mut self.pal_idx, i, *n);
                }
                ui.selectable_value(&mut self.pal_idx, custom_idx, "custom");
            });
        if self.pal_idx < custom_idx {
            self.palette = palette::BUILTIN[self.pal_idx].1;
            self.pal_error = None;
            if before_pal != self.pal_idx {
                self.custom_pal_text = palette::BUILTIN[self.pal_idx]
                    .1
                    .iter()
                    .map(|c| format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2]))
                    .collect::<Vec<_>>()
                    .join(",");
            }
        } else {
            ui.label("Four hex colours, darkest first:");
            if ui.text_edit_singleline(&mut self.custom_pal_text).changed() || before_pal != self.pal_idx {
                match palette::resolve(&self.custom_pal_text) {
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
            });
        let have = self.source.is_some();
        ui.horizontal(|ui| {
            let label = if self.exit_on_save { "Save & quit  (Enter)" } else { "Save  (Enter)" };
            if ui.add_enabled(have, egui::Button::new(label)).clicked() {
                self.quick_save(ctx);
            }
            if ui.add_enabled(have, egui::Button::new("Save as…")).clicked() {
                self.save_as();
            }
        });
        if have {
            let dest = match &self.output_override {
                Some(p) => p.clone(),
                None => cli::default_output(&self.source.as_ref().map(|s| s.path.clone()).unwrap_or_default(), self.format),
            };
            ui.weak(format!("→ {}", file_name(&dest)));
        }
        ui.separator();
        ui.label(&self.status);
        ui.add_space(6.0);
        ui.weak("Drag inside the box to move it; drag a handle to resize (opposite side stays put). Drop another image anywhere to switch.");
    }

    fn main_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let hovering = ctx.input(|i| !i.raw.hovered_files.is_empty());
        let Some((tex_id, iw, ih)) = self.source.as_ref().map(|s| (s.tex.id(), s.w as f64, s.h as f64)) else {
            ui.centered_and_justified(|ui| {
                ui.heading(if hovering { "Release to open" } else { "Drop an image here, or click “Open image…”" });
            });
            return;
        };

        let avail = ui.available_size();
        let source_h = (avail.y * 0.58).max(160.0);
        let (resp, painter) = ui.allocate_painter(vec2(avail.x, source_h), Sense::click_and_drag());
        let area = resp.rect;
        let scale = (area.width() as f64 / iw).min(area.height() as f64 / ih).max(1e-6);
        let img_rect = Rect::from_center_size(area.center(), vec2((iw * scale) as f32, (ih * scale) as f32));
        painter.image(tex_id, img_rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);

        let to_img = |p: egui::Pos2| ((p.x - img_rect.min.x) as f64 / scale, (p.y - img_rect.min.y) as f64 / scale);
        let to_screen = |x: f64, y: f64| pos2(img_rect.min.x + (x * scale) as f32, img_rect.min.y + (y * scale) as f32);

        if !self.fit_contain {
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
            painter.rect_filled(Rect::from_min_max(img_rect.min, pos2(img_rect.max.x, crop_rect.min.y)), 0.0, dim);
            painter.rect_filled(Rect::from_min_max(pos2(img_rect.min.x, crop_rect.max.y), img_rect.max), 0.0, dim);
            painter.rect_filled(
                Rect::from_min_max(pos2(img_rect.min.x, crop_rect.min.y), pos2(crop_rect.min.x, crop_rect.max.y)),
                0.0,
                dim,
            );
            painter.rect_filled(
                Rect::from_min_max(pos2(crop_rect.max.x, crop_rect.min.y), pos2(img_rect.max.x, crop_rect.max.y)),
                0.0,
                dim,
            );
            painter.rect_stroke(crop_rect, 0.0, Stroke::new(2.0, Color32::WHITE));
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
                painter.rect_stroke(r, 0.0, Stroke::new(1.0, Color32::BLACK));
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
        let pv_size = ui.available_size();
        let (pv_rect, _) = ui.allocate_exact_size(pv_size, Sense::hover());
        if let Some(pv) = &self.preview {
            let (gw, gh) = (pv.grid.0 as f32, pv.grid.1 as f32);
            let fit = (pv_rect.width() / gw).min(pv_rect.height() / gh);
            let s = if fit >= 1.0 { fit.floor() } else { fit }; // whole-number zoom keeps pixels crisp
            let r = Rect::from_center_size(pv_rect.center(), vec2(gw * s, gh * s));
            let p = ui.painter_at(pv_rect);
            p.image(pv.tex.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
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

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Dropped files (native: the path is available)
        let dropped = ctx.input(|i| i.raw.dropped_files.iter().find_map(|f| f.path.clone()));
        if let Some(p) = dropped {
            self.load(ctx, &p.to_string_lossy());
        }

        // Keyboard: Enter saves, Esc quits (only when started from the command line with a file)
        if !ctx.wants_keyboard_input() {
            if ctx.input(|i| i.key_pressed(egui::Key::Enter)) && self.source.is_some() {
                self.quick_save(ctx);
            }
            if self.exit_on_save && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }

        egui::SidePanel::right("options").min_width(300.0).default_width(320.0).show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| self.options_ui(ui, ctx));
        });
        egui::CentralPanel::default().show(ctx, |ui| self.main_ui(ui, ctx));

        self.sync_crop_aspect();
        self.refresh_preview(ctx);
    }
}
