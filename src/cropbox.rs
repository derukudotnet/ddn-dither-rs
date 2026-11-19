//! Fixed-aspect crop rectangle logic for the interactive window.
//!
//! All coordinates are source-image pixels (f64). Pure geometry, no GUI types, so it is
//! unit-tested here. Resizing keeps the opposite corner (or edge) fixed; side handles keep
//! the box centred on the other axis.

use crate::resample::BoxF;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CropBox {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handle {
    Move,
    N,
    S,
    E,
    W,
    NE,
    NW,
    SE,
    SW,
}

impl CropBox {
    pub fn right(&self) -> f64 {
        self.x + self.w
    }

    pub fn bottom(&self) -> f64 {
        self.y + self.h
    }

    pub fn center(&self) -> (f64, f64) {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }

    pub fn to_box(&self) -> BoxF {
        BoxF { x0: self.x, y0: self.y, x1: self.right(), y1: self.bottom() }
    }

    /// Largest box of this aspect that fits the image, centred.
    pub fn max_fit(img_w: f64, img_h: f64, aspect: f64) -> CropBox {
        let (w, h) = if img_w / img_h > aspect { (img_h * aspect, img_h) } else { (img_w, img_w / aspect) };
        CropBox { x: (img_w - w) / 2.0, y: (img_h - h) / 2.0, w, h }
    }

    /// Change the aspect ratio but keep the centre and (as far as possible) the area.
    pub fn with_aspect(&self, aspect: f64, img_w: f64, img_h: f64) -> CropBox {
        let area = (self.w * self.h).max(1.0);
        let mut h = (area / aspect).sqrt();
        let mut w = h * aspect;
        let (maxw, maxh) = max_size(img_w, img_h, aspect);
        if w > maxw {
            w = maxw;
            h = w / aspect;
        }
        if h > maxh {
            h = maxh;
            w = h * aspect;
        }
        let (cx, cy) = self.center();
        CropBox { x: cx - w / 2.0, y: cy - h / 2.0, w, h }.clamped_position(img_w, img_h)
    }

    /// Slide the box inside the image without changing its size.
    pub fn clamped_position(mut self, img_w: f64, img_h: f64) -> CropBox {
        self.x = self.x.clamp(0.0, (img_w - self.w).max(0.0));
        self.y = self.y.clamp(0.0, (img_h - self.h).max(0.0));
        self
    }

    /// Which handle (if any) is under the image-space point `(px, py)`.
    /// `tol` is the grab distance in image pixels.
    pub fn hit(&self, px: f64, py: f64, tol: f64) -> Option<Handle> {
        let (l, t, r, b) = (self.x, self.y, self.right(), self.bottom());
        let near_l = (px - l).abs() <= tol;
        let near_r = (px - r).abs() <= tol;
        let near_t = (py - t).abs() <= tol;
        let near_b = (py - b).abs() <= tol;
        let in_x = px >= l - tol && px <= r + tol;
        let in_y = py >= t - tol && py <= b + tol;

        if near_t && near_l {
            return Some(Handle::NW);
        }
        if near_t && near_r {
            return Some(Handle::NE);
        }
        if near_b && near_l {
            return Some(Handle::SW);
        }
        if near_b && near_r {
            return Some(Handle::SE);
        }
        if near_t && in_x {
            return Some(Handle::N);
        }
        if near_b && in_x {
            return Some(Handle::S);
        }
        if near_l && in_y {
            return Some(Handle::W);
        }
        if near_r && in_y {
            return Some(Handle::E);
        }
        if px >= l && px <= r && py >= t && py <= b {
            return Some(Handle::Move);
        }
        None
    }

    /// New box while dragging `handle`. `start` is the box when the drag began,
    /// `start_ptr` the pointer then, `ptr` the pointer now (all in image pixels).
    #[allow(clippy::too_many_arguments)]
    pub fn drag(
        start: &CropBox,
        handle: Handle,
        start_ptr: (f64, f64),
        ptr: (f64, f64),
        aspect: f64,
        img_w: f64,
        img_h: f64,
        min_w: f64,
    ) -> CropBox {
        let min_w = min_w.max(1.0);
        let (maxw_img, maxh_img) = max_size(img_w, img_h, aspect);
        let max_w_total = maxw_img.min(maxh_img * aspect);

        match handle {
            Handle::Move => CropBox {
                x: start.x + (ptr.0 - start_ptr.0),
                y: start.y + (ptr.1 - start_ptr.1),
                w: start.w,
                h: start.h,
            }
            .clamped_position(img_w, img_h),

            Handle::NE | Handle::NW | Handle::SE | Handle::SW => {
                let east = matches!(handle, Handle::NE | Handle::SE);
                let south = matches!(handle, Handle::SE | Handle::SW);
                // Anchor = the opposite corner.
                let ax = if east { start.x } else { start.right() };
                let ay = if south { start.y } else { start.bottom() };
                let raw_w = if east { ptr.0 - ax } else { ax - ptr.0 };
                let raw_h = if south { ptr.1 - ay } else { ay - ptr.1 };
                let mut w = raw_w.max(raw_h * aspect).max(min_w);
                let avail_w = if east { img_w - ax } else { ax };
                let avail_h = if south { img_h - ay } else { ay };
                w = w.min(avail_w).min(avail_h * aspect).min(max_w_total);
                w = w.max(min_w.min(avail_w).min(avail_h * aspect));
                let h = w / aspect;
                CropBox {
                    x: if east { ax } else { ax - w },
                    y: if south { ay } else { ay - h },
                    w,
                    h,
                }
            }

            Handle::E | Handle::W => {
                let east = handle == Handle::E;
                let ax = if east { start.x } else { start.right() };
                let raw_w = if east { ptr.0 - ax } else { ax - ptr.0 };
                let avail_w = if east { img_w - ax } else { ax };
                let w = raw_w.max(min_w).min(avail_w).min(max_w_total);
                let h = w / aspect;
                let (_, cy) = start.center();
                CropBox { x: if east { ax } else { ax - w }, y: cy - h / 2.0, w, h }
                    .clamped_position(img_w, img_h)
            }

            Handle::N | Handle::S => {
                let south = handle == Handle::S;
                let ay = if south { start.y } else { start.bottom() };
                let raw_h = if south { ptr.1 - ay } else { ay - ptr.1 };
                let avail_h = if south { img_h - ay } else { ay };
                let h = raw_h.max(min_w / aspect).min(avail_h).min(max_w_total / aspect);
                let w = h * aspect;
                let (cx, _) = start.center();
                CropBox { x: cx - w / 2.0, y: if south { ay } else { ay - h }, w, h }
                    .clamped_position(img_w, img_h)
            }
        }
    }
}

/// Largest (w, h) of this aspect that fits inside the image.
fn max_size(img_w: f64, img_h: f64, aspect: f64) -> (f64, f64) {
    if img_w / img_h > aspect {
        (img_h * aspect, img_h)
    } else {
        (img_w, img_w / aspect)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const IW: f64 = 4000.0;
    const IH: f64 = 3000.0;
    const A: f64 = 16.0 / 7.0;

    fn check(b: &CropBox) {
        assert!((b.w / b.h - A).abs() < 1e-9, "aspect broken: {b:?}");
        assert!(b.x >= -1e-9 && b.y >= -1e-9, "outside: {b:?}");
        assert!(b.right() <= IW + 1e-9 && b.bottom() <= IH + 1e-9, "outside: {b:?}");
        assert!(b.w >= 1.0 - 1e-9);
    }

    #[test]
    fn max_fit_is_centred_and_fits() {
        let b = CropBox::max_fit(IW, IH, A);
        check(&b);
        assert_eq!(b.w, IW);
        assert!((b.y - (IH - b.h) / 2.0).abs() < 1e-9);
    }

    #[test]
    fn every_handle_keeps_aspect_and_bounds() {
        let start = CropBox { x: 1000.0, y: 1000.0, w: 1600.0, h: 1600.0 / A };
        let handles = [
            Handle::Move, Handle::N, Handle::S, Handle::E, Handle::W,
            Handle::NE, Handle::NW, Handle::SE, Handle::SW,
        ];
        let pointers = [
            (0.0, 0.0), (4000.0, 3000.0), (-500.0, 5000.0), (2000.0, 1500.0),
            (1000.0, 1000.0), (2600.0, 1900.0), (1005.0, 2000.0), (9999.0, -9999.0),
        ];
        for h in handles {
            for p in pointers {
                let b = CropBox::drag(&start, h, (1500.0, 1200.0), p, A, IW, IH, 16.0);
                check(&b);
            }
        }
    }

    #[test]
    fn corner_drag_keeps_opposite_corner_fixed() {
        let start = CropBox { x: 1000.0, y: 1000.0, w: 1600.0, h: 1600.0 / A };
        let b = CropBox::drag(&start, Handle::SE, (2600.0, 1700.0), (3000.0, 2000.0), A, IW, IH, 16.0);
        assert_eq!((b.x, b.y), (1000.0, 1000.0));
        assert!(b.w > start.w);
        let b = CropBox::drag(&start, Handle::NW, (1000.0, 1000.0), (800.0, 700.0), A, IW, IH, 16.0);
        assert!((b.right() - start.right()).abs() < 1e-9);
        assert!((b.bottom() - start.bottom()).abs() < 1e-9);
    }

    #[test]
    fn move_stops_at_the_edges() {
        let start = CropBox { x: 100.0, y: 100.0, w: 800.0, h: 800.0 / A };
        let b = CropBox::drag(&start, Handle::Move, (0.0, 0.0), (-5000.0, 9000.0), A, IW, IH, 16.0);
        assert_eq!(b.x, 0.0);
        assert!((b.bottom() - IH).abs() < 1e-9);
    }

    #[test]
    fn aspect_change_keeps_centre() {
        let start = CropBox { x: 1000.0, y: 800.0, w: 1200.0, h: 900.0 };
        let b = start.with_aspect(21.0 / 9.0, IW, IH);
        assert!((b.w / b.h - 21.0 / 9.0).abs() < 1e-9);
        let (c0, c1) = (start.center(), b.center());
        assert!((c0.0 - c1.0).abs() < 1e-6 && (c0.1 - c1.1).abs() < 1e-6);
    }

    #[test]
    fn hit_testing() {
        let b = CropBox { x: 100.0, y: 100.0, w: 400.0, h: 200.0 };
        assert_eq!(b.hit(100.0, 100.0, 5.0), Some(Handle::NW));
        assert_eq!(b.hit(500.0, 300.0, 5.0), Some(Handle::SE));
        assert_eq!(b.hit(300.0, 100.0, 5.0), Some(Handle::N));
        assert_eq!(b.hit(300.0, 200.0, 5.0), Some(Handle::Move));
        assert_eq!(b.hit(50.0, 50.0, 5.0), None);
    }
}
