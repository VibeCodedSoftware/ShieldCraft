//! Editable presets. Presets are application configurations — starting points, not
//! scientifically validated "optimal" settings.

use crate::engines::gridnoise::{Balance, CellShape, GridNoiseParams};
use crate::engines::EngineParams;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Preset {
    pub name: String,
    pub description: String,
    pub builtin: bool,
    pub params: EngineParams,
}

pub fn builtin() -> Vec<Preset> {
    let base = GridNoiseParams::default();
    let mk = |name: &str, desc: &str, f: &dyn Fn(&mut GridNoiseParams)| {
        let mut p = base.clone();
        f(&mut p);
        Preset { name: name.into(), description: desc.into(), builtin: true, params: EngineParams::GridNoise(p) }
    };
    vec![
        mk("Balanced (default)", "Moderate flicker; close to the original tool's defaults.", &|_| {}),
        mk("Subtle shimmer", "Low amplitude, fine grid. Pleasant to watch; frames are less scrambled.", &|p| {
            p.cells = 64;
            p.hue_deg = 8.0;
            p.lightness = 3.0;
            p.saturation = 3.0;
            p.balance = Balance::Antiphase;
        }),
        mk("Strong scramble", "High amplitude for maximum single-frame disruption; visible flicker.", &|p| {
            p.hue_deg = 45.0;
            p.lightness = 14.0;
            p.saturation = 12.0;
            p.warp_px = 5.0;
        }),
        mk("Exact average", "4-frame cycle: the frames sum exactly back to the artwork (before clipping and blur).", &|p| {
            p.balance = Balance::Cycle;
            p.cycle_frames = 4;
            p.blur_enabled = false;
            p.warp_enabled = false;
        }),
        mk("Jigsaw (Voronoi + detail)", "Irregular cells with a separate line-art layer.", &|p| {
            p.shape = CellShape::Voronoi;
            p.detail.enabled = true;
            p.detail.radius = 6.0;
            p.detail.edge_strength = 30.0;
        }),
        mk("Holographic", "Experimental decorative effects: iridescent wave and sparkle.", &|p| {
            p.iridescent.enabled = true;
            p.sparkle.enabled = true;
        }),
        mk("Static layers for stills", "Tiled notice text and visible mark for still images.", &|p| {
            p.text_tile.enabled = true;
            p.visible_mark.enabled = true;
        }),
        Preset {
            name: "Hope Noise · gentle (10)".into(),
            description: "±12/255 budget, 60 iterations per tile. ShieldCraft's default.".into(),
            builtin: true,
            params: EngineParams::Hope(crate::engines::hope::HopeParams::default()),
        },
        Preset {
            name: "Hope Noise · Hope:RE default (30)".into(),
            description: "Hope:RE's own default strength (±37/255) and its 'default' quality (250 iterations). Visible and slow.".into(),
            builtin: true,
            params: EngineParams::Hope(crate::engines::hope::HopeParams { intensity: 0.30, iterations: 250, ..Default::default() }),
        },
        Preset {
            name: "Style Cloak · gentle (10)".into(),
            description: "Decoy style Abstract, ±9/255 budget, 60 iterations per tile. ShieldCraft's default.".into(),
            builtin: true,
            params: EngineParams::Hope(crate::engines::hope::HopeParams::for_algorithm(crate::engines::hope::HopeAlgorithm::Style)),
        },
        Preset {
            name: "Style Cloak · Hope:RE default (30)".into(),
            description: "Hope:RE's own UI defaults: intensity 30 (±28/255) and 'default' quality (300 iterations). Visible and slow.".into(),
            builtin: true,
            params: EngineParams::Hope(crate::engines::hope::HopeParams {
                intensity: 0.30,
                iterations: 300,
                ..crate::engines::hope::HopeParams::for_algorithm(crate::engines::hope::HopeAlgorithm::Style)
            }),
        },
        Preset {
            name: "Concept Poison · recommended".into(),
            description: "Decoy concept Dog, intensity 30, 150 iterations per tile. ShieldCraft's default: in testing this converged with a max change of only 11/255.".into(),
            builtin: true,
            params: EngineParams::Hope(crate::engines::hope::HopeParams::for_algorithm(crate::engines::hope::HopeAlgorithm::Poison)),
        },
        Preset {
            name: "Concept Poison · Hope:RE default (30)".into(),
            description: "Hope:RE's own UI defaults: intensity 30 (±24/255) and 'default' quality (375 iterations). Visible and slow.".into(),
            builtin: true,
            params: EngineParams::Hope(crate::engines::hope::HopeParams {
                intensity: 0.30,
                iterations: 375,
                ..crate::engines::hope::HopeParams::for_algorithm(crate::engines::hope::HopeAlgorithm::Poison)
            }),
        },
        Preset {
            name: "Signature (no key)".into(),
            description: "Invisible authorship text; readable with the text length.".into(),
            builtin: true,
            params: EngineParams::Signature(Default::default()),
        },
    ]
}

pub fn load_user() -> Vec<Preset> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(crate::paths::presets_dir()) {
        for e in rd.flatten() {
            if e.path().extension().and_then(|x| x.to_str()) == Some("json") {
                if let Ok(p) = std::fs::read(e.path()).map_err(|_| ()).and_then(|b| serde_json::from_slice::<Preset>(&b).map_err(|_| ())) {
                    out.push(Preset { builtin: false, ..p });
                }
            }
        }
    }
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    out
}

fn file_for(name: &str) -> std::path::PathBuf {
    crate::paths::presets_dir().join(format!("{}.json", crate::image_io::sanitize_file_stem(name)))
}

pub fn save_user(p: &Preset) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(&Preset { builtin: false, ..p.clone() }).map_err(|e| e.to_string())?;
    crate::image_io::write_atomic(&file_for(&p.name), &bytes, None, false)
}

pub fn delete_user(name: &str) -> Result<(), String> {
    std::fs::remove_file(file_for(name)).map_err(|e| e.to_string())
}
