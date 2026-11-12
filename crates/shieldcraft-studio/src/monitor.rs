//! Effect Monitor: a histogram-style panel that shows, for any engine, how strongly and in what
//! pattern the image is being changed. Fed with (original, modified) pairs of the same size.

use egui::{Color32, Pos2, Rect, RichText, Sense, Stroke, Vec2};
use image::RgbaImage;

pub const BINS: usize = 32;
const TILE: u32 = 128;

pub struct Monitor {
    pub key: (usize, usize),
    pub source_label: String,
    /// Counts of per-channel absolute change, binned over 0..=range.
    pub hist: [u64; BINS],
    pub range: u8,
    pub max: u8,
    pub mean: f32,
    pub changed_pct: f32,
    pub psnr: f64,
    pub gain: f32,
    pub tile: egui::TextureHandle,
}

/// Compute statistics and an amplified pattern tile (centre crop at 1:1).
pub fn analyse(ctx: &egui::Context, key: (usize, usize), label: &str, orig: &RgbaImage, modified: &RgbaImage) -> Option<Monitor> {
    if orig.dimensions() != modified.dimensions() || orig.width() == 0 {
        return None;
    }
    let mut counts = [0u64; 256];
    let mut changed = 0u64;
    let mut se = 0f64;
    let mut sum = 0u64;
    for (a, b) in orig.pixels().zip(modified.pixels()) {
        let mut any = false;
        for c in 0..3 {
            let d = a[c].abs_diff(b[c]);
            counts[d as usize] += 1;
            sum += d as u64;
            se += (d as f64) * (d as f64);
            any |= d > 0;
        }
        changed += any as u64;
    }
    let n = orig.width() as u64 * orig.height() as u64;
    let max = (0..256).rev().find(|&i| counts[i] > 0).unwrap_or(0) as u8;
    let range = max.max(8);
    let mut hist = [0u64; BINS];
    for (v, &c) in counts.iter().enumerate().take(range as usize + 1) {
        let bin = (v * (BINS - 1)) / range as usize;
        hist[bin] += c;
    }
    let mse = se / (n * 3).max(1) as f64;
    let psnr = if mse == 0.0 { 99.0 } else { 10.0 * (65025.0 / mse).log10() };

    // Amplify so the largest change maps to near-white, whatever the engine's strength.
    let gain = (220.0 / max.max(1) as f32).clamp(1.0, 60.0);
    let (w, h) = orig.dimensions();
    let tw = TILE.min(w);
    let th = TILE.min(h);
    let (x0, y0) = ((w - tw) / 2, (h - th) / 2);
    let mut px = Vec::with_capacity((tw * th * 4) as usize);
    for y in y0..y0 + th {
        for x in x0..x0 + tw {
            let (a, b) = (orig.get_pixel(x, y), modified.get_pixel(x, y));
            for c in 0..3 {
                px.push((a[c].abs_diff(b[c]) as f32 * gain).min(255.0) as u8);
            }
            px.push(255);
        }
    }
    let img = egui::ColorImage::from_rgba_unmultiplied([tw as usize, th as usize], &px);
    let opts = egui::TextureOptions { magnification: egui::TextureFilter::Nearest, ..egui::TextureOptions::LINEAR };
    let tile = ctx.load_texture("monitor_tile", img, opts);
    Some(Monitor {
        key,
        source_label: label.to_string(),
        hist,
        range,
        max,
        mean: sum as f32 / (n * 3).max(1) as f32,
        changed_pct: changed as f32 / n.max(1) as f32 * 100.0,
        psnr,
        gain,
        tile,
    })
}

pub fn show(ui: &mut egui::Ui, m: Option<&Monitor>) {
    let Some(m) = m else {
        ui.label(RichText::new("No change to show yet: import an image, or adjust an engine.").small().weak());
        return;
    };
    ui.horizontal(|ui| {
        // Pattern tile.
        ui.vertical(|ui| {
            let size = Vec2::splat(112.0);
            let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
            ui.painter().image(m.tile.id(), rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
            ui.painter().rect_stroke(rect, 0.0, Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color), egui::StrokeKind::Outside);
            ui.label(RichText::new(format!("pattern ×{:.0}", m.gain)).small().weak()).on_hover_text("Centre crop at 1:1, amplified so the largest change is near-white. Black = unchanged.");
        });
        // Histogram.
        ui.vertical(|ui| {
            let w = ui.available_width().max(120.0);
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 92.0), Sense::hover());
            let painter = ui.painter_at(rect);
            painter.rect_filled(rect, 2.0, ui.visuals().extreme_bg_color);
            let peak = m.hist.iter().copied().max().unwrap_or(1).max(1) as f32;
            let bw = rect.width() / BINS as f32;
            for (i, &c) in m.hist.iter().enumerate() {
                if c == 0 {
                    continue;
                }
                // Square-root scale so small tails stay visible next to the zero bin.
                let hgt = (c as f32 / peak).sqrt() * (rect.height() - 4.0);
                let x = rect.left() + i as f32 * bw;
                let col = if i == 0 { Color32::from_gray(110) } else { crate::theme::ACCENT };
                painter.rect_filled(Rect::from_min_max(Pos2::new(x + 0.5, rect.bottom() - hgt), Pos2::new(x + bw - 0.5, rect.bottom())), 0.0, col);
            }
            resp.on_hover_text("How many colour values change by how much. Grey bar: unchanged. Blue: changed, from small (left) to the maximum (right). Square-root scale.");
            ui.horizontal(|ui| {
                ui.label(RichText::new("0").small().weak());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(format!("{}/255", m.range)).small().weak());
                });
            });
        });
    });
    ui.label(
        RichText::new(format!(
            "max {}/255 · mean {:.2} · {:.0}% of pixels changed · PSNR {:.1} dB",
            m.max, m.mean, m.changed_pct, m.psnr
        ))
        .small(),
    );
    ui.label(RichText::new(&m.source_label).small().weak());
}
