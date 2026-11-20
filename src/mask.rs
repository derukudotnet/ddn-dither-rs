//! Locally painted gamma adjustment for a grey plane, driven by an 8-bit coverage mask.
//!
//! The mask shares its dimensions with the plane it is applied to (`Plane` at source
//! resolution); 0 means "no adjustment", 255 means "full local gamma". Brush strokes
//! accumulate into the mask with a soft radial falloff, and the pipeline blends the
//! masked pixels toward the gamma curve before the contrast stretch.

use crate::autocontrast::gamma_lut;
use crate::gray::Plane;

/// Paint one soft brush dab centred on `cx`, `cy` with radius `r`.
///
/// `soft` (0..=0.95) is the fraction of the radius spent feathering: 0 is a hard
/// circle, the default 0.5 keeps full coverage over the inner half before the rim
/// feathers out. New coverage merges by taking the maximum, so scrubbing the same
/// area never over-applies. A radius of 0 or an out-of-bounds centre is safe.
pub fn dab(mask: &mut Plane, cx: f64, cy: f64, r: f64, soft: f64) {
    for_dab(mask, cx, cy, r, soft, |old, cov| *old = (*old).max(cov));
}

/// Lower existing coverage inside the dab, matching the falloff of [`dab`].
pub fn erase(mask: &mut Plane, cx: f64, cy: f64, r: f64, soft: f64) {
    for_dab(mask, cx, cy, r, soft, |old, cov| *old = old.saturating_sub(cov));
}

fn for_dab(
    mask: &mut Plane,
    cx: f64,
    cy: f64,
    r: f64,
    soft: f64,
    merge: impl Fn(&mut u8, u8),
) {
    if r <= 0.0 || mask.w == 0 || mask.h == 0 {
        return;
    }
    let x0 = ((cx - r).floor().max(0.0)) as usize;
    let x1 = ((cx + r).ceil() as usize + 1).min(mask.w);
    let y0 = ((cy - r).floor().max(0.0)) as usize;
    let y1 = ((cy + r).ceil() as usize + 1).min(mask.h);
    for y in y0..y1 {
        for x in x0..x1 {
            let d = (x as f64 - cx).hypot(y as f64 - cy) / r;
            if d >= 1.0 {
                continue;
            }
            let cov = (coverage(d, soft) * 255.0) as u8;
            merge(&mut mask.data[y * mask.w + x], cov);
        }
    }
}

/// Falloff within one dab radius, from full coverage at the centre to zero at the rim.
fn coverage(d: f64, soft: f64) -> f64 {
    let soft = soft.clamp(0.0, 0.95);
    if soft <= 0.0 {
        return 1.0;
    }
    let core = 1.0 - soft;
    if d <= core {
        1.0
    } else {
        (1.0 - (d - core) / soft).clamp(0.0, 1.0)
    }
}

