//! Tiled SPSA perturbation optimiser — Rust port of Hope:RE's `tiling.rs`, `spsa.rs` and
//! `preprocessing.rs` (Copyright (c) 2025 HopeArtOrg, MIT License).
//!
//! The ONNX model maps a 1×224×224×3 image (values 0..1, NHWC) to a scalar loss. SPSA estimates
//! the loss gradient from paired ± random-sign probes, so no autograd is needed.

use super::{HopeAlgorithm, HopeParams};
use crate::engine::{CancelToken, EngineError, ProgressFn};
use crate::util::Rng;
use image::{imageops::FilterType, RgbaImage};
use ort::session::Session;
use ort::value::Tensor;

pub const TILE: u32 = 224;
pub const OVERLAP: u32 = 32;
pub const DIRECTIONS_PER_ITER: usize = 4;
const MOMENTUM_BETA: f32 = 0.85;

#[derive(Clone, Debug)]
pub struct AlgorithmParams {
    pub epsilon: f32,
    pub alpha_multiplier: f32,
    pub perceptual_weight: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct TileRegion {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

pub fn tile_regions(width: u32, height: u32) -> Vec<TileRegion> {
    let stride = TILE - OVERLAP;
    let tiles = |len: u32| if len <= TILE { 1 } else { ((len - OVERLAP) as f32 / stride as f32).ceil() as u32 };
    let (tx, ty) = (tiles(width), tiles(height));
    let mut out = Vec::new();
    for j in 0..ty {
        for i in 0..tx {
            let x = (i * stride).min(width.saturating_sub(TILE));
            let y = (j * stride).min(height.saturating_sub(TILE));
            out.push(TileRegion { x, y, w: TILE.min(width - x), h: TILE.min(height - y) });
        }
    }
    out
}

pub fn open_session(path: &std::path::Path, params: &HopeParams) -> Result<Session, String> {
    use ort::session::builder::GraphOptimizationLevel;
    let mut b = Session::builder()
        .map_err(|e| e.to_string())?
        .with_optimization_level(GraphOptimizationLevel::Level3)
        .map_err(|e| e.to_string())?;
    if params.threads > 0 {
        b = b.with_intra_threads(params.threads as usize).map_err(|e| e.to_string())?;
    }
    if params.device == super::ExecutionDevice::DirectMlExperimental {
        b = b
            .with_execution_providers([ort::ep::DirectML::default().build().error_on_failure()])
            .map_err(|e| format!("DirectML could not be enabled: {e}"))?;
    }
    b.commit_from_file(path).map_err(|e| format!("Failed to load model {}: {e}", path.display()))
}

fn run_model(session: &mut Session, alg: HopeAlgorithm, extra: i32, data: &[f32]) -> Result<f32, EngineError> {
    let t = TILE as usize;
    let input = Tensor::from_array(([1usize, t, t, 3], data.to_vec().into_boxed_slice()))
        .map_err(|e| EngineError::Failed(format!("Tensor creation failed: {e}")))?;
    let outputs = match alg {
        HopeAlgorithm::Noise => session.run(ort::inputs![input]),
        HopeAlgorithm::Style | HopeAlgorithm::Poison => {
            let idx = Tensor::from_array(([1usize], vec![extra].into_boxed_slice()))
                .map_err(|e| EngineError::Failed(format!("Tensor creation failed: {e}")))?;
            session.run(ort::inputs![input, idx])
        }
    }
    .map_err(|e| crate::engine::translate_error(&format!("Model inference failed: {e}")))?;
    outputs[0]
        .try_extract_scalar::<f32>()
        .map_err(|e| EngineError::Failed(format!("Unexpected model output: {e}")))
}

fn preprocess_tile(img: &RgbaImage, r: &TileRegion) -> Vec<f32> {
    let crop = image::imageops::crop_imm(img, r.x, r.y, r.w, r.h).to_image();
    let tile = if r.w == TILE && r.h == TILE { crop } else { image::imageops::resize(&crop, TILE, TILE, FilterType::Triangle) };
    tile.pixels().flat_map(|p| [p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0]).collect()
}

fn edge_weights(tile: &[f32]) -> Vec<f32> {
    let (w, h) = (TILE as usize, TILE as usize);
    let gray: Vec<f32> = tile.chunks(3).map(|c| (c[0] + c[1] + c[2]) / 3.0).collect();
    let mut e = vec![0f32; w * h];
    for y in 0..h {
        let ny = if y + 1 < h { y + 1 } else { y - 1 };
        for x in 0..w {
            let nx = if x + 1 < w { x + 1 } else { x - 1 };
            let gx = (gray[ny * w + x] - gray[y * w + x]).abs();
            let gy = (gray[y * w + nx] - gray[y * w + x]).abs();
            e[y * w + x] = (gx * gx + gy * gy).sqrt();
        }
    }
    let (mn, mx) = e.iter().fold((f32::MAX, f32::MIN), |(a, b), &v| (a.min(v), b.max(v)));
    let inv = 1.0 / (mx - mn + 1e-8);
    // Expand to one weight per channel value.
    e.iter().flat_map(|v| {
        let w = 0.5 + 0.5 * (v - mn) * inv;
        [w, w, w]
    }).collect()
}

/// Optimiser state for one tile, kept between rounds so auto-iterate can continue a tile
/// exactly where it stopped (same perturbation, momentum, random stream and step schedule).
struct TileState {
    pert: Vec<f32>,
    momentum: Vec<f32>,
    rng: Rng,
    /// Iterations completed so far.
    k: u32,
}

impl TileState {
    fn new(n: usize, seed: u64) -> Self {
        Self { pert: vec![0f32; n], momentum: vec![0f32; n], rng: Rng::new(seed), k: 0 }
    }
    fn image(&self, base: &[f32]) -> Vec<f32> {
        base.iter().zip(&self.pert).map(|(b, d)| (b + d).clamp(0.0, 1.0)).collect()
    }
}

/// Run `iterations` more SPSA steps on a tile. `schedule_iters` is the user's per-tile
/// iteration setting; it fixes the base step size so extra rounds keep Hope:RE's schedule.
#[allow(clippy::too_many_arguments)]
fn spsa_tile(
    session: &mut Session,
    base: &[f32],
    p: &AlgorithmParams,
    hp: &HopeParams,
    state: &mut TileState,
    iterations: u32,
    schedule_iters: u32,
    edges: &[f32],
    on_iter: &dyn Fn(u32),
    cancel: &CancelToken,
) -> Result<(), EngineError> {
    let n = base.len();
    let eps = p.epsilon;
    let alpha = eps * p.alpha_multiplier / schedule_iters.max(1) as f32;
    let ck0 = eps * 0.1;
    let extra = match hp.algorithm {
        HopeAlgorithm::Noise => 0,
        HopeAlgorithm::Style => hp.style_index as i32,
        HopeAlgorithm::Poison => hp.target_index as i32,
    };
    let TileState { pert, momentum, rng, k: k_done } = state;
    let mut plus = vec![0f32; n];
    let mut minus = vec![0f32; n];
    let mut dir = vec![0f32; n];

    for k in *k_done..*k_done + iterations {
        cancel.check()?;
        let ck = ck0 / ((k + 1) as f32).powf(0.101);
        let ak = alpha / ((k + 1) as f32).powf(0.602);
        let mut grad = vec![0f32; n];
        for _ in 0..DIRECTIONS_PER_ITER {
            cancel.check()?;
            for d in dir.iter_mut() {
                *d = if rng.bool() { 1.0 } else { -1.0 };
            }
            for i in 0..n {
                plus[i] = (base[i] + pert[i] + ck * dir[i]).clamp(0.0, 1.0);
                minus[i] = (base[i] + pert[i] - ck * dir[i]).clamp(0.0, 1.0);
            }
            let p_diff = if p.perceptual_weight > 0.0 {
                let s: f32 = (0..n)
                    .map(|i| {
                        let dp = plus[i] - base[i];
                        let dm = minus[i] - base[i];
                        (dp * dp - dm * dm) * (1.5 - edges[i]).clamp(0.5, 1.5)
                    })
                    .sum();
                p.perceptual_weight * s / n as f32 * 100.0
            } else {
                0.0
            };
            let lp = run_model(session, hp.algorithm, extra, &plus)?;
            let lm = run_model(session, hp.algorithm, extra, &minus)?;
            let g = (lp - lm + p_diff) / (2.0 * ck);
            for i in 0..n {
                grad[i] += g * dir[i];
            }
        }
        let scale = 1.0 / DIRECTIONS_PER_ITER as f32;
        for i in 0..n {
            let wg = grad[i] * scale * edges[i];
            momentum[i] = MOMENTUM_BETA * momentum[i] + (1.0 - MOMENTUM_BETA) * wg;
            let s = if momentum[i] > 0.0 { 1.0 } else if momentum[i] < 0.0 { -1.0 } else { 0.0 };
            pert[i] = (pert[i] - ak * s).clamp(-eps, eps);
        }
        on_iter(k + 1);
    }
    *k_done += iterations;
    Ok(())
}

/// Measured effect on the surrogate model: mean loss over all tiles before and after.
#[derive(Clone, Copy, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct LossChange {
    pub before: f32,
    pub after: f32,
    /// Tiles that needed extra rounds (auto-iterate).
    pub tiles_extended: u32,
    /// Tiles whose loss still had not improved when the iteration cap was reached.
    pub tiles_unconverged: u32,
    /// Iterations actually run, summed over all tiles.
    pub total_iterations: u64,
    /// Tiles skipped because they lie entirely outside the selection.
    #[serde(default)]
    pub tiles_skipped: u32,
}

/// A tile counts as improved only if its loss dropped by a small margin, not by noise.
fn improved(before: f32, after: f32) -> bool {
    after < before - 1e-4
}

/// Protect a full-resolution image tile by tile. Alpha is copied from the source unchanged.
pub fn protect(
    img: &RgbaImage,
    session: &mut Session,
    hp: &HopeParams,
    mask: Option<&crate::selection::Mask>,
    progress: ProgressFn,
    cancel: &CancelToken,
) -> Result<(RgbaImage, LossChange), EngineError> {
    hp.validate().map_err(EngineError::InvalidParams)?;
    let ap = hp.algorithm_params();
    let (w, h) = img.dimensions();
    let regions = tile_regions(w, h);
    let total = regions.len() as u32;
    let mut acc = vec![0f32; (w * h * 3) as usize];
    let mut wacc = vec![0f32; (w * h) as usize];
    let iters = hp.iterations;
    let mut loss = LossChange::default();

    let cap = if hp.auto_iterate { hp.max_iterations().max(iters) } else { iters };
    let mask = mask.map(|m| crate::selection::resized(m, w, h));
    let mut optimised = 0u32;
    for (ti, r) in regions.iter().enumerate() {
        cancel.check()?;
        if let Some(m) = &mask {
            let touches = (r.y..r.y + r.h).any(|y| (r.x..r.x + r.w).any(|x| m.get_pixel(x, y)[0] > 0));
            if !touches {
                loss.tiles_skipped += 1;
                continue;
            }
        }
        optimised += 1;
        let base = preprocess_tile(img, r);
        let edges = edge_weights(&base);
        let ti32 = ti as u32;
        let extra = match hp.algorithm {
            HopeAlgorithm::Noise => 0,
            HopeAlgorithm::Style => hp.style_index as i32,
            HopeAlgorithm::Poison => hp.target_index as i32,
        };
        let before = run_model(session, hp.algorithm, extra, &base)?;
        let mut state = TileState::new(base.len(), crate::util::hash_keys(&[hp.seed, ti as u64]));
        let mut tile;
        let mut after;
        let mut round = 0u32;
        loop {
            let run = iters.min(cap - state.k);
            let report = |k: u32| {
                if k.is_multiple_of(2) {
                    let f = (ti32 as f32 + (k.min(iters) as f32 / iters as f32)) / total as f32;
                    let extra_note = if round > 0 { format!(" · auto-iterate round {} (loss not improved yet)", round + 1) } else { String::new() };
                    progress(f * 0.98, &format!("Tile {} / {total} · iteration {k}{extra_note}", ti32 + 1));
                }
            };
            spsa_tile(session, &base, &ap, hp, &mut state, run, iters, &edges, &report, cancel)?;
            tile = state.image(&base);
            after = run_model(session, hp.algorithm, extra, &tile)?;
            if improved(before, after) || state.k >= cap {
                break;
            }
            round += 1;
        }
        if round > 0 {
            loss.tiles_extended += 1;
        }
        if !improved(before, after) {
            loss.tiles_unconverged += 1;
        }
        loss.total_iterations += state.k as u64;
        loss.before += before;
        loss.after += after;
        blend_tile(&tile, r, w, &mut acc, &mut wacc);
    }

    let mut out = img.clone();
    for (i, px) in out.pixels_mut().enumerate() {
        let wt = wacc[i];
        if wt > 0.0 {
            for c in 0..3 {
                px[c] = (acc[i * 3 + c] / wt).clamp(0.0, 255.0) as u8;
            }
        }
    }
    if let Some(m) = &mask {
        crate::selection::composite(img, &mut out, m);
    }
    progress(1.0, "Done");
    loss.before /= optimised.max(1) as f32;
    loss.after /= optimised.max(1) as f32;
    Ok((out, loss))
}

fn blend_tile(tile: &[f32], r: &TileRegion, width: u32, acc: &mut [f32], wacc: &mut [f32]) {
    let ts = TILE as usize;
    let exact = r.w == TILE && r.h == TILE;
    for py in 0..r.h {
        for px in 0..r.w {
            let idx = ((r.y + py) * width + r.x + px) as usize;
            let ex = px.min(r.w - 1 - px).min(OVERLAP) as f32 / OVERLAP as f32;
            let ey = py.min(r.h - 1 - py).min(OVERLAP) as f32 / OVERLAP as f32;
            let wgt = ex.min(ey).max(0.01);
            let color = if exact {
                let i = (py as usize * ts + px as usize) * 3;
                [tile[i], tile[i + 1], tile[i + 2]]
            } else {
                let fx = px as f32 * TILE as f32 / r.w as f32;
                let fy = py as f32 * TILE as f32 / r.h as f32;
                let (x0, y0) = ((fx as usize).min(ts - 1), (fy as usize).min(ts - 1));
                let (x1, y1) = ((x0 + 1).min(ts - 1), (y0 + 1).min(ts - 1));
                let (ax, ay) = (fx - x0 as f32, fy - y0 as f32);
                let s = |x: usize, y: usize, c: usize| tile[(y * ts + x) * 3 + c];
                let mut col = [0f32; 3];
                for (c, v) in col.iter_mut().enumerate() {
                    *v = s(x0, y0, c) * (1.0 - ax) * (1.0 - ay) + s(x1, y0, c) * ax * (1.0 - ay) + s(x0, y1, c) * (1.0 - ax) * ay + s(x1, y1, c) * ax * ay;
                }
                col
            };
            for c in 0..3 {
                acc[idx * 3 + c] += color[c] * 255.0 * wgt;
            }
            wacc[idx] += wgt;
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn tiles_cover_image() {
        for (w, h) in [(100, 80), (224, 224), (1000, 640)] {
            let regions = super::tile_regions(w, h);
            let mut covered = vec![false; (w * h) as usize];
            for r in &regions {
                for y in r.y..r.y + r.h {
                    for x in r.x..r.x + r.w {
                        covered[(y * w + x) as usize] = true;
                    }
                }
            }
            assert!(covered.iter().all(|c| *c), "{w}x{h}");
        }
    }
}
