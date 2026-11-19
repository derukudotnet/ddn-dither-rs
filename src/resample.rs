//! Lanczos-3 resampling that follows Pillow's `Image.resize(..., LANCZOS, box=...)`
//! (coefficient generation, 22-bit fixed point, horizontal pass then vertical),
//! so results match the original Python tool.

use crate::gray::{Plane, RgbImage};

const PRECISION_BITS: u32 = 32 - 8 - 2;

/// Source box in source-pixel coordinates (may be fractional).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoxF {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
}

impl BoxF {
    pub fn full(w: usize, h: usize) -> BoxF {
        BoxF { x0: 0.0, y0: 0.0, x1: w as f64, y1: h as f64 }
    }

    pub fn width(&self) -> f64 {
        self.x1 - self.x0
    }

    pub fn height(&self) -> f64 {
        self.y1 - self.y0
    }

    /// Clamp into a w x h image and make sure it is at least 1 px in each direction.
    pub fn clamped(&self, w: usize, h: usize) -> BoxF {
        let (wf, hf) = (w as f64, h as f64);
        let x0 = self.x0.clamp(0.0, (wf - 1.0).max(0.0));
        let y0 = self.y0.clamp(0.0, (hf - 1.0).max(0.0));
        let x1 = self.x1.clamp(x0 + 1.0_f64.min(wf - x0), wf);
        let y1 = self.y1.clamp(y0 + 1.0_f64.min(hf - y0), hf);
        BoxF { x0, y0, x1, y1 }
    }
}

fn sinc(x: f64) -> f64 {
    if x == 0.0 {
        1.0
    } else {
        let px = x * std::f64::consts::PI;
        px.sin() / px
    }
}

fn lanczos3(x: f64) -> f64 {
    if (-3.0..3.0).contains(&x) {
        sinc(x) * sinc(x / 3.0)
    } else {
        0.0
    }
}

struct Coeffs {
    ksize: usize,
    /// (first source index, number of taps) per output index
    bounds: Vec<(usize, usize)>,
    /// fixed-point weights, `ksize` per output index
    kk: Vec<i32>,
}

fn precompute(in_size: usize, in0: f64, in1: f64, out_size: usize) -> Coeffs {
    let scale = (in1 - in0) / out_size as f64;
    let filterscale = scale.max(1.0);
    let support = 3.0 * filterscale;
    let ksize = support.ceil() as usize * 2 + 1;
    let ss = 1.0 / filterscale;

    let mut bounds = Vec::with_capacity(out_size);
    let mut kk = vec![0i32; out_size * ksize];
    let mut pre = vec![0f64; ksize];

    for xx in 0..out_size {
        let center = in0 + (xx as f64 + 0.5) * scale;
        // C casts truncate toward zero; Rust's `as` does the same.
        let mut xmin = (center - support + 0.5) as i64;
        if xmin < 0 {
            xmin = 0;
        }
        let mut xmax = (center + support + 0.5) as i64;
        if xmax > in_size as i64 {
            xmax = in_size as i64;
        }
        let cnt = (xmax - xmin).max(0) as usize;

        let mut ww = 0.0;
        for (x, slot) in pre.iter_mut().take(cnt).enumerate() {
            let w = lanczos3(((x as i64 + xmin) as f64 - center + 0.5) * ss);
            *slot = w;
            ww += w;
        }
        if ww != 0.0 {
            for w in pre.iter_mut().take(cnt) {
                *w /= ww;
            }
        }
        let scale_fp = (1u32 << PRECISION_BITS) as f64;
        for x in 0..cnt {
            let w = pre[x];
            let v = if w < 0.0 { w * scale_fp - 0.5 } else { w * scale_fp + 0.5 };
            kk[xx * ksize + x] = v as i32;
        }
        bounds.push((xmin as usize, cnt));
    }
    Coeffs { ksize, bounds, kk }
}

#[inline]
fn clip8(v: i64) -> u8 {
    v.clamp(0, 255) as u8
}

