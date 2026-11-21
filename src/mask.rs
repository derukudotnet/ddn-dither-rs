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

/// Set or erase the coverage inside a closed polygon, feathering outward by `feather` pixels.
///
/// `erase` scales the covered coverage down by the fill strength, otherwise it is merged in at
/// full strength. The interior is always full coverage; `feather` extends a linear falloff
/// beyond the outline so edges glow softly instead of cutting off. Fewer than three points or
/// self-intersecting outlines follow the standard even-odd rule.
pub fn fill_polygon(mask: &mut Plane, pts: &[(f64, f64)], feather: f64, erase: bool) {
    let n = pts.len();
    if n < 3 || mask.w == 0 || mask.h == 0 {
        return;
    }
    let band = feather.max(0.0);
    let min_x = pts.iter().map(|p| p.0).fold(f64::INFINITY, f64::min) - band;
    let max_x = pts.iter().map(|p| p.0).fold(f64::NEG_INFINITY, f64::max) + band;
    let min_y = pts.iter().map(|p| p.1).fold(f64::INFINITY, f64::min) - band;
    let max_y = pts.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max) + band;
    let x0 = (min_x.floor().max(0.0) as usize).min(mask.w);
    let x1 = ((max_x.ceil() as usize) + 1).min(mask.w);
    let y0 = (min_y.floor().max(0.0) as usize).min(mask.h);
    let y1 = ((max_y.ceil() as usize) + 1).min(mask.h);
    for y in y0..y1 {
        for x in x0..x1 {
            let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
            let strength = if point_in_polygon(px, py, pts) {
                1.0
            } else if band > 0.0 {
                (1.0 - edge_distance(px, py, pts) / band).clamp(0.0, 1.0)
            } else {
                0.0
            };
            if strength <= 0.0 {
                continue;
            }
            let i = y * mask.w + x;
            mask.data[i] = if erase {
                (mask.data[i] as f64 * (1.0 - strength)) as u8
            } else {
                mask.data[i].max((strength * 255.0) as u8)
            };
        }
    }
}

fn point_in_polygon(px: f64, py: f64, pts: &[(f64, f64)]) -> bool {
    let n = pts.len();
    let mut inside = false;
    for i in 0..n {
        let (ax, ay) = pts[i];
        let (bx, by) = pts[(i + 1) % n];
        if (ay <= py && by > py) || (by <= py && ay > py) {
            let x_int = ax + (py - ay) / (by - ay) * (bx - ax);
            if x_int >= px {
                inside = !inside;
            }
        }
    }
    inside
}