/// Curve masked pixels toward the gamma `g` (1.0 = no-op), blended by coverage.
pub fn apply(p: &mut Plane, mask: &Plane, g: f64) {
    debug_assert_eq!((p.w, p.h), (mask.w, mask.h), "mask and plane must match");
    if (g - 1.0).abs() < 1e-9 {
        return;
    }
    let lut = gamma_lut(g);
    for (v, &m) in p.data.iter_mut().zip(&mask.data) {
        let c = m as f64 / 255.0;
        if c > 0.0 {
            let curved = lut[*v as usize] as f64;
            *v = (*v as f64 + (curved - *v as f64) * c) as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gamma_one_is_a_noop_even_when_fully_covered() {
        let mask = Plane::new(3, 1, vec![255, 255, 255]);
        let mut p = Plane::new(3, 1, vec![0, 64, 255]);
        let want = p.data.clone();
        apply(&mut p, &mask, 1.0);
        assert_eq!(p.data, want);
    }

    #[test]
    fn zero_coverage_is_untouched() {
        let mask = Plane::new(3, 1, vec![0; 3]);
        let mut p = Plane::new(3, 1, vec![0, 64, 255]);
        let want = p.data.clone();
        apply(&mut p, &mask, 0.5);
        assert_eq!(p.data, want);
    }

    #[test]
    fn full_coverage_matches_the_gamma_lut() {
        let mask = Plane::new(3, 1, vec![255; 3]);
        let mut p = Plane::new(3, 1, vec![0, 64, 255]);
        apply(&mut p, &mask, 0.5);
        let lut = gamma_lut(0.5);
        assert_eq!(p.data, vec![lut[0], lut[64], lut[255]]);
    }

    #[test]
    fn partial_coverage_blends_toward_the_curve() {
        let lut = gamma_lut(0.5);
        for (cov_byte, v) in [(0u8, 64u8), (128, 64), (255, 64)] {
            let mask = Plane::new(1, 1, vec![cov_byte]);
            let mut p = Plane::new(1, 1, vec![v]);
            apply(&mut p, &mask, 0.5);
            let c = cov_byte as f64 / 255.0;
            let want = (v as f64 + (lut[v as usize] as f64 - v as f64) * c) as u8;
            assert_eq!(p.data[0], want);
        }
    }

    #[test]
    fn dab_gives_full_coverage_in_the_core_and_feathers_at_the_rim() {
        let mut mask = Plane::new(21, 21, vec![0; 21 * 21]);
        dab(&mut mask, 10.0, 10.0, 8.0, 0.5);
        assert_eq!(mask.data[10 * 21 + 10], 255);
        let rim = mask.data[10 * 21 + 17];
        assert!(rim > 0 && rim < 255);
        assert_eq!(mask.data[10 * 21 + 19], 0);
    }

    #[test]
    fn dabbing_twice_never_lowers_coverage() {
        let mut mask = Plane::new(21, 21, vec![0; 21 * 21]);
        dab(&mut mask, 8.0, 10.0, 6.0, 0.5);
        let after_first = mask.data.clone();
        dab(&mut mask, 12.0, 10.0, 6.0, 0.5);
        assert!(mask.data.iter().zip(&after_first).all(|(a, b)| a >= b));
    }

    #[test]
    fn erase_removes_what_the_same_dab_painted() {
        let mut mask = Plane::new(21, 21, vec![0; 21 * 21]);
        dab(&mut mask, 10.0, 10.0, 8.0, 0.5);
        erase(&mut mask, 10.0, 10.0, 8.0, 0.5);
        assert_eq!(mask.data[10 * 21 + 10], 0);
        assert!(mask.data.iter().all(|&v| v == 0));
    }

    #[test]
    fn zero_softness_is_a_hard_circle() {
        let mut mask = Plane::new(21, 21, vec![0; 21 * 21]);
        dab(&mut mask, 10.0, 10.0, 8.0, 0.0);
        assert_eq!(mask.data[10 * 21 + 17], 255);
        assert_eq!(mask.data[10 * 21 + 19], 0);
    }

    #[test]
    fn high_softness_feathers_across_the_whole_radius() {
        let mut mask = Plane::new(21, 21, vec![0; 21 * 21]);
        dab(&mut mask, 10.0, 10.0, 8.0, 0.95);
        let near_rim = mask.data[10 * 21 + 17];
        assert!(near_rim > 0 && near_rim < 255, "still feathering near the rim: {near_rim}");
        let off_centre = mask.data[10 * 21 + 11];
        assert!(off_centre < 255, "even the inner zone is no longer flat");
    }

    #[test]
    fn dabs_outside_or_with_zero_radius_are_safe() {
        let mut mask = Plane::new(4, 4, vec![0; 16]);
        dab(&mut mask, -5.0, -5.0, 6.0, 0.5);
        dab(&mut mask, 10.0, 10.0, 6.0, 0.5);
        dab(&mut mask, 2.0, 2.0, 0.0, 0.5);
        assert!(mask.data.iter().all(|&v| v == 0));
    }
}
