//! Image import and export.
//!
//! Sources are decoded once into an immutable [`SourceImage`]. Every transformation that the
//! import applies (orientation, bit-depth reduction) is recorded so the UI can disclose it.
//! Exports are written to a temporary file, re-decoded to validate them, then renamed into
//! place; an existing file is never replaced unless the caller explicitly allows it, and the
//! original source file is never a valid export target.

use image::{DynamicImage, ImageDecoder, ImageEncoder, ImageReader, RgbaImage};
use serde::{Deserialize, Serialize};
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub const SUPPORTED_IMPORT_EXT: &[&str] = &["png", "jpg", "jpeg", "webp", "tif", "tiff", "bmp", "gif"];

#[derive(Clone, Debug)]
pub struct SourceImage {
    pub name: String,
    pub path: Option<PathBuf>,
    pub pixels: Arc<RgbaImage>,
    pub info: SourceInfo,
    pub icc_profile: Option<Vec<u8>>,
    pub exif: Option<Vec<u8>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SourceInfo {
    pub format: String,
    pub width: u32,
    pub height: u32,
    /// Colour layout of the decoded file, e.g. "RGB8", "RGBA16".
    pub color_type: String,
    pub has_alpha_channel: bool,
    /// True if at least one pixel is not fully opaque.
    pub uses_transparency: bool,
    pub file_size: u64,
    pub transformations: Vec<String>,
}

impl SourceImage {
    pub fn width(&self) -> u32 {
        self.pixels.width()
    }
    pub fn height(&self) -> u32 {
        self.pixels.height()
    }

    pub fn from_rgba(name: &str, img: RgbaImage) -> Self {
        let uses_transparency = img.pixels().any(|p| p[3] != 255);
        let (w, h) = img.dimensions();
        Self {
            name: name.to_string(),
            path: None,
            info: SourceInfo {
                format: "In-memory".into(),
                width: w,
                height: h,
                color_type: "RGBA8".into(),
                has_alpha_channel: true,
                uses_transparency,
                file_size: 0,
                transformations: vec![],
            },
            pixels: Arc::new(img),
            icc_profile: None,
            exif: None,
        }
    }
}

#[derive(Debug)]
pub enum ImportError {
    Unsupported(String),
    Io(String),
    Decode(String),
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImportError::Unsupported(s) | ImportError::Io(s) | ImportError::Decode(s) => f.write_str(s),
        }
    }
}

pub fn is_supported_path(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| SUPPORTED_IMPORT_EXT.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

/// Explain formats that are recognised but deliberately not decoded.
fn unsupported_reason(ext: &str) -> Option<String> {
    match ext {
        "psd" | "psb" => Some(
            "Layered Photoshop files (.psd/.psb) are not processed directly, because protection \
             engines work on flattened pixels and ShieldCraft will not silently flatten your layers. \
             Export a flattened PNG copy from your editor (keep transparency if you need it), then \
             import that copy. Your layered original stays untouched."
                .into(),
        ),
        "kra" | "clip" | "xcf" | "ora" | "procreate" => Some(format!(
            "'.{ext}' is a layered painting-app format. Export a flattened PNG copy and import it."
        )),
        "heic" | "heif" | "avif" | "jxl" => {
            Some(format!("'.{ext}' decoding is not included in this build. Convert to PNG first."))
        }
        _ => None,
    }
}

pub fn load_image(path: &Path) -> Result<SourceImage, ImportError> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    if let Some(reason) = unsupported_reason(&ext) {
        return Err(ImportError::Unsupported(reason));
    }
    let bytes = std::fs::read(path).map_err(|e| ImportError::Io(format!("Could not read {}: {e}", path.display())))?;
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("image").to_string();
    let mut src = decode_bytes(&name, &bytes)?;
    src.path = Some(path.to_path_buf());
    Ok(src)
}

