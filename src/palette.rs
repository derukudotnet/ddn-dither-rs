//! 4-colour palettes (darkest to lightest).

pub type Rgb = [u8; 3];
pub type Palette = [Rgb; 4];

pub const DEFAULT_NAME: &str = "ddn";

pub const BUILTIN: [(&str, Palette); 9] = [
    ("ddn", [[15, 15, 15], [74, 74, 74], [134, 134, 134], [193, 193, 193]]),
    ("green", [[48, 58, 29], [108, 117, 88], [167, 177, 148], [227, 236, 207]]),
    ("bgb", [[8, 24, 32], [52, 104, 86], [136, 192, 112], [224, 248, 208]]),
    ("pocket", [[57, 57, 57], [117, 117, 117], [176, 176, 176], [236, 236, 236]]),
    ("bw", [[0, 0, 0], [85, 85, 85], [170, 170, 170], [255, 255, 255]]),
    ("demichrome", [[33, 30, 32], [85, 85, 104], [160, 160, 139], [233, 239, 236]]),
    ("icecream", [[124, 63, 88], [235, 107, 111], [249, 168, 117], [255, 246, 211]]),
    ("kirokaze", [[51, 44, 80], [70, 135, 143], [148, 227, 68], [226, 243, 228]]),
    ("color", [[30, 30, 110], [100, 100, 180], [240, 200, 90], [250, 250, 200]]),
];

pub fn by_name(name: &str) -> Option<Palette> {
    let name = name.trim().to_ascii_lowercase();
    BUILTIN.iter().find(|(n, _)| *n == name).map(|(_, p)| *p)
}

pub fn names() -> Vec<&'static str> {
    BUILTIN.iter().map(|(n, _)| *n).collect()
}

pub fn parse_hex(s: &str) -> Result<Rgb, String> {
    let t = s.trim().trim_start_matches('#');
    let t: String = if t.len() == 3 && t.chars().all(|c| c.is_ascii_hexdigit()) {
        t.chars().flat_map(|c| [c, c]).collect()
    } else {
        t.to_string()
    };
    if t.len() != 6 || !t.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!("'{s}' is not a colour like #303a1d"));
    }
    let v = u32::from_str_radix(&t, 16).map_err(|e| e.to_string())?;
    Ok([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

/// Colour a buffer of dither levels (0..=3) with `pal`, as interleaved RGB.
pub fn colour_levels(levels: &[u8], pal: &Palette) -> Vec<u8> {
    levels.iter().flat_map(|&l| pal[l as usize & 3]).collect()
}

/// Accepts a built-in name ("green") or four comma-separated hex colours, darkest first.
pub fn resolve(spec: &str) -> Result<Palette, String> {
    if let Some(p) = by_name(spec) {
        return Ok(p);
    }
    let parts: Vec<&str> = spec.split(',').collect();
    if parts.len() != 4 {
        return Err(format!(
            "unknown palette '{spec}' (use one of: {} or four hex colours like #000,...)",
            names().join(", ")
        ));
    }
    let mut out = [[0u8; 3]; 4];
    for (i, p) in parts.iter().enumerate() {
        out[i] = parse_hex(p)?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_and_custom() {
        assert_eq!(by_name("GREEN").unwrap()[0], [48, 58, 29]);
        let p = resolve("#000000,#555555,#aaaaaa,#ffffff").unwrap();
        assert_eq!(p, by_name("bw").unwrap());
        assert!(resolve("nope").is_err());
        assert!(parse_hex("#12345").is_err());
        assert_eq!(parse_hex("#555").unwrap(), [85, 85, 85]);
        assert_eq!(parse_hex("0f380f").unwrap(), [15, 56, 15]);
        assert_eq!(colour_levels(&[0, 3], &by_name("bw").unwrap()), vec![0, 0, 0, 255, 255, 255]);
    }

    #[test]
    fn builtins_are_ordered_dark_to_light() {
        let lum = |c: Rgb| 0.2126 * c[0] as f64 + 0.7152 * c[1] as f64 + 0.0722 * c[2] as f64;
        for (name, p) in BUILTIN.iter() {
            for w in p.windows(2) {
                assert!(lum(w[0]) <= lum(w[1]), "{name}: {w:?} not dark to light");
            }
        }
        assert_eq!(by_name("kirokaze").unwrap()[0], [51, 44, 80]);
        assert_eq!(resolve("BGB").unwrap(), by_name("bgb").unwrap());
    }
}
