//! Four-tone frames (like the Game Boy Camera's).
//!
//! A frame is an image whose pixels are one of four levels (0 = darkest .. 3 = lightest)
//! plus a transparent "window" where the photo shows through. Because the frame stores
//! levels, not colours, the palette recolours photo and frame together.
//!
//! Frames that are not strictly four-tone (anti-aliased art, tinted art) are snapped to
//! four levels on load with the same thresholds as the dither step; `source_grey_values`
//! tells the caller how many distinct grey values the source had so it can warn.

use crate::dither::threshold_level;
use crate::gray::luma;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: usize,
    pub y: usize,
    pub w: usize,
    pub h: usize,
}

#[derive(Clone, Debug)]
pub struct Frame {
    pub name: String,
    pub w: usize,
    pub h: usize,
    /// Level 0..=3 per pixel (meaningless where `opaque` is false).
    pub levels: Vec<u8>,
    pub opaque: Vec<bool>,
    /// Bounding box of the transparent window.
    pub hole: Rect,
    /// Distinct grey values among opaque pixels in the source (4 or fewer = already to spec).
    pub source_grey_values: usize,
    /// True if every pixel inside `hole` is transparent (a clean rectangular window).
    pub hole_is_rect: bool,
}

impl Frame {
    /// Build from straight RGBA bytes. Pixels with alpha < 128 are the window.
    pub fn from_rgba(name: &str, w: usize, h: usize, rgba: &[u8]) -> Result<Frame, String> {
        if rgba.len() != w * h * 4 || w == 0 || h == 0 {
            return Err("frame: pixel data does not match its size".into());
        }
        let mut levels = vec![0u8; w * h];
        let mut opaque = vec![true; w * h];
        let mut seen = [false; 256];
        let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0usize, 0usize);
        let mut transparent = 0usize;

        for y in 0..h {
            for x in 0..w {
                let i = y * w + x;
                let p = &rgba[i * 4..i * 4 + 4];
                if p[3] < 128 {
                    opaque[i] = false;
                    transparent += 1;
                    x0 = x0.min(x);
                    y0 = y0.min(y);
                    x1 = x1.max(x);
                    y1 = y1.max(y);
                } else {
                    let l = luma(p[0], p[1], p[2]);
                    seen[l as usize] = true;
                    levels[i] = threshold_level(l);
                }
            }
        }
        if transparent == 0 {
            return Err("frame has no transparent window (alpha channel needed)".into());
        }
        let hole = Rect { x: x0, y: y0, w: x1 - x0 + 1, h: y1 - y0 + 1 };
        Ok(Frame {
            name: name.to_string(),
            w,
            h,
            levels,
            opaque,
            hole,
            source_grey_values: seen.iter().filter(|&&s| s).count(),
            hole_is_rect: transparent == hole.w * hole.h,
        })
    }

    /// The built-in default frame (a four-tone 160x144 frame with a 128x112 window).
    pub fn builtin() -> Frame {
        let data: &[u8] = include_bytes!("../assets/frame_default.bin");
        let w = data[0] as usize | (data[1] as usize) << 8;
        let h = data[2] as usize | (data[3] as usize) << 8;
        let px = &data[4..];
        assert_eq!(px.len(), w * h, "corrupt built-in frame");
        let mut rgba = Vec::with_capacity(w * h * 4);
        const GREYS: [u8; 4] = [0, 85, 170, 255];
        for &v in px {
            if v >= 4 {
                rgba.extend_from_slice(&[0, 0, 0, 0]);
            } else {
                let g = GREYS[v as usize];
                rgba.extend_from_slice(&[g, g, g, 255]);
            }
        }
        Frame::from_rgba("Deruku (built-in)", w, h, &rgba).expect("built-in frame is valid")
    }

    pub fn hole_aspect(&self) -> f64 {
        self.hole.w as f64 / self.hole.h as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_frame_matches_the_game_boy_layout() {
        let f = Frame::builtin();
        assert_eq!((f.w, f.h), (160, 144));
        assert_eq!(f.hole, Rect { x: 16, y: 16, w: 128, h: 112 });
        assert!(f.hole_is_rect);
        assert!(f.source_grey_values <= 4);
        assert!(f.levels.iter().zip(&f.opaque).all(|(&l, &o)| !o || l <= 3));
    }

    #[test]
    fn snaps_many_greys_to_four_levels() {
        // 4x1: opaque 10, 100, 150, 250 and a transparent pixel at the end -> make 5x1
        let mut rgba = Vec::new();
        for g in [10u8, 100, 150, 250] {
            rgba.extend_from_slice(&[g, g, g, 255]);
        }
        rgba.extend_from_slice(&[0, 0, 0, 0]);
        let f = Frame::from_rgba("t", 5, 1, &rgba).unwrap();
        assert_eq!(&f.levels[..4], &[0, 1, 2, 3]);
        assert_eq!(f.source_grey_values, 4);
        assert_eq!(f.hole, Rect { x: 4, y: 0, w: 1, h: 1 });
    }

    #[test]
    fn frame_without_window_is_rejected() {
        let rgba = vec![0u8, 0, 0, 255, 0, 0, 0, 255];
        assert!(Frame::from_rgba("t", 2, 1, &rgba).is_err());
    }
}
