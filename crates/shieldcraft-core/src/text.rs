//! Minimal text rasterisation (coverage masks) for tiled text and visible watermarks.
//! Uses the Hack font that ships with egui's default fonts (MIT / Bitstream Vera licence).

use ab_glyph::{Font, FontRef, PxScale, ScaleFont};
use std::sync::OnceLock;

fn font() -> &'static FontRef<'static> {
    static FONT: OnceLock<FontRef<'static>> = OnceLock::new();
    FONT.get_or_init(|| FontRef::try_from_slice(epaint_default_fonts::HACK_REGULAR).expect("bundled font is valid"))
}

/// A rendered single line of text: coverage values 0..1, row-major.
pub struct TextMask {
    pub width: usize,
    pub height: usize,
    pub coverage: Vec<f32>,
}

impl TextMask {
    pub fn at(&self, x: usize, y: usize) -> f32 {
        self.coverage[y * self.width + x]
    }
}

/// Rasterise `text` at `px` pixel height. `bold` thickens strokes by a 1px dilation.
pub fn render_line(text: &str, px: f32, bold: bool) -> TextMask {
    let f = font();
    let scale = PxScale::from(px.max(4.0));
    let sf = f.as_scaled(scale);
    let mut glyphs = Vec::new();
    let mut x = 0.0f32;
    let mut prev = None;
    for ch in text.chars() {
        let id = sf.glyph_id(ch);
        if let Some(p) = prev {
            x += sf.kern(p, id);
        }
        glyphs.push(id.with_scale_and_position(scale, ab_glyph::point(x, sf.ascent())));
        x += sf.h_advance(id);
        prev = Some(id);
    }
    let pad = if bold { 2 } else { 1 };
    let width = (x.ceil() as usize + pad * 2).max(1);
    let height = ((sf.ascent() - sf.descent()).ceil() as usize + pad * 2).max(1);
    let mut coverage = vec![0f32; width * height];
    for g in glyphs {
        if let Some(outlined) = f.outline_glyph(g) {
            let bb = outlined.px_bounds();
            outlined.draw(|gx, gy, c| {
                let xx = bb.min.x as i32 + gx as i32 + pad as i32;
                let yy = bb.min.y as i32 + gy as i32 + pad as i32;
                if xx >= 0 && yy >= 0 && (xx as usize) < width && (yy as usize) < height {
                    let i = yy as usize * width + xx as usize;
                    coverage[i] = coverage[i].max(c);
                }
            });
        }
    }
    if bold {
        let src = coverage.clone();
        for y in 0..height {
            for x in 0..width {
                let mut m = src[y * width + x];
                if x > 0 {
                    m = m.max(src[y * width + x - 1]);
                }
                if y > 0 {
                    m = m.max(src[(y - 1) * width + x]);
                }
                coverage[y * width + x] = m;
            }
        }
    }
    TextMask { width, height, coverage }
}

/// A full-image mask with `text` tiled in staggered rows (every other row offset by half a
/// tile), like a repeating "do not train" notice.
pub fn tiled_mask(text: &str, w: usize, h: usize, px: f32) -> Vec<f32> {
    let mut mask = vec![0f32; w * h];
    if text.trim().is_empty() || w == 0 || h == 0 {
        return mask;
    }
    let line = render_line(&format!("{text}   "), px, true);
    let row_h = (px * 1.7).max(1.0) as usize;
    let tw = line.width.max(1);
    let mut row = 0usize;
    let mut y0 = 0usize;
    while y0 < h + row_h {
        let offset = if row % 2 == 1 { tw / 2 } else { 0 };
        for y in 0..line.height {
            let yy = y0 + y;
            if yy >= h {
                break;
            }
            for x in 0..w {
                let lx = (x + offset) % tw;
                let c = line.at(lx, y);
                if c > 0.0 {
                    let i = yy * w + x;
                    mask[i] = mask[i].max(c);
                }
            }
        }
        y0 += row_h;
        row += 1;
    }
    mask
}

#[cfg(test)]
mod tests {
    #[test]
    fn renders_visible_coverage() {
        let m = super::render_line("NO AI", 24.0, true);
        assert!(m.width > 40 && m.height > 15);
        assert!(m.coverage.iter().any(|c| *c > 0.9));
    }
}