/// Resize `src` (optionally only the region `bx`) to `dw` x `dh`.
pub fn resize_plane(src: &Plane, dw: usize, dh: usize, bx: BoxF) -> Plane {
    assert!(dw > 0 && dh > 0, "output size must be positive");
    let need_h = dw != src.w || bx.x0 != 0.0 || bx.x1 != src.w as f64;
    let need_v = dh != src.h || bx.y0 != 0.0 || bx.y1 != src.h as f64;

    if !need_h && !need_v {
        return src.clone();
    }

    let vert = if need_v { Some(precompute(src.h, bx.y0, bx.y1, dh)) } else { None };

    // Rows of the source that the vertical pass actually reads.
    let (row0, row1) = match &vert {
        Some(c) => {
            let first = c.bounds.iter().map(|b| b.0).min().unwrap_or(0);
            let last = c.bounds.iter().map(|b| b.0 + b.1).max().unwrap_or(src.h);
            (first, last.min(src.h))
        }
        None => (0, src.h),
    };

    // Horizontal pass -> temp image of width dw, rows row0..row1.
    let trows = row1 - row0;
    let mut temp = vec![0u8; trows * dw];
    if need_h {
        let c = precompute(src.w, bx.x0, bx.x1, dw);
        for ty in 0..trows {
            let srow = &src.data[(row0 + ty) * src.w..(row0 + ty + 1) * src.w];
            let trow = &mut temp[ty * dw..(ty + 1) * dw];
            for (xx, out) in trow.iter_mut().enumerate() {
                let (xmin, cnt) = c.bounds[xx];
                let k = &c.kk[xx * c.ksize..xx * c.ksize + cnt];
                let mut acc: i64 = 1 << (PRECISION_BITS - 1);
                for (i, kv) in k.iter().enumerate() {
                    acc += srow[xmin + i] as i64 * *kv as i64;
                }
                *out = clip8(acc >> PRECISION_BITS);
            }
        }
    } else {
        for ty in 0..trows {
            temp[ty * dw..(ty + 1) * dw]
                .copy_from_slice(&src.data[(row0 + ty) * src.w..(row0 + ty) * src.w + dw]);
        }
    }

    // Vertical pass.
    match vert {
        None => Plane { w: dw, h: dh, data: temp },
        Some(c) => {
            let mut out = vec![0u8; dw * dh];
            for yy in 0..dh {
                let (ymin, cnt) = c.bounds[yy];
                let k = &c.kk[yy * c.ksize..yy * c.ksize + cnt];
                for x in 0..dw {
                    let mut acc: i64 = 1 << (PRECISION_BITS - 1);
                    for (i, kv) in k.iter().enumerate() {
                        acc += temp[(ymin + i - row0) * dw + x] as i64 * *kv as i64;
                    }
                    out[yy * dw + x] = clip8(acc >> PRECISION_BITS);
                }
            }
            Plane { w: dw, h: dh, data: out }
        }
    }
}

/// Channel-wise resize of an RGB image.
pub fn resize_rgb(src: &RgbImage, dw: usize, dh: usize, bx: BoxF) -> RgbImage {
    let r = resize_plane(&src.plane(0), dw, dh, bx);
    let g = resize_plane(&src.plane(1), dw, dh, bx);
    let b = resize_plane(&src.plane(2), dw, dh, bx);
    RgbImage::from_planes(&r, &g, &b)
}

/// Integer nearest-neighbour upscale of a `w` x `h` buffer with `bpp` bytes per pixel.
pub fn upscale_nearest(src: &[u8], w: usize, h: usize, bpp: usize, s: usize) -> Vec<u8> {
    assert!(s >= 1);
    let (ow, oh) = (w * s, h * s);
    let mut out = Vec::with_capacity(ow * oh * bpp);
    for y in 0..oh {
        let sy = y / s;
        for x in 0..ow {
            let sx = x / s;
            let i = (sy * w + sx) * bpp;
            out.extend_from_slice(&src[i..i + bpp]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_resize_returns_same() {
        let p = Plane::new(3, 2, vec![1, 2, 3, 4, 5, 6]);
        assert_eq!(resize_plane(&p, 3, 2, BoxF::full(3, 2)), p);
    }

    #[test]
    fn constant_image_stays_constant() {
        let p = Plane::filled(37, 23, 129);
        let out = resize_plane(&p, 10, 7, BoxF::full(37, 23));
        assert!(out.data.iter().all(|&v| v == 129));
        let up = resize_plane(&p, 100, 80, BoxF::full(37, 23));
        assert!(up.data.iter().all(|&v| v == 129));
    }

    #[test]
    fn upscale_nearest_blocks() {
        let out = upscale_nearest(&[1, 2, 3, 4], 2, 2, 1, 2);
        assert_eq!(out, vec![1, 1, 2, 2, 1, 1, 2, 2, 3, 3, 4, 4, 3, 3, 4, 4]);
    }
}