pub fn decode_bytes(name: &str, bytes: &[u8]) -> Result<SourceImage, ImportError> {
    let reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| ImportError::Decode(e.to_string()))?;
    let format = reader
        .format()
        .ok_or_else(|| ImportError::Unsupported(format!("{name}: unrecognised image format")))?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(32_768);
    limits.max_image_height = Some(32_768);
    limits.max_alloc = Some(3 * 1024 * 1024 * 1024);
    let mut reader = reader;
    reader.limits(limits);
    let mut decoder = reader.into_decoder().map_err(|e| ImportError::Decode(format!("{name}: {e}")))?;

    let icc_profile = decoder.icc_profile().ok().flatten();
    let exif = decoder.exif_metadata().ok().flatten();
    let orientation = decoder.orientation().unwrap_or(image::metadata::Orientation::NoTransforms);
    let color = decoder.color_type();

    let mut img = DynamicImage::from_decoder(decoder).map_err(|e| ImportError::Decode(format!("{name}: {e}")))?;
    let mut transformations = Vec::new();
    if orientation != image::metadata::Orientation::NoTransforms {
        img.apply_orientation(orientation);
        transformations.push(format!("Applied EXIF orientation ({orientation:?}) so pixels are upright"));
    }
    if color.bytes_per_pixel() / color.channel_count().max(1) > 1 {
        transformations.push("High bit-depth source reduced to 8 bits per channel for processing".into());
    }
    if format == image::ImageFormat::Gif {
        transformations.push("Animated GIF sources: only the first frame is imported".into());
    }

    let rgba = img.to_rgba8();
    let uses_transparency = rgba.pixels().any(|p| p[3] != 255);
    let (w, h) = rgba.dimensions();
    Ok(SourceImage {
        name: name.to_string(),
        path: None,
        info: SourceInfo {
            format: format!("{format:?}").to_uppercase(),
            width: w,
            height: h,
            color_type: format!("{color:?}"),
            has_alpha_channel: color.has_alpha(),
            uses_transparency,
            file_size: bytes.len() as u64,
            transformations,
        },
        pixels: Arc::new(rgba),
        icc_profile,
        exif: if orientation == image::metadata::Orientation::NoTransforms { exif } else { None },
    })
}

// ───────────────────────────── export ─────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExportFormat {
    Png,
    Jpeg,
    WebpLossless,
    Tiff,
    Bmp,
}

impl ExportFormat {
    pub const ALL: [ExportFormat; 5] =
        [ExportFormat::Png, ExportFormat::Jpeg, ExportFormat::WebpLossless, ExportFormat::Tiff, ExportFormat::Bmp];
    pub fn ext(self) -> &'static str {
        match self {
            ExportFormat::Png => "png",
            ExportFormat::Jpeg => "jpg",
            ExportFormat::WebpLossless => "webp",
            ExportFormat::Tiff => "tif",
            ExportFormat::Bmp => "bmp",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            ExportFormat::Png => "PNG (lossless, alpha)",
            ExportFormat::Jpeg => "JPEG (lossy, no alpha)",
            ExportFormat::WebpLossless => "WebP (lossless, alpha)",
            ExportFormat::Tiff => "TIFF (lossless, alpha)",
            ExportFormat::Bmp => "BMP (no alpha)",
        }
    }
    pub fn supports_alpha(self) -> bool {
        !matches!(self, ExportFormat::Jpeg | ExportFormat::Bmp)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MetadataPolicy {
    /// Strip everything (most private).
    StripAll,
    /// Keep the colour profile only (default: correct colour, no camera/GPS data).
    IccOnly,
    /// Keep colour profile and EXIF (may contain camera, date and location data).
    IccAndExif,
}

impl MetadataPolicy {
    pub fn label(self) -> &'static str {
        match self {
            MetadataPolicy::StripAll => "Strip all metadata",
            MetadataPolicy::IccOnly => "Keep colour profile only",
            MetadataPolicy::IccAndExif => "Keep colour profile + EXIF",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExportOptions {
    pub format: ExportFormat,
    pub jpeg_quality: u8,
    pub metadata: MetadataPolicy,
    /// Background used when flattening transparency for formats without alpha.
    pub flatten_background: [u8; 3],
    pub suffix: String,
    pub allow_overwrite: bool,
    pub write_report: bool,
    pub side_by_side: bool,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            format: ExportFormat::Png,
            jpeg_quality: 95,
            metadata: MetadataPolicy::IccOnly,
            flatten_background: [255, 255, 255],
            suffix: "_protected".into(),
            allow_overwrite: false,
            write_report: true,
            side_by_side: false,
        }
    }
}

pub fn encode(img: &RgbaImage, opts: &ExportOptions, icc: Option<&[u8]>, exif: Option<&[u8]>) -> Result<(Vec<u8>, Vec<String>), String> {
    let mut warnings = Vec::new();
    let (w, h) = img.dimensions();
    let has_transparency = img.pixels().any(|p| p[3] != 255);
    let icc = if opts.metadata == MetadataPolicy::StripAll { None } else { icc };
    let exif = if opts.metadata == MetadataPolicy::IccAndExif { exif } else { None };

    let mut out = Vec::new();
    match opts.format {
        ExportFormat::Png => {
            let mut enc = image::codecs::png::PngEncoder::new_with_quality(
                &mut out,
                image::codecs::png::CompressionType::Best,
                image::codecs::png::FilterType::Adaptive,
            );
            if let Some(p) = icc {
                let _ = enc.set_icc_profile(p.to_vec());
            }
            enc.write_image(img.as_raw(), w, h, image::ExtendedColorType::Rgba8).map_err(|e| e.to_string())?;
        }
        ExportFormat::WebpLossless => {
            let mut enc = image::codecs::webp::WebPEncoder::new_lossless(&mut out);
            if let Some(p) = icc {
                let _ = enc.set_icc_profile(p.to_vec());
            }
            enc.write_image(img.as_raw(), w, h, image::ExtendedColorType::Rgba8).map_err(|e| e.to_string())?;
        }
        ExportFormat::Tiff => {
            let enc = image::codecs::tiff::TiffEncoder::new(Cursor::new(&mut out));
            if icc.is_some() {
                warnings.push("TIFF export in this build does not embed the colour profile".into());
            }
            enc.write_image(img.as_raw(), w, h, image::ExtendedColorType::Rgba8).map_err(|e| e.to_string())?;
        }
        ExportFormat::Jpeg | ExportFormat::Bmp => {
            if has_transparency {
                warnings.push(format!(
                    "{} has no alpha channel: transparency was flattened onto RGB({}, {}, {})",
                    opts.format.ext().to_uppercase(),
                    opts.flatten_background[0],
                    opts.flatten_background[1],
                    opts.flatten_background[2]
                ));
            }
            let rgb = flatten(img, opts.flatten_background);
            if opts.format == ExportFormat::Jpeg {
                let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, opts.jpeg_quality.clamp(1, 100));
                if let Some(p) = icc {
                    let _ = enc.set_icc_profile(p.to_vec());
                }
                enc.write_image(rgb.as_raw(), w, h, image::ExtendedColorType::Rgb8).map_err(|e| e.to_string())?;
                warnings.push(format!(
                    "JPEG is lossy (quality {}); compression can weaken fine adversarial perturbations",
                    opts.jpeg_quality
                ));
            } else {
                let mut enc = image::codecs::bmp::BmpEncoder::new(&mut out);
                enc.encode(rgb.as_raw(), w, h, image::ExtendedColorType::Rgb8).map_err(|e| e.to_string())?;
            }
        }
    }

    if let Some(exif) = exif {
        match attach_exif(&out, opts.format, exif) {
            Some(b) => out = b,
            None => warnings.push("EXIF could not be embedded in this format and was dropped".into()),
        }
    }
    Ok((out, warnings))
}

