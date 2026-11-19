//! 4-tone ordered (Bayer 4x4) dithering. Same thresholds and matrix as the Python tool.

/// Level thresholds for 4-tone quantisation.
pub const LEVELS: [i32; 3] = [64, 128, 192];

/// 4x4 Bayer matrix (values 0..15). The Python code scales `m / 16 * 64`, i.e. `m * 4`.
pub const BAYER: [[i32; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];

/// Dither a grey buffer to levels 0..=3.
pub fn bayer_4tone(gray: &[u8], w: usize, h: usize) -> Vec<u8> {
    assert_eq!(gray.len(), w * h);
    let mut out = vec![0u8; w * h];
    for y in 0..h {
        for x in 0..w {
            let v = gray[y * w + x] as i32;
            let b = BAYER[y % 4][x % 4] * 4;
            out[y * w + x] = if v > LEVELS[2] + b {
                3
            } else if v > LEVELS[1] + b {
                2
            } else if v > LEVELS[0] + b {
                1
            } else {
                0
            };
        }
    }
    out
}

/// Plain thresholding without dithering (`np.digitize(v, LEVELS)`), used for frames.
#[inline]
pub fn threshold_level(v: u8) -> u8 {
    let v = v as i32;
    (v >= LEVELS[0]) as u8 + (v >= LEVELS[1]) as u8 + (v >= LEVELS[2]) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_black_and_white() {
        assert!(bayer_4tone(&[0; 16], 4, 4).iter().all(|&l| l == 0));
        assert!(bayer_4tone(&[255; 16], 4, 4).iter().all(|&l| l == 3));
    }

    #[test]
    fn mid_grey_mixes_two_levels() {
        let out = bayer_4tone(&[100; 16], 4, 4);
        assert!(out.contains(&1));
        assert!(out.contains(&0));
        assert!(out.iter().all(|&l| l <= 1));
    }

    #[test]
    fn threshold_edges() {
        assert_eq!(threshold_level(63), 0);
        assert_eq!(threshold_level(64), 1);
        assert_eq!(threshold_level(127), 1);
        assert_eq!(threshold_level(128), 2);
        assert_eq!(threshold_level(192), 3);
    }
}
