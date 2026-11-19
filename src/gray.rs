//! Basic image containers and the grey conversion (identical to Pillow's "L").

/// Pillow's RGB -> L conversion, in integer arithmetic.
#[inline]
pub fn luma(r: u8, g: u8, b: u8) -> u8 {
    ((19595 * r as u32 + 38470 * g as u32 + 7471 * b as u32 + 0x8000) >> 16) as u8
}

/// A single 8-bit channel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plane {
    pub w: usize,
    pub h: usize,
    pub data: Vec<u8>,
}

impl Plane {
    pub fn new(w: usize, h: usize, data: Vec<u8>) -> Plane {
        assert_eq!(data.len(), w * h, "plane data length mismatch");
        Plane { w, h, data }
    }

    pub fn filled(w: usize, h: usize, v: u8) -> Plane {
        Plane { w, h, data: vec![v; w * h] }
    }
}

/// Interleaved 8-bit RGB.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RgbImage {
    pub w: usize,
    pub h: usize,
    pub data: Vec<u8>,
}

impl RgbImage {
    pub fn new(w: usize, h: usize, data: Vec<u8>) -> RgbImage {
        assert_eq!(data.len(), w * h * 3, "rgb data length mismatch");
        RgbImage { w, h, data }
    }

    pub fn to_gray(&self) -> Plane {
        let data = self.data.chunks_exact(3).map(|p| luma(p[0], p[1], p[2])).collect();
        Plane { w: self.w, h: self.h, data }
    }

    pub fn plane(&self, channel: usize) -> Plane {
        let data = self.data.chunks_exact(3).map(|p| p[channel]).collect();
        Plane { w: self.w, h: self.h, data }
    }

    pub fn from_planes(r: &Plane, g: &Plane, b: &Plane) -> RgbImage {
        let mut data = Vec::with_capacity(r.w * r.h * 3);
        for i in 0..r.data.len() {
            data.push(r.data[i]);
            data.push(g.data[i]);
            data.push(b.data[i]);
        }
        RgbImage { w: r.w, h: r.h, data }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn luma_extremes() {
        assert_eq!(luma(0, 0, 0), 0);
        assert_eq!(luma(255, 255, 255), 255);
        assert_eq!(luma(255, 0, 0), 76);
    }

    #[test]
    fn planes_roundtrip() {
        let img = RgbImage::new(2, 1, vec![1, 2, 3, 4, 5, 6]);
        let back = RgbImage::from_planes(&img.plane(0), &img.plane(1), &img.plane(2));
        assert_eq!(img, back);
    }
}
