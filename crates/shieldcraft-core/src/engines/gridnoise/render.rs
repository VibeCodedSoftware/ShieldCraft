//! Grid Noise Studio frame renderer.
//!
//! Original implementation for ShieldCraft of the temporal grid-noise technique
//! demonstrated by "Grid Noise Animator" (stelmosfire13-alt). No code from that project is
//! used. Randomness is counter-based (`hash(seed, frame, cell, salt)`), so any frame can be
//! rendered on its own, in any order, and reproduced exactly from the seed.

use super::params::*;
use crate::imgproc::{chroma_offset, gaussian_blur, hsl_to_rgb, polygon_mask, rgb_to_hsl};
use crate::util::{hash_keys, signed_f32, unit_f32};
use image::RgbaImage;
use rayon::prelude::*;
use std::f32::consts::{PI, TAU};
use std::sync::{Arc, Mutex};

const IRID_ANGLE: f32 = PI / 4.0;
const IRID_SPEED_HZ: f32 = 0.15;
const SPARKLE_MAX_PROB: f32 = 0.18;
const SPARKLE_MAX_DARKEN: f32 = 32.0;
const RAINBOW_L: f32 = 42.0;
const RAINBOW_MAX_MIX: f32 = 0.62;

// Salts that separate independent random streams.
const S_THETA: u64 = 1;
const S_DL: u64 = 2;
const S_DS: u64 = 3;
const S_WX: u64 = 4;
const S_WY: u64 = 5;
const S_EDGE: u64 = 6;
const S_AMP: u64 = 7;
const S_JIT: u64 = 8;
const S_FRAME: u64 = 9;
const S_SPARK: u64 = 10;
const S_SPARK_KIND: u64 = 11;
const S_SPARK_MIX: u64 = 12;
const S_SPARK_L: u64 = 13;
const S_VOR_X: u64 = 20;
const S_VOR_Y: u64 = 21;
const S_GRATING: u64 = 30;

/// Static data derived from the source image and the "static" settings. Rebuild it when
/// [`static_signature`] changes; per-frame settings can change freely.
pub struct Prepared {
    pub w: usize,
    pub h: usize,
    /// Render pixels per output pixel (1.0 for export, < 1 for a reduced preview).
    pub scale: f32,
    pub cols: usize,
    pub rows: usize,
    src_rgb: Vec<[f32; 3]>,
    alpha: Vec<u8>,
    hsl: Vec<[f32; 3]>,
    detail: Option<(Vec<[f32; 3]>, Vec<[f32; 3]>)>,
    overlay: Option<Vec<[f32; 3]>>,
    region: Option<Vec<bool>>,
    mark: Option<MarkStamp>,
    voronoi: Mutex<Option<(u64, Arc<Vec<u32>>)>>,
    pub signature: u64,
}

struct MarkStamp {
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    pad: usize,
    opacity: f32,
    text: crate::text::TextMask,
}

/// Hash of every setting that `Prepared::new` bakes in.
pub fn static_signature(p: &GridNoiseParams) -> u64 {
    let subset = serde_json::json!({
        "seed": p.seed, "cells": p.cells, "detail": p.detail.enabled, "radius": p.detail.radius,
        "grating": p.grating, "text": p.text_tile, "mark": p.visible_mark, "region": p.region,
    });
    let s = subset.to_string();
    hash_keys(&s.bytes().map(u64::from).collect::<Vec<_>>())
}

/// Output size of an animation for a source of `w × h`.
pub fn output_size(w: u32, h: u32, max_side: u32) -> (u32, u32) {
    if w.max(h) <= max_side {
        return (w, h);
    }
    let s = max_side as f32 / w.max(h) as f32;
    (((w as f32 * s).round() as u32).max(1), ((h as f32 * s).round() as u32).max(1))
}

pub fn grid_dims(w: usize, h: usize, cells: u32) -> (usize, usize) {
    let n = cells.max(1) as f32;
    if w >= h {
        (n as usize, ((n * h as f32 / w as f32).round() as usize).max(1))
    } else {
        (((n * w as f32 / h as f32).round() as usize).max(1), n as usize)
    }
}

