//! Right-hand property panels.

use shieldcraft_core::engines::gridnoise::{AnimFormat, Balance, CellShape, Corner, GridNoiseParams};
use shieldcraft_core::engines::hope::{HopeAlgorithm, HopeParams, STYLES, TARGETS};
use shieldcraft_core::engines::signature::{self, SignatureParams};
use shieldcraft_core::image_io::{ExportFormat, ExportOptions, MetadataPolicy};
use egui::{RichText, Slider, Ui};
use egui_phosphor::regular as icon;

#[derive(Default)]
pub struct GridNoiseActions {
    pub pick_audio: bool,
    pub clear_regions: bool,
    pub new_seed: bool,
}

fn section(ui: &mut Ui, title: &str, open: bool, body: impl FnOnce(&mut Ui)) {
    egui::CollapsingHeader::new(RichText::new(title).strong()).default_open(open).show(ui, body);
}

fn toggle_slider(ui: &mut Ui, label: &str, on: &mut bool, v: &mut f32, range: std::ops::RangeInclusive<f32>, suffix: &str) {
    ui.horizontal(|ui| {
        ui.checkbox(on, label);
        ui.add_enabled(*on, Slider::new(v, range).suffix(suffix));
    });
}

pub fn gridnoise(ui: &mut Ui, p: &mut GridNoiseParams, show_grid: &mut bool, source_size: Option<(u32, u32)>, ffmpeg: bool) -> GridNoiseActions {
    let mut act = GridNoiseActions::default();
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new("Technique by").small().weak());
        ui.hyperlink_to(RichText::new("stelmosfire13-alt · Grid Noise Animator").small(), "https://github.com/stelmosfire13-alt/Grid-Noise-Animator");
    });
    ui.label(RichText::new("Independent ShieldCraft implementation; no original code included.").small().weak());
    ui.add_space(4.0);

    section(ui, "Grid", true, |ui| {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut p.shape, CellShape::Square, format!("{} Square", icon::GRID_FOUR));
            ui.selectable_value(&mut p.shape, CellShape::Voronoi, format!("{} Voronoi", icon::POLYGON));
        });
        ui.add(Slider::new(&mut p.cells, 4..=256).text("cells (long side)").logarithmic(true));
        if p.shape == CellShape::Voronoi {
            ui.add(Slider::new(&mut p.voronoi_reseed, 0..=120).text("reshape every N frames (0 = never)"));
        }
        ui.checkbox(show_grid, "Show grid overlay on canvas");
    });

    section(ui, "Colour noise", true, |ui| {
        ui.add(Slider::new(&mut p.hue_deg, 0.0..=90.0).text("hue ±").suffix("°"));
        ui.add(Slider::new(&mut p.lightness, 0.0..=25.0).text("lightness ±").suffix("%"));
        ui.add(Slider::new(&mut p.saturation, 0.0..=25.0).text("saturation ±").suffix("%"));
        toggle_slider(ui, "Warp", &mut p.warp_enabled, &mut p.warp_px, 0.0..=20.0, " px");
        toggle_slider(ui, "Blur", &mut p.blur_enabled, &mut p.blur_px, 0.0..=10.0, " px");
    });

    section(ui, "Temporal balance", true, |ui| {
        egui::ComboBox::from_id_salt("balance").selected_text(p.balance.label()).show_ui(ui, |ui| {
            for b in [Balance::Random, Balance::Antiphase, Balance::Cycle] {
                ui.selectable_value(&mut p.balance, b, b.label());
            }
        });
        if p.balance == Balance::Cycle {
            ui.add(Slider::new(&mut p.cycle_frames, 2..=24).text("frames per cycle"));
        }
        ui.label(
            RichText::new(match p.balance {
                Balance::Random => "Independent draws per cell and frame; averages to zero over time.",
                Balance::Antiphase => "Neighbouring cells move in opposite phase; smoother flicker.",
                Balance::Cycle => "Every N frames sum exactly back to the artwork (before clipping, blur and warp).",
            })
            .small()
            .weak(),
        );
        ui.horizontal(|ui| {
            ui.label("Seed");
            ui.add(egui::DragValue::new(&mut p.seed).range(0..=999_999_999));
            if ui.button(icon::DICE_FIVE).on_hover_text("New random seed").clicked() {
                act.new_seed = true;
            }
        });
    });

    section(ui, "Detail split (line layer)", false, |ui| {
        ui.checkbox(&mut p.detail.enabled, "Separate colour and line layers");
        ui.add_enabled_ui(p.detail.enabled, |ui| {
            ui.add(Slider::new(&mut p.detail.radius, 1.0..=40.0).text("split radius").suffix(" px"));
            ui.add(Slider::new(&mut p.detail.edge_strength, 0.0..=100.0).text("line gain ±").suffix("%"));
        });
    });

    section(ui, "Static layers", false, |ui| {
        ui.label(RichText::new("Fixed across frames, so they also appear in still exports.").small().weak());
        ui.checkbox(&mut p.grating.enabled, "High-frequency grating overlay");
        ui.add_enabled_ui(p.grating.enabled, |ui| {
            ui.add(Slider::new(&mut p.grating.amplitude, 0.0..=16.0).text("amplitude"));
            ui.add(Slider::new(&mut p.grating.components, 1..=12).text("components"));
            ui.horizontal(|ui| {
                ui.label("period");
                ui.add(egui::DragValue::new(&mut p.grating.min_period).range(1.0..=20.0).speed(0.1).suffix(" px"));
                ui.label("to");
                ui.add(egui::DragValue::new(&mut p.grating.max_period).range(1.0..=40.0).speed(0.1).suffix(" px"));
            });
        });
        ui.label(RichText::new("No effect on AI models is claimed or measured for this overlay.").small().weak());
        ui.separator();
        ui.checkbox(&mut p.text_tile.enabled, "Tiled low-contrast text");
        ui.add_enabled_ui(p.text_tile.enabled, |ui| {
            ui.add(egui::TextEdit::singleline(&mut p.text_tile.text).char_limit(32));
            ui.add(Slider::new(&mut p.text_tile.contrast, 2.0..=60.0).text("contrast"));
            ui.checkbox(&mut p.text_tile.dark, "Dark text");
        });
        ui.separator();
        ui.checkbox(&mut p.visible_mark.enabled, "Visible mark");
        ui.add_enabled_ui(p.visible_mark.enabled, |ui| {
            ui.add(egui::TextEdit::singleline(&mut p.visible_mark.text).char_limit(48));
            egui::ComboBox::from_id_salt("corner").selected_text(format!("{:?}", p.visible_mark.corner)).show_ui(ui, |ui| {
                for c in [Corner::BottomLeft, Corner::BottomRight, Corner::TopLeft, Corner::TopRight] {
                    ui.selectable_value(&mut p.visible_mark.corner, c, format!("{c:?}"));
                }
            });
            ui.add(Slider::new(&mut p.visible_mark.opacity, 0.0..=1.0).text("box opacity"));
            ui.add(Slider::new(&mut p.visible_mark.size_pct, 1.0..=10.0).text("size").suffix("%"));
        });
    });

    section(ui, "Region (legacy)", false, |ui| {
        ui.label(RichText::new("To limit protection to part of the image, use the selection tools (M, L, B) — they work with every engine.").small().weak());
        ui.label(format!("{} region(s) from older projects.", p.region.polygons.len()));
        ui.horizontal(|ui| {
            ui.radio_value(&mut p.region.apply_inside, true, "Noise inside");
            ui.radio_value(&mut p.region.apply_inside, false, "Noise outside");
        });
        if ui.add_enabled(!p.region.polygons.is_empty(), egui::Button::new(format!("{} Clear regions", icon::ERASER))).clicked() {
            act.clear_regions = true;
        }
    });

    section(ui, "Experimental effects", false, |ui| {
        ui.label(RichText::new("Decorative; carried over from the original tool's test features.").small().weak());
        toggle_slider(ui, "Iridescent wave", &mut p.iridescent.enabled, &mut p.iridescent.strength, 0.0..=100.0, "%");
        toggle_slider(ui, "Sparkle", &mut p.sparkle.enabled, &mut p.sparkle.strength, 0.0..=100.0, "%");
        toggle_slider(ui, "Cell rainbow", &mut p.cell_rainbow.enabled, &mut p.cell_rainbow.strength, 0.0..=100.0, "%");
    });

    section(ui, "Animation output", true, |ui| {
        egui::ComboBox::from_id_salt("animfmt").selected_text(p.animation.format.label()).width(220.0).show_ui(ui, |ui| {
            for f in AnimFormat::ALL {
                ui.add_enabled_ui(!f.needs_ffmpeg() || ffmpeg, |ui| {
                    ui.selectable_value(&mut p.animation.format, f, f.label());
                });
            }
        });
        if !ffmpeg {
            ui.label(RichText::new("MP4/WebM need FFmpeg (set its path in Preferences).").small().color(crate::theme::WARN));
        }
        ui.add(Slider::new(&mut p.animation.fps, 1..=60).text("fps"));
        ui.add(Slider::new(&mut p.animation.duration_s, 0.5..=60.0).text("seconds").logarithmic(true));
        ui.add(Slider::new(&mut p.animation.max_side, 128..=4096).text("max side").suffix(" px").logarithmic(true));
        if p.animation.format == AnimFormat::Mp4 {
            ui.add(Slider::new(&mut p.animation.mp4_crf, 0..=40).text("quality (CRF, lower = better)"));
        }
        ui.checkbox(&mut p.preserve_alpha, "Keep transparency");
        if p.animation.format.needs_ffmpeg() {
            ui.horizontal(|ui| {
                let name = p.animation.audio_path.as_deref().map(|a| std::path::Path::new(a).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default());
                ui.label(format!("Audio: {}", name.as_deref().unwrap_or("none")));
                if ui.small_button(icon::MUSIC_NOTES).on_hover_text("Choose audio file").clicked() {
                    act.pick_audio = true;
                }
                if p.animation.audio_path.is_some() && ui.small_button(icon::X).clicked() {
                    p.animation.audio_path = None;
                }
            });
        }
        if let Some((w, h)) = source_size {
            let (ow, oh) = shieldcraft_core::engines::gridnoise::output_size(w, h, p.animation.max_side);
            let frames = p.frame_count();
            let raw = ow as u64 * oh as u64 * 4 * frames as u64;
            ui.label(RichText::new(format!("{ow}×{oh} px · {frames} frames · {} raw", shieldcraft_core::util::human_bytes(raw))).small().weak());
        }
    });
    act
}