fn edge_distance(px: f64, py: f64, pts: &[(f64, f64)]) -> f64 {
    let n = pts.len();
    let mut best = f64::INFINITY;
    for i in 0..n {
        let (ax, ay) = pts[i];
        let (bx, by) = pts[(i + 1) % n];
        let (dx, dy) = (bx - ax, by - ay);
        let len2 = dx * dx + dy * dy;
        let t = if len2 > 0.0 {
            (((px - ax) * dx + (py - ay) * dy) / len2).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let d = (px - (ax + t * dx)).hypot(py - (ay + t * dy));
        best = best.min(d);
    }
    best
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


/// One recorded touch-up action, replayable in order to rebuild the mask.
///
/// Brush parameters are frozen when the action is created, so replaying after the
/// sliders have moved reproduces exactly what was painted at the time.
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    /// Brush dabs along `pts`, in source pixel positions.
    Brush {
        erase: bool,
        radius: f64,
        soft: f64,
        pts: Vec<(f64, f64)>,
    },
    /// A closed polygon fill feathering outward by `feather` pixels.
    Polygon {
        erase: bool,
        feather: f64,
        pts: Vec<(f64, f64)>,
    },
    /// Removes all coverage recorded so far.
    Clear,
}

/// Rebuild the coverage by applying `steps` in order, optionally starting from a
/// previously taken `(step_count, plane)` checkpoint to shorten the replay.
///
/// Returns `None` when no painting happened since a `Clear` or when `steps` is empty.
pub fn replay(steps: &[Step], w: usize, h: usize, checkpoint: Option<(usize, &Plane)>) -> Option<Plane> {
    let mut out: Option<Plane> = None;
    let mut start = 0usize;
    if let Some((n, plane)) = checkpoint {
        if n <= steps.len() && (plane.w, plane.h) == (w, h) {
            out = Some(plane.clone());
            start = n;
        }
    }
    for step in &steps[start..] {
        match step {
            Step::Clear => out = None,
            Step::Brush { erase: do_erase, radius, soft, pts } => {
                let (r, s) = (*radius, *soft);
                let m = out.get_or_insert_with(|| Plane::new(w, h, vec![0; w * h]));
                for &(x, y) in pts {
                    if *do_erase {
                        erase(m, x, y, r, s);
                    } else {
                        dab(m, x, y, r, s);
                    }
                }
            }
            Step::Polygon { erase, feather, pts } => {
                let m = out.get_or_insert_with(|| Plane::new(w, h, vec![0; w * h]));
                fill_polygon(m, pts, *feather, *erase);
            }
        }
    }
    out
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
    fn polygon_fill_covers_the_inside_only() {
        let mut mask = Plane::new(10, 10, vec![0; 100]);
        fill_polygon(
            &mut mask,
            &[(2.0, 2.0), (8.0, 2.0), (8.0, 8.0), (2.0, 8.0)],
            0.0,
            false,
        );
        for y in 0..10 {
            for x in 0..10 {
                let want = (2..=7).contains(&x) && (2..=7).contains(&y);
                assert_eq!(mask.data[y * 10 + x] == 255, want, "pixel {x},{y}");
            }
        }
    }

    #[test]
    fn polygon_erase_resets_the_inside() {
        let mut mask = Plane::new(10, 10, vec![0; 100]);
        let square = [(2.0, 2.0), (8.0, 2.0), (8.0, 8.0), (2.0, 8.0)];
        fill_polygon(&mut mask, &square, 0.0, false);
        dab(&mut mask, 5.0, 5.0, 2.0, 0.5);
        assert!(mask.data[5 * 10 + 5] == 255);
        fill_polygon(&mut mask, &square, 0.0, true);
        assert!(mask.data.iter().all(|&v| v == 0));
    }

    #[test]
    fn polygon_triangle_is_half_the_bounding_square() {
        let mut mask = Plane::new(8, 8, vec![0; 64]);
        fill_polygon(&mut mask, &[(0.0, 0.0), (8.0, 0.0), (0.0, 8.0)], 0.0, false);
        let covered = mask.data.iter().filter(|&&v| v == 255).count();
        assert_eq!(covered, 36, "8+7+6+...+1 pixels fill the upper-left triangle");
    }

    #[test]
    fn polygon_feather_grows_outward_softly() {
        let mut mask = Plane::new(12, 10, vec![0; 120]);
        let square = [(2.0, 2.0), (8.0, 2.0), (8.0, 8.0), (2.0, 8.0)];
        fill_polygon(&mut mask, &square, 2.0, false);
        assert_eq!(mask.data[5 * 12 + 5], 255);
        let glow = mask.data[5 * 12 + 9];
        assert!(glow > 0 && glow < 255, "one pixel outside fades softly: {glow}");
        assert_eq!(mask.data[5 * 12 + 11], 0);

        let mut full = Plane::new(12, 10, vec![255; 120]);
        fill_polygon(&mut full, &square, 2.0, true);
        assert_eq!(full.data[5 * 12 + 5], 0);
        let faded = full.data[5 * 12 + 9];
        assert!(faded > 0 && faded < 255, "erasing fades instead of hard-zeroing: {faded}");
        assert_eq!(full.data[5 * 12 + 11], 255);
    }

    #[test]
    fn polygon_with_fewer_than_three_points_is_ignored() {
        let mut mask = Plane::new(4, 4, vec![0; 16]);
        fill_polygon(&mut mask, &[(1.0, 1.0), (3.0, 3.0)], 0.0, false);
        assert!(mask.data.iter().all(|&v| v == 0));
    }

    #[test]
    fn replay_matches_live_dabbing() {
        let mut live = Plane::new(40, 40, vec![0; 1600]);
        dab(&mut live, 10.0, 10.0, 6.0, 0.5);
        dab(&mut live, 20.0, 12.0, 6.0, 0.0);
        erase(&mut live, 10.0, 10.0, 4.0, 0.0);
        let steps = [
            Step::Brush { erase: false, radius: 6.0, soft: 0.5, pts: vec![(10.0, 10.0)] },
            Step::Brush { erase: false, radius: 6.0, soft: 0.0, pts: vec![(20.0, 12.0)] },
            Step::Brush { erase: true, radius: 4.0, soft: 0.0, pts: vec![(10.0, 10.0)] },
        ];
        assert_eq!(replay(&steps, 40, 40, None).unwrap(), live);
    }

    #[test]
    fn dropping_the_last_step_changes_the_replay() {
        let steps = [
            Step::Brush { erase: false, radius: 6.0, soft: 0.0, pts: vec![(10.0, 10.0)] },
            Step::Brush { erase: false, radius: 6.0, soft: 0.0, pts: vec![(25.0, 25.0)] },
        ];
        let with_both = replay(&steps, 40, 40, None).unwrap();
        let undone = replay(&steps[..1], 40, 40, None).unwrap();
        assert_ne!(with_both, undone);
        assert_eq!(undone.data[10 * 40 + 10], 255);
        assert_eq!(undone.data[25 * 40 + 25], 0);
    }

    #[test]
    fn replaying_from_a_checkpoint_matches_a_full_replay() {
        let steps = [
            Step::Brush { erase: false, radius: 6.0, soft: 0.0, pts: vec![(10.0, 10.0)] },
            Step::Brush { erase: false, radius: 6.0, soft: 0.0, pts: vec![(25.0, 25.0)] },
            Step::Brush { erase: false, radius: 4.0, soft: 0.5, pts: vec![(18.0, 12.0)] },
        ];
        let ckpt = replay(&steps[..2], 40, 40, None).unwrap();
        assert_eq!(replay(&steps, 40, 40, Some((2, &ckpt))).unwrap(), replay(&steps, 40, 40, None).unwrap());
        let fresh = Plane::new(8, 8, vec![0; 64]);
        assert_eq!(replay(&steps, 40, 40, Some((2, &fresh))).unwrap(), replay(&steps, 40, 40, None).unwrap());
    }

    #[test]
    fn a_clear_step_empties_the_replay_until_new_painting() {
        let steps = [
            Step::Brush { erase: false, radius: 6.0, soft: 0.0, pts: vec![(10.0, 10.0)] },
            Step::Clear,
            Step::Brush { erase: false, radius: 4.0, soft: 0.0, pts: vec![(30.0, 30.0)] },
        ];
        assert!(replay(&steps[..2], 40, 40, None).is_none());
        let after = replay(&steps, 40, 40, None).unwrap();
        assert_eq!(after.data[30 * 40 + 30], 255);
        assert_eq!(after.data[10 * 40 + 10], 0);
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