impl Prepared {
    /// `src` must already be at render size; `scale` = render size / output size.
    pub fn new(src: &RgbaImage, p: &GridNoiseParams, scale: f32) -> Self {
        let (w, h) = (src.width() as usize, src.height() as usize);
        let n = w * h;
        let mut src_rgb = Vec::with_capacity(n);
        let mut alpha = Vec::with_capacity(n);
        for px in src.pixels() {
            src_rgb.push([px[0] as f32, px[1] as f32, px[2] as f32]);
            alpha.push(px[3]);
        }
        let hsl: Vec<[f32; 3]> = src_rgb.par_iter().map(|c| rgb_to_hsl(c[0], c[1], c[2])).collect();

        let detail = p.detail.enabled.then(|| {
            let mut low: Vec<f32> = src_rgb.iter().flat_map(|c| *c).collect();
            gaussian_blur(&mut low, w, h, 3, (p.detail.radius * scale).max(0.5));
            let low_hsl: Vec<[f32; 3]> =
                low.par_chunks(3).map(|c| rgb_to_hsl(c[0], c[1], c[2])).collect();
            let edge: Vec<[f32; 3]> = src_rgb
                .par_iter()
                .zip(low.par_chunks(3))
                .map(|(s, l)| [s[0] - l[0], s[1] - l[1], s[2] - l[2]])
                .collect();
            (low_hsl, edge)
        });

        let overlay = build_overlay(p, w, h, scale);
        let region = (!p.region.polygons.is_empty()).then(|| {
            let inside = polygon_mask(&p.region.polygons, w, h);
            inside.into_iter().map(|v| v == p.region.apply_inside).collect()
        });
        let mark = build_mark(&p.visible_mark, w, h, scale);
        let (cols, rows) = grid_dims(w, h, p.cells);
        Prepared {
            w,
            h,
            scale,
            cols,
            rows,
            src_rgb,
            alpha,
            hsl,
            detail,
            overlay,
            region,
            mark,
            voronoi: Mutex::new(None),
            signature: static_signature(p),
        }
    }

    pub fn cell_count(&self) -> usize {
        self.cols * self.rows
    }

    #[inline]
    fn square_cell(&self, x: usize, y: usize) -> usize {
        let c = (x * self.cols / self.w).min(self.cols - 1);
        let r = (y * self.rows / self.h).min(self.rows - 1);
        r * self.cols + c
    }

    fn voronoi_map(&self, seed: u64, epoch: u64) -> Arc<Vec<u32>> {
        if let Some((e, m)) = self.voronoi.lock().unwrap().as_ref() {
            if *e == epoch {
                return m.clone();
            }
        }
        let (cols, rows, w, h) = (self.cols, self.rows, self.w, self.h);
        let cw = w as f32 / cols as f32;
        let ch = h as f32 / rows as f32;
        let seeds: Vec<[f32; 2]> = (0..cols * rows)
            .map(|ci| {
                let (c, r) = ((ci % cols) as f32, (ci / cols) as f32);
                let jx = unit_f32(hash_keys(&[seed, epoch, ci as u64, S_VOR_X]));
                let jy = unit_f32(hash_keys(&[seed, epoch, ci as u64, S_VOR_Y]));
                [(c + 0.25 + 0.5 * jx) * cw, (r + 0.25 + 0.5 * jy) * ch]
            })
            .collect();
        let mut map = vec![0u32; w * h];
        map.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
            let rc = ((y as f32 / ch) as usize).min(rows - 1);
            for (x, out) in row.iter_mut().enumerate() {
                let cc = ((x as f32 / cw) as usize).min(cols - 1);
                let mut best = f32::MAX;
                let mut best_i = 0usize;
                for r in rc.saturating_sub(1)..=(rc + 1).min(rows - 1) {
                    for c in cc.saturating_sub(1)..=(cc + 1).min(cols - 1) {
                        let i = r * cols + c;
                        let dx = x as f32 + 0.5 - seeds[i][0];
                        let dy = y as f32 + 0.5 - seeds[i][1];
                        let d = dx * dx + dy * dy;
                        if d < best {
                            best = d;
                            best_i = i;
                        }
                    }
                }
                *out = best_i as u32;
            }
        });
        let map = Arc::new(map);
        *self.voronoi.lock().unwrap() = Some((epoch, map.clone()));
        map
    }
}

