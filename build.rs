// Auto-registers every built-in frame found in `assets/frame_<id>.bin`.
//
// Scans assets/ for files named `frame_*.bin`, and generates `builtin_frames.rs`
// in OUT_DIR with one entry per file: `(id, display name, raw bytes)`. Details:
//   * id          = the part after `frame_` (used by `--frame <id>`)
//   * display     = `<Id> (built-in)` (shown in the window's frame dropdown)
//   * bytes       = include_bytes! of the asset, format: u16le w, u16le h, then
//                   w*h bytes of levels (0..3) where >= 4 means transparent.
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

    let mut files: Vec<_> = fs::read_dir(&assets)
        .map(|rd| {
            rd.filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| {
                    p.extension().and_then(|e| e.to_str()) == Some("bin")
                        && p.file_stem().and_then(|s| s.to_str()).map(|s| s.starts_with("frame_")).unwrap_or(false)
                })
                .collect()
        })
        .unwrap_or_default();
    files.sort();

    let mut code = String::from("pub static BUILTIN_FRAME_FILES: &[(&str, &str, &[u8])] = &[\n");
    for p in &files {
        let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("frame");
        let id = stem.strip_prefix("frame_").unwrap_or(stem);
        let mut name = id.to_string();
        if let Some(first) = name.get_mut(0..1) {
            first.make_ascii_uppercase();
        }
        code.push_str(&format!(
            "    (\"{id}\", \"{name} (built-in)\", include_bytes!({:?})),\n",
            p.to_string_lossy()
        ));
    }
    code.push_str("];\n");
    fs::write(Path::new(&out_dir).join("builtin_frames.rs"), code)
        .expect("cannot write generated built-in frame list");
}
