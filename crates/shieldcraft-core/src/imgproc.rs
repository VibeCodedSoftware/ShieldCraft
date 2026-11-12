//! Pixel-level helpers shared by the engines: colour conversion, blur, polygon masks.

use rayon::prelude::*;

/// RGB in 0..=255 → (hue degrees 0..360, saturation 0..100, lightness 0..100).
#[inline]
pub fn rgb_to_hsl(r: f32, g: f32, b: f32) -> [f32; 3] {
    let (r, g, b) = (r / 255.0, g / 255.0, b / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) * 0.5;
    let d = max - min;
    if d <= 1e-6 {
        return [0.0, 0.0, l * 100.0];
    }
    let s = if l > 0.5 { d / (2.0 - max - min) } else { d / (max + min) };
    let h = if max == r {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    [h * 60.0, s * 100.0, l * 100.0]
}

/// (hue degrees, saturation 0..100, lightness 0..100) → RGB in 0..=255 (unclamped float).
#[inline]
pub fn hsl_to_rgb(h: f32, s: f32, l: f32) -> [f32; 3] {
    let s = (s * 0.01).clamp(0.0, 1.0);
    let l = (l * 0.01).clamp(0.0, 1.0);
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = (h.rem_euclid(360.0)) / 60.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r1, g1, b1) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c * 0.5;
    [(r1 + m) * 255.0, (g1 + m) * 255.0, (b1 + m) * 255.0]
}

/// A zero-sum RGB offset of magnitude `amp` pointing along hue angle `theta` (radians).
/// The three components always sum to zero, so it shifts colour without shifting brightness.
#[inline]
pub fn chroma_offset(theta: f32, amp: f32) -> [f32; 3] {
    let (s, c) = theta.sin_cos();
    [amp * c, amp * (-0.5 * c - 0.866_025_4 * s), amp * (-0.5 * c + 0.866_025_4 * s)]
}

/// Box sizes for approximating a Gaussian with three box blurs (Kovesi).
fn boxes_for_gauss(sigma: f32, n: usize) -> Vec<usize> {
    let w_ideal = (12.0 * sigma * sigma / n as f32 + 1.0).sqrt();
    let mut wl = w_ideal.floor() as i32;
    if wl % 2 == 0 {
        wl -= 1;
    }
    let wu = wl + 2;
    let m_ideal = (12.0 * sigma * sigma - (n as i32 * wl * wl) as f32 - (4 * n as i32 * wl) as f32 - 3.0 * n as f32)
        / (-4.0 * wl as f32 - 4.0);
    let m = m_ideal.round() as i32;
    (0..n as i32).map(|i| if i < m { wl.max(1) as usize } else { wu.max(1) as usize }).collect()
}

/// In-place Gaussian blur (approximation) of an interleaved buffer with `ch` channels.
pub fn gaussian_blur(buf: &mut [f32], w: usize, h: usize, ch: usize, sigma: f32) {
    if sigma < 0.3 || w == 0 || h == 0 {
        return;
    }
    let mut tmp = vec![0f32; buf.len()];
    for b in boxes_for_gauss(sigma, 3) {
        let r = (b - 1) / 2;
        if r == 0 {
            continue;
        }
        box_blur_h(buf, &mut tmp, w, ch, r);
        box_blur_v(&tmp, buf, w, h, ch, r);
    }
}

fn box_blur_h(src: &[f32], dst: &mut [f32], w: usize, ch: usize, r: usize) {
    let inv = 1.0 / (2 * r + 1) as f32;
    dst.par_chunks_mut(w * ch).zip(src.par_chunks(w * ch)).for_each(|(drow, srow)| {
        for c in 0..ch {
            let px = |x: isize| srow[(x.clamp(0, w as isize - 1) as usize) * ch + c];
            let mut acc: f32 = (-(r as isize)..=r as isize).map(px).sum();
            for x in 0..w {
                drow[x * ch + c] = acc * inv;
                acc += px(x as isize + r as isize + 1) - px(x as isize - r as isize);
            }
        }
    });
}

