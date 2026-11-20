//! The rendering pipeline: crop -> resize -> autocontrast -> dither -> (frame) -> levels.
//!
//! `render` produces a *level image* (values 0..=3) at the native dithering resolution plus an
//! integer upscale factor. The palette is applied afterwards (`Rendered::rgb`), so changing the
//! palette never needs a re-dither.

use crate::autocontrast::{autocontrast, gamma, DEFAULT_CUTOFF};
use crate::dither::bayer_4tone;
use crate::frame::Frame;
use crate::gray::Plane;
use crate::mask;
use crate::palette::Palette;
use crate::resample::{resize_plane, upscale_nearest, BoxF};

/// With "auto" pixel size, aim for roughly this many dithered pixels across.
pub const AUTO_GRID_WIDTH: f64 = 384.0;

#[derive(Clone, Debug)]
pub enum Fit {
    /// Use this region of the source (it should already have the target aspect ratio).
    Cover(BoxF),
    /// Show the whole image, padded with the darkest level.
    Contain,
    /// Show the whole image, repeating the outermost pixels outwards to fill.
    EdgeRepeat,
}

pub struct Params<'a> {
    pub fit: Fit,
    /// Output size without a frame (ignored when `frame` is set).
    pub out_w: usize,
    pub out_h: usize,
    /// Output pixels per dithered pixel; 0 = auto. Ignored when `frame` is set.
    pub pixel_size: usize,
    pub frame: Option<&'a Frame>,
    /// Integer upscale of the framed 160x144 image.
    pub frame_scale: usize,
    pub autocontrast: bool,
    /// Gamma correction before the contrast stretch (1.0 = no change).
    pub gamma: f64,
    /// Local brush coverage at source resolution; painted areas follow `mask_gamma`.
    pub mask: Option<&'a Plane>,
    /// Gamma applied where the mask covers (1.0 = no change).
    pub mask_gamma: f64,
}

pub struct Rendered {
    /// Level grid (0..=3), `grid_w` x `grid_h`.
    pub grid_w: usize,
    pub grid_h: usize,
    pub levels: Vec<u8>,
    /// Integer upscale factor from grid to output.
    pub scale: usize,
    /// Final output size (the upscaled grid, cropped at the right/bottom if needed).
    pub out_w: usize,
    pub out_h: usize,
}

pub fn auto_pixel_size(out_w: usize) -> usize {
    ((out_w as f64 / AUTO_GRID_WIDTH).round() as usize).max(1)
}

/// Final output size for these parameters.
pub fn output_size(p: &Params) -> (usize, usize) {
    match p.frame {
        Some(f) => (f.w * p.frame_scale.max(1), f.h * p.frame_scale.max(1)),
        None => (p.out_w.max(1), p.out_h.max(1)),
    }
}

/// Aspect ratio the crop region must have for these parameters.
pub fn crop_aspect(p: &Params) -> f64 {
    match p.frame {
        Some(f) => f.hole_aspect(),
        None => p.out_w.max(1) as f64 / p.out_h.max(1) as f64,
    }
}

fn photo_levels(gray: &Plane, fit: &Fit, gw: usize, gh: usize, ac: bool, g: f64) -> Vec<u8> {
    match fit {
        Fit::Cover(b) => {
            let b = b.clamped(gray.w, gray.h);
            let mut small = resize_plane(gray, gw, gh, b);
            gamma(&mut small, g);
            if ac {
                autocontrast(&mut small, DEFAULT_CUTOFF);
            }
            bayer_4tone(&small.data, gw, gh)
        }
        Fit::Contain => {
            let s = (gw as f64 / gray.w as f64).min(gh as f64 / gray.h as f64);
            let fw = ((gray.w as f64 * s).round() as usize).clamp(1, gw);
            let fh = ((gray.h as f64 * s).round() as usize).clamp(1, gh);
            let mut small = resize_plane(gray, fw, fh, BoxF::full(gray.w, gray.h));
            gamma(&mut small, g);
            if ac {
                autocontrast(&mut small, DEFAULT_CUTOFF);
            }
            let mut canvas = Plane::filled(gw, gh, 0);
            let (ox, oy) = ((gw - fw) / 2, (gh - fh) / 2);
            for y in 0..fh {
                canvas.data[(oy + y) * gw + ox..(oy + y) * gw + ox + fw]
                    .copy_from_slice(&small.data[y * fw..(y + 1) * fw]);
            }
            bayer_4tone(&canvas.data, gw, gh)
        }
        Fit::EdgeRepeat => {
            let s = (gw as f64 / gray.w as f64).min(gh as f64 / gray.h as f64);
            let fw = ((gray.w as f64 * s).round() as usize).clamp(1, gw);
            let fh = ((gray.h as f64 * s).round() as usize).clamp(1, gh);
            let mut small = resize_plane(gray, fw, fh, BoxF::full(gray.w, gray.h));
            gamma(&mut small, g);
            if ac {
                autocontrast(&mut small, DEFAULT_CUTOFF);
            }
            let (ox, oy) = ((gw - fw) / 2, (gh - fh) / 2);
            let mut canvas = Vec::with_capacity(gw * gh);
            for y in 0..gh {
                let sy = y.saturating_sub(oy).min(fh - 1);
                for x in 0..gw {
                    let sx = x.saturating_sub(ox).min(fw - 1);
                    canvas.push(small.data[sy * fw + sx]);
                }
            }
            bayer_4tone(&canvas, gw, gh)
        }
    }
}

