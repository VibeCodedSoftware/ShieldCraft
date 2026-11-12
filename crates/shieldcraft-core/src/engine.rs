//! The engine contract shared by every protection method.

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
pub enum EngineId {
    GridNoise,
    HopeNoise,
    HopeStyle,
    HopePoison,
    SignatureInk,
}

impl EngineId {
    pub const ALL: [EngineId; 5] = [
        EngineId::GridNoise,
        EngineId::HopeNoise,
        EngineId::HopeStyle,
        EngineId::HopePoison,
        EngineId::SignatureInk,
    ];
    pub fn info(self) -> &'static EngineInfo {
        crate::engines::info(self)
    }
}

/// What kind of protection a method provides. These are deliberately kept separate so the UI
/// never presents, say, a watermark as if it were adversarial protection.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MethodKind {
    TemporalNoise,
    AdversarialPerturbation,
    StyleCloak,
    ConceptPoisoning,
    InvisibleSignature,
    VisibleWatermark,
}

impl MethodKind {
    pub fn label(self) -> &'static str {
        match self {
            MethodKind::TemporalNoise => "Procedural / temporal noise",
            MethodKind::AdversarialPerturbation => "Adversarial perturbation",
            MethodKind::StyleCloak => "Style protection (cloaking)",
            MethodKind::ConceptPoisoning => "Concept poisoning",
            MethodKind::InvisibleSignature => "Invisible signature (provenance)",
            MethodKind::VisibleWatermark => "Visible watermark",
        }
    }
    pub fn explanation(self) -> &'static str {
        match self {
            MethodKind::TemporalNoise => {
                "Animates the artwork with zero-mean colour noise per grid cell. A viewer's eye averages the \
                 frames back into the picture, while any single frame (a screenshot or a scraped poster \
                 frame) is visibly scrambled. It does not alter how a model learns from a clean copy."
            }
            MethodKind::AdversarialPerturbation => {
                "Optimises a small, bounded pixel change against a specific surrogate model so that model's \
                 features or predictions move. Transfer to other models, resizing, re-compression and \
                 purification is not guaranteed."
            }
            MethodKind::StyleCloak => {
                "An adversarial perturbation aimed at shifting the apparent artistic style seen by a feature \
                 extractor, so style-mimicry fine-tuning learns a different style. Effectiveness depends on \
                 the model and training pipeline."
            }
            MethodKind::ConceptPoisoning => {
                "An adversarial perturbation that makes the image's features resemble a different concept, so \
                 a model trained on many such images may associate prompts with the wrong concept. It only \
                 matters in aggregate and against matching model families."
            }
            MethodKind::InvisibleSignature => {
                "Hides a short text in the image's frequency domain so you can later show the file came from \
                 you. It does not prevent training, copying or imitation."
            }
            MethodKind::VisibleWatermark => {
                "Draws visible text or marks on the image. It signals ownership and intent but can be cropped \
                 or painted out, and does not prevent training."
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunLocation {
    /// Runs inside the ShieldCraft process.
    InProcess,
    /// Runs in a separate ShieldCraft worker process (crash isolation).
    IsolatedWorker,
}

pub struct EngineInfo {
    pub id: EngineId,
    pub name: &'static str,
    pub short: &'static str,
    pub kind: MethodKind,
    pub location: RunLocation,
    pub origin: &'static str,
    pub credit: &'static str,
    pub repo_url: &'static str,
    pub license: &'static str,
    pub summary: &'static str,
    pub geometry: &'static str,
    pub alpha: &'static str,
    pub limitations: &'static [&'static str],
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Availability {
    Checking,
    Available { detail: String },
    MissingDependencies { detail: String },
    ModelDownloadRequired { detail: String, bytes: u64 },
    UnsupportedHardware { detail: String },
    OptionalInstallRequired { detail: String },
    LicensingReviewRequired { detail: String },
    IntegrationIncomplete { detail: String },
}

impl Availability {
    pub fn is_available(&self) -> bool {
        matches!(self, Availability::Available { .. })
    }
    pub fn label(&self) -> &'static str {
        match self {
            Availability::Checking => "Checking…",
            Availability::Available { .. } => "Available",
            Availability::MissingDependencies { .. } => "Missing dependencies",
            Availability::ModelDownloadRequired { .. } => "Model download required",
            Availability::UnsupportedHardware { .. } => "Unsupported hardware",
            Availability::OptionalInstallRequired { .. } => "Optional installation required",
            Availability::LicensingReviewRequired { .. } => "Licensing review required",
            Availability::IntegrationIncomplete { .. } => "Integration incomplete",
        }
    }
    pub fn detail(&self) -> &str {
        match self {
            Availability::Checking => "",
            Availability::Available { detail }
            | Availability::MissingDependencies { detail }
            | Availability::ModelDownloadRequired { detail, .. }
            | Availability::UnsupportedHardware { detail }
            | Availability::OptionalInstallRequired { detail }
            | Availability::LicensingReviewRequired { detail }
            | Availability::IntegrationIncomplete { detail } => detail,
        }
    }
}

/// Cooperative cancellation shared between the UI and a running job.
#[derive(Clone, Default, Debug)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst)
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
    pub fn check(&self) -> Result<(), EngineError> {
        if self.is_cancelled() { Err(EngineError::Cancelled) } else { Ok(()) }
    }
}

/// Progress callback: fraction in 0..=1 and a short status message.
pub type ProgressFn<'a> = &'a (dyn Fn(f32, &str) + Sync);

#[derive(Debug, Clone, PartialEq)]
pub enum EngineError {
    Cancelled,
    InvalidParams(String),
    Unavailable(String),
    OutOfMemory(String),
    Failed(String),
}

impl std::fmt::Display for EngineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EngineError::Cancelled => f.write_str("Cancelled by user"),
            EngineError::InvalidParams(s) => write!(f, "Invalid settings: {s}"),
            EngineError::Unavailable(s) => write!(f, "Engine unavailable: {s}"),
            EngineError::OutOfMemory(s) => write!(f, "Out of memory: {s}"),
            EngineError::Failed(s) => f.write_str(s),
        }
    }
}

impl std::error::Error for EngineError {}

/// Map common low-level failure messages onto friendlier errors.
pub fn translate_error(raw: &str) -> EngineError {
    let l = raw.to_ascii_lowercase();
    if l.contains("out of memory") || l.contains("cuda oom") || l.contains("bad_alloc") || l.contains("failed to allocate") {
        EngineError::OutOfMemory(format!(
            "{raw}\n\nTry a smaller output size, the low-VRAM/block option, or close other GPU applications."
        ))
    } else if l.contains("cancel") {
        EngineError::Cancelled
    } else {
        EngineError::Failed(raw.to_string())
    }
}
