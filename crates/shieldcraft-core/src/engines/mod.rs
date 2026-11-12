//! Engine registry: descriptive metadata, availability and parameter types.

pub mod gridnoise;
pub mod hope;
pub mod signature;
pub mod worker;

use crate::engine::{Availability, EngineId, EngineInfo, MethodKind, RunLocation};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum EngineParams {
    GridNoise(gridnoise::GridNoiseParams),
    Signature(signature::SignatureParams),
    Hope(hope::HopeParams),
}

impl EngineParams {
    pub fn engine(&self) -> EngineId {
        match self {
            EngineParams::GridNoise(_) => EngineId::GridNoise,
            EngineParams::Signature(_) => EngineId::SignatureInk,
            EngineParams::Hope(h) => match h.algorithm {
                hope::HopeAlgorithm::Noise => EngineId::HopeNoise,
                hope::HopeAlgorithm::Style => EngineId::HopeStyle,
                hope::HopeAlgorithm::Poison => EngineId::HopePoison,
            },
        }
    }
}

const HOPE_LIMITS: &[&str] = &[
    "Optimised against Hope:RE's CLIP ViT-B/32-based loss only; transfer to other AI models is not guaranteed.",
    "Resizing, re-compression (especially JPEG) and denoising can weaken the perturbation. Export as PNG.",
    "Slow on the CPU: every 224 px tile needs iterations × 8 model evaluations.",
];

const POISON_LIMITS: &[&str] = &[
    "Inspired by Nightshade, but it is Hope:RE's own model, not the University of Chicago Nightshade tool, and not equivalent to it.",
    "Concept poisoning only matters in aggregate: a model has to be trained on many affected images of the same concept.",
    "Optimised against one CLIP ViT-B/32-based loss; transfer to the models used by real training pipelines is not guaranteed.",
    "Resizing, re-compression (especially JPEG) and denoising can weaken it. Export as PNG.",
];

const STYLE_LIMITS: &[&str] = &[
    "Inspired by Glaze, but it is Hope:RE's own model, not the University of Chicago Glaze tool, and not equivalent to it.",
    "Optimised against one CLIP ViT-B/32-based loss; transfer to the models used for style mimicry is not guaranteed.",
    "Resizing, re-compression (especially JPEG) and denoising can weaken it. Export as PNG.",
    "Slow on the CPU: every 224 px tile needs iterations × 8 model evaluations.",
];

const GNA_LIMITS: &[&str] = &[
    "The protection is temporal: it only works where the artwork is shown as an animation. A still \
     cannot be averaged by the eye, so still exports get the static layers only.",
    "Someone who captures many frames can average them to recover the artwork.",
    "Video formats compress colour (4:2:0) and soften per-pixel noise.",
    "The grating overlay and tiled text are fixed patterns. ShieldCraft does not measure any effect on AI \
     models; the numbers shown are image-fidelity statistics only.",
];