pub fn render(gray: &Plane, p: &Params) -> Rendered {
    let mut masked;
    let gray = match p.mask {
        Some(m) => {
            masked = gray.clone();
            mask::apply(&mut masked, m, p.mask_gamma);
            &masked
        }
        None => gray,
    };
    match p.frame {
        Some(f) => {
            let (hw, hh) = (f.hole.w, f.hole.h);
            let photo = photo_levels(gray, &p.fit, hw, hh, p.autocontrast, p.gamma);
            let mut canvas = vec![0u8; f.w * f.h];
            for y in 0..hh {
                let dst = (f.hole.y + y) * f.w + f.hole.x;
                canvas[dst..dst + hw].copy_from_slice(&photo[y * hw..(y + 1) * hw]);
            }
            for i in 0..canvas.len() {
                if f.opaque[i] {
                    canvas[i] = f.levels[i];
                }
            }
            let scale = p.frame_scale.max(1);
            Rendered { grid_w: f.w, grid_h: f.h, levels: canvas, scale, out_w: f.w * scale, out_h: f.h * scale }
        }
        None => {
            let (ow, oh) = (p.out_w.max(1), p.out_h.max(1));
            let ps = if p.pixel_size == 0 { auto_pixel_size(ow) } else { p.pixel_size };
            let (gw, gh) = (ow.div_ceil(ps), oh.div_ceil(ps));
            let levels = photo_levels(gray, &p.fit, gw, gh, p.autocontrast, p.gamma);
            Rendered { grid_w: gw, grid_h: gh, levels, scale: ps, out_w: ow, out_h: oh }
        }
    }
}

impl Rendered {
    /// Level image at output resolution (`out_w` x `out_h`).
    pub fn levels_upscaled(&self) -> Vec<u8> {
        let full = upscale_nearest(&self.levels, self.grid_w, self.grid_h, 1, self.scale);
        let fw = self.grid_w * self.scale;
        if fw == self.out_w && self.grid_h * self.scale == self.out_h {
            return full;
        }
        let mut out = Vec::with_capacity(self.out_w * self.out_h);
        for y in 0..self.out_h {
            out.extend_from_slice(&full[y * fw..y * fw + self.out_w]);
        }
        out
    }

    /// RGB at output resolution.
    pub fn rgb(&self, pal: &Palette) -> Vec<u8> {
        self.levels_upscaled().iter().flat_map(|&l| pal[l as usize & 3]).collect()
    }