fn build_overlay(p: &GridNoiseParams, w: usize, h: usize, scale: f32) -> Option<Vec<[f32; 3]>> {
    let text_on = p.text_tile.enabled && !p.text_tile.text.trim().is_empty();
    if !p.grating.enabled && !text_on {
        return None;
    }
    let mut ov = vec![[0f32; 3]; w * h];
    if p.grating.enabled && p.grating.amplitude > 0.0 && p.grating.components > 0 {
        let k = p.grating.components as usize;
        let a = p.grating.amplitude / k as f32;
        for ch in 0..3 {
            for comp in 0..k {
                let r = |s: u64| unit_f32(hash_keys(&[p.seed, S_GRATING, ch as u64, comp as u64, s]));
                let period_out = p.grating.min_period + r(0) * (p.grating.max_period - p.grating.min_period);
                let period = period_out * scale;
                // A reduced preview cannot represent periods below ~2 px; fade them out instead
                // of aliasing them into large false patterns.
                let fidelity = ((period - 1.2) / 0.8).clamp(0.0, 1.0);
                if fidelity <= 0.0 {
                    continue;
                }
                let angle = r(1) * PI;
                let phase = r(2) * TAU;
                let (sa, ca) = angle.sin_cos();
                let k2 = TAU / period;
                let amp = a * fidelity;
                ov.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
                    for (x, v) in row.iter_mut().enumerate() {
                        v[ch] += amp * (k2 * (x as f32 * ca + y as f32 * sa) + phase).sin();
                    }
                });
            }
        }
    }
    if text_on {
        let px = (w.min(h) as f32 * 0.06).max(10.0 * scale.max(0.5));
        let mask = crate::text::tiled_mask(&p.text_tile.text, w, h, px);
        let amp = if p.text_tile.dark { -p.text_tile.contrast } else { p.text_tile.contrast };
        ov.par_iter_mut().zip(mask.par_iter()).for_each(|(v, m)| {
            if *m > 0.0 {
                for c in v.iter_mut() {
                    *c += m * amp;
                }
            }
        });
    }
    // Remove the per-channel mean so the overlay does not shift overall brightness.
    for ch in 0..3 {
        let mean = ov.par_iter().map(|v| v[ch] as f64).sum::<f64>() / (w * h).max(1) as f64;
        ov.par_iter_mut().for_each(|v| v[ch] -= mean as f32);
    }
    Some(ov)
}

fn build_mark(m: &VisibleMark, w: usize, h: usize, scale: f32) -> Option<MarkStamp> {
    if !m.enabled || m.text.trim().is_empty() {
        return None;
    }
    let px = (w.min(h) as f32 * m.size_pct / 100.0).max(11.0 * scale.max(0.4));
    let text = crate::text::render_line(&m.text, px, true);
    let pad = ((7.0 * scale).round() as usize).max(2);
    let bw = text.width + pad * 2;
    let bh = text.height + pad;
    if bw + pad * 2 > w || bh + pad * 2 > h {
        return None;
    }
    let (x, y) = match m.corner {
        Corner::BottomLeft => (pad, h - bh - pad),
        Corner::BottomRight => (w - bw - pad, h - bh - pad),
        Corner::TopLeft => (pad, pad),
        Corner::TopRight => (w - bw - pad, pad),
    };
    Some(MarkStamp { x, y, w: bw, h: bh, pad, opacity: m.opacity.clamp(0.0, 1.0), text })
}

#[derive(Clone, Copy, Default)]
struct CellCoef {
    chroma: [f32; 3],
    dl: f32,
    ds: f32,
    wx: f32,
    wy: f32,
    edge_gain: f32,
    sparkle: Option<([f32; 3], f32)>,
    rainbow_hue: f32,
}

