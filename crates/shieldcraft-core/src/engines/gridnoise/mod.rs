//! Grid Noise Studio — the flagship engine.
//!
//! An independent Rust implementation of the temporal grid-noise protection technique
//! demonstrated by Grid Noise Animator (https://github.com/stelmosfire13-alt/Grid-Noise-Animator,
//! by stelmosfire13-alt). That project publishes no licence, so none of its code is copied here;
//! the creator is credited wherever this engine is listed.

pub mod export;
pub mod params;
pub mod render;

pub use export::{export_animation, find_ffmpeg, ExportSummary, Panel};
pub use params::*;
pub use render::{analyse, output_size, render_image, static_signature, Analysis, Prepared, RenderMode};

use image::RgbaImage;
use serde::{Deserialize, Serialize};

/// Still-image outputs. An animation is the engine's primary output; these exist because many
/// platforms only accept still images.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum StillMode {
    /// Clean source + static layers (grating overlay, tiled text, visible mark) at full
    /// source resolution. No temporal noise, because a still cannot be averaged by the eye.
    StaticLayers,
    /// A single scrambled frame at output resolution.
    SingleFrame(u32),
}

pub fn render_still(src: &RgbaImage, p: &GridNoiseParams, mode: StillMode) -> RgbaImage {
    match mode {
        StillMode::StaticLayers => {
            let pr = Prepared::new(src, p, 1.0);
            render_image(&pr, p, 0, RenderMode::StaticOnly)
        }
        StillMode::SingleFrame(f) => {
            let (w, h) = output_size(src.width(), src.height(), p.animation.max_side);
            let img = if (w, h) == src.dimensions() {
                src.clone()
            } else {
                image::imageops::resize(src, w, h, image::imageops::FilterType::Lanczos3)
            };
            let pr = Prepared::new(&img, p, 1.0);
            render_image(&pr, p, f, RenderMode::Frame)
        }
    }
}
