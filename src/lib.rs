//! ddn-dither core.
//!
//! Everything in this library is plain `std` Rust (no external crates), so it
//! can be compiled and tested anywhere. File decoding/encoding (`image`, `png`)
//! and the window (`eframe`) live in the binary (`main.rs`, `io.rs`, `gui.rs`).
//!
//! Pipeline in one sentence: grey -> crop + Lanczos resize -> autocontrast ->
//! 4x4 Bayer dither to 4 levels -> (optional frame, also 4 levels) -> palette ->
//! integer nearest-neighbour upscale.

pub mod autocontrast;
pub mod cli;
pub mod cropbox;
pub mod dither;
pub mod frame;
pub mod gray;
pub mod mask;
pub mod pack;
pub mod palette;
pub mod pipeline;
pub mod resample;
