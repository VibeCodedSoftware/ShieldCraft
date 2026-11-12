//! Selection masks: which part of an image a protection engine may change.
//!
//! A mask is an 8-bit grayscale image at source resolution: 255 = fully selected, 0 = not
//! selected. No selection at all means "the whole image" and is represented by `None` upstream.

use image::{GrayImage, Luma, RgbaImage};
use rayon::prelude::*;

pub type Mask = GrayImage;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Replace,
    Add,
    Subtract,
}

fn combine(mask: &mut Mask, shape: &GrayImage, op: Op) {
    match op {
        Op::Replace => mask.as_mut().copy_from_slice(shape.as_raw()),
        Op::Add => mask.as_mut().par_iter_mut().zip(shape.as_raw().par_iter()).for_each(|(m, s)| *m = (*m).max(*s)),
        Op::Subtract => mask.as_mut().par_iter_mut().zip(shape.as_raw().par_iter()).for_each(|(m, s)| *m = (*m).min(255 - *s)),
    }
}

/// Apply a rectangle given in image pixel coordinates (any corner order).
pub fn rect(mask: &mut Mask, a: (f32, f32), b: (f32, f32), op: Op) {
    let (w, h) = mask.dimensions();
    let (x0, x1) = (a.0.min(b.0).max(0.0) as u32, (a.0.max(b.0).ceil() as u32).min(w));
    let (y0, y1) = (a.1.min(b.1).max(0.0) as u32, (a.1.max(b.1).ceil() as u32).min(h));
    let mut shape = GrayImage::new(w, h);
    for y in y0..y1 {
        for x in x0..x1 {
            shape.put_pixel(x, y, Luma([255]));
        }
    }
    combine(mask, &shape, op);
}

/// Apply an ellipse inscribed in the rectangle a–b, with anti-aliased edges.
pub fn ellipse(mask: &mut Mask, a: (f32, f32), b: (f32, f32), op: Op) {
    let (w, h) = mask.dimensions();
    let (cx, cy) = ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
    let (rx, ry) = (((a.0 - b.0).abs() / 2.0).max(0.5), ((a.1 - b.1).abs() / 2.0).max(0.5));
    let mut shape = GrayImage::new(w, h);
    shape.as_mut().par_chunks_mut(w as usize).enumerate().for_each(|(y, row)| {
        let dy = (y as f32 + 0.5 - cy) / ry;
        if dy.abs() > 1.0 + 2.0 / ry {
            return;
        }
        for (x, v) in row.iter_mut().enumerate() {
            let dx = (x as f32 + 0.5 - cx) / rx;
            let d = (dx * dx + dy * dy).sqrt();
            // ~1 px soft edge.
            let edge = (1.0 - d) * rx.min(ry);
            *v = (edge.clamp(-0.5, 0.5) + 0.5).mul_add(255.0, 0.0) as u8;
        }
    });
    combine(mask, &shape, op);
}

/// Apply a closed polygon given in normalised (0..1) coordinates.
pub fn polygon(mask: &mut Mask, pts: &[[f32; 2]], op: Op) {
    let (w, h) = mask.dimensions();
    let inside = crate::imgproc::polygon_mask(&[pts.to_vec()], w as usize, h as usize);
    let shape = GrayImage::from_raw(w, h, inside.into_iter().map(|v| if v { 255 } else { 0 }).collect()).unwrap();
    combine(mask, &shape, op);
}