fn cell_coefficients(pr: &Prepared, p: &GridNoiseParams, frame: u32) -> Vec<CellCoef> {
    let n = pr.cell_count();
    let seed = p.seed;
    let f = frame as u64;
    let chroma_amp = p.hue_deg / 180.0 * 128.0;
    let warp = if p.warp_enabled { p.warp_px * pr.scale } else { 0.0 };
    let edge = p.detail.edge_strength / 100.0;
    let t = frame as f32 / p.animation.fps.max(1) as f32;
    let spark_prob = if p.sparkle.enabled { p.sparkle.strength / 100.0 * SPARKLE_MAX_PROB } else { 0.0 };
    let irid_period = pr.w.max(pr.h) as f32 / 2.5;
    let (isa, ica) = IRID_ANGLE.sin_cos();
    let frame_phase = unit_f32(hash_keys(&[seed, f, S_FRAME])) * TAU;
    let cyc = p.cycle_frames.max(2);
    let cycle_ph = (frame % cyc) as f32 * TAU / cyc as f32;

    (0..n)
        .into_par_iter()
        .map(|ci| {
            let c = ci as u64;
            let (col, row) = (ci % pr.cols, ci / pr.cols);
            let u = |salt: u64| unit_f32(hash_keys(&[seed, f, c, salt]));
            let sgn = |salt: u64| signed_f32(hash_keys(&[seed, f, c, salt]));
            let base = |salt: u64| unit_f32(hash_keys(&[seed, c, salt, 0xC1C1E])) * TAU;
            let (theta, dl, ds, wx, wy, eg) = match p.balance {
                Balance::Cycle => (
                    base(S_THETA) + cycle_ph,
                    (base(S_DL) + cycle_ph).cos() * p.lightness,
                    (base(S_DS) + cycle_ph).cos() * p.saturation,
                    (base(S_WX) + cycle_ph).cos() * warp,
                    (base(S_WY) + cycle_ph).cos() * warp,
                    (base(S_EDGE) + cycle_ph).cos() * edge,
                ),
                Balance::Antiphase => {
                    let checker = ((col + row) & 1) as f32;
                    let phase = frame_phase + checker * PI;
                    let amp = 0.5 + 0.5 * u(S_AMP);
                    (
                        phase + (u(S_JIT) - 0.5) * (PI / 3.0),
                        phase.cos() * p.lightness * amp,
                        phase.sin() * p.saturation * amp,
                        sgn(S_WX) * warp,
                        sgn(S_WY) * warp,
                        sgn(S_EDGE) * edge,
                    )
                }
                Balance::Random => (
                    u(S_THETA) * TAU,
                    sgn(S_DL) * p.lightness,
                    sgn(S_DS) * p.saturation,
                    sgn(S_WX) * warp,
                    sgn(S_WY) * warp,
                    sgn(S_EDGE) * edge,
                ),
            };
            let cx = (col as f32 + 0.5) * pr.w as f32 / pr.cols as f32;
            let cy = (row as f32 + 0.5) * pr.h as f32 / pr.rows as f32;
            let spatial = (cx * ica + cy * isa) / irid_period;
            let hue_here = ((spatial + t * IRID_SPEED_HZ).rem_euclid(1.0)) * 360.0;
            let sparkle = (spark_prob > 0.0 && u(S_SPARK) < spark_prob).then(|| {
                if u(S_SPARK_KIND) < 0.5 {
                    (hsl_to_rgb(hue_here, 100.0, 50.0), 0.65 + 0.30 * u(S_SPARK_MIX))
                } else {
                    let l = 10.0 + 18.0 * u(S_SPARK_L);
                    (hsl_to_rgb(hue_here, 100.0, l), 0.35 + 0.45 * u(S_SPARK_MIX))
                }
            });
            CellCoef {
                chroma: chroma_offset(theta, chroma_amp),
                dl,
                ds,
                wx,
                wy,
                edge_gain: eg,
                sparkle,
                rainbow_hue: hue_here,
            }
        })
        .collect()
}

/// What to include when rendering.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RenderMode {
    /// Full animated frame.
    Frame,
    /// Only the static layers (overlay, visible mark) on the clean source — for a still export.
    StaticOnly,
}