pub fn info(id: EngineId) -> &'static EngineInfo {
    static INFOS: std::sync::OnceLock<Vec<EngineInfo>> = std::sync::OnceLock::new();
    let all = INFOS.get_or_init(|| {
        vec![
            EngineInfo {
                id: EngineId::GridNoise,
                name: "Grid Noise Studio",
                short: "Grid Noise",
                kind: MethodKind::TemporalNoise,
                location: RunLocation::InProcess,
                origin: "Independent ShieldCraft implementation of the techniques shown by Grid Noise Animator",
                credit: "stelmosfire13-alt (Grid Noise Animator technique)",
                repo_url: "https://github.com/stelmosfire13-alt/Grid-Noise-Animator",
                license: "ShieldCraft code. The original project publishes no license; none of its code is included.",
                summary: "Animated per-cell colour noise that averages back to your artwork for viewers while \
                          scrambling any single frame.",
                geometry: "Animations fit within the chosen maximum side; still exports keep full resolution.",
                alpha: "Transparency preserved (GIF: 1-bit; MP4: none).",
                limitations: GNA_LIMITS,
            },
            EngineInfo {
                id: EngineId::SignatureInk,
                name: "Signature Ink",
                short: "Signature",
                kind: MethodKind::InvisibleSignature,
                location: RunLocation::InProcess,
                origin: "blind_watermark crate, configured to match Hope:RE's Signature Ink",
                credit: "blind_watermark by naganohara-yoshino; configuration from Hope:RE (HopeArtOrg)",
                repo_url: "https://github.com/naganohara-yoshino/blind-watermark-rust",
                license: "MIT OR Apache-2.0 (crate); MIT (Hope:RE)",
                summary: "Hides a short text in the image so you can later demonstrate authorship.",
                geometry: "Full resolution kept.",
                alpha: "Transparency preserved.",
                limitations: &[
                    "Provenance only: it does not prevent training, copying or imitation.",
                    "Reading it back needs the exact text length in bytes (shown after embedding) and the key, if one was used.",
                    "Heavy edits, crops or rescaling can destroy it.",
                ],
            },
            EngineInfo {
                id: EngineId::HopeNoise,
                name: "Hope:RE Noise",
                short: "Hope Noise",
                kind: MethodKind::AdversarialPerturbation,
                location: RunLocation::IsolatedWorker,
                origin: "Rust port of Hope:RE's tiled SPSA optimiser with Hope:RE's noise model",
                credit: "HopeArtOrg (Hope:RE)",
                repo_url: "https://github.com/HopeArtOrg/hope-re",
                license: "MIT (code and model); ONNX Runtime MIT (Microsoft)",
                summary: "Small, bounded noise optimised against a CLIP image model, applied tile by tile at full resolution.",
                geometry: "Full resolution kept (224 px tiles with 32 px overlap).",
                alpha: "Transparency preserved.",
                limitations: HOPE_LIMITS,
            },
            EngineInfo {
                id: EngineId::HopeStyle,
                name: "Hope:RE Style Cloak (Glaze mode)",
                short: "Style Cloak",
                kind: MethodKind::StyleCloak,
                location: RunLocation::IsolatedWorker,
                origin: "Rust port of Hope:RE's tiled SPSA optimiser with Hope:RE's style (\"glaze\") model",
                credit: "HopeArtOrg (Hope:RE)",
                repo_url: "https://github.com/HopeArtOrg/hope-re",
                license: "MIT (code and model); ONNX Runtime MIT (Microsoft)",
                summary: "Small, bounded noise that nudges a CLIP image model's view of the artwork's style toward a chosen decoy style.",
                geometry: "Full resolution kept (224 px tiles with 32 px overlap).",
                alpha: "Transparency preserved.",
                limitations: STYLE_LIMITS,
            },
            EngineInfo {
                id: EngineId::HopePoison,
                name: "Hope:RE Concept Poison (Nightshade mode)",
                short: "Concept Poison",
                kind: MethodKind::ConceptPoisoning,
                location: RunLocation::IsolatedWorker,
                origin: "Rust port of Hope:RE's tiled SPSA optimiser with Hope:RE's concept (\"nightshade\") model",
                credit: "HopeArtOrg (Hope:RE)",
                repo_url: "https://github.com/HopeArtOrg/hope-re",
                license: "MIT (code and model); ONNX Runtime MIT (Microsoft)",
                summary: "Small, bounded noise that makes a CLIP image model read the artwork as a chosen decoy concept, \
                          so models trained on scraped copies learn a wrong association.",
                geometry: "Full resolution kept (224 px tiles with 32 px overlap).",
                alpha: "Transparency preserved.",
                limitations: POISON_LIMITS,
            },
        ]
    });
    all.iter().find(|i| i.id == id).expect("every engine has info")
}

pub fn availability(id: EngineId) -> Availability {
    match id {
        EngineId::GridNoise => Availability::Available { detail: "Built in · CPU, multithreaded".into() },
        EngineId::SignatureInk => Availability::Available { detail: "Built in".into() },
        EngineId::HopeNoise | EngineId::HopeStyle | EngineId::HopePoison => {
            use hope::models::{quick_state, ModelState};
            let spec = match id {
                EngineId::HopeStyle => &hope::models::STYLE,
                EngineId::HopePoison => &hope::models::POISON,
                _ => &hope::models::NOISE,
            };
            if worker::worker_exe().is_none() {
                return Availability::MissingDependencies { detail: "shieldcraft-cli.exe (the isolated worker) is missing next to the application".into() };
            }
            if !worker::runtime_dll().map(|p| p.is_file()).unwrap_or(false) {
                return Availability::MissingDependencies { detail: "onnxruntime.dll is missing next to the application".into() };
            }
            match quick_state(spec) {
                ModelState::Present | ModelState::Verified => Availability::Available { detail: "Model present · runs on the CPU in an isolated worker".into() },
                ModelState::Missing => Availability::ModelDownloadRequired {
                    detail: format!("{} ({}) from the Hope:RE v2.5.1 release on GitHub", spec.file, crate::util::human_bytes(spec.bytes)),
                    bytes: spec.bytes,
                },
                ModelState::Corrupt(why) => Availability::ModelDownloadRequired { detail: format!("Model file is damaged ({why}); download it again"), bytes: spec.bytes },
            }
        }
    }
}