    /// RGB at the native (not upscaled) grid resolution, e.g. for a preview texture.
    pub fn rgb_native(&self, pal: &Palette) -> Vec<u8> {
        crate::palette::colour_levels(&self.levels, pal)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mask::dab;
    use crate::palette::by_name;

    fn ramp(w: usize, h: usize) -> Plane {
        let mut d = Vec::new();
        for _ in 0..h {
            for x in 0..w {
                d.push((x * 255 / (w - 1)) as u8);
            }
        }
        Plane::new(w, h, d)
    }

    #[test]
    fn frameless_sizes() {
        let g = ramp(1200, 800);
        let p = Params {
            fit: Fit::Cover(BoxF::full(1200, 800)),
            out_w: 800,
            out_h: 350,
            pixel_size: 0,
            frame: None,
            frame_scale: 4,
            autocontrast: true,
            gamma: 1.0,
            mask: None,
            mask_gamma: 1.0,
        };
        let r = render(&g, &p);
        assert_eq!((r.out_w, r.out_h), (800, 350));
        assert_eq!(r.scale, 2);
        assert_eq!((r.grid_w, r.grid_h), (400, 175));
        assert_eq!(r.levels_upscaled().len(), 800 * 350);
        assert_eq!(r.rgb(&by_name("bw").unwrap()).len(), 800 * 350 * 3);
    }

    #[test]
    fn odd_sizes_are_cropped_not_stretched() {
        let g = ramp(100, 100);
        let p = Params {
            fit: Fit::Cover(BoxF::full(100, 100)),
            out_w: 10,
            out_h: 7,
            pixel_size: 4,
            frame: None,
            frame_scale: 1,
            autocontrast: false,
            gamma: 1.0,
            mask: None,
            mask_gamma: 1.0,
        };
        let r = render(&g, &p);
        assert_eq!((r.grid_w, r.grid_h), (3, 2));
        assert_eq!(r.levels_upscaled().len(), 70);
    }

    #[test]
    fn framed_output_uses_only_four_levels_and_frame_size() {
        let f = Frame::builtin();
        let g = ramp(640, 560);
        let p = Params {
            fit: Fit::Cover(BoxF::full(640, 560)),
            out_w: 0,
            out_h: 0,
            pixel_size: 0,
            frame: Some(&f),
            frame_scale: 4,
            autocontrast: true,
            gamma: 1.0,
            mask: None,
            mask_gamma: 1.0,
        };
        let r = render(&g, &p);
        assert_eq!((r.out_w, r.out_h), (640, 576));
        assert!(r.levels.iter().all(|&l| l <= 3));
        // frame pixels override the photo
        for i in 0..r.levels.len() {
            if f.opaque[i] {
                assert_eq!(r.levels[i], f.levels[i]);
            }
        }
    }

    #[test]
    fn palette_recolours_frame_and_photo_together() {
        let f = Frame::builtin();
        let g = ramp(128, 112);
        let p = Params {
            fit: Fit::Contain,
            out_w: 0,
            out_h: 0,
            pixel_size: 0,
            frame: Some(&f),
            frame_scale: 1,
            autocontrast: true,
            gamma: 1.0,
            mask: None,
            mask_gamma: 1.0,
        };
        let r = render(&g, &p);
        let pal = by_name("green").unwrap();
        let rgb = r.rgb(&pal);
        let used: std::collections::HashSet<[u8; 3]> = rgb.chunks(3).map(|c| [c[0], c[1], c[2]]).collect();
        assert!(used.iter().all(|c| pal.contains(c)));
    }

    #[test]
    fn gamma_changes_the_levels() {
        let g = Plane::filled(40, 40, 96);
        let mut base = Params {
            fit: Fit::Cover(BoxF::full(40, 40)),
            out_w: 40,
            out_h: 40,
            pixel_size: 1,
            frame: None,
            frame_scale: 1,
            autocontrast: false,
            gamma: 1.0,
            mask: None,
            mask_gamma: 1.0,
        };
        let flat = render(&g, &base);
        assert!(flat.levels.contains(&0));
        assert!(flat.levels.contains(&1));
        base.gamma = 0.5;
        let brightened = render(&g, &base);
        assert_ne!(brightened.levels, flat.levels);
        assert!(brightened.levels.contains(&2));
    }

    #[test]
    fn edge_repeat_fills_padding_with_edge_pixels() {
        let g = Plane::filled(200, 100, 255);
        let p = Params {
            fit: Fit::EdgeRepeat,
            out_w: 100,
            out_h: 100,
            pixel_size: 1,
            frame: None,
            frame_scale: 1,
            autocontrast: false,
            gamma: 1.0,
            mask: None,
            mask_gamma: 1.0,
        };
        let r = render(&g, &p);
        assert_eq!(r.levels[0], 3); // top-left is smeared white (not dark padding)
        assert_eq!(r.levels[50 * 100 + 50], 3); // centre is the (white) photo
    }

    #[test]
    fn mask_only_changes_the_painted_area() {
        let gray = Plane::filled(8, 8, 96);
        let mut mask = Plane::filled(8, 8, 0);
        dab(&mut mask, 6.0, 6.0, 1.5);
        let common = Params {
            fit: Fit::Cover(BoxF::full(8, 8)),
            out_w: 8,
            out_h: 8,
            pixel_size: 1,
            frame: None,
            frame_scale: 1,
            autocontrast: false,
            gamma: 1.0,
            mask: None,
            mask_gamma: 1.0,
        };
        let base = render(&gray, &common);
        let mut local = Params { mask: Some(&mask), mask_gamma: 0.5, ..common };
        local.gamma = 1.0;
        let brushed = render(&gray, &local);
        for (i, &m) in mask.data.iter().enumerate() {
            if m == 0 {
                assert_eq!(brushed.levels[i], base.levels[i]);
            }
        }
        assert_ne!(brushed.levels, base.levels);
    }

    #[test]
    fn contain_pads_with_darkest_level() {
        let g = Plane::filled(200, 100, 255);
        let p = Params {
            fit: Fit::Contain,
            out_w: 100,
            out_h: 100,
            pixel_size: 1,
            frame: None,
            frame_scale: 1,
            autocontrast: false,
            gamma: 1.0,
            mask: None,
            mask_gamma: 1.0,
        };
        let r = render(&g, &p);
        assert_eq!(r.levels[0], 0); // top-left is padding
        assert_eq!(r.levels[50 * 100 + 50], 3); // centre is the (white) photo
    }
}