/// Render one frame into an RGBA buffer of `w × h × 4` bytes.
pub fn render_frame(pr: &Prepared, p: &GridNoiseParams, frame: u32, mode: RenderMode, out: &mut [u8]) {
    let (w, h) = (pr.w, pr.h);
    assert_eq!(out.len(), w * h * 4, "output buffer size mismatch");
    let mut buf = vec![0f32; w * h * 3];

    if mode == RenderMode::Frame {
        let coef = cell_coefficients(pr, p, frame);
        let vor = (p.shape == CellShape::Voronoi).then(|| {
            let epoch = if p.voronoi_reseed == 0 { 0 } else { (frame / p.voronoi_reseed) as u64 };
            pr.voronoi_map(p.seed, epoch)
        });
        let t = frame as f32 / p.animation.fps.max(1) as f32;
        let irid_amp = if p.iridescent.enabled { p.iridescent.strength / 100.0 * 90.0 } else { 0.0 };
        let irid_period = pr.w.max(pr.h) as f32 / 2.5;
        let (isa, ica) = IRID_ANGLE.sin_cos();
        let irid_base = t * IRID_SPEED_HZ * TAU;
        let darken = if p.sparkle.enabled { p.sparkle.strength / 100.0 * SPARKLE_MAX_DARKEN } else { 0.0 };
        let rb_mix = if p.cell_rainbow.enabled { p.cell_rainbow.strength / 100.0 * RAINBOW_MAX_MIX } else { 0.0 };
        let cell_w = w as f32 / pr.cols as f32;
        let cell_h = h as f32 / pr.rows as f32;
        let rb_span = 1.0 / cell_w.max(cell_h).max(1.0);

        buf.par_chunks_mut(w * 3).enumerate().for_each(|(y, row)| {
            for x in 0..w {
                let i = y * w + x;
                let ci = match &vor {
                    Some(m) => m[i] as usize,
                    None => pr.square_cell(x, y),
                };
                let k = &coef[ci];
                let sx = ((x as f32 + k.wx) as isize).clamp(0, w as isize - 1) as usize;
                let sy = ((y as f32 + k.wy) as isize).clamp(0, h as isize - 1) as usize;
                let si = sy * w + sx;

                let mut rgb = if let Some((low, edge)) = &pr.detail {
                    let base = low[i];
                    let mut c = hsl_to_rgb(base[0], base[1] + k.ds, base[2] + k.dl - darken);
                    let e = edge[si];
                    let gain = 1.0 + k.edge_gain;
                    for ch in 0..3 {
                        c[ch] = (c[ch] + k.chroma[ch]).clamp(0.0, 255.0) + e[ch] * gain;
                    }
                    c
                } else {
                    let base = pr.hsl[si];
                    let mut c = hsl_to_rgb(base[0], base[1] + k.ds, base[2] + k.dl - darken);
                    for ch in 0..3 {
                        c[ch] += k.chroma[ch];
                    }
                    c
                };

                if irid_amp > 0.0 {
                    let ph = (x as f32 * ica + y as f32 * isa) / irid_period * TAU + irid_base;
                    let o = chroma_offset(ph, irid_amp);
                    for ch in 0..3 {
                        rgb[ch] += o[ch];
                    }
                }
                if let Some((col, mix)) = k.sparkle {
                    for ch in 0..3 {
                        rgb[ch] = rgb[ch].clamp(0.0, 255.0) * (1.0 - mix) + col[ch] * mix;
                    }
                }
                if rb_mix > 0.0 {
                    let cx0 = (ci % pr.cols) as f32 * cell_w;
                    let cy0 = (ci / pr.cols) as f32 * cell_h;
                    let local = ((x as f32 - cx0) * ica + (y as f32 - cy0) * isa) * rb_span;
                    let rb = hsl_to_rgb(k.rainbow_hue + local * 360.0, 100.0, RAINBOW_L);
                    for ch in 0..3 {
                        rgb[ch] = rgb[ch].clamp(0.0, 255.0) * (1.0 - rb_mix) + rb[ch] * rb_mix;
                    }
                }
                if let Some(ov) = &pr.overlay {
                    for ch in 0..3 {
                        rgb[ch] += ov[i][ch];
                    }
                }
                let o = &mut row[x * 3..x * 3 + 3];
                for ch in 0..3 {
                    o[ch] = rgb[ch].clamp(0.0, 255.0);
                }
            }
        });
        if p.blur_enabled && p.blur_px > 0.0 {
            gaussian_blur(&mut buf, w, h, 3, p.blur_px * pr.scale);
        }
    } else {
        buf.par_chunks_mut(w * 3).enumerate().for_each(|(y, row)| {
            for x in 0..w {
                let i = y * w + x;
                let mut c = pr.src_rgb[i];
                if let Some(ov) = &pr.overlay {
                    for ch in 0..3 {
                        c[ch] = (c[ch] + ov[i][ch]).clamp(0.0, 255.0);
                    }
                }
                row[x * 3..x * 3 + 3].copy_from_slice(&c);
            }
        });
    }

    let keep_alpha = p.preserve_alpha;
    out.par_chunks_mut(w * 4).enumerate().for_each(|(y, row)| {
        for x in 0..w {
            let i = y * w + x;
            let use_noise = pr.region.as_ref().map(|r| r[i]).unwrap_or(true);
            let c = if use_noise || mode == RenderMode::StaticOnly {
                [buf[i * 3], buf[i * 3 + 1], buf[i * 3 + 2]]
            } else {
                pr.src_rgb[i]
            };
            let o = &mut row[x * 4..x * 4 + 4];
            o[0] = (c[0] + 0.5) as u8;
            o[1] = (c[1] + 0.5) as u8;
            o[2] = (c[2] + 0.5) as u8;
            o[3] = if keep_alpha { pr.alpha[i] } else { 255 };
        }
    });

    if let Some(m) = &pr.mark {
        stamp_mark(m, w, out);
    }
}

