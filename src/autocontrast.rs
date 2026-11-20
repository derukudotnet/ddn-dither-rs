//! Port of Pillow's `ImageOps.autocontrast(img, cutoff=(lo, hi))` for one grey plane,
//! plus a manual contrast adjustment around mid-grey.

use crate::gray::Plane;

/// Cutoff used by the original script: nothing from the dark end, 10 % from the light end.
pub const DEFAULT_CUTOFF: (f64, f64) = (0.0, 10.0);

pub fn lut(hist: &[u64; 256], cutoff: (f64, f64)) -> [u8; 256] {
    let mut h = *hist;
    let n: u64 = h.iter().sum();

    if cutoff.0 != 0.0 || cutoff.1 != 0.0 {
        let mut cut = (n as f64 * cutoff.0 / 100.0).floor() as u64;
        for item in h.iter_mut() {
            if cut > *item {
                cut -= *item;
                *item = 0;
            } else {
                *item -= cut;
                cut = 0;
            }
            if cut == 0 {
                break;
            }
        }
        let mut cut = (n as f64 * cutoff.1 / 100.0).floor() as u64;
        for item in h.iter_mut().rev() {
            if cut > *item {
                cut -= *item;
                *item = 0;
            } else {
                *item -= cut;
                cut = 0;
            }
            if cut == 0 {
                break;
            }
        }
    }

    let lo = h.iter().position(|&v| v != 0).unwrap_or(0);
    let hi = h.iter().rposition(|&v| v != 0).unwrap_or(0);

    let mut out = [0u8; 256];
    if hi <= lo {
        for (i, o) in out.iter_mut().enumerate() {
            *o = i as u8;
        }
    } else {
        let scale = 255.0 / (hi - lo) as f64;
        let offset = -(lo as f64) * scale;
        for (i, o) in out.iter_mut().enumerate() {
            let v = (i as f64 * scale + offset) as i64; // truncates toward zero, like int()
            *o = v.clamp(0, 255) as u8;
        }
    }
    out
}

/// Curve the plane with a gamma exponent before the contrast stretch. A value of 1.0 is a no-op.
pub fn gamma(p: &mut Plane, g: f64) {
    if (g - 1.0).abs() < 1e-9 {
        return;
    }
    let lut = gamma_lut(g);
    for v in p.data.iter_mut() {
        *v = lut[*v as usize];
    }
}

pub fn gamma_lut(g: f64) -> [u8; 256] {
    let mut out = [0u8; 256];
    for (i, o) in out.iter_mut().enumerate() {
        *o = (255.0 * (i as f64 / 255.0).powf(g)).round().clamp(0.0, 255.0) as u8;
    }
    out
}

pub fn autocontrast(p: &mut Plane, cutoff: (f64, f64)) {
    let mut hist = [0u64; 256];
    for &v in &p.data {
        hist[v as usize] += 1;
    }
    let table = lut(&hist, cutoff);
    for v in p.data.iter_mut() {
        *v = table[*v as usize];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stretches_range() {
        let mut p = Plane::new(4, 1, vec![50, 100, 150, 200]);
        autocontrast(&mut p, (0.0, 0.0));
        assert_eq!(p.data[0], 0);
        assert_eq!(p.data[3], 255);
    }

    #[test]
    fn flat_image_is_unchanged() {
        let mut p = Plane::filled(5, 5, 77);
        autocontrast(&mut p, DEFAULT_CUTOFF);
        assert!(p.data.iter().all(|&v| v == 77));
    }

    #[test]
    fn gamma_one_is_identity() {
        let mut p = Plane::new(4, 1, vec![0, 64, 128, 255]);
        gamma(&mut p, 1.0);
        assert_eq!(p.data, &[0, 64, 128, 255]);
    }

    #[test]
    fn gamma_below_one_lifts_shadows() {
        let mut p = Plane::new(5, 1, vec![0, 64, 128, 224, 255]);
        gamma(&mut p, 0.5);
        assert_eq!(p.data[0], 0);
        assert_eq!(p.data[1], 128);
        assert!(p.data[1] < p.data[2]);
        assert_eq!(p.data[4], 255);
    }
}
