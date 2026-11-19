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

/// Look-up table that boosts contrast around mid-grey. `gain` 1.0 = unchanged,
/// > 1.0 = more contrast, < 1.0 = washed out.
pub fn contrast_lut(gain: f64) -> [u8; 256] {
    let mut out = [0u8; 256];
    for (i, o) in out.iter_mut().enumerate() {
        let v = 128.0 + (i as f64 - 128.0) * gain;
        *o = v.round().clamp(0.0, 255.0) as u8;
    }
    out
}

/// Apply manual contrast to a plane. A gain of 1.0 leaves the image unchanged.
pub fn contrast(p: &mut Plane, gain: f64) {
    if (gain - 1.0).abs() < 1e-9 {
        return;
    }
    let lut = contrast_lut(gain);
    for v in p.data.iter_mut() {
        *v = lut[*v as usize];
    }
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
        let mut p = Plane::filled(5, 5, 128);
        contrast(&mut p, 2.0);
        assert!(p.data.iter().all(|&v| v == 128));
    }

    #[test]
    fn contrast_gain_one_is_nop_and_doubles_push_outwards() {
        let p0 = Plane::new(5, 1, vec![0, 64, 128, 192, 255]);
        let mut p = p0.clone();
        contrast(&mut p, 1.0);
        assert_eq!(p.data, p0.data);
        let mut p = p0.clone();
        contrast(&mut p, 2.0);
        assert_eq!(p.data, vec![0, 0, 128, 255, 255]);
        let mut p = p0.clone();
        contrast(&mut p, 0.5);
        assert_eq!(p.data, vec![64, 96, 128, 160, 192]);
        assert_eq!(contrast_lut(2.0)[128], 128);
    }
}