fn stamp_mark(m: &MarkStamp, w: usize, out: &mut [u8]) {
    for yy in 0..m.h {
        for xx in 0..m.w {
            let i = ((m.y + yy) * w + m.x + xx) * 4;
            let tx = xx as isize - m.pad as isize;
            let ty = yy as isize - (m.pad / 2) as isize;
            let cov = if tx >= 0 && ty >= 0 && (tx as usize) < m.text.width && (ty as usize) < m.text.height {
                m.text.at(tx as usize, ty as usize)
            } else {
                0.0
            };
            for ch in 0..3 {
                let base = out[i + ch] as f32 * (1.0 - m.opacity) + 17.0 * m.opacity;
                out[i + ch] = (base * (1.0 - cov) + 255.0 * cov + 0.5) as u8;
            }
            out[i + 3] = out[i + 3].max((m.opacity * 255.0) as u8).max((cov * 255.0) as u8);
        }
    }
}

pub fn render_image(pr: &Prepared, p: &GridNoiseParams, frame: u32, mode: RenderMode) -> RgbaImage {
    let mut out = vec![0u8; pr.w * pr.h * 4];
    render_frame(pr, p, frame, mode, &mut out);
    RgbaImage::from_raw(pr.w as u32, pr.h as u32, out).expect("buffer size matches")
}

/// Measured (not estimated) reconstruction statistics over `frames` consecutive frames.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Analysis {
    pub frames: u32,
    /// PSNR of the temporal average against the clean source (higher = closer to the art).
    pub average_psnr_db: f64,
    /// Mean PSNR of individual frames against the source (lower = each frame more scrambled).
    pub single_frame_psnr_db: f64,
    /// Fraction of pixels whose single-frame error exceeds 8/255 in any channel.
    pub scrambled_fraction: f64,
}