pub struct SignatureUi {
    pub use_key: bool,
    pub verify_len: usize,
    pub verify_key: u64,
    pub verify_use_key: bool,
    pub verify_result: Option<Result<String, String>>,
}

impl Default for SignatureUi {
    fn default() -> Self {
        Self { use_key: false, verify_len: 16, verify_key: 0, verify_use_key: false, verify_result: None }
    }
}

/// Returns true when the user asked to verify the image currently shown.
pub fn signature(ui: &mut Ui, p: &mut SignatureParams, s: &mut SignatureUi, size: Option<(u32, u32)>) -> bool {
    ui.label(RichText::new("Provenance only — proves authorship later; does not prevent training or copying.").small().color(crate::theme::WARN));
    crate::theme::caption(ui, "Embed");
    ui.add(egui::TextEdit::singleline(&mut p.text).hint_text("Signature text"));
    let bytes = p.text.len();
    ui.horizontal(|ui| {
        if ui.checkbox(&mut s.use_key, "Key").changed() {
            p.seed = if s.use_key { Some(p.seed.unwrap_or(shieldcraft_core::util::fresh_seed())) } else { None };
        }
        if let Some(k) = p.seed.as_mut() {
            ui.add(egui::DragValue::new(k));
        }
    });
    ui.label(RichText::new(format!("Length: {bytes} bytes — note this number; it is needed to read the signature back.")).small());
    if let Some((w, h)) = size {
        let cap = signature::capacity_bits(w, h) / 8;
        let col = if bytes > cap { crate::theme::BAD } else { ui.visuals().weak_text_color() };
        ui.label(RichText::new(format!("Approximate capacity for this image: {cap} bytes")).small().color(col));
    }
    ui.add_space(6.0);
    crate::theme::caption(ui, "Verify");
    ui.horizontal(|ui| {
        ui.label("Byte length");
        ui.add(egui::DragValue::new(&mut s.verify_len).range(1..=4096));
        if ui.small_button("Use length above").clicked() {
            s.verify_len = bytes.max(1);
            s.verify_use_key = p.seed.is_some();
            s.verify_key = p.seed.unwrap_or(0);
        }
    });
    ui.horizontal(|ui| {
        ui.checkbox(&mut s.verify_use_key, "Key");
        ui.add_enabled(s.verify_use_key, egui::DragValue::new(&mut s.verify_key));
    });
    let clicked = ui.button(format!("{} Read signature from shown image", icon::SEAL_CHECK)).clicked();
    match &s.verify_result {
        Some(Ok(t)) => {
            ui.label(RichText::new(format!("Found: “{t}”")).color(crate::theme::OK));
        }
        Some(Err(e)) => {
            ui.label(RichText::new(e).color(crate::theme::BAD));
        }
        None => {}
    }
    clicked
}

pub enum HopeAction {
    None,
    Run,
    NewSeed,
}

/// Instant, approximate preview shown while adjusting Hope:RE settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HopePreview {
    /// Show the real result (or the original if there is none yet).
    Off,
    /// Random noise at the selected strength, edge-weighted like Hope:RE.
    Simulated,
    /// Magenta tint showing where and how strongly the image will change.
    Overlay,
}

pub fn hope_preview_controls(ui: &mut Ui, preview: &mut HopePreview, has_result: bool) {
    crate::theme::caption(ui, "Live preview");
    ui.horizontal(|ui| {
        ui.selectable_value(preview, HopePreview::Simulated, "Simulated");
        ui.selectable_value(preview, HopePreview::Overlay, "Pattern overlay");
        ui.selectable_value(preview, HopePreview::Off, if has_result { "Real result" } else { "Off" });
    });
    let note = match preview {
        HopePreview::Simulated => "Random noise at exactly this strength, shaped by edges like Hope:RE's. The real run optimises the pattern, but its strength is the same. Zoom in to see the grain.",
        HopePreview::Overlay => "Magenta shows where and how strongly the image will change at this strength. Use the Difference view for an amplified pattern.",
        HopePreview::Off => if has_result { "Showing the real optimised result." } else { "No result yet: showing the original." },
    };
    ui.label(RichText::new(note).small().weak());
}

/// Seconds per model evaluation measured on an 8-core laptop CPU; used only for rough time estimates.
/// Used only for the rough time estimate shown before running.
const SECONDS_PER_EVAL: f32 = 0.048;

pub fn hope(ui: &mut Ui, p: &mut HopeParams, size: Option<(u32, u32)>, ready: bool, busy: bool) -> HopeAction {
    let mut act = HopeAction::None;
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new("Method and model by").small().weak());
        ui.hyperlink_to(RichText::new("HopeArtOrg · Hope:RE").small(), "https://github.com/HopeArtOrg/hope-re");
        ui.label(RichText::new("(MIT)").small().weak());
    });
    ui.add_space(4.0);
    if p.algorithm == HopeAlgorithm::Style {
        crate::theme::caption(ui, "Decoy style");
        let mut idx = p.style_index as usize;
        egui::ComboBox::from_id_salt("decoy_style").selected_text(STYLES[idx]).show_ui(ui, |ui| {
            for (i, s) in STYLES.iter().enumerate() {
                ui.selectable_value(&mut idx, i, *s);
            }
        });
        p.style_index = idx as u32;
        ui.label(RichText::new("The noise nudges the model's reading of your style toward this decoy. Pick one far from your real style.").small().weak());
    }
    if p.algorithm == HopeAlgorithm::Poison {
        crate::theme::caption(ui, "Decoy concept");
        let mut idx = p.target_index as usize;
        egui::ComboBox::from_id_salt("decoy_concept").selected_text(TARGETS[idx]).show_ui(ui, |ui| {
            for (i, s) in TARGETS.iter().enumerate() {
                ui.selectable_value(&mut idx, i, *s);
            }
        });
        p.target_index = idx as u32;
        ui.label(RichText::new("The noise makes the model read your artwork as this concept. Pick one unrelated to what the artwork shows. It only has an effect when many images are affected.").small().weak());
    }
    crate::theme::caption(ui, "Strength");
    let mut slider = (p.intensity * 100.0).round() as u32;
    if ui.add(Slider::new(&mut slider, 1..=50).text("intensity")).changed() {
        p.intensity = slider as f32 / 100.0;
    }
    let hope_default = HopeParams { intensity: 0.30, ..p.clone() }.budget_255();
    ui.label(RichText::new(format!("Maximum change per colour channel: ±{:.0}/255. Hope:RE's own default is 30 (±{hope_default:.0}/255).", p.budget_255())).small().weak());
    crate::theme::caption(ui, "Quality");
    ui.add(Slider::new(&mut p.iterations, 5..=500).text("iterations per tile").logarithmic(true));
    ui.checkbox(&mut p.auto_iterate, "Auto-iterate until it works")
        .on_hover_text("After each tile's iterations, the model's loss is measured. If it has not improved, that tile keeps optimising in further rounds, up to the cap below.");
    if p.auto_iterate {
        ui.horizontal(|ui| {
            ui.add(Slider::new(&mut p.auto_max_factor, 2..=10).prefix("cap ").suffix("×"));
            ui.label(RichText::new(format!("(≤ {} iterations per tile)", p.max_iterations())).small().weak());
        });
    }
    if p.algorithm == HopeAlgorithm::Poison && p.iterations < 150 && !p.auto_iterate {
        ui.label(RichText::new("Concept Poison needs about 150 iterations: in testing, runs with 60 or fewer often failed to improve the surrogate loss. The result is checked and flagged either way.").small().color(crate::theme::WARN));
    }
    ui.horizontal(|ui| {
        ui.label("Seed");
        ui.add(egui::DragValue::new(&mut p.seed));
        if ui.button(icon::DICE_FIVE).on_hover_text("New random seed").clicked() {
            act = HopeAction::NewSeed;
        }
    });
    if let Some((w, h)) = size {
        let tiles = HopeParams::tile_count(w, h);
        let evals = p.evaluations(w, h);
        // Plus ~10 s to start the worker and load the 335 MB model (measured).
        let secs = evals as f32 * SECONDS_PER_EVAL + 10.0;
        if p.auto_iterate {
            let worst = evals as f32 * (p.max_iterations() as f32 / p.iterations.max(1) as f32) * SECONDS_PER_EVAL + 10.0;
            ui.label(RichText::new(format!(
                "{tiles} tiles · roughly {} on this CPU, up to {} if every tile needs extra rounds",
                shieldcraft_core::util::human_duration(secs as f64),
                shieldcraft_core::util::human_duration(worst as f64)
            )).small());
        } else {
            ui.label(RichText::new(format!("{tiles} tiles · {evals} model evaluations · roughly {} on this CPU", shieldcraft_core::util::human_duration(secs as f64))).small());
        }
        if secs > 1800.0 {
            ui.label(RichText::new("This will take a long time. Lower the iterations or work on a smaller copy.").small().color(crate::theme::WARN));
        }
    }
    ui.add_space(6.0);
    let label = if busy { format!("{} Running…", icon::SPINNER) } else { format!("{} Protect & Save…", icon::SHIELD_CHECK) };
    if ui.add_enabled(ready && !busy && size.is_some(), egui::Button::new(RichText::new(label).strong()).fill(crate::theme::ACCENT)).clicked() {
        act = HopeAction::Run;
    }
    ui.label(RichText::new("Runs in a separate worker process on the CPU. You can keep working; progress and Cancel are in the Queue. The result opens here when done. Save as PNG: JPEG compression weakens the noise.").small().weak());
    act
}

pub fn export_options(ui: &mut Ui, o: &mut ExportOptions) {
    egui::ComboBox::from_id_salt("expfmt").selected_text(o.format.label()).width(200.0).show_ui(ui, |ui| {
        for f in ExportFormat::ALL {
            ui.selectable_value(&mut o.format, f, f.label());
        }
    });
    if o.format == ExportFormat::Jpeg {
        ui.add(Slider::new(&mut o.jpeg_quality, 50..=100).text("JPEG quality"));
    }
    egui::ComboBox::from_id_salt("meta").selected_text(o.metadata.label()).width(200.0).show_ui(ui, |ui| {
        for m in [MetadataPolicy::IccOnly, MetadataPolicy::StripAll, MetadataPolicy::IccAndExif] {
            ui.selectable_value(&mut o.metadata, m, m.label());
        }
    });
    if o.metadata == MetadataPolicy::IccAndExif {
        ui.label(RichText::new("EXIF can include camera, date and GPS location.").small().color(crate::theme::WARN));
    }
    ui.horizontal(|ui| {
        ui.label("Filename suffix");
        ui.add(egui::TextEdit::singleline(&mut o.suffix).desired_width(110.0));
    });
    ui.checkbox(&mut o.write_report, "Write processing report (.report.json)");
    ui.checkbox(&mut o.side_by_side, "Also save side-by-side comparison");
    ui.checkbox(&mut o.allow_overwrite, "Allow replacing existing output files");
    ui.label(RichText::new("The original source file can never be overwritten.").small().weak());
}
