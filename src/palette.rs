//! 4-colour palettes (darkest to lightest).

pub type Rgb = [u8; 3];
pub type Palette = [Rgb; 4];

pub const DEFAULT_NAME: &str = "ddn";

pub const BUILTIN: [(&str, Palette); 5] = [
    ("ddn", [[15, 15, 15], [74, 74, 74], [134, 134, 134], [193, 193, 193]]),
    ("green", [[48, 58, 29], [108, 117, 88], [167, 177, 148], [227, 236, 207]]),
    ("bw", [[0, 0, 0], [85, 85, 85], [170, 170, 170], [255, 255, 255]]),
    ("pocket", [[57, 57, 57], [117, 117, 117], [176, 176, 176], [236, 236, 236]]),
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
    if t.len() != 6 || !t.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!("'{s}' is not a colour like #303a1d"));
    }
    let v = u32::from_str_radix(t, 16).map_err(|e| e.to_string())?;
    Ok([(v >> 16) as u8, (v >> 8) as u8, v as u8])
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
    }
}