pub fn analyse(pr: &Prepared, p: &GridNoiseParams, start: u32, frames: u32) -> (Analysis, RgbaImage) {
    let frames = frames.max(1);
    let n = pr.w * pr.h;
    let mut acc = vec![0f64; n * 3];
    let mut psnr_sum = 0f64;
    let mut scrambled = 0f64;
    let mut buf = vec![0u8; n * 4];
    for f in start..start + frames {
        render_frame(pr, p, f, RenderMode::Frame, &mut buf);
        let mut se = 0f64;
        let mut changed = 0u64;
        for i in 0..n {
            let mut big = false;
            for ch in 0..3 {
                let v = buf[i * 4 + ch] as f64;
                acc[i * 3 + ch] += v;
                let d = v - pr.src_rgb[i][ch] as f64;
                se += d * d;
                big |= d.abs() > 8.0;
            }
            changed += big as u64;
        }
        let mse = se / (n * 3) as f64;
        psnr_sum += if mse == 0.0 { 99.0 } else { 10.0 * (65025.0 / mse).log10() };
        scrambled += changed as f64 / n as f64;
    }
    let mut avg = RgbaImage::new(pr.w as u32, pr.h as u32);
    let mut se = 0f64;
    for (i, px) in avg.pixels_mut().enumerate() {
        for ch in 0..3 {
            let v = acc[i * 3 + ch] / frames as f64;
            let d = v - pr.src_rgb[i][ch] as f64;
            se += d * d;
            px[ch] = (v + 0.5) as u8;
        }
        px[3] = pr.alpha[i];
    }
    let mse = se / (n * 3) as f64;
    let analysis = Analysis {
        frames,
        average_psnr_db: if mse == 0.0 { 99.0 } else { 10.0 * (65025.0 / mse).log10() },
        single_frame_psnr_db: psnr_sum / frames as f64,
        scrambled_fraction: scrambled / frames as f64,
    };
    (analysis, avg)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_image() -> RgbaImage {
        RgbaImage::from_fn(96, 64, |x, y| image::Rgba([(x * 2) as u8 + 40, (y * 3) as u8 + 30, 128, if x < 4 { 0 } else { 255 }]))
    }

    fn quiet_params() -> GridNoiseParams {
        let mut p = GridNoiseParams::default();
        p.grating.enabled = false;
        p.visible_mark.enabled = false;
        p.blur_enabled = false;
        p.warp_enabled = false;
        p
    }

    #[test]
    fn frames_are_reproducible_and_distinct() {
        let img = test_image();
        let p = GridNoiseParams::default();
        let pr = Prepared::new(&img, &p, 1.0);
        let a = render_image(&pr, &p, 3, RenderMode::Frame);
        let b = render_image(&pr, &p, 3, RenderMode::Frame);
        let c = render_image(&pr, &p, 4, RenderMode::Frame);
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn alpha_is_preserved() {
        let img = test_image();
        let p = GridNoiseParams::default();
        let pr = Prepared::new(&img, &p, 1.0);
        let out = render_image(&pr, &p, 0, RenderMode::Frame);
        assert_eq!(out.get_pixel(1, 30)[3], 0);
        assert_eq!(out.get_pixel(50, 30)[3], 255);
    }

    #[test]
    fn exact_cycle_averages_back_to_source() {
        let img = test_image();
        let mut p = quiet_params();
        p.balance = Balance::Cycle;
        p.cycle_frames = 4;
        let pr = Prepared::new(&img, &p, 1.0);
        let (a, _) = analyse(&pr, &p, 0, 4);
        assert!(a.single_frame_psnr_db < 35.0, "frames should be visibly noisy: {a:?}");
        assert!(a.average_psnr_db > a.single_frame_psnr_db + 8.0, "average must be much closer: {a:?}");
    }

    #[test]
    fn region_outside_keeps_source_pixels() {
        let img = test_image();
        let mut p = quiet_params();
        p.region.polygons = vec![vec![[0.5, 0.0], [1.0, 0.0], [1.0, 1.0], [0.5, 1.0]]];
        p.region.apply_inside = true;
        let pr = Prepared::new(&img, &p, 1.0);
        let out = render_image(&pr, &p, 1, RenderMode::Frame);
        assert_eq!(out.get_pixel(10, 10), img.get_pixel(10, 10));
    }

    #[test]
    fn voronoi_and_detail_render() {
        let img = test_image();
        let mut p = GridNoiseParams::default();
        p.shape = CellShape::Voronoi;
        p.detail.enabled = true;
        p.iridescent.enabled = true;
        p.sparkle.enabled = true;
        p.cell_rainbow.enabled = true;
        p.text_tile.enabled = true;
        let pr = Prepared::new(&img, &p, 1.0);
        let _ = render_image(&pr, &p, 11, RenderMode::Frame);
        let _ = render_image(&pr, &p, 0, RenderMode::StaticOnly);
    }
}