fn attach_exif(encoded: &[u8], format: ExportFormat, exif: &[u8]) -> Option<Vec<u8>> {
    use img_parts::{Bytes, ImageEXIF};
    let data = Bytes::copy_from_slice(encoded);
    let exif = Bytes::copy_from_slice(exif);
    match format {
        ExportFormat::Jpeg => {
            let mut j = img_parts::jpeg::Jpeg::from_bytes(data).ok()?;
            j.set_exif(Some(exif));
            Some(j.encoder().bytes().to_vec())
        }
        ExportFormat::Png => {
            let mut p = img_parts::png::Png::from_bytes(data).ok()?;
            p.set_exif(Some(exif));
            Some(p.encoder().bytes().to_vec())
        }
        ExportFormat::WebpLossless => {
            let mut w = img_parts::webp::WebP::from_bytes(data).ok()?;
            w.set_exif(Some(exif));
            Some(w.encoder().bytes().to_vec())
        }
        _ => None,
    }
}

pub fn flatten(img: &RgbaImage, bg: [u8; 3]) -> image::RgbImage {
    image::RgbImage::from_fn(img.width(), img.height(), |x, y| {
        let p = img.get_pixel(x, y);
        let a = p[3] as u32;
        let mix = |c: u8, b: u8| ((c as u32 * a + b as u32 * (255 - a) + 127) / 255) as u8;
        image::Rgb([mix(p[0], bg[0]), mix(p[1], bg[1]), mix(p[2], bg[2])])
    })
}

/// Pick a non-colliding output path `dir/stem{suffix}.ext`, appending ` (2)`, ` (3)`, ...
pub fn plan_output_path(dir: &Path, stem: &str, suffix: &str, ext: &str, allow_overwrite: bool) -> PathBuf {
    let base = sanitize_file_stem(&format!("{stem}{suffix}"));
    let first = dir.join(format!("{base}.{ext}"));
    if allow_overwrite || !first.exists() {
        return first;
    }
    for n in 2..10_000 {
        let p = dir.join(format!("{base} ({n}).{ext}"));
        if !p.exists() {
            return p;
        }
    }
    dir.join(format!("{base} ({}).{ext}", crate::util::unix_now()))
}