fn box_blur_v(src: &[f32], dst: &mut [f32], w: usize, h: usize, ch: usize, r: usize) {
    let inv = 1.0 / (2 * r + 1) as f32;
    let stride = w * ch;
    // Process column strips in parallel; each strip writes disjoint columns of `dst`.
    let cols: Vec<usize> = (0..stride).collect();
    let results: Vec<(usize, Vec<f32>)> = cols
        .par_chunks(64)
        .map(|chunk| {
            let mut out = vec![0f32; chunk.len() * h];
            for (ci, &col) in chunk.iter().enumerate() {
                let px = |y: isize| src[(y.clamp(0, h as isize - 1) as usize) * stride + col];
                let mut acc: f32 = (-(r as isize)..=r as isize).map(px).sum();
                for y in 0..h {
                    out[ci * h + y] = acc * inv;
                    acc += px(y as isize + r as isize + 1) - px(y as isize - r as isize);
                }
            }
            (chunk[0], out)
        })
        .collect();
    for (start, out) in results {
        let n = out.len() / h;
        for ci in 0..n {
            for y in 0..h {
                dst[y * stride + start + ci] = out[ci * h + y];
            }
        }
    }
}

/// Rasterise polygons (normalised 0..1 coordinates) into a mask using the even-odd rule over
/// the union of polygons (a pixel is inside if it is inside any polygon).
pub fn polygon_mask(polys: &[Vec<[f32; 2]>], w: usize, h: usize) -> Vec<bool> {
    let mut mask = vec![false; w * h];
    for poly in polys.iter().filter(|p| p.len() >= 3) {
        let pts: Vec<(f32, f32)> = poly.iter().map(|p| (p[0] * w as f32, p[1] * h as f32)).collect();
        mask.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
            let yc = y as f32 + 0.5;
            let mut xs: Vec<f32> = Vec::new();
            for i in 0..pts.len() {
                let (x0, y0) = pts[i];
                let (x1, y1) = pts[(i + 1) % pts.len()];
                if (y0 <= yc && y1 > yc) || (y1 <= yc && y0 > yc) {
                    xs.push(x0 + (yc - y0) / (y1 - y0) * (x1 - x0));
                }
            }
            xs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            for pair in xs.chunks(2) {
                if let [a, b] = pair {
                    let start = (a - 0.5).ceil().max(0.0) as usize;
                    let end = ((b - 0.5).floor() as isize).min(w as isize - 1);
                    if end >= start as isize {
                        for v in &mut row[start..=end as usize] {
                            *v = true;
                        }
                    }
                }
            }
        });
    }
    mask
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hsl_roundtrip() {
        for &(r, g, b) in &[(255.0, 0.0, 0.0), (12.0, 200.0, 99.0), (128.0, 128.0, 128.0), (3.0, 4.0, 250.0)] {
            let [h, s, l] = rgb_to_hsl(r, g, b);
            let [r2, g2, b2] = hsl_to_rgb(h, s, l);
            assert!((r - r2).abs() < 0.6 && (g - g2).abs() < 0.6 && (b - b2).abs() < 0.6, "{r},{g},{b}");
        }
    }

    #[test]
    fn chroma_offset_is_zero_sum() {
        for i in 0..32 {
            let o = chroma_offset(i as f32 * 0.37, 50.0);
            assert!((o[0] + o[1] + o[2]).abs() < 1e-3);
        }
    }

    #[test]
    fn blur_preserves_constant() {
        let mut b = vec![7.0f32; 30 * 20 * 3];
        gaussian_blur(&mut b, 30, 20, 3, 4.0);
        assert!(b.iter().all(|v| (v - 7.0).abs() < 1e-3));
    }

    #[test]
    fn polygon_mask_square() {
        let m = polygon_mask(&[vec![[0.25, 0.25], [0.75, 0.25], [0.75, 0.75], [0.25, 0.75]]], 8, 8);
        assert_eq!(m.iter().filter(|v| **v).count(), 16);
    }
}
