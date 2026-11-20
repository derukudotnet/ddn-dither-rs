// Auto-registers every built-in frame found in `assets/` (and one folder level
// below, e.g. `assets/wild/frame_x.bin`).
//
// Scans assets/frame_*.bin and assets/*/frame_*.bin, and generates
// `builtin_frames.rs` in OUT_DIR with one entry per file: `(id, group, raw bytes)`.
// Details:
//   * id     = the part after `frame_` (used by `--frame <id>`; folder files that
//              collide with another id get a `<folder>_` prefix)
//   * group  = folder dropdown section: top-level frames are "Built-in", frames in
//              assets/<sub>/ form the group `<sub>`
//   * bytes  = include_bytes! of the asset, format: u16le w, u16le h, then
//              w*h bytes of levels (0..3) where >= 4 means transparent.
// Use tools/genframe.py to convert a frame PNG into that format.
//
// cargo runs this as the package build script; tools/verify.sh runs it manually
// with the same environment variables.

use std::env;
use std::fs;
use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=assets");

    let manifest = env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| String::from("."));
    let out_dir = env::var("OUT_DIR").unwrap_or_else(|_| String::from("."));
    let assets = Path::new(&manifest).join("assets");

    let is_frame_bin = |p: &Path| {
        p.extension().and_then(|e| e.to_str()) == Some("bin")
            && p.file_stem().and_then(|s| s.to_str()).map(|s| s.starts_with("frame_")).unwrap_or(false)
    };

    let mut entries: Vec<(String, std::path::PathBuf)> = Vec::new();
    for e in fs::read_dir(&assets).into_iter().flatten().filter_map(|e| e.ok()) {
        let p = e.path();
        if p.is_dir() {
            for f in fs::read_dir(&p).into_iter().flatten().filter_map(|e| e.ok().map(|e| e.path())) {
                if is_frame_bin(&f) {
                    entries.push((e.file_name().to_string_lossy().to_string(), f));
                }
            }
        } else if is_frame_bin(&p) {
            entries.push((String::new(), p));
        }
    }
    entries.sort();

    let mut code = String::from("pub static BUILTIN_FRAME_FILES: &[(&str, &str, &[u8])] = &[\n");
    let mut seen_ids: Vec<String> = Vec::new();
    for (group, p) in &entries {
        let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("frame");
        let mut id = stem.strip_prefix("frame_").unwrap_or(stem).to_string();
        if seen_ids.contains(&id) {
            id = format!("{group}_{id}");
        }
        seen_ids.push(id.clone());
        let group = if group.is_empty() { "Built-in" } else { group.as_str() };
        code.push_str(&format!(
            "    (\"{id}\", \"{group}\", include_bytes!({:?})),\n",
            p.to_string_lossy()
        ));
    }
    code.push_str("];\n");
    fs::write(Path::new(&out_dir).join("builtin_frames.rs"), code)
        .expect("cannot write generated built-in frame list");
}
