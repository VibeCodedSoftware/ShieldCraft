//! Signature Ink — invisible frequency-domain text signature (provenance, not protection).
//!
//! Uses the `blind_watermark` crate (MIT OR Apache-2.0) in memory with the same configuration
//! Hope:RE (MIT, HopeArtOrg) uses (strength 36, second strength 20 when a seed is given), so signatures embedded
//! by either application can be read by the other. Unlike the crate's file helpers, the alpha
//! channel is kept.

use blind_watermark::config::{WatermarkConfig, WatermarkConfigBuilder, WatermarkMode};
use blind_watermark::YCrBrAMat;
use image::{Rgba32FImage, RgbaImage};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SignatureParams {
    pub text: String,
    /// Optional numeric key; the same key is needed to read the signature back.
    pub seed: Option<u64>,
}

impl Default for SignatureParams {
    fn default() -> Self {
        Self { text: "© Your Name 2026".into(), seed: None }
    }
}

fn config(seed: Option<u64>) -> Result<WatermarkConfig, String> {
    match seed {
        None => Ok(WatermarkConfig::default()),
        Some(s) => WatermarkConfigBuilder::default()
            .mode(WatermarkMode::Strategy(s))
            .strength_2(20)
            .build()
            .map_err(|e| e.to_string()),
    }
}

/// Number of 4×4 low-frequency blocks available, which bounds the signature length in bits.
pub fn capacity_bits(w: u32, h: u32) -> usize {
    // One DWT level halves each side; blocks are 4×4 over the padded LL band.
    let lw = w.div_ceil(2) as usize;
    let lh = h.div_ceil(2) as usize;
    (lw / 4) * (lh / 4)
}

pub fn embed(img: &RgbaImage, p: &SignatureParams) -> Result<RgbaImage, String> {
    let bytes = p.text.as_bytes();
    if bytes.is_empty() {
        return Err("Signature text is empty".into());
    }
    let bits = bytes.len() * 8;
    let cap = capacity_bits(img.width(), img.height());
    if bits > cap {
        return Err(format!(
            "Signature is too long for this image: {bits} bits needed, about {cap} available. Use shorter text."
        ));
    }
    let cfg = config(p.seed)?;
    let f32img: Rgba32FImage = image::DynamicImage::ImageRgba8(img.clone()).into_rgba32f();
    let mat: YCrBrAMat = f32img.into();
    let wm = bitvec::slice::BitSlice::<u8, bitvec::order::Lsb0>::from_slice(bytes);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        mat.add_padding().dwt().cut().embed_watermark_bits(wm, &cfg).assemble().idwt().remove_padding()
    }))
    .map_err(|_| "The watermark library rejected this image (too small for the signature?)".to_string())?;
    let out: Rgba32FImage = result.into();
    let mut rgba = image::DynamicImage::ImageRgba32F(out).to_rgba8();
    // Restore the original alpha exactly.
    for (o, s) in rgba.pixels_mut().zip(img.pixels()) {
        o[3] = s[3];
    }
    Ok(rgba)
}

pub fn extract(img: &RgbaImage, byte_len: usize, seed: Option<u64>) -> Result<String, String> {
    if byte_len == 0 || byte_len > 4096 {
        return Err("Signature length must be between 1 and 4096 bytes".into());
    }
    let cfg = config(seed)?;
    let f32img: Rgba32FImage = image::DynamicImage::ImageRgba8(img.clone()).into_rgba32f();
    let mat: YCrBrAMat = f32img.into();
    let bits = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        mat.add_padding().dwt().cut().extract_watermark_bits(byte_len * 8, &cfg)
    }))
    .map_err(|_| "Extraction failed (image too small for that length?)".to_string())?;
    let bytes = bits.into_vec();
    let text = String::from_utf8_lossy(&bytes).to_string();
    let printable = text.chars().filter(|c| !c.is_control() && *c != '\u{FFFD}').count();
    if printable * 10 < text.chars().count().max(1) * 7 {
        return Err("No readable signature found with that length and key".into());
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_with_and_without_seed() {
        let img = RgbaImage::from_fn(256, 192, |x, y| image::Rgba([(x ^ y) as u8, (x * 3) as u8, (y * 2) as u8, 200]));
        for seed in [None, Some(42)] {
            let p = SignatureParams { text: "ShieldCraft".into(), seed };
            let signed = embed(&img, &p).unwrap();
            assert_eq!(signed.get_pixel(5, 5)[3], 200);
            assert_eq!(extract(&signed, p.text.len(), seed).unwrap(), "ShieldCraft");
        }
    }
}
