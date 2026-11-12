//! Application icon: built from assets/icon-source.png by build.rs and embedded here.

use image::RgbaImage;

const ICON_PNG: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon-256.png"));

/// The icon at `size`×`size` (rounded tile, transparent corners).
pub fn icon(size: u32) -> RgbaImage {
    let img = image::load_from_memory(ICON_PNG).expect("embedded icon is valid").to_rgba8();
    if size == img.width() {
        img
    } else {
        image::imageops::resize(&img, size, size, image::imageops::FilterType::Lanczos3)
    }
}