/// Paint (or erase) a round brush along a segment, in pixel coordinates.
pub fn brush(mask: &mut Mask, from: (f32, f32), to: (f32, f32), radius: f32, erase: bool) {
    let (w, h) = mask.dimensions();
    let r = radius.max(0.5);
    let len = ((to.0 - from.0).powi(2) + (to.1 - from.1).powi(2)).sqrt();
    let steps = ((len / (r * 0.35)).ceil() as usize).max(1);
    for s in 0..=steps {
        let t = s as f32 / steps as f32;
        let (cx, cy) = (from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t);
        let (x0, x1) = ((cx - r - 1.0).max(0.0) as u32, ((cx + r + 1.0) as u32).min(w.saturating_sub(1)));
        let (y0, y1) = ((cy - r - 1.0).max(0.0) as u32, ((cy + r + 1.0) as u32).min(h.saturating_sub(1)));
        for y in y0..=y1 {
            for x in x0..=x1 {
                let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
                let a = ((r - d) + 0.5).clamp(0.0, 1.0);
                if a <= 0.0 {
                    continue;
                }
                let p = mask.get_pixel_mut(x, y);
                let v = (a * 255.0) as u8;
                p[0] = if erase { p[0].min(255 - v) } else { p[0].max(v) };
            }
        }
    }
}

pub fn invert(mask: &mut Mask) {
    mask.as_mut().par_iter_mut().for_each(|v| *v = 255 - *v);
}

pub fn is_empty(mask: &Mask) -> bool {
    mask.as_raw().par_iter().all(|v| *v == 0)
}

pub fn selected_fraction(mask: &Mask) -> f32 {
    let sum: u64 = mask.as_raw().par_iter().map(|v| *v as u64).sum();
    sum as f32 / (255.0 * mask.as_raw().len().max(1) as f32)
}

/// Soften the edge by `radius` pixels (Gaussian).
pub fn feather(mask: &Mask, radius: f32) -> Mask {
    if radius < 0.5 {
        return mask.clone();
    }
    let (w, h) = mask.dimensions();
    let mut buf: Vec<f32> = mask.as_raw().iter().map(|v| *v as f32).collect();
    crate::imgproc::gaussian_blur(&mut buf, w as usize, h as usize, 1, radius);
    GrayImage::from_raw(w, h, buf.into_iter().map(|v| v.round().clamp(0.0, 255.0) as u8).collect()).unwrap()
}

/// Resample a mask to another size (for previews, video output sizes, etc.).
pub fn resized(mask: &Mask, w: u32, h: u32) -> Mask {
    let (sw, sh) = mask.dimensions();
    if (sw, sh) == (w, h) {
        return mask.clone();
    }
    if w == 0 || h == 0 || w > sw || h > sh {
        return image::imageops::resize(mask, w, h, image::imageops::FilterType::Triangle);
    }
    // Downscale by area averaging. Much faster than a Triangle filter on large masks, which
    // matters because the overlay and preview mask are rebuilt on every brush stroke.
    let src = mask.as_raw();
    let mut out = vec![0u8; (w * h) as usize];
    out.par_chunks_mut(w as usize).enumerate().for_each(|(y, row)| {
        let y0 = y as u64 * sh as u64 / h as u64;
        let y1 = ((y as u64 + 1) * sh as u64 / h as u64).max(y0 + 1);
        for (x, v) in row.iter_mut().enumerate() {
            let x0 = x as u64 * sw as u64 / w as u64;
            let x1 = ((x as u64 + 1) * sw as u64 / w as u64).max(x0 + 1);
            let mut sum = 0u64;
            for sy in y0..y1 {
                let line = &src[(sy * sw as u64) as usize..];
                sum += line[x0 as usize..x1 as usize].iter().map(|v| *v as u64).sum::<u64>();
            }
            let n = (y1 - y0) * (x1 - x0);
            *v = ((sum + n / 2) / n) as u8;
        }
    });
    GrayImage::from_raw(w, h, out).unwrap()
}

/// Keep `processed` inside the selection and `original` outside it (soft edges blend).
/// Alpha always comes from `processed`'s source, i.e. it is left as processed.
pub fn composite(original: &RgbaImage, processed: &mut RgbaImage, mask: &Mask) {
    let (w, h) = processed.dimensions();
    let m = resized(mask, w, h);
    let orig = if original.dimensions() == (w, h) { None } else { Some(image::imageops::resize(original, w, h, image::imageops::FilterType::Triangle)) };
    let o = orig.as_ref().unwrap_or(original);
    processed.as_mut().par_chunks_mut(4).zip(o.as_raw().par_chunks(4)).zip(m.as_raw().par_iter()).for_each(|((p, s), a)| {
        let a = *a as u32;
        if a == 255 {
            return;
        }
        for c in 0..3 {
            p[c] = ((p[c] as u32 * a + s[c] as u32 * (255 - a) + 127) / 255) as u8;
        }
    });
}

