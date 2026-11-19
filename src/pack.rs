//! Packing of 2-bit indexed pixels for PNG output.

/// Pack level values (0..=3) into 2 bits per pixel, MSB first, rows padded to whole bytes.
pub fn pack_2bit(levels: &[u8], w: usize, h: usize) -> Vec<u8> {
    assert_eq!(levels.len(), w * h);
    let row_bytes = w.div_ceil(4);
    let mut out = vec![0u8; row_bytes * h];
    for y in 0..h {
        for x in 0..w {
            let v = levels[y * w + x] & 3;
            out[y * row_bytes + x / 4] |= v << (6 - 2 * (x % 4));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packs_msb_first_with_row_padding() {
        // 5 pixels wide: second row starts on a new byte.
        let out = pack_2bit(&[0, 1, 2, 3, 3, 3, 2, 1, 0, 0], 5, 2);
        assert_eq!(out, vec![0b00_01_10_11, 0b11_00_00_00, 0b11_10_01_00, 0b00_00_00_00]);
    }
}