pub fn sanitize_file_stem(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .map(|c| if c.is_control() || r#"<>:"/\|?*"#.contains(c) { '_' } else { c })
        .collect();
    let trimmed = cleaned.trim().trim_end_matches('.').to_string();
    if trimmed.is_empty() { "image".into() } else { trimmed }
}

/// Write bytes atomically: temp file in the same folder, optional validation, then rename.
/// Refuses to write over `protect` (the source file) under any circumstances.
pub fn write_atomic(target: &Path, bytes: &[u8], protect: Option<&Path>, validate_image: bool) -> Result<(), String> {
    if let Some(src) = protect {
        if same_file(src, target) {
            return Err("Refusing to overwrite the original source file. Choose another name or folder.".into());
        }
    }
    let dir = target.parent().ok_or("Output path has no parent folder")?;
    std::fs::create_dir_all(dir).map_err(|e| format!("Cannot create output folder {}: {e}", dir.display()))?;
    let tmp = dir.join(format!(
        ".shieldcraft-{}-{}.tmp",
        std::process::id(),
        crate::util::splitmix64(crate::util::unix_now() ^ bytes.len() as u64) % 1_000_000
    ));
    std::fs::write(&tmp, bytes).map_err(|e| format!("Cannot write {}: {e}", tmp.display()))?;
    if validate_image {
        if let Err(e) = image::load_from_memory(bytes) {
            let _ = std::fs::remove_file(&tmp);
            return Err(format!("Encoded output failed validation and was not saved: {e}"));
        }
    }
    if target.exists() {
        let _ = std::fs::remove_file(target);
    }
    std::fs::rename(&tmp, target).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("Cannot move output into place at {}: {e}", target.display())
    })
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}

/// Compose original and result side by side (for comparison exports).
pub fn side_by_side(a: &RgbaImage, b: &RgbaImage) -> RgbaImage {
    let h = a.height().max(b.height());
    let gap = 16;
    let mut out = RgbaImage::from_pixel(a.width() + b.width() + gap, h, image::Rgba([24, 24, 27, 255]));
    image::imageops::overlay(&mut out, a, 0, 0);
    image::imageops::overlay(&mut out, b, (a.width() + gap) as i64, 0);
    out
}

/// Resize to fit within `max_side` (no upscaling).
pub fn fit_within(img: &RgbaImage, max_side: u32) -> RgbaImage {
    let (w, h) = img.dimensions();
    if w.max(h) <= max_side {
        return img.clone();
    }
    let s = max_side as f32 / w.max(h) as f32;
    let nw = ((w as f32 * s).round() as u32).max(1);
    let nh = ((h as f32 * s).round() as u32).max(1);
    image::imageops::resize(img, nw, nh, image::imageops::FilterType::Triangle)
}

/// Peak signal-to-noise ratio between two same-sized RGBA images (RGB channels only).
pub fn psnr(a: &RgbaImage, b: &RgbaImage) -> Option<f64> {
    if a.dimensions() != b.dimensions() {
        return None;
    }
    let mut se = 0f64;
    let mut n = 0u64;
    for (p, q) in a.pixels().zip(b.pixels()) {
        for c in 0..3 {
            let d = p[c] as f64 - q[c] as f64;
            se += d * d;
        }
        n += 3;
    }
    let mse = se / n.max(1) as f64;
    Some(if mse == 0.0 { f64::INFINITY } else { 10.0 * (255.0 * 255.0 / mse).log10() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_roundtrip_keeps_alpha_and_icc() {
        let mut img = RgbaImage::from_pixel(8, 6, image::Rgba([10, 20, 30, 128]));
        img.put_pixel(0, 0, image::Rgba([1, 2, 3, 0]));
        let icc = vec![7u8; 64];
        let (bytes, warnings) = encode(&img, &ExportOptions::default(), Some(&icc), None).unwrap();
        assert!(warnings.is_empty());
        let back = decode_bytes("t.png", &bytes).unwrap();
        assert_eq!(*back.pixels, img);
        assert!(back.info.uses_transparency);
        assert_eq!(back.icc_profile.as_deref(), Some(&icc[..]));
    }

    #[test]
    fn jpeg_flattens_with_warning() {
        let img = RgbaImage::from_pixel(4, 4, image::Rgba([0, 0, 0, 0]));
        let opts = ExportOptions { format: ExportFormat::Jpeg, ..Default::default() };
        let (_, warnings) = encode(&img, &opts, None, None).unwrap();
        assert!(warnings.iter().any(|w| w.contains("flattened")));
    }

    #[test]
    fn collision_naming_and_source_protection() {
        let dir = std::env::temp_dir().join(format!("shieldcraft-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p1 = plan_output_path(&dir, "art", "_p", "png", false);
        std::fs::write(&p1, b"x").unwrap();
        let p2 = plan_output_path(&dir, "art", "_p", "png", false);
        assert!(p2.to_string_lossy().ends_with("art_p (2).png"));
        assert!(write_atomic(&p1, b"y", Some(&p1), false).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