/// Composite on a raw RGBA buffer (used per animation frame).
pub fn composite_raw(original: &[u8], processed: &mut [u8], mask: &[u8]) {
    processed.par_chunks_mut(4).zip(original.par_chunks(4)).zip(mask.par_iter()).for_each(|((p, s), a)| {
        let a = *a as u32;
        if a == 255 {
            return;
        }
        for c in 0..3 {
            p[c] = ((p[c] as u32 * a + s[c] as u32 * (255 - a) + 127) / 255) as u8;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shapes_ops_and_invert() {
        let mut m = Mask::new(40, 20);
        rect(&mut m, (0.0, 0.0), (20.0, 20.0), Op::Replace);
        assert_eq!(m.get_pixel(5, 5)[0], 255);
        assert_eq!(m.get_pixel(30, 5)[0], 0);
        rect(&mut m, (10.0, 0.0), (20.0, 20.0), Op::Subtract);
        assert_eq!(m.get_pixel(15, 5)[0], 0);
        ellipse(&mut m, (20.0, 0.0), (40.0, 20.0), Op::Add);
        assert_eq!(m.get_pixel(30, 10)[0], 255);
        assert_eq!(m.get_pixel(5, 5)[0], 255, "add keeps the earlier area");
        invert(&mut m);
        assert_eq!(m.get_pixel(5, 5)[0], 0);
        assert_eq!(m.get_pixel(15, 5)[0], 255);
    }

    #[test]
    fn brush_and_composite() {
        let mut m = Mask::new(30, 30);
        brush(&mut m, (5.0, 15.0), (25.0, 15.0), 3.0, false);
        assert_eq!(m.get_pixel(15, 15)[0], 255);
        assert_eq!(m.get_pixel(15, 2)[0], 0);
        let orig = RgbaImage::from_pixel(30, 30, image::Rgba([0, 0, 0, 255]));
        let mut done = RgbaImage::from_pixel(30, 30, image::Rgba([200, 200, 200, 255]));
        composite(&orig, &mut done, &m);
        assert_eq!(done.get_pixel(15, 15)[0], 200, "selected area keeps the protection");
        assert_eq!(done.get_pixel(15, 2)[0], 0, "unselected area is restored to the original");
    }

    #[test]
    #[ignore]
    fn bench_large_mask() {
        let t = std::time::Instant::now;
        let mut m = Mask::new(6000, 4000);
        let s = t();
        ellipse(&mut m, (1000.0, 800.0), (4000.0, 3000.0), Op::Replace);
        eprintln!("ellipse {:?}", s.elapsed());
        let s = t();
        brush(&mut m, (100.0, 100.0), (140.0, 120.0), 30.0, false);
        eprintln!("brush segment {:?}", s.elapsed());
        let s = t();
        let c = m.clone();
        eprintln!("clone {:?}", s.elapsed());
        let s = t();
        let small = resized(&c, 1024, 683);
        eprintln!("resize to 1024 {:?}", s.elapsed());
        let s = t();
        let pv = resized(&c, 960, 640);
        let _ = feather(&pv, 2.0 * 960.0 / 6000.0 + 0.6);
        eprintln!("preview mask {:?}", s.elapsed());
        let s = t();
        let _ = selected_fraction(&c);
        let _ = is_empty(&c);
        eprintln!("fraction+empty {:?}", s.elapsed());
        let s = t();
        let _ = feather(&c, 2.0);
        eprintln!("full-res feather {:?}", s.elapsed());
        let _ = small;
    }
}