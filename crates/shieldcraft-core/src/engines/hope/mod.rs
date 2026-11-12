//! Hope:RE protection engines (Noise, Style Cloak, Concept Poison) and Signature Ink.
//!
//! The tiling and SPSA optimiser below are a Rust port of code from Hope:RE
//! (https://github.com/HopeArtOrg/hope-re), Copyright (c) 2025 HopeArtOrg, used under the MIT
//! License (full text in licenses/Hope-RE-MIT.txt). Changes for ShieldCraft: seeded and
//! reproducible random directions, progress/cancellation through ShieldCraft's job system, no
//! silent fallback when a model is missing, ONNX models verified against pinned SHA-256 hashes.
//!
//! The ONNX models are Hope:RE's own CLIP ViT-B/32-based losses. They are *inspired by* Glaze
//! and Nightshade but are not the University of Chicago tools.

pub mod models;
pub mod spsa;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HopeAlgorithm {
    Noise,
    Style,
    Poison,
}

impl HopeAlgorithm {
    pub fn model(self) -> &'static models::ModelSpec {
        match self {
            HopeAlgorithm::Noise => &models::NOISE,
            HopeAlgorithm::Style => &models::STYLE,
            HopeAlgorithm::Poison => &models::POISON,
        }
    }
}

pub const STYLES: [&str; 5] = ["Abstract", "Impressionist", "Cubist", "Sketch", "Watercolor"];
pub const TARGETS: [&str; 8] = ["Dog", "Cat", "Car", "Landscape", "Person", "Building", "Food", "Abstract"];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExecutionDevice {
    Cpu,
    /// DirectML. Hope:RE disabled it after access violations with these models; ShieldCraft
    /// offers it only as an experiment, inside the isolated worker process.
    DirectMlExperimental,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HopeParams {
    pub algorithm: HopeAlgorithm,
    /// Intensity 0..=1 (Hope:RE's slider value / 100). Maps to the L∞ budget.
    pub intensity: f32,
    /// SPSA iterations per 224 px tile. Hope:RE's maximum is 500/600/750 for the three modes.
    pub iterations: u32,
    pub style_index: u32,
    pub target_index: u32,
    pub seed: u64,
    pub device: ExecutionDevice,
    pub threads: u32,
    /// Keep optimising a tile in further rounds until its measured loss improves.
    pub auto_iterate: bool,
    /// Cap for auto-iterate, as a multiple of `iterations` (per tile).
    pub auto_max_factor: u32,
}

impl Default for HopeParams {
    fn default() -> Self {
        Self {
            algorithm: HopeAlgorithm::Noise,
            // Hope:RE's slider runs 1–50 (default 30); ShieldCraft defaults to a gentler 10,
            // which is an L∞ budget of about ±12/255.
            intensity: 0.10,
            iterations: 60,
            style_index: 0,
            target_index: 0,
            seed: 7,
            device: ExecutionDevice::Cpu,
            threads: 0,
            auto_iterate: true,
            auto_max_factor: 4,
        }
    }
}

impl HopeParams {
    pub fn for_algorithm(a: HopeAlgorithm) -> Self {
        match a {
            // Measured on a 256 px test image (decoy Dog): 60 iterations at intensity 10 did not
            // improve the surrogate loss; 150 iterations at intensity 30 reduced it from 0.0146 to
            // 0.0035 with a max change of only 11/255 (PSNR 41.8 dB).
            HopeAlgorithm::Poison => Self { algorithm: a, intensity: 0.30, iterations: 150, ..Default::default() },
            _ => Self { algorithm: a, ..Default::default() },
        }
    }

    /// Most iterations a tile may get when auto-iterate keeps extending it.
    pub fn max_iterations(&self) -> u32 {
        (self.iterations.saturating_mul(self.auto_max_factor.clamp(1, 10))).min(2000)
    }

    /// Maximum per-channel pixel change, in 0..255 units.
    pub fn budget_255(&self) -> f32 {
        self.algorithm_params().epsilon * 255.0
    }

    pub fn validate(&self) -> Result<(), String> {
        if !(0.0..=1.0).contains(&self.intensity) || !self.intensity.is_finite() {
            return Err("Intensity must be between 0 and 100 %".into());
        }
        if !(1..=2000).contains(&self.iterations) {
            return Err("Iterations must be between 1 and 2000".into());
        }
        if self.style_index as usize >= STYLES.len() || self.target_index as usize >= TARGETS.len() {
            return Err("Unknown style or target".into());
        }
        Ok(())
    }

    /// Hope:RE's parameter mapping (algorithms.rs), unchanged.
    pub fn algorithm_params(&self) -> spsa::AlgorithmParams {
        match self.algorithm {
            HopeAlgorithm::Noise => spsa::AlgorithmParams {
                epsilon: self.intensity * 0.24 / 0.5,
                alpha_multiplier: 5.0,
                perceptual_weight: 0.15,
            },
            HopeAlgorithm::Style => spsa::AlgorithmParams {
                epsilon: self.intensity * 0.18 / 0.5,
                alpha_multiplier: 4.5,
                perceptual_weight: 0.3,
            },
            HopeAlgorithm::Poison => spsa::AlgorithmParams {
                epsilon: self.intensity * 0.16 / 0.5,
                alpha_multiplier: 5.0,
                perceptual_weight: 0.4,
            },
        }
    }

    pub fn tile_count(width: u32, height: u32) -> u32 {
        spsa::tile_regions(width, height).len() as u32
    }

    /// Model evaluations needed: tiles × iterations × directions × 2.
    pub fn evaluations(&self, width: u32, height: u32) -> u64 {
        Self::tile_count(width, height) as u64 * self.iterations as u64 * spsa::DIRECTIONS_PER_ITER as u64 * 2
    }
}
