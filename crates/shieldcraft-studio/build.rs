// Builds the application icon from assets/icon-source.png:
//   * finds the blue rounded tile and crops away the dark surround,
//   * applies an anti-aliased rounded-corner alpha mask (transparent corners),
//   * renders 16–256 px sizes with Lanczos resampling, sharpening the small ones,
//   * writes a multi-size .ico plus a 256 px PNG used at runtime (window icon, startup logo),
//   * embeds the .ico and version information into the Windows executable.

use image::{imageops, Rgba, RgbaImage};
use std::path::PathBuf;

fn tile_bounds(img: &RgbaImage) -> (u32, u32, u32, u32) {
    // The tile is the saturated blue region; scan rows/columns for enough blue pixels.
    let blue = |p: &Rgba<u8>| p[2] > 150 && p[2] as i32 > p[0] as i32 + 60 && p[2] as i32 > p[1] as i32 + 20;
    let (w, h) = img.dimensions();
    let col_has = |x: u32| (0..h).filter(|&y| blue(img.get_pixel(x, y))).count() > (h / 20) as usize;
    let row_has = |y: u32| (0..w).filter(|&x| blue(img.get_pixel(x, y))).count() > (w / 20) as usize;
    let x0 = (0..w).find(|&x| col_has(x)).unwrap_or(0);
    let x1 = (0..w).rev().find(|&x| col_has(x)).unwrap_or(w - 1);
    let y0 = (0..h).find(|&y| row_has(y)).unwrap_or(0);
    let y1 = (0..h).rev().find(|&y| row_has(y)).unwrap_or(h - 1);
    (x0, y0, x1 - x0 + 1, y1 - y0 + 1)
}

fn rounded_mask(img: &mut RgbaImage, radius_frac: f32) {
    let (w, h) = img.dimensions();
    let r = radius_frac * w.min(h) as f32;
    for y in 0..h {
        for x in 0..w {
            // Signed distance to the rounded rectangle, in pixels (pixel centres).
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let dx = (px - w as f32 / 2.0).abs() - (w as f32 / 2.0 - r);
            let dy = (py - h as f32 / 2.0).abs() - (h as f32 / 2.0 - r);
            let outside = dx.max(0.0).hypot(dy.max(0.0)) + dx.max(dy).min(0.0) - r;
            let a = (0.5 - outside).clamp(0.0, 1.0);
            let p = img.get_pixel_mut(x, y);
            p[3] = (p[3] as f32 * a).round() as u8;
        }
    }
}

fn render(master: &RgbaImage, size: u32) -> RgbaImage {
    let mut img = imageops::resize(master, size, size, imageops::FilterType::Lanczos3);
    if size <= 48 {
        // Small icons lose the engraving detail; a light unsharp mask keeps edges crisp.
        img = imageops::unsharpen(&img, 0.6, 2);
    }
    rounded_mask(&mut img, 0.2);
    img
}

/// Classic ICO layout: 32-bit BMP (DIB) frames below 256 px for maximum compatibility with
/// GDI/shell code paths, and a PNG frame at 256 px.
fn build_ico(master: &RgbaImage, sizes: &[u32]) -> Vec<u8> {
    let mut blobs: Vec<Vec<u8>> = Vec::new();
    for &s in sizes {
        let img = render(master, s);
        if s >= 256 {
            let mut png = Vec::new();
            img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).expect("png frame");
            blobs.push(png);
            continue;
        }
        let mut b = Vec::new();
        let mask_row = s.div_ceil(32) * 4;
        b.extend_from_slice(&40u32.to_le_bytes()); // BITMAPINFOHEADER size
        b.extend_from_slice(&(s as i32).to_le_bytes());
        b.extend_from_slice(&((s * 2) as i32).to_le_bytes()); // XOR + AND mask height
        b.extend_from_slice(&1u16.to_le_bytes()); // planes
        b.extend_from_slice(&32u16.to_le_bytes()); // bpp
        b.extend_from_slice(&0u32.to_le_bytes()); // BI_RGB
        b.extend_from_slice(&(s * s * 4 + mask_row * s).to_le_bytes());
        b.extend_from_slice(&[0u8; 16]); // resolution + palette counts
        for y in (0..s).rev() {
            for x in 0..s {
                let p = img.get_pixel(x, y);
                b.extend_from_slice(&[p[2], p[1], p[0], p[3]]);
            }
        }
        for y in (0..s).rev() {
            let mut row = vec![0u8; mask_row as usize];
            for x in 0..s {
                if img.get_pixel(x, y)[3] == 0 {
                    row[(x / 8) as usize] |= 0x80 >> (x % 8);
                }
            }
            b.extend_from_slice(&row);
        }
        blobs.push(b);
    }
    let mut out = Vec::new();
    out.extend_from_slice(&[0, 0, 1, 0]);
    out.extend_from_slice(&(sizes.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * sizes.len() as u32;
    for (&s, blob) in sizes.iter().zip(&blobs) {
        let d = if s >= 256 { 0 } else { s as u8 };
        out.extend_from_slice(&[d, d, 0, 0]);
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&32u16.to_le_bytes());
        out.extend_from_slice(&(blob.len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset += blob.len() as u32;
    }
    for blob in blobs {
        out.extend_from_slice(&blob);
    }
    out
}

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let source = manifest.join("../../assets/icon-source.png");
    println!("cargo:rerun-if-changed={}", source.display());
    println!("cargo:rerun-if-changed=build.rs");
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());

    // Place the official ONNX Runtime DLL next to the built executables (target/<profile>/).
    let dll = manifest.join("../../third_party/onnxruntime/onnxruntime.dll");
    println!("cargo:rerun-if-changed={}", dll.display());
    if let Some(profile_dir) = out_dir.ancestors().nth(3) {
        let _ = std::fs::copy(&dll, profile_dir.join("onnxruntime.dll"));
    }

    let src = image::open(&source).expect("assets/icon-source.png is required").to_rgba8();
    let (x, y, w, h) = tile_bounds(&src);
    let side = w.max(h);
    let cx = x + w / 2;
    let cy = y + h / 2;
    let left = cx.saturating_sub(side / 2).min(src.width() - side);
    let top = cy.saturating_sub(side / 2).min(src.height() - side);
    let master = imageops::crop_imm(&src, left, top, side, side).to_image();

    let png256 = render(&master, 256);
    png256.save(out_dir.join("icon-256.png")).expect("write icon png");

    let ico = out_dir.join("shieldcraft.ico");
    std::fs::write(&ico, build_ico(&master, &[16, 20, 24, 32, 40, 48, 64, 128, 256])).expect("write ico");
    // Keep a copy in assets/ for packaging and shortcuts.
    let _ = std::fs::copy(&ico, manifest.join("../../assets/shieldcraft.ico"));

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon(ico.to_str().unwrap());
        res.set("ProductName", "ShieldCraft");
        res.set("FileDescription", "ShieldCraft - artwork protection workstation");
        res.set("CompanyName", "ShieldCraft");
        res.set("LegalCopyright", "ShieldCraft contributors. Third-party notices: licenses folder.");
        if let Err(e) = res.compile() {
            println!("cargo:warning=Icon/version resource not embedded: {e}");
        }
    }
}
