//! ShieldCraft application shell.

use crate::canvas::{self, CanvasState, Tool, ViewMode};
use crate::licenses_window::LicensesWindow;
use crate::panels::{self, SignatureUi};
use crate::preview::{PreviewKind, PreviewRenderer, Request};
use crate::tasks::{self, BatchMode, Shared, Status, Work};
use crate::theme;
use shieldcraft_core::engine::{Availability, EngineId};
use shieldcraft_core::engines::gridnoise::{self, AnimFormat, Analysis, Balance, GridNoiseParams, Panel, StillMode};
use shieldcraft_core::engines::hope::{self as hope_engine, HopeParams};
use shieldcraft_core::selection::{self, Mask};
use shieldcraft_core::engines::signature::{self, SignatureParams};
use shieldcraft_core::engines::{self, EngineParams};
use shieldcraft_core::hardware::Hardware;
use shieldcraft_core::image_io::{self, SourceImage};
use shieldcraft_core::presets::{self, Preset};
use shieldcraft_core::settings::{Settings, Theme};
use shieldcraft_core::util;
use crossbeam_channel::{unbounded, Receiver, Sender};
use egui::{Color32, RichText};
use egui_phosphor::regular as icon;
use image::RgbaImage;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

type Snapshot = (GridNoiseParams, SignatureParams, HopeParams, HopeParams, HopeParams, SelRef);

/// A document's selection. Compared by version (not pixels) so undo tracking stays cheap.
#[derive(Clone, Default)]
struct SelRef {
    mask: Option<Arc<Mask>>,
    version: u64,
}

impl PartialEq for SelRef {
    fn eq(&self, other: &Self) -> bool {
        self.version == other.version
    }
}

static SEL_VERSION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn next_sel_version() -> u64 {
    SEL_VERSION.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

fn default_style_params() -> HopeParams {
    HopeParams::for_algorithm(hope_engine::HopeAlgorithm::Style)
}

fn default_poison_params() -> HopeParams {
    HopeParams::for_algorithm(hope_engine::HopeAlgorithm::Poison)
}

fn is_hope(e: EngineId) -> bool {
    matches!(e, EngineId::HopeNoise | EngineId::HopeStyle | EngineId::HopePoison)
}

pub struct Document {
    id: u64,
    source: Arc<SourceImage>,
    preview_src: Arc<RgbaImage>,
    preview_scale: f32,
    gn: GridNoiseParams,
    sig: SignatureParams,
    hope: HopeParams,
    style: HopeParams,
    poison: HopeParams,
    selection: SelRef,
    /// (version, feather bits) → feathered full-resolution mask for processing.
    sel_full: Option<(u64, u32, Arc<Mask>)>,
    /// (version, feather bits) → feathered mask at preview size.
    sel_preview: Option<(u64, u32, Arc<Mask>)>,
    /// (version, settings key) → tinted overlay texture.
    sel_overlay: Option<(u64, u64, egui::TextureHandle)>,
    /// (version, percent selected), so the status bar doesn't rescan the mask every frame.
    sel_pct: std::cell::Cell<(u64, f32)>,
    still: Option<Arc<RgbaImage>>,
    hope_still: Option<Arc<RgbaImage>>,
    style_still: Option<Arc<RgbaImage>>,
    poison_still: Option<Arc<RgbaImage>>,
    /// (texture, result pointer, gain key, max channel change, PSNR dB)
    tex_diff: Option<(egui::TextureHandle, usize, u32, u8, f64)>,
    /// Key of the simulated Hope:RE preview currently uploaded.
    sim_key: Option<u64>,
    /// Latest Grid Noise preview frame (or frame average) at preview size, for the Effect Monitor.
    last_frame: Option<(Arc<RgbaImage>, PreviewKind)>,
    /// Simulated Hope:RE image at preview size, keyed by settings, for the Effect Monitor.
    sim_img: Option<(u64, Arc<RgbaImage>)>,
    tex_orig: Option<egui::TextureHandle>,
    tex_result: Option<egui::TextureHandle>,
    result_key: Option<(u64, u32, PreviewKind)>,
    analysis: Option<Analysis>,
    committed: Snapshot,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    in_storyboard: bool,
    panel_duration: f32,
}

impl Document {
    fn new(source: SourceImage, preview_max: u32, id: u64) -> Self {
        let gn = GridNoiseParams::default();
        let sig = SignatureParams::default();
        let hope = HopeParams::default();
        let style = default_style_params();
        let poison = default_poison_params();
        let mut d = Document {
            id,
            preview_src: Arc::new(RgbaImage::new(1, 1)),
            preview_scale: 1.0,
            source: Arc::new(source),
            committed: (gn.clone(), sig.clone(), hope.clone(), style.clone(), poison.clone(), SelRef::default()),
            gn,
            sig,
            hope,
            style,
            poison,
            selection: SelRef::default(),
            sel_full: None,
            sel_preview: None,
            sel_overlay: None,
            sel_pct: std::cell::Cell::new((0, 0.0)),
            still: None,
            hope_still: None,
            style_still: None,
            poison_still: None,
            tex_diff: None,
            sim_key: None,
            last_frame: None,
            sim_img: None,
            tex_orig: None,
            tex_result: None,
            result_key: None,
            analysis: None,
            undo: vec![],
            redo: vec![],
            in_storyboard: false,
            panel_duration: 2.0,
        };
        d.rebuild_preview_source(preview_max);
        d
    }

    fn rebuild_preview_source(&mut self, preview_max: u32) {
        let (ow, _) = gridnoise::output_size(self.source.width(), self.source.height(), self.gn.animation.max_side);
        let target = preview_max.min(self.source.width().max(self.source.height()));
        let img = image_io::fit_within(&self.source.pixels, target);
        let (_, _) = (img.width(), img.height());
        let out_long = ow.max(gridnoise::output_size(self.source.width(), self.source.height(), self.gn.animation.max_side).1);
        self.preview_scale = img.width().max(img.height()) as f32 / out_long.max(1) as f32;
        self.preview_src = Arc::new(img);
        self.result_key = None;
    }

    fn name(&self) -> &str {
        &self.source.name
    }

    fn snapshot(&self) -> Snapshot {
        (self.gn.clone(), self.sig.clone(), self.hope.clone(), self.style.clone(), self.poison.clone(), self.selection.clone())
    }

    fn restore(&mut self, s: Snapshot) {
        (self.gn, self.sig, self.hope, self.style, self.poison, self.selection) = s.clone();
        self.committed = s;
        self.result_key = None;
    }

    fn has_selection(&self) -> bool {
        self.selection.mask.is_some()
    }

    fn selected_pct(&self) -> Option<f32> {
        let m = self.selection.mask.as_ref()?;
        let (v, pct) = self.sel_pct.get();
        if v == self.selection.version {
            return Some(pct);
        }
        let pct = selection::selected_fraction(m) * 100.0;
        self.sel_pct.set((self.selection.version, pct));
        Some(pct)
    }

    /// Apply one edit from the canvas to the selection.
    fn edit_selection(&mut self, edit: &crate::canvas::SelEdit, brush_radius: f32) {
        use crate::canvas::SelEdit;
        let (w, h) = (self.source.width(), self.source.height());
        // Edit in place when nothing else (undo history, a running job) shares the mask; brush
        // strokes arrive every frame and copying a large mask each time would stall the UI.
        let mut arc = self.selection.mask.take().unwrap_or_else(|| Arc::new(Mask::new(w, h)));
        let mask = Arc::make_mut(&mut arc);
        let painting = matches!(edit, SelEdit::Brush { erase: false, .. });
        match edit {
            SelEdit::Rect(a, b, op) => selection::rect(mask, *a, *b, *op),
            SelEdit::Ellipse(a, b, op) => selection::ellipse(mask, *a, *b, *op),
            SelEdit::Polygon(pts, op) => selection::polygon(mask, pts, *op),
            SelEdit::Brush { from, to, erase, .. } => selection::brush(mask, *from, *to, brush_radius, *erase),
        }
        let keep = painting || !selection::is_empty(mask);
        self.selection = SelRef { mask: keep.then_some(arc), version: next_sel_version() };
        self.result_key = None;
    }

    /// Tinted texture covering the unselected area (cached per selection version and style).
    fn selection_overlay(&mut self, ctx: &egui::Context, color: [u8; 3], opacity: f32) -> Option<egui::TextureHandle> {
        let m = self.selection.mask.clone()?;
        let style = u64::from_le_bytes([color[0], color[1], color[2], 0, 0, 0, 0, 0]) ^ ((opacity.to_bits() as u64) << 24);
        if let Some((v, s, tex)) = &self.sel_overlay {
            if *v == self.selection.version && *s == style {
                return Some(tex.clone());
            }
        }
        let (w, h) = m.dimensions();
        let scale = (1024.0 / w.max(h) as f32).min(1.0);
        let (ow, oh) = (((w as f32 * scale).round() as u32).max(1), ((h as f32 * scale).round() as u32).max(1));
        let small = if (ow, oh) == (w, h) { (*m).clone() } else { selection::resized(&m, ow, oh) };
        let a = opacity.clamp(0.0, 1.0);
        let mut rgba = Vec::with_capacity((ow * oh * 4) as usize);
        for p in small.pixels() {
            rgba.extend_from_slice(&[color[0], color[1], color[2], (a * (255 - p.0[0]) as f32).round() as u8]);
        }
        let img = egui::ColorImage::from_rgba_unmultiplied([ow as usize, oh as usize], &rgba);
        let tex = ctx.load_texture(format!("sel-{}", self.id), img, egui::TextureOptions::LINEAR);
        self.sel_overlay = Some((self.selection.version, style, tex.clone()));
        Some(tex)
    }

    fn set_selection(&mut self, mask: Option<Mask>) {
        self.selection = SelRef { mask: mask.map(Arc::new), version: next_sel_version() };
        self.result_key = None;
    }

    /// Feathered, full-resolution mask for processing (cached).
    fn processing_mask(&mut self, feather: f32) -> Option<Arc<Mask>> {
        let m = self.selection.mask.clone()?;
        let key = (self.selection.version, feather.to_bits());
        if let Some((v, f, cached)) = &self.sel_full {
            if (*v, *f) == key {
                return Some(cached.clone());
            }
        }
        let out = Arc::new(selection::feather(&m, feather));
        self.sel_full = Some((key.0, key.1, out.clone()));
        Some(out)
    }

    /// Feathered mask at preview size (cached).
    fn preview_mask(&mut self, feather: f32) -> Option<Arc<Mask>> {
        let m = self.selection.mask.clone()?;
        let key = (self.selection.version, feather.to_bits());
        if let Some((v, f, cached)) = &self.sel_preview {
            if (*v, *f) == key && cached.dimensions() == self.preview_src.dimensions() {
                return Some(cached.clone());
            }
        }
        let (pw, ph) = self.preview_src.dimensions();
        let scale = pw as f32 / self.source.width().max(1) as f32;
        let small = selection::resized(&m, pw, ph);
        let out = Arc::new(selection::feather(&small, feather * scale));
        self.sel_preview = Some((key.0, key.1, out.clone()));
        Some(out)
    }

    fn hope_params(&self, e: EngineId) -> &HopeParams {
        match e {
            EngineId::HopeStyle => &self.style,
            EngineId::HopePoison => &self.poison,
            _ => &self.hope,
        }
    }

    fn hope_params_mut(&mut self, e: EngineId) -> &mut HopeParams {
        match e {
            EngineId::HopeStyle => &mut self.style,
            EngineId::HopePoison => &mut self.poison,
            _ => &mut self.hope,
        }
    }

    fn set_hope_result(&mut self, e: EngineId, img: Option<Arc<RgbaImage>>) {
        match e {
            EngineId::HopeStyle => self.style_still = img,
            EngineId::HopePoison => self.poison_still = img,
            _ => self.hope_still = img,
        }
    }

    /// The still result currently relevant for an engine (Signature, Hope Noise, Style Cloak).
    fn still_for(&self, e: EngineId) -> Option<Arc<RgbaImage>> {
        match e {
            EngineId::SignatureInk => self.still.clone(),
            EngineId::HopeNoise => self.hope_still.clone(),
            EngineId::HopeStyle => self.style_still.clone(),
            EngineId::HopePoison => self.poison_still.clone(),
            _ => None,
        }
    }
}

#[derive(Serialize, Deserialize)]
struct ProjectFile {
    format: String,
    version: u32,
    active: usize,
    engine: EngineId,
    documents: Vec<ProjectDoc>,
}

#[derive(Serialize, Deserialize)]
struct ProjectDoc {
    path: String,
    grid_noise: GridNoiseParams,
    signature: SignatureParams,
    #[serde(default)]
    hope_noise: HopeParams,
    #[serde(default = "default_style_params")]
    hope_style: HopeParams,
    #[serde(default = "default_poison_params")]
    hope_poison: HopeParams,
    in_storyboard: bool,
    panel_duration: f32,
}

enum InitMsg {
    Step { name: &'static str, ok: bool, detail: String },
    Done { hardware: Hardware, ffmpeg: Option<PathBuf>, user_presets: Vec<Preset> },
}

struct Welcome {
    steps: Vec<(&'static str, bool, String)>,
    rx: Option<Receiver<InitMsg>>,
    done: bool,
    dont_show: bool,
    started: Instant,
}

#[derive(PartialEq, Clone, Copy)]
enum BottomTab {
    Queue,
    History,
    Log,
}

#[derive(PartialEq, Clone, Copy)]
enum RightTab {
    Properties,
    Presets,
    Export,
    Engines,
    Provenance,
}

pub struct App {
    settings: Settings,
    welcome: Option<Welcome>,
    welcome_modal: bool,
    main_layout_applied: bool,
    docs: Vec<Document>,
    active: usize,
    next_doc: u64,
    engine: EngineId,
    canvas: CanvasState,
    view: ViewMode,
    tool: Tool,
    preview: PreviewRenderer,
    playing: bool,
    frame: u32,
    last_tick: Instant,
    preview_ms: f32,
    still_tx: Sender<(u64, Result<RgbaImage, String>)>,
    still_rx: Receiver<(u64, Result<RgbaImage, String>)>,
    still_busy: bool,
    tasks: Vec<Shared>,
    log: Vec<(Instant, Color32, String)>,
    licenses: LicensesWindow,
    show_about: bool,
    show_prefs: bool,
    show_batch: bool,
    show_info: bool,
    show_left: bool,
    show_right: bool,
    show_bottom: bool,
    bottom_tab: BottomTab,
    right_tab: RightTab,
    hardware: Hardware,
    ffmpeg: Option<PathBuf>,
    builtin_presets: Vec<Preset>,
    user_presets: Vec<Preset>,
    preset_name: String,
    sig_ui: SignatureUi,
    project_path: Option<PathBuf>,
    project_dirty: bool,
    hover: Option<(u32, u32)>,
    batch_files: Vec<PathBuf>,
    batch_mode: BatchMode,
    batch_out: Option<PathBuf>,
    title_cache: String,
    epoch: Instant,
    smoke: Option<Smoke>,
    /// Hope:RE runs whose results should open in the canvas when they finish.
    hope_runs: Vec<(Shared, u64, EngineId)>,
    hope_preview: panels::HopePreview,
    show_monitor: bool,
    show_sel_overlay: bool,
    show_shortcuts: bool,
    monitor: Option<crate::monitor::Monitor>,
    lantern: crate::lantern_panel::LanternUi,
    /// Last Hope settings seen, to switch the live preview on when a slider moves.
    hope_watch: Option<(u64, EngineId, u32, u64, u32)>,
    confirm_download: Option<EngineId>,
    download: Option<ModelDownload>,
}

struct ModelDownload {
    state: Arc<std::sync::Mutex<(u64, u64, Option<Result<(), String>>)>>,
    cancel: shieldcraft_core::engine::CancelToken,
    engine: EngineId,
}

/// Automated smoke test driven by environment variables (used for verification builds):
/// SHIELDCRAFT_SMOKE=<screenshot.png>, SHIELDCRAFT_SMOKE_OPEN=<image>, SHIELDCRAFT_SMOKE_VIEW=split|average|side|original,
/// SHIELDCRAFT_SMOKE_STAGE=welcome|main|licenses|signature.
struct Smoke {
    out: PathBuf,
    stage: String,
    frames: u32,
    requested: bool,
}

impl App {
    pub fn new(cc: &eframe::CreationContext, settings: Settings, settings_warning: Option<String>) -> Self {
        theme::install_fonts(&cc.egui_ctx);
        theme::apply(&cc.egui_ctx, settings.theme, settings.ui_scale);
        let (still_tx, still_rx) = unbounded();
        let welcome = settings.show_welcome.then(|| Welcome { steps: vec![], rx: Some(start_init(cc.egui_ctx.clone())), done: false, dont_show: false, started: Instant::now() });
        let mut app = App {
            welcome_modal: false,
            main_layout_applied: !settings.show_welcome,
            welcome,
            docs: vec![],
            active: 0,
            next_doc: 1,
            engine: EngineId::GridNoise,
            canvas: CanvasState::default(),
            view: ViewMode::Result,
            tool: Tool::Hand,
            preview: PreviewRenderer::start(cc.egui_ctx.clone()),
            playing: false,
            frame: 0,
            last_tick: Instant::now(),
            preview_ms: 0.0,
            still_tx,
            still_rx,
            still_busy: false,
            tasks: vec![],
            log: vec![],
            licenses: LicensesWindow::default(),
            show_about: false,
            show_prefs: false,
            show_batch: false,
            show_info: false,
            show_left: true,
            show_right: true,
            show_bottom: true,
            bottom_tab: BottomTab::Queue,
            right_tab: RightTab::Properties,
            hardware: Hardware::default(),
            ffmpeg: None,
            builtin_presets: presets::builtin(),
            user_presets: vec![],
            preset_name: "My preset".into(),
            sig_ui: SignatureUi::default(),
            project_path: None,
            project_dirty: false,
            hover: None,
            batch_files: vec![],
            batch_mode: BatchMode::Animation,
            batch_out: None,
            title_cache: String::new(),
            epoch: Instant::now(),
            smoke: std::env::var("SHIELDCRAFT_SMOKE").ok().map(|o| Smoke {
                out: PathBuf::from(o),
                stage: std::env::var("SHIELDCRAFT_SMOKE_STAGE").unwrap_or_else(|_| "main".into()),
                frames: 0,
                requested: false,
            }),
            hope_runs: vec![],
            hope_preview: panels::HopePreview::Simulated,
            show_monitor: true,
            show_sel_overlay: true,
            show_shortcuts: false,
            monitor: None,
            lantern: Default::default(),
            hope_watch: None,
            confirm_download: None,
            download: None,
            settings,
        };
        if app.welcome.is_none() {
            // No welcome screen: run the same initialisation, synchronously (it takes milliseconds).
            app.hardware = shieldcraft_core::hardware::detect();
            app.ffmpeg = gridnoise::find_ffmpeg(app.settings.ffmpeg_path.as_deref().map(Path::new));
            app.user_presets = presets::load_user();
        }
        if let Some(w) = settings_warning {
            app.warn(w);
        }
        app.info(format!("ShieldCraft {} ready. Data folder: {}", shieldcraft_core::APP_VERSION, shieldcraft_core::paths::data_root().display()));
        app
    }

    // ───────────────────────── logging ─────────────────────────
    fn info(&mut self, m: impl Into<String>) {
        self.log.push((Instant::now(), Color32::GRAY, m.into()));
    }
    fn warn(&mut self, m: impl Into<String>) {
        self.log.push((Instant::now(), theme::WARN, m.into()));
    }
    fn error(&mut self, m: impl Into<String>) {
        let m = m.into();
        self.log.push((Instant::now(), theme::BAD, m.clone()));
        self.bottom_tab = BottomTab::Log;
        self.show_bottom = true;
    }

    fn doc(&self) -> Option<&Document> {
        self.docs.get(self.active)
    }
    fn doc_mut(&mut self) -> Option<&mut Document> {
        self.docs.get_mut(self.active)
    }

    // ───────────────────────── files ─────────────────────────
    fn import_dialog(&mut self) {
        let mut d = rfd::FileDialog::new().add_filter("Images", image_io::SUPPORTED_IMPORT_EXT).add_filter("All files", &["*"]);
        if let Some(dir) = &self.settings.last_open_dir {
            d = d.set_directory(dir);
        }
        if let Some(files) = d.pick_files() {
            for f in files {
                self.open_path(&f);
            }
        }
    }

    fn open_path(&mut self, path: &Path) {
        if let Some(i) = self.docs.iter().position(|d| d.source.path.as_deref() == Some(path)) {
            self.active = i;
            return;
        }
        match image_io::load_image(path) {
            Ok(src) => {
                for t in &src.info.transformations {
                    self.warn(format!("{}: {t}", src.name));
                }
                if src.info.uses_transparency {
                    self.info(format!("{}: transparency detected and will be preserved where the format allows", src.name));
                }
                self.info(format!("Imported {} ({}×{}, {}, {})", src.name, src.width(), src.height(), src.info.format, util::human_bytes(src.info.file_size)));
                let doc = Document::new(src, self.settings.preview_max_side, self.next_doc);
                self.next_doc += 1;
                self.docs.push(doc);
                self.active = self.docs.len() - 1;
                self.canvas.fit_pending = true;
                self.frame = 0;
                self.project_dirty = true;
                let p = path.display().to_string();
                self.settings.push_recent(&p);
                self.settings.last_open_dir = path.parent().map(|p| p.display().to_string());
                let _ = self.settings.save();
            }
            Err(e) => self.error(format!("Could not import {}: {e}", path.display())),
        }
    }

    fn close_active(&mut self) {
        if self.active < self.docs.len() {
            self.docs.remove(self.active);
            self.active = self.active.min(self.docs.len().saturating_sub(1));
            self.canvas.fit_pending = true;
            self.project_dirty = true;
        }
    }

    fn output_dir_default(&self, doc: &Document) -> Option<PathBuf> {
        self.settings.last_output_dir.as_ref().map(PathBuf::from).or_else(|| doc.source.path.as_ref().and_then(|p| p.parent().map(Path::to_path_buf)))
    }

    fn save_dialog(&self, doc: &Document, ext: &str, filter: &str, suffix: &str) -> Option<PathBuf> {
        let stem = Path::new(&doc.source.name).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "artwork".into());
        let mut d = rfd::FileDialog::new().set_file_name(format!("{stem}{suffix}.{ext}")).add_filter(filter, &[ext]);
        if let Some(dir) = self.output_dir_default(doc) {
            d = d.set_directory(dir);
        }
        d.save_file()
    }

    fn remember_out_dir(&mut self, p: &Path) {
        self.settings.last_output_dir = p.parent().map(|d| d.display().to_string());
        let _ = self.settings.save();
    }

    fn export_animation(&mut self, ctx: &egui::Context) {
        let feather = self.settings.selection_feather;
        let masks: Vec<Option<Arc<Mask>>> = self.docs.iter_mut().map(|d| d.processing_mask(feather)).collect();
        let Some(doc) = self.doc() else { return };
        let p = doc.gn.clone();
        if let Err(e) = p.validate() {
            self.error(e);
            return;
        }
        if p.animation.format.needs_ffmpeg() && self.ffmpeg.is_none() {
            self.error("MP4/WebM export needs FFmpeg. Set its path in Edit › Preferences or choose GIF/APNG.");
            return;
        }
        let out = if p.animation.format == AnimFormat::PngSequence {
            let mut d = rfd::FileDialog::new();
            if let Some(dir) = self.output_dir_default(doc) {
                d = d.set_directory(dir);
            }
            d.pick_folder().map(|f| {
                let stem = Path::new(&doc.source.name).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                image_io::plan_output_path(&f, &stem, "_frames", "", false).with_extension("")
            })
        } else {
            self.save_dialog(doc, p.animation.format.ext(), "Animation", &self.settings.export.suffix)
        };
        let Some(out) = out else { return };
        if Some(out.as_path()) == doc.source.path.as_deref() {
            self.error("Refusing to overwrite the original source file.");
            return;
        }
        // Storyboard: every document marked "in storyboard", in tab order; otherwise just this one.
        let story: Vec<usize> = (0..self.docs.len()).filter(|&i| self.docs[i].in_storyboard).collect();
        let panels: Vec<Panel> = if story.len() >= 2 {
            story.iter().map(|&i| Panel { image: self.docs[i].source.pixels.clone(), duration_s: self.docs[i].panel_duration, mask: masks[i].clone() }).collect()
        } else {
            vec![Panel { image: doc.source.pixels.clone(), duration_s: p.animation.duration_s, mask: masks[self.active].clone() }]
        };
        let title = if panels.len() > 1 { format!("Storyboard animation ({} panels)", panels.len()) } else { format!("Animation · {}", doc.name()) };
        let work = Work::Animation {
            source_name: doc.source.name.clone(),
            source_size: (doc.source.width(), doc.source.height()),
            panels,
            params: p,
            out: out.clone(),
            ffmpeg: self.ffmpeg.clone(),
            write_report: self.settings.export.write_report,
        };
        self.remember_out_dir(&out);
        self.tasks.push(tasks::spawn(title, EngineId::GridNoise, work, ctx.clone()));
        self.bottom_tab = BottomTab::Queue;
        self.show_bottom = true;
    }

    fn export_still(&mut self, ctx: &egui::Context, mode: StillMode) {
        if is_hope(self.engine) {
            // Hope:RE runs take minutes, so running and saving are one step.
            if engines::availability(self.engine).is_available() {
                self.run_hope(ctx, self.engine);
            } else {
                self.warn(format!("{} needs its model first: see the Properties panel.", engines::info(self.engine).name));
            }
            return;
        }
        let feather = self.settings.selection_feather;
        let engine = self.engine;
        // Signature Ink always covers the whole image: a partial signature could not be read back.
        let mask = match self.doc_mut() {
            Some(d) if engine != EngineId::SignatureInk => d.processing_mask(feather),
            _ => None,
        };
        let Some(doc) = self.doc() else { return };
        let params = match self.engine {
            EngineId::SignatureInk => EngineParams::Signature(doc.sig.clone()),
            _ => EngineParams::GridNoise(doc.gn.clone()),
        };
        let opts = self.settings.export.clone();
        let Some(out) = self.save_dialog(doc, opts.format.ext(), "Image", &opts.suffix) else { return };
        if Some(out.as_path()) == doc.source.path.as_deref() {
            self.error("Refusing to overwrite the original source file.");
            return;
        }
        let title = format!("{} · {}", engines::info(params.engine()).short, doc.name());
        let work = Work::Still { source: doc.source.clone(), params: params.clone(), mode, out: out.clone(), opts, mask };
        self.remember_out_dir(&out);
        self.tasks.push(tasks::spawn(title, params.engine(), work, ctx.clone()));
        self.bottom_tab = BottomTab::Queue;
        self.show_bottom = true;
    }

    fn run_hope(&mut self, ctx: &egui::Context, engine: EngineId) {
        let mut opts = self.settings.export.clone();
        if opts.format == image_io::ExportFormat::Jpeg {
            opts.format = image_io::ExportFormat::Png;
            self.warn("Hope:RE results are saved as PNG: JPEG compression weakens adversarial noise.");
        }
        let suffix = match engine {
            EngineId::HopeStyle => "_stylecloak",
            EngineId::HopePoison => "_conceptpoison",
            _ => "_hope",
        };
        let Some(doc) = self.doc() else { return };
        let Some(out) = self.save_dialog(doc, opts.format.ext(), "Image", suffix) else { return };
        self.run_hope_to(ctx, engine, out, opts);
    }

    fn run_hope_to(&mut self, ctx: &egui::Context, engine: EngineId, out: PathBuf, opts: image_io::ExportOptions) {
        let feather = self.settings.selection_feather;
        let mask = self.doc_mut().and_then(|d| d.processing_mask(feather));
        let Some(doc) = self.doc() else { return };
        if Some(out.as_path()) == doc.source.path.as_deref() {
            self.error("Refusing to overwrite the original source file.");
            return;
        }
        let params = EngineParams::Hope(doc.hope_params(engine).clone());
        let work = Work::Still { source: doc.source.clone(), params, mode: StillMode::StaticLayers, out: out.clone(), opts, mask };
        let title = format!("{} · {}", engines::info(engine).short, doc.name());
        let id = doc.id;
        self.remember_out_dir(&out);
        let task = tasks::spawn(title, engine, work, ctx.clone());
        self.hope_runs.push((task.clone(), id, engine));
        self.tasks.push(task);
        self.bottom_tab = BottomTab::Queue;
        self.show_bottom = true;
    }

    fn poll_hope_runs(&mut self) {
        let mut finished = Vec::new();
        self.hope_runs.retain(|(t, doc, engine)| {
            let t = t.lock().unwrap();
            if t.is_running() {
                return true;
            }
            finished.push((t.items.first().cloned(), *doc, *engine));
            false
        });
        for (item, doc_id, engine) in finished {
            let Some(item) = item else { continue };
            let name = engines::info(engine).name;
            match (&item.status, &item.output) {
                (Status::Done, Some(out)) => match image::open(out) {
                    Ok(img) => {
                        if let Some(d) = self.docs.iter_mut().find(|d| d.id == doc_id) {
                            d.set_hope_result(engine, Some(Arc::new(img.to_rgba8())));
                            d.result_key = None;
                        }
                        let report = out.with_file_name(format!("{}.report.json", out.file_name().unwrap_or_default().to_string_lossy()));
                        let note = std::fs::read(&report)
                            .ok()
                            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                            .and_then(|v| v.get("execution").and_then(|e| e.as_str()).map(String::from))
                            .unwrap_or_default();
                        if note.contains("WARNING") {
                            self.error(format!("{name} finished but did NOT improve the surrogate loss: {} — {note}", out.display()));
                        } else {
                            self.info(format!("{name} finished: {} — {note}", out.display()));
                        }
                        // The change is meant to be invisible; open the amplified Difference view so
                        // it is obvious what was changed. Split/Original are one click away.
                        self.view = ViewMode::Difference;
                        self.hope_preview = panels::HopePreview::Off;
                    }
                    Err(e) => self.error(format!("Result saved but could not be previewed: {e}")),
                },
                (Status::Failed(e), _) => self.error(format!("{name} failed: {e}")),
                _ => {}
            }
        }
    }

    /// Feed the Effect Monitor with the (original, modified) pair relevant to the current engine.
    fn update_monitor(&mut self, ctx: &egui::Context) {
        if !self.show_monitor || !self.show_right {
            return;
        }
        let engine = self.engine;
        let frame = self.frame;
        let preview_on = is_hope(engine) && self.hope_preview != panels::HopePreview::Off;
        let short = engines::info(engine).short;
        let feather = self.settings.selection_feather;
        let Some(d) = self.docs.get_mut(self.active) else {
            self.monitor = None;
            return;
        };
        let pair: Option<(Arc<RgbaImage>, Arc<RgbaImage>, String)> = if engine == EngineId::GridNoise {
            d.last_frame.as_ref().filter(|(img, _)| img.dimensions() == d.preview_src.dimensions()).map(|(img, kind)| {
                let label = match kind {
                    PreviewKind::Frame => format!("{short} · frame {} · preview size", frame + 1),
                    PreviewKind::Average(n) => format!("{short} · average of {n} frames · preview size"),
                };
                (d.preview_src.clone(), img.clone(), label)
            })
        } else if preview_on {
            let p = d.hope_params(engine).clone();
            let key = util::hash_keys(&[d.id, engine as u64, p.budget_255().to_bits() as u64, p.seed, d.selection.version, feather.to_bits() as u64]);
            if d.sim_img.as_ref().map(|s| s.0) != Some(key) {
                let m = d.preview_mask(feather);
                let (img, _, _) = simulate_hope(&d.preview_src, p.budget_255(), p.seed, SimOut::Image, 1.0, m.as_deref());
                d.sim_img = Some((key, Arc::new(img)));
            }
            let sim = d.sim_img.as_ref().unwrap().1.clone();
            Some((d.preview_src.clone(), sim, format!("{short} · simulated at ±{:.0}/255 · preview size", p.budget_255())))
        } else {
            d.still_for(engine).map(|still| (d.source.pixels.clone(), still, format!("{short} · real result · full size")))
        };
        let Some((orig, modified, label)) = pair else {
            self.monitor = None;
            return;
        };
        let key = (Arc::as_ptr(&orig) as usize, Arc::as_ptr(&modified) as usize);
        if self.monitor.as_ref().map(|m| m.key) != Some(key) {
            self.monitor = crate::monitor::analyse(ctx, key, &label, &orig, &modified);
        }
    }

    fn model_spec(engine: EngineId) -> &'static hope_engine::models::ModelSpec {
        match engine {
            EngineId::HopeStyle => &hope_engine::models::STYLE,
            EngineId::HopePoison => &hope_engine::models::POISON,
            _ => &hope_engine::models::NOISE,
        }
    }

    fn start_model_download(&mut self, ctx: &egui::Context, engine: EngineId) {
        let spec = Self::model_spec(engine);
        let state = Arc::new(std::sync::Mutex::new((0u64, spec.bytes, None)));
        let cancel = shieldcraft_core::engine::CancelToken::new();
        let (st, c, ctx2) = (state.clone(), cancel.clone(), ctx.clone());
        std::thread::spawn(move || {
            let r = hope_engine::models::download(
                spec,
                &|done, total| {
                    let mut s = st.lock().unwrap();
                    s.0 = done;
                    s.1 = total;
                    ctx2.request_repaint();
                },
                &c,
            );
            st.lock().unwrap().2 = Some(r);
            ctx2.request_repaint();
        });
        self.download = Some(ModelDownload { state, cancel, engine });
    }

    fn download_window(&mut self, ctx: &egui::Context) {
        if let Some(engine) = self.confirm_download {
            let spec = Self::model_spec(engine);
            let mut choice = None;
            egui::Modal::new(egui::Id::new("confirm_dl")).show(ctx, |ui| {
                ui.set_max_width(460.0);
                ui.heading(format!("Download the {} model?", engines::info(engine).name));
                ui.add_space(4.0);
                egui::Grid::new("dl").num_columns(2).show(ui, |ui| {
                    ui.label(RichText::new("File").weak());
                    ui.label(spec.file);
                    ui.end_row();
                    ui.label(RichText::new("Size").weak());
                    ui.label(util::human_bytes(spec.bytes));
                    ui.end_row();
                    ui.label(RichText::new("From").weak());
                    ui.label(RichText::new(spec.url).small());
                    ui.end_row();
                    ui.label(RichText::new("License").weak());
                    ui.label("MIT (Hope:RE, HopeArtOrg); contains OpenAI CLIP weights (MIT)");
                    ui.end_row();
                    ui.label(RichText::new("Saved to").weak());
                    ui.label(RichText::new(hope_engine::models::model_path(spec).display().to_string()).small());
                    ui.end_row();
                });
                ui.label(RichText::new("The file is checked against a pinned SHA-256 before use. Nothing else is sent or uploaded.").small().weak());
                ui.horizontal(|ui| {
                    if ui.button(RichText::new("Download").strong()).clicked() {
                        choice = Some(true);
                    }
                    if ui.button("Cancel").clicked() {
                        choice = Some(false);
                    }
                });
            });
            match choice {
                Some(true) => {
                    self.confirm_download = None;
                    self.start_model_download(ctx, engine);
                }
                Some(false) => self.confirm_download = None,
                None => {}
            }
        }
        let mut finished = None;
        if let Some(dl) = &self.download {
            let (done, total, result) = dl.state.lock().unwrap().clone();
            if let Some(r) = result {
                finished = Some((r, dl.engine));
            } else {
                egui::Window::new("Downloading model").collapsible(false).resizable(false).show(ctx, |ui| {
                    ui.add(egui::ProgressBar::new(done as f32 / total.max(1) as f32).show_percentage().desired_width(320.0));
                    ui.label(format!("{} of {}", util::human_bytes(done), util::human_bytes(total)));
                    if ui.button("Cancel").clicked() {
                        dl.cancel.cancel();
                    }
                });
            }
        }
        if let Some((r, engine)) = finished {
            self.download = None;
            match r {
                Ok(()) => self.info(format!("{} model downloaded and verified.", engines::info(engine).name)),
                Err(e) => self.error(format!("Model download failed: {e}")),
            }
        }
    }

    fn apply_signature_preview(&mut self) {
        let Some(doc) = self.doc() else { return };
        let (id, src, p) = (doc.id, doc.source.clone(), doc.sig.clone());
        let tx = self.still_tx.clone();
        self.still_busy = true;
        std::thread::spawn(move || {
            let _ = tx.send((id, signature::embed(&src.pixels, &p)));
        });
    }

    fn transform_image(&mut self, op: &str) {
        let Some(doc) = self.doc_mut() else { return };
        let px = &*doc.source.pixels;
        let img = match op {
            "cw" => image::imageops::rotate90(px),
            "ccw" => image::imageops::rotate270(px),
            "180" => image::imageops::rotate180(px),
            "h" => image::imageops::flip_horizontal(px),
            _ => image::imageops::flip_vertical(px),
        };
        let mut src = (*doc.source).clone();
        src.pixels = Arc::new(img);
        src.info.width = src.pixels.width();
        src.info.height = src.pixels.height();
        src.info.transformations.push(format!("Working copy transformed in ShieldCraft ({op}); the file on disk is unchanged"));
        doc.source = Arc::new(src);
        doc.gn.region.polygons.clear();
        doc.set_selection(None);
        doc.tex_orig = None;
        doc.still = None;
        let max = self.settings.preview_max_side;
        self.doc_mut().unwrap().rebuild_preview_source(max);
        self.canvas.fit_pending = true;
        self.info("Image transformed (working copy only). The selection was cleared.");
    }

    // ───────────────────────── projects ─────────────────────────
    fn save_project(&mut self, as_new: bool) {
        let path = if as_new || self.project_path.is_none() {
            rfd::FileDialog::new().add_filter("ShieldCraft project", &["shieldcraft"]).set_file_name("project.shieldcraft").save_file()
        } else {
            self.project_path.clone()
        };
        let Some(path) = path else { return };
        let docs: Vec<ProjectDoc> = self
            .docs
            .iter()
            .filter_map(|d| {
                d.source.path.as_ref().map(|p| ProjectDoc {
                    path: p.display().to_string(),
                    grid_noise: d.gn.clone(),
                    signature: d.sig.clone(),
                    hope_noise: d.hope.clone(),
                    hope_style: d.style.clone(),
                    hope_poison: d.poison.clone(),
                    in_storyboard: d.in_storyboard,
                    panel_duration: d.panel_duration,
                })
            })
            .collect();
        let pf = ProjectFile { format: "shieldcraft-project".into(), version: 1, active: self.active, engine: self.engine, documents: docs };
        match serde_json::to_vec_pretty(&pf).map_err(|e| e.to_string()).and_then(|b| image_io::write_atomic(&path, &b, None, false)) {
            Ok(()) => {
                self.info(format!("Project saved to {}", path.display()));
                self.project_path = Some(path);
                self.project_dirty = false;
            }
            Err(e) => self.error(format!("Could not save project: {e}")),
        }
    }

    fn open_project(&mut self) {
        let Some(path) = rfd::FileDialog::new().add_filter("ShieldCraft project", &["shieldcraft"]).pick_file() else { return };
        let pf: ProjectFile = match std::fs::read(&path).map_err(|e| e.to_string()).and_then(|b| serde_json::from_slice(&b).map_err(|e| e.to_string())) {
            Ok(p) => p,
            Err(e) => {
                self.error(format!("Could not open project: {e}"));
                return;
            }
        };
        self.docs.clear();
        for pd in pf.documents {
            let before = self.docs.len();
            self.open_path(Path::new(&pd.path));
            if self.docs.len() > before {
                let max = self.settings.preview_max_side;
                let d = self.docs.last_mut().unwrap();
                d.gn = pd.grid_noise;
                d.sig = pd.signature;
                d.hope = pd.hope_noise;
                d.style = pd.hope_style;
                d.poison = pd.hope_poison;
                d.committed = d.snapshot();
                d.in_storyboard = pd.in_storyboard;
                d.panel_duration = pd.panel_duration;
                d.rebuild_preview_source(max);
            } else {
                self.warn(format!("Project image missing: {}", pd.path));
            }
        }
        self.active = pf.active.min(self.docs.len().saturating_sub(1));
        self.engine = pf.engine;
        self.project_path = Some(path);
        self.project_dirty = false;
    }

    // ───────────────────────── undo ─────────────────────────
    fn track_undo(&mut self, ctx: &egui::Context) {
        let pointer_down = ctx.input(|i| i.pointer.any_down());
        let max = self.settings.preview_max_side;
        if let Some(d) = self.docs.get_mut(self.active) {
            let now = d.snapshot();
            if now != d.committed && !pointer_down {
                let prev = std::mem::replace(&mut d.committed, now);
                if prev.0.animation.max_side != d.gn.animation.max_side {
                    d.rebuild_preview_source(max);
                }
                d.undo.push(prev);
                if d.undo.len() > 200 {
                    d.undo.remove(0);
                }
                // Selection masks are full resolution; cap the memory they hold in the history.
                const MASK_BUDGET: usize = 512 << 20;
                while d.undo.len() > 1 && undo_mask_bytes(&d.undo) > MASK_BUDGET {
                    d.undo.remove(0);
                }
                d.redo.clear();
                self.project_dirty = true;
            }
        }
    }

    fn undo(&mut self) {
        if let Some(d) = self.doc_mut() {
            if let Some(prev) = d.undo.pop() {
                d.redo.push(d.snapshot());
                d.restore(prev);
            }
        }
    }
    fn redo(&mut self) {
        if let Some(d) = self.doc_mut() {
            if let Some(next) = d.redo.pop() {
                d.undo.push(d.snapshot());
                d.restore(next);
            }
        }
    }

    // ───────────────────────── preview ─────────────────────────
    fn pump_preview(&mut self, ctx: &egui::Context) {
        while let Ok(r) = self.preview.rx.try_recv() {
            self.preview.busy = false;
            self.preview_ms = r.millis;
            if let Some(d) = self.docs.iter_mut().find(|d| d.id == r.doc) {
                let ci = egui::ColorImage::from_rgba_unmultiplied([r.image.width() as usize, r.image.height() as usize], r.image.as_raw());
                match &mut d.tex_result {
                    Some(t) => t.set(ci, tex_opts()),
                    None => d.tex_result = Some(ctx.load_texture(format!("res{}", d.id), ci, tex_opts())),
                }
                if r.analysis.is_some() {
                    d.analysis = r.analysis;
                }
                d.last_frame = Some((Arc::new(r.image), r.kind));
            }
        }
        while let Ok((id, res)) = self.still_rx.try_recv() {
            self.still_busy = false;
            match res {
                Ok(img) => {
                    if let Some(d) = self.docs.iter_mut().find(|d| d.id == id) {
                        d.still = Some(Arc::new(img));
                        d.result_key = None;
                    }
                    self.info("Signature applied to the preview. Use File › Export Still to save it.");
                }
                Err(e) => self.error(format!("Signature failed: {e}")),
            }
        }

        let engine = self.engine;
        let view = self.view;
        let diff_gain = self.canvas.diff_gain;
        let hope_preview = self.hope_preview;
        let feather = self.settings.selection_feather;
        let frame = self.frame;
        let Some(d) = self.docs.get_mut(self.active) else { return };
        if d.tex_orig.is_none() {
            let ci = egui::ColorImage::from_rgba_unmultiplied([d.source.width() as usize, d.source.height() as usize], d.source.pixels.as_raw());
            d.tex_orig = Some(ctx.load_texture(format!("orig{}", d.id), ci, tex_opts()));
        }
        if is_hope(engine) && hope_preview != panels::HopePreview::Off {
            let p = d.hope_params(engine).clone();
            let out = match (view, hope_preview) {
                (ViewMode::Difference, _) => SimOut::Difference,
                (_, panels::HopePreview::Overlay) => SimOut::Overlay,
                _ => SimOut::Image,
            };
            let key = util::hash_keys(&[d.id, engine as u64, p.budget_255().to_bits() as u64, p.seed, out as u64, (diff_gain * 10.0) as u64, d.selection.version, feather.to_bits() as u64]);
            if d.sim_key != Some(key) {
                let gain = if out == SimOut::Overlay { 12.0 } else { diff_gain };
                let m = d.preview_mask(feather);
                let (img, max, psnr) = simulate_hope(&d.preview_src, p.budget_255(), p.seed, out, gain, m.as_deref());
                let ci = egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw());
                let tex = ctx.load_texture(format!("sim{}", d.id), ci, tex_opts());
                if out == SimOut::Difference {
                    d.tex_diff = Some((tex, 0, 0, max, psnr));
                } else {
                    d.tex_result = Some(tex);
                }
                d.sim_key = Some(key);
                d.result_key = None;
            }
            return;
        }
        d.sim_key = None;
        if engine != EngineId::GridNoise {
            let tag = u64::MAX - engine as u64;
            if let Some(still) = d.still_for(engine) {
                let ptr = Arc::as_ptr(&still) as usize;
                let key = (tag, ptr as u32, PreviewKind::Frame);
                if d.result_key != Some(key) {
                    let ci = egui::ColorImage::from_rgba_unmultiplied([still.width() as usize, still.height() as usize], still.as_raw());
                    d.tex_result = Some(ctx.load_texture(format!("still{}", d.id), ci, tex_opts()));
                    d.result_key = Some(key);
                }
                // Amplified difference |protected − original| for the Difference view.
                let gain_key = (diff_gain * 10.0) as u32;
                if view == ViewMode::Difference && d.tex_diff.as_ref().map(|t| (t.1, t.2)) != Some((ptr, gain_key)) && still.dimensions() == d.source.pixels.dimensions() {
                    let mut px = Vec::with_capacity(still.as_raw().len());
                    let mut max = 0u8;
                    for (a, b) in still.pixels().zip(d.source.pixels.pixels()) {
                        for c in 0..3 {
                            let delta = a[c].abs_diff(b[c]);
                            max = max.max(delta);
                            px.push((delta as f32 * diff_gain).min(255.0) as u8);
                        }
                        px.push(255);
                    }
                    let psnr = image_io::psnr(&still, &d.source.pixels).unwrap_or(0.0);
                    let ci = egui::ColorImage::from_rgba_unmultiplied([still.width() as usize, still.height() as usize], &px);
                    d.tex_diff = Some((ctx.load_texture(format!("diff{}", d.id), ci, tex_opts()), ptr, gain_key, max, psnr));
                }
            } else if d.result_key.map(|k| k.0 != 0).unwrap_or(true) {
                d.tex_result = d.tex_orig.clone();
                d.result_key = Some((0, 0, PreviewKind::Frame));
            }
            return;
        }
        let kind = if view == ViewMode::Average {
            PreviewKind::Average(if d.gn.balance == Balance::Cycle { d.gn.cycle_frames } else { 24 })
        } else {
            PreviewKind::Frame
        };
        let hash = util::hash_keys(&serde_json::to_string(&d.gn).unwrap_or_default().bytes().map(u64::from).collect::<Vec<_>>());
        let hash = util::hash_keys(&[hash, d.selection.version, feather.to_bits() as u64]);
        let key = (hash ^ d.id, if kind == PreviewKind::Frame { frame } else { 0 }, kind);
        if d.result_key != Some(key) && !self.preview.busy {
            let mask = d.preview_mask(feather);
            self.preview.request(Request { doc: d.id, source: d.preview_src.clone(), params: d.gn.clone(), scale: d.preview_scale, frame: key.1, kind, mask });
            d.result_key = Some(key);
        }
    }

    fn tick_animation(&mut self, ctx: &egui::Context) {
        if !self.playing || self.engine != EngineId::GridNoise {
            self.last_tick = Instant::now();
            return;
        }
        let fps = self.doc().map(|d| d.gn.animation.fps).unwrap_or(30).max(1);
        let interval = Duration::from_secs_f32(1.0 / fps as f32);
        if self.last_tick.elapsed() >= interval && !self.preview.busy {
            let total = self.doc().map(|d| d.gn.frame_count()).unwrap_or(1).max(1);
            self.frame = (self.frame + 1) % total;
            self.last_tick = Instant::now();
        }
        ctx.request_repaint_after(interval / 2);
    }

    // ───────────────────────── input ─────────────────────────
    fn shortcuts(&mut self, ctx: &egui::Context) {
        use egui::{Key, KeyboardShortcut as KS, Modifiers as M};
        let fire = |m: M, k: Key| ctx.input_mut(|i| i.consume_shortcut(&KS::new(m, k)));
        if fire(M::COMMAND, Key::O) {
            self.import_dialog();
        }
        if fire(M::COMMAND | M::SHIFT, Key::S) {
            self.save_project(true);
        } else if fire(M::COMMAND, Key::S) {
            self.save_project(false);
        }
        if fire(M::COMMAND | M::SHIFT, Key::E) {
            self.export_animation(ctx);
        } else if fire(M::COMMAND, Key::E) {
            self.export_still(ctx, StillMode::StaticLayers);
        }
        if fire(M::COMMAND, Key::Z) {
            self.undo();
        }
        if fire(M::COMMAND, Key::Y) || fire(M::COMMAND | M::SHIFT, Key::Z) {
            self.redo();
        }
        if fire(M::COMMAND, Key::W) {
            self.close_active();
        }
        // Selection commands; skipped while typing so Ctrl+A still selects text in text boxes.
        if !ctx.wants_keyboard_input() {
            if fire(M::COMMAND | M::SHIFT, Key::I) {
                self.invert_selection();
            }
            if fire(M::COMMAND, Key::A) {
                self.select_all();
            }
            if fire(M::COMMAND, Key::D) {
                self.deselect();
            }
            if fire(M::COMMAND, Key::H) {
                self.show_sel_overlay = !self.show_sel_overlay;
            }
        }
        if fire(M::NONE, Key::F1) {
            self.show_shortcuts = !self.show_shortcuts;
        }
        if fire(M::COMMAND, Key::Num0) {
            self.canvas.fit_pending = true;
        }
        if fire(M::COMMAND, Key::Num1) {
            self.canvas.set_actual_pixels(ctx);
        }
        if fire(M::COMMAND, Key::Plus) || fire(M::COMMAND, Key::Equals) {
            self.canvas.zoom_by(1.25);
        }
        if fire(M::COMMAND, Key::Minus) {
            self.canvas.zoom_by(0.8);
        }
        if ctx.wants_keyboard_input() {
            return;
        }
        ctx.input(|i| {
            if i.key_pressed(Key::H) {
                self.tool = Tool::Hand;
            }
            if i.key_pressed(Key::Z) && !i.modifiers.command {
                self.tool = Tool::Zoom;
            }
            if i.key_pressed(Key::L) {
                self.tool = Tool::Lasso;
            }
            if i.key_pressed(Key::M) && !i.modifiers.command {
                self.tool = if i.modifiers.shift {
                    // Shift+M cycles between the two marquee shapes, like Photoshop.
                    if self.tool == Tool::Marquee { Tool::Ellipse } else { Tool::Marquee }
                } else if self.tool == Tool::Ellipse {
                    Tool::Ellipse
                } else {
                    Tool::Marquee
                };
            }
            if i.key_pressed(Key::B) && !i.modifiers.command {
                self.tool = Tool::Brush;
            }
            if i.key_pressed(Key::OpenBracket) {
                self.settings.brush_size = (self.settings.brush_size / 1.2).max(2.0);
            }
            if i.key_pressed(Key::CloseBracket) {
                self.settings.brush_size = (self.settings.brush_size * 1.2).min(2000.0);
            }
            if i.key_pressed(Key::P) {
                self.tool = Tool::Inspect;
            }
            if i.key_pressed(Key::G) && !i.modifiers.command {
                self.canvas.show_grid = !self.canvas.show_grid;
            }
            if i.key_pressed(Key::Tab) {
                let all = self.show_left || self.show_right || self.show_bottom;
                self.show_left = !all;
                self.show_right = !all;
                self.show_bottom = !all;
            }
            if i.key_pressed(Key::Enter) && i.modifiers.command {
                self.playing = !self.playing;
            }
        });
        if ctx.input(|i| i.key_pressed(egui::Key::Space) && i.modifiers.is_none()) && self.tool != Tool::Hand {
            // Space is also temporary panning; only toggle playback when not dragging.
            if !ctx.input(|i| i.pointer.any_down()) {
                self.playing = !self.playing;
            }
        }
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect());
        for p in dropped {
            if p.is_dir() {
                self.add_batch_folder(&p);
            } else {
                self.open_path(&p);
            }
        }
    }

    fn select_all(&mut self) {
        if let Some(d) = self.doc_mut() {
            let (w, h) = (d.source.width(), d.source.height());
            d.set_selection(Some(Mask::from_pixel(w, h, image::Luma([255]))));
        }
    }

    fn deselect(&mut self) {
        if let Some(d) = self.doc_mut() {
            if d.has_selection() {
                d.set_selection(None);
            }
        }
    }

    fn invert_selection(&mut self) {
        let Some(d) = self.doc_mut() else { return };
        let (w, h) = (d.source.width(), d.source.height());
        // No selection means "the whole image"; its inverse would be empty, so start from
        // "nothing selected" inverted = everything, which keeps the operation meaningful.
        let mut m = d.selection.mask.as_ref().map(|m| (**m).clone()).unwrap_or_else(|| Mask::new(w, h));
        selection::invert(&mut m);
        let empty = selection::is_empty(&m);
        d.set_selection(if empty { None } else { Some(m) });
        if empty {
            self.info("Inverted selection is empty: the whole image is used again.");
        }
    }

    fn add_batch_folder(&mut self, dir: &Path) {
        let mut n = 0;
        if let Ok(rd) = std::fs::read_dir(dir) {
            for e in rd.flatten() {
                let p = e.path();
                if p.is_file() && image_io::is_supported_path(&p) && !self.batch_files.contains(&p) {
                    self.batch_files.push(p);
                    n += 1;
                }
            }
        }
        self.batch_files.sort();
        self.show_batch = true;
        self.info(format!("Added {n} image(s) from {} to the batch list", dir.display()));
    }
}

#[derive(Clone, Copy, PartialEq)]
enum SimOut {
    Image,
    Overlay,
    Difference,
}

/// Approximate what a Hope:RE run at this strength does to the picture, instantly: sign noise
/// at the full L∞ budget, scaled by an edge weight (0.5–1) like Hope:RE's optimiser. The real
/// pattern is optimised during the run; only its strength and distribution are simulated here.
fn simulate_hope(src: &RgbaImage, eps255: f32, seed: u64, out: SimOut, gain: f32, mask: Option<&Mask>) -> (RgbaImage, u8, f64) {
    let (w, h) = src.dimensions();
    let gray: Vec<f32> = src.pixels().map(|p| (p[0] as f32 + p[1] as f32 + p[2] as f32) / 3.0).collect();
    let mut edge = vec![0f32; gray.len()];
    for y in 0..h as usize {
        for x in 0..w as usize {
            let i = y * w as usize + x;
            let nx = if x + 1 < w as usize { i + 1 } else { i };
            let ny = if y + 1 < h as usize { i + w as usize } else { i };
            edge[i] = ((gray[nx] - gray[i]).powi(2) + (gray[ny] - gray[i]).powi(2)).sqrt();
        }
    }
    let (mn, mx) = edge.iter().fold((f32::MAX, f32::MIN), |(a, b), &v| (a.min(v), b.max(v)));
    let inv = 1.0 / (mx - mn + 1e-6);
    let mut img = src.clone();
    let mut max_change = 0u8;
    let mut se = 0f64;
    for (i, (o, s)) in img.pixels_mut().zip(src.pixels()).enumerate() {
        let sel = mask.map(|m| m.as_raw().get(i).copied().unwrap_or(255) as f32 / 255.0).unwrap_or(1.0);
        let wgt = (0.5 + 0.5 * (edge[i] - mn) * inv) * sel;
        let mut mag = 0f32;
        for c in 0..3 {
            let sign = if util::splitmix64(seed ^ (i as u64 * 3 + c as u64)) & 1 == 0 { 1.0 } else { -1.0 };
            let v = (s[c] as f32 + sign * eps255 * wgt).clamp(0.0, 255.0);
            let delta = (v - s[c] as f32).abs();
            mag += delta / 3.0;
            max_change = max_change.max(delta.round() as u8);
            se += (delta as f64).powi(2);
            o[c] = match out {
                SimOut::Image => v.round() as u8,
                SimOut::Difference => (delta * gain).min(255.0) as u8,
                SimOut::Overlay => s[c],
            };
        }
        match out {
            SimOut::Overlay => {
                let a = (mag * gain / 255.0).clamp(0.0, 0.85);
                let tint = [255.0, 40.0, 210.0];
                for c in 0..3 {
                    o[c] = (o[c] as f32 * (1.0 - a) + tint[c] * a) as u8;
                }
            }
            SimOut::Difference => o[3] = 255,
            SimOut::Image => {}
        }
    }
    let mse = se / (w as f64 * h as f64 * 3.0).max(1.0);
    let psnr = if mse == 0.0 { 99.0 } else { 10.0 * (65025.0 / mse).log10() };
    (img, max_change, psnr)
}

fn tex_opts() -> egui::TextureOptions {
    egui::TextureOptions { magnification: egui::TextureFilter::Nearest, minification: egui::TextureFilter::Linear, wrap_mode: egui::TextureWrapMode::ClampToEdge, mipmap_mode: None }
}

fn start_init(ctx: egui::Context) -> Receiver<InitMsg> {
    let (tx, rx) = unbounded();
    std::thread::spawn(move || {
        let send = |name: &'static str, ok: bool, detail: String| {
            let _ = tx.send(InitMsg::Step { name, ok, detail });
            ctx.request_repaint();
        };
        send("Configuration", true, format!("Settings loaded from {}", shieldcraft_core::paths::data_root().display()));
        let hw = shieldcraft_core::hardware::detect();
        send("Hardware", true, format!("{} · {} threads · {:.0} GB RAM", hw.cpu, hw.cores, hw.ram_gb));
        let available: Vec<&str> = EngineId::ALL.iter().filter(|e| engines::availability(**e).is_available()).map(|e| engines::info(*e).short).collect();
        send("Protection engines", !available.is_empty(), format!("Available: {}", available.join(", ")));
        let settings = Settings::load().0;
        let ff = gridnoise::find_ffmpeg(settings.ffmpeg_path.as_deref().map(Path::new));
        send("FFmpeg (optional)", true, ff.as_ref().map(|p| format!("Found at {}", p.display())).unwrap_or_else(|| "Not found — MP4/WebM export disabled; GIF/APNG available".into()));
        let lic = shieldcraft_core::paths::resource_dir("licenses").is_some();
        send("License notices", lic, if lic { "Third-party notices found".into() } else { "licenses folder missing next to the application".into() });
        let user = presets::load_user();
        send("Presets", true, format!("{} built-in, {} user preset(s)", presets::builtin().len(), user.len()));
        let _ = tx.send(InitMsg::Done { hardware: hw, ffmpeg: ff, user_presets: user });
        ctx.request_repaint();
    });
    rx
}

// ───────────────────────── frame update ─────────────────────────

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.smoke_step(ctx);
        if self.welcome.is_some() {
            self.welcome_screen(ctx);
            return;
        }
        if !self.main_layout_applied {
            ctx.send_viewport_cmd(egui::ViewportCommand::Decorations(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Resizable(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(egui::vec2(960.0, 600.0)));
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(1440.0, 900.0)));
            ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(true));
            self.main_layout_applied = true;
        }
        self.shortcuts(ctx);
        self.track_undo(ctx);
        self.tick_animation(ctx);
        self.poll_hope_runs();
        self.pump_preview(ctx);
        self.update_monitor(ctx);
        let doc_name = self.doc().map(|d| d.source.name.clone()).unwrap_or_default();
        if let Some((ok, msg)) = self.lantern.poll(&doc_name) {
            if ok {
                self.info(msg);
            } else {
                self.error(msg);
            }
        }
        self.update_title(ctx);

        self.menu_bar(ctx);
        self.status_bar(ctx);
        if self.show_bottom {
            self.bottom_dock(ctx);
        }
        self.toolbar(ctx);
        if self.show_left {
            self.left_panel(ctx);
        }
        if self.show_right {
            self.right_panel(ctx);
        }
        self.central(ctx);

        self.licenses.show(ctx);
        self.about_window(ctx);
        self.prefs_window(ctx);
        self.shortcuts_window(ctx);
        self.batch_window(ctx);
        self.info_window(ctx);
        self.download_window(ctx);
        if self.welcome_modal {
            self.welcome_modal_window(ctx);
        }
        if self.tasks.iter().any(|t| t.lock().unwrap().is_running()) || self.still_busy {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        for t in &self.tasks {
            t.lock().unwrap().cancel.cancel();
        }
        let _ = self.settings.save();
        let log: String = self.log.iter().map(|(t, _, m)| format!("+{:.1}s {m}\n", t.duration_since(self.epoch).as_secs_f32())).collect();
        let _ = std::fs::write(shieldcraft_core::paths::logs_dir().join("last-session.log"), log);
    }
}

impl App {
    fn smoke_step(&mut self, ctx: &egui::Context) {
        let Some(s) = self.smoke.as_mut() else { return };
        s.frames += 1;
        let stage = s.stage.clone();
        let out_path = s.out.clone();
        if s.frames == 1 && stage != "welcome" {
            if let Some(w) = self.welcome.as_mut() {
                // Let init finish but skip the screen.
                while let Some(Ok(m)) = w.rx.as_ref().map(|r| r.recv()) {
                    if let InitMsg::Done { hardware, ffmpeg, user_presets } = m {
                        self.hardware = hardware;
                        self.ffmpeg = ffmpeg;
                        self.user_presets = user_presets;
                        break;
                    }
                }
                self.welcome = None;
                self.main_layout_applied = false;
            }
            if let Ok(p) = std::env::var("SHIELDCRAFT_SMOKE_OPEN") {
                self.open_path(Path::new(&p));
            }
            self.view = match std::env::var("SHIELDCRAFT_SMOKE_VIEW").as_deref() {
                Ok("difference") => ViewMode::Difference,
                Ok("split") => ViewMode::Split,
                Ok("average") => ViewMode::Average,
                Ok("side") => ViewMode::SideBySide,
                Ok("original") => ViewMode::Original,
                _ => ViewMode::Result,
            };
            match stage.as_str() {
                "licenses" => self.licenses.open = true,
                "signature" => {
                    self.engine = EngineId::SignatureInk;
                    self.apply_signature_preview();
                }
                "engines" => {
                    self.right_tab = RightTab::Engines;
                }
                "provenance" => {
                    self.right_tab = RightTab::Provenance;
                    self.settings.lantern_enabled = true;
                }
                "selection" | "selection_sim" => {
                    // Ellipse plus a brush stroke, then show the Grid Noise preview (or Hope:RE simulation) limited to it.
                    if stage == "selection_sim" {
                        self.engine = EngineId::HopeStyle;
                        self.hope_preview = panels::HopePreview::Overlay;
                    }
                    self.tool = Tool::Ellipse;
                    if let Some(d) = self.doc_mut() {
                        let (w, h) = (d.source.width() as f32, d.source.height() as f32);
                        d.edit_selection(&crate::canvas::SelEdit::Ellipse((w * 0.15, h * 0.2), (w * 0.6, h * 0.8), selection::Op::Replace), 0.0);
                        d.edit_selection(&crate::canvas::SelEdit::Brush { from: (w * 0.65, h * 0.15), to: (w * 0.9, h * 0.6), erase: false }, w * 0.04);
                        d.committed = d.snapshot();
                    }
                }
                "preview_overlay" | "preview_sim" => {
                    self.engine = EngineId::HopeStyle;
                    self.hope_preview = if stage == "preview_overlay" { panels::HopePreview::Overlay } else { panels::HopePreview::Simulated };
                    if let Some(d) = self.doc_mut() {
                        d.style.intensity = 0.30;
                        d.committed = d.snapshot();
                    }
                    if stage == "preview_sim" {
                        self.canvas.set_actual_pixels(ctx);
                    }
                }
                "hope" | "style" | "poison" => {
                    let engine = match stage.as_str() {
                        "style" => EngineId::HopeStyle,
                        "poison" => EngineId::HopePoison,
                        _ => EngineId::HopeNoise,
                    };
                    self.engine = engine;
                    if let Some(d) = self.doc_mut() {
                        let p = d.hope_params_mut(engine);
                        if engine != EngineId::HopePoison {
                            p.iterations = 5;
                        }
                        if engine == EngineId::HopeStyle {
                            p.style_index = 2; // Cubist
                        }
                        d.committed = d.snapshot();
                    }
                    let out = out_path.with_file_name(format!("{stage}_result.png"));
                    let opts = self.settings.export.clone();
                    self.run_hope_to(ctx, engine, out, opts);
                }
                _ => {
                    self.canvas.show_grid = true;
                    self.frame = 7;
                }
            }
        }
        let s = self.smoke.as_mut().unwrap();
        let has_size = ctx.input(|i| i.viewport().inner_rect.map(|r| r.width() > 50.0 && r.height() > 50.0).unwrap_or(false));
        let ready = s.frames > 90 && has_size && !self.preview.busy && !self.still_busy && self.hope_runs.is_empty();
        if ready && !s.requested {
            s.requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
        }
        let shot = ctx.input(|i| {
            i.raw.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(img) = shot {
            let [w, h] = img.size;
            let raw: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_array()).collect();
            if let Some(buf) = RgbaImage::from_raw(w as u32, h as u32, raw) {
                let _ = buf.save(&self.smoke.as_ref().unwrap().out);
            }
            let log: String = self.log.iter().map(|(_, _, m)| format!("{m}\n")).collect();
            let _ = std::fs::write(self.smoke.as_ref().unwrap().out.with_extension("log"), log);
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        ctx.request_repaint();
    }

    fn update_title(&mut self, ctx: &egui::Context) {
        let name = self.project_path.as_ref().and_then(|p| p.file_stem()).map(|s| s.to_string_lossy().to_string()).or_else(|| self.doc().map(|d| d.name().to_string())).unwrap_or_else(|| "Untitled".into());
        let t = format!("ShieldCraft — {name}{}", if self.project_dirty { " •" } else { "" });
        if t != self.title_cache {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(t.clone()));
            self.title_cache = t;
        }
    }

    // ───────── welcome ─────────
    fn welcome_screen(&mut self, ctx: &egui::Context) {
        let mut action = None;
        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(Color32::from_rgb(22, 23, 27))).show(ctx, |ui| {
            action = self.welcome_contents(ui, true);
        });
        match action {
            Some(WelcomeAction::Continue) => {
                let dont = self.welcome.as_ref().map(|w| w.dont_show).unwrap_or(false);
                if dont {
                    self.settings.show_welcome = false;
                    let _ = self.settings.save();
                }
                self.welcome = None;
                self.main_layout_applied = false;
                ctx.request_repaint();
            }
            Some(WelcomeAction::Licenses) => {
                self.welcome = None;
                self.main_layout_applied = false;
                self.licenses.open = true;
            }
            None => {}
        }
    }

    fn welcome_modal_window(&mut self, ctx: &egui::Context) {
        if self.welcome.is_none() {
            self.welcome = Some(Welcome { steps: vec![], rx: Some(start_init(ctx.clone())), done: false, dont_show: !self.settings.show_welcome, started: Instant::now() });
        }
        let mut action = None;
        egui::Modal::new(egui::Id::new("welcome_modal")).frame(egui::Frame::NONE.fill(Color32::from_rgb(22, 23, 27))).show(ctx, |ui| {
            ui.set_min_size(egui::vec2(760.0, 450.0));
            action = self.welcome_contents(ui, false);
        });
        if let Some(a) = action {
            if let Some(w) = &self.welcome {
                self.settings.show_welcome = !w.dont_show;
                let _ = self.settings.save();
            }
            self.welcome = None;
            self.welcome_modal = false;
            if a == WelcomeAction::Licenses {
                self.licenses.open = true;
            }
        }
    }

    fn welcome_contents(&mut self, ui: &mut egui::Ui, standalone: bool) -> Option<WelcomeAction> {
        let mut action = None;
        if let Some(w) = self.welcome.as_mut() {
            if let Some(rx) = &w.rx {
                while let Ok(m) = rx.try_recv() {
                    match m {
                        InitMsg::Step { name, ok, detail } => w.steps.push((name, ok, detail)),
                        InitMsg::Done { hardware, ffmpeg, user_presets } => {
                            self.hardware = hardware;
                            self.ffmpeg = ffmpeg;
                            self.user_presets = user_presets;
                            w.done = true;
                        }
                    }
                }
            }
        }
        let w = self.welcome.as_mut().unwrap();
        let rect = ui.max_rect();
        let art = egui::Rect::from_min_size(rect.min, egui::vec2(300.0, rect.height()));
        draw_welcome_art(ui.painter(), art, w.started.elapsed().as_secs_f32());
        ui.ctx().request_repaint_after(Duration::from_millis(60));

        let content = egui::Rect::from_min_max(egui::pos2(art.right() + 28.0, rect.top() + 26.0), rect.max - egui::vec2(24.0, 12.0));
        ui.scope_builder(egui::UiBuilder::new().max_rect(content), |ui| {
            ui.set_max_width(content.width());
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
            ui.horizontal(|ui| {
                let tex = ui.ctx().data_mut(|d| d.get_temp::<egui::TextureHandle>(egui::Id::new("logo_tex"))).unwrap_or_else(|| {
                    let i = crate::branding::icon(128);
                    let t = ui.ctx().load_texture("logo", egui::ColorImage::from_rgba_unmultiplied([128, 128], i.as_raw()), egui::TextureOptions::LINEAR);
                    ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("logo_tex"), t.clone()));
                    t
                });
                ui.image((tex.id(), egui::vec2(56.0, 56.0)));
                ui.vertical(|ui| {
                    ui.label(RichText::new("ShieldCraft").size(26.0).strong().color(Color32::from_rgb(236, 238, 244)));
                    ui.label(RichText::new(format!("Version {} · {} build · local processing only", shieldcraft_core::APP_VERSION, shieldcraft_core::BUILD_PROFILE)).small().color(Color32::GRAY));
                });
            });
            ui.add_space(14.0);
            for (name, ok, detail) in &w.steps {
                ui.horizontal(|ui| {
                    let (i, c) = if *ok { (icon::CHECK_CIRCLE, theme::OK) } else { (icon::WARNING_CIRCLE, theme::WARN) };
                    ui.label(RichText::new(i).color(c));
                    ui.allocate_ui_with_layout(egui::vec2(140.0, 16.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.set_min_width(140.0);
                        ui.label(RichText::new(*name).color(Color32::from_rgb(220, 222, 228)));
                    });
                    ui.add(egui::Label::new(RichText::new(detail).small().color(Color32::GRAY)).truncate()).on_hover_text(detail);
                });
            }
            if !w.done {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(RichText::new("Initialising…").color(Color32::GRAY));
                });
            }
            ui.add_space(10.0);
            ui.label(RichText::new(shieldcraft_core::licenses::STARTUP_STATEMENT).small().color(Color32::from_gray(150)));
            ui.add_space(4.0);
            ui.label(RichText::new("Grid Noise Studio is based on techniques by stelmosfire13-alt (Grid Noise Animator).").small().color(Color32::from_gray(150)));
            ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                ui.checkbox(&mut w.dont_show, RichText::new("Don't show this screen again").color(Color32::GRAY));
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    let cont = ui.add_enabled(w.done, egui::Button::new(RichText::new(format!("Continue to ShieldCraft  {}", icon::ARROW_RIGHT)).strong()).fill(theme::ACCENT).min_size(egui::vec2(220.0, 30.0)));
                    if cont.clicked() || (w.done && ui.input(|i| i.key_pressed(egui::Key::Enter))) {
                        action = Some(WelcomeAction::Continue);
                    }
                    if ui.add(egui::Button::new(format!("{} View Licenses", icon::SCROLL)).min_size(egui::vec2(0.0, 30.0))).clicked() {
                        action = Some(WelcomeAction::Licenses);
                    }
                    let _ = standalone;
                });
            });
        });
        action
    }

    // ───────── menus ─────────
    fn menu_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("menu").show(ctx, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.label(RichText::new(format!("{} ShieldCraft", icon::SHIELD_CHECK)).strong().color(theme::ACCENT));
                ui.menu_button("File", |ui| {
                    if ui.add(egui::Button::new(format!("{} Import…", icon::FOLDER_OPEN)).shortcut_text("Ctrl+O")).clicked() {
                        ui.close();
                        self.import_dialog();
                    }
                    ui.menu_button("Open Recent", |ui| {
                        let recent = self.settings.recent_files.clone();
                        if recent.is_empty() {
                            ui.label(RichText::new("No recent files").weak());
                        }
                        for r in recent {
                            if ui.button(&r).clicked() {
                                ui.close();
                                self.open_path(Path::new(&r));
                            }
                        }
                    });
                    ui.separator();
                    if ui.button("Open Project…").clicked() {
                        ui.close();
                        self.open_project();
                    }
                    if ui.add(egui::Button::new("Save Project").shortcut_text("Ctrl+S")).clicked() {
                        ui.close();
                        self.save_project(false);
                    }
                    if ui.add(egui::Button::new("Save Project As…").shortcut_text("Ctrl+Shift+S")).clicked() {
                        ui.close();
                        self.save_project(true);
                    }
                    ui.separator();
                    let has = self.doc().is_some();
                    if ui.add_enabled(has, egui::Button::new(format!("{} Export Animation…", icon::FILM_STRIP)).shortcut_text("Ctrl+Shift+E")).clicked() {
                        ui.close();
                        self.export_animation(ctx);
                    }
                    if ui.add_enabled(has, egui::Button::new(format!("{} Export Still…", icon::EXPORT)).shortcut_text("Ctrl+E")).clicked() {
                        ui.close();
                        self.export_still(ctx, StillMode::StaticLayers);
                    }
                    if ui.add_enabled(has && self.engine == EngineId::GridNoise, egui::Button::new("Export Current Frame…")).clicked() {
                        ui.close();
                        let f = self.frame;
                        self.export_still(ctx, StillMode::SingleFrame(f));
                    }
                    if ui.button(format!("{} Batch Process…", icon::STACK)).clicked() {
                        ui.close();
                        self.show_batch = true;
                    }
                    ui.separator();
                    if ui.add_enabled(has, egui::Button::new("Close Image").shortcut_text("Ctrl+W")).clicked() {
                        ui.close();
                        self.close_active();
                    }
                    if ui.button("Exit").clicked() {
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
                ui.menu_button("Edit", |ui| {
                    let (u, r) = self.doc().map(|d| (!d.undo.is_empty(), !d.redo.is_empty())).unwrap_or((false, false));
                    if ui.add_enabled(u, egui::Button::new(format!("{} Undo", icon::ARROW_COUNTER_CLOCKWISE)).shortcut_text("Ctrl+Z")).clicked() {
                        ui.close();
                        self.undo();
                    }
                    if ui.add_enabled(r, egui::Button::new(format!("{} Redo", icon::ARROW_CLOCKWISE)).shortcut_text("Ctrl+Y")).clicked() {
                        ui.close();
                        self.redo();
                    }
                    ui.separator();
                    if ui.add_enabled(self.doc().is_some(), egui::Button::new("Reset Engine Settings")).clicked() {
                        ui.close();
                        let e = self.engine;
                        if let Some(d) = self.doc_mut() {
                            match e {
                                EngineId::SignatureInk => d.sig = SignatureParams::default(),
                                _ => d.gn = GridNoiseParams::default(),
                            }
                        }
                    }
                    ui.separator();
                    if ui.button(format!("{} Preferences…", icon::GEAR)).clicked() {
                        ui.close();
                        self.show_prefs = true;
                    }
                });
                ui.menu_button("Image", |ui| {
                    let has = self.doc().is_some();
                    for (label, op) in [("Rotate 90° Clockwise", "cw"), ("Rotate 90° Counter-clockwise", "ccw"), ("Rotate 180°", "180"), ("Flip Horizontal", "h"), ("Flip Vertical", "v")] {
                        if ui.add_enabled(has, egui::Button::new(label)).clicked() {
                            ui.close();
                            self.transform_image(op);
                        }
                    }
                    ui.separator();
                    if ui.add_enabled(has, egui::Button::new(format!("{} Image Information…", icon::INFO))).clicked() {
                        ui.close();
                        self.show_info = true;
                    }
                });
                ui.menu_button("Select", |ui| {
                    let has = self.doc().is_some();
                    let sel = self.doc().map(|d| d.has_selection()).unwrap_or(false);
                    if ui.add_enabled(has, egui::Button::new(format!("{} All", icon::SELECTION_ALL)).shortcut_text("Ctrl+A")).clicked() {
                        ui.close();
                        self.select_all();
                    }
                    if ui.add_enabled(sel, egui::Button::new(format!("{} Deselect", icon::SELECTION_SLASH)).shortcut_text("Ctrl+D")).clicked() {
                        ui.close();
                        self.deselect();
                    }
                    if ui.add_enabled(sel, egui::Button::new(format!("{} Inverse", icon::SELECTION_INVERSE)).shortcut_text("Ctrl+Shift+I")).clicked() {
                        ui.close();
                        self.invert_selection();
                    }
                    ui.separator();
                    for (t, label, key) in [(Tool::Marquee, "Rectangle Tool", "M"), (Tool::Ellipse, "Ellipse Tool", "Shift+M"), (Tool::Lasso, "Lasso Tool", "L"), (Tool::Brush, "Selection Brush", "B")] {
                        if ui.add(egui::Button::selectable(self.tool == t, label).shortcut_text(key)).clicked() {
                            ui.close();
                            self.tool = t;
                        }
                    }
                    ui.separator();
                    if ui.add(egui::Button::selectable(self.show_sel_overlay, "Show Selection Overlay").shortcut_text("Ctrl+H")).clicked() {
                        ui.close();
                        self.show_sel_overlay = !self.show_sel_overlay;
                    }
                    if ui.button("Selection Preferences…").clicked() {
                        ui.close();
                        self.show_prefs = true;
                    }
                    ui.separator();
                    ui.label(RichText::new("Optional — with no selection, protection applies to the whole image.").small().weak());
                });
                ui.menu_button("Protection", |ui| {
                    for id in EngineId::ALL {
                        let info = engines::info(id);
                        let avail = engines::availability(id);
                        let label = format!("{}  ·  {}", info.name, info.kind.label());
                        let selectable = avail.is_available() || matches!(avail, Availability::ModelDownloadRequired { .. });
                        if selectable {
                            let label = if avail.is_available() { label } else { format!("{label}  ({})", avail.label()) };
                            if ui.selectable_label(self.engine == id, label).clicked() {
                                ui.close();
                                self.engine = id;
                                self.right_tab = RightTab::Properties;
                            }
                        } else {
                            ui.add_enabled(false, egui::Button::new(format!("{label}  ({})", avail.label())));
                        }
                    }
                    ui.separator();
                    if ui.button("Engine Status…").clicked() {
                        ui.close();
                        self.show_right = true;
                        self.right_tab = RightTab::Engines;
                    }
                });
                ui.menu_button("View", |ui| {
                    if ui.add(egui::Button::new("Fit on Screen").shortcut_text("Ctrl+0")).clicked() {
                        ui.close();
                        self.canvas.fit_pending = true;
                    }
                    if ui.add(egui::Button::new("Actual Pixels (100%)").shortcut_text("Ctrl+1")).clicked() {
                        ui.close();
                        self.canvas.set_actual_pixels(ctx);
                    }
                    if ui.add(egui::Button::new("Zoom In").shortcut_text("Ctrl++")).clicked() {
                        self.canvas.zoom_by(1.25);
                    }
                    if ui.add(egui::Button::new("Zoom Out").shortcut_text("Ctrl+-")).clicked() {
                        self.canvas.zoom_by(0.8);
                    }
                    ui.separator();
                    for m in ViewMode::ALL {
                        ui.radio_value(&mut self.view, m, m.label());
                    }
                    ui.separator();
                    ui.checkbox(&mut self.canvas.show_grid, "Grid Overlay (G)");
                    ui.separator();
                    let mut t = self.settings.theme;
                    ui.radio_value(&mut t, Theme::Dark, "Dark Theme");
                    ui.radio_value(&mut t, Theme::Light, "Light Theme");
                    if t != self.settings.theme {
                        self.settings.theme = t;
                        theme::apply(ctx, t, self.settings.ui_scale);
                        let _ = self.settings.save();
                    }
                });
                ui.menu_button("Window", |ui| {
                    ui.checkbox(&mut self.show_left, "Images & Stack");
                    ui.checkbox(&mut self.show_right, "Properties");
                    ui.checkbox(&mut self.show_monitor, "Effect Monitor");
                    ui.checkbox(&mut self.show_bottom, "Queue / History / Log");
                    ui.label(RichText::new("Tab toggles all panels").small().weak());
                });
                ui.menu_button("Help", |ui| {
                    if ui.button(format!("{} Third-Party Licenses & Credits", icon::SCROLL)).clicked() {
                        ui.close();
                        self.licenses.open = true;
                    }
                    if ui.add(egui::Button::new(format!("{} Keyboard Shortcuts", icon::KEYBOARD)).shortcut_text("F1")).clicked() {
                        ui.close();
                        self.show_shortcuts = true;
                    }
                    if ui.button("Welcome Screen").clicked() {
                        ui.close();
                        self.welcome_modal = true;
                    }
                    if ui.button("Open Logs Folder").clicked() {
                        ui.close();
                        open_in_explorer(&shieldcraft_core::paths::logs_dir());
                    }
                    if ui.button("Open Data Folder").clicked() {
                        ui.close();
                        open_in_explorer(shieldcraft_core::paths::data_root());
                    }
                    ui.separator();
                    if ui.button(format!("{} About ShieldCraft", icon::INFO)).clicked() {
                        ui.close();
                        self.show_about = true;
                    }
                });

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let running: Vec<(String, f32)> = self.tasks.iter().filter_map(|t| {
                        let t = t.lock().unwrap();
                        t.is_running().then(|| (t.title.clone(), t.progress()))
                    }).collect();
                    if let Some((title, p)) = running.first() {
                        ui.add(egui::ProgressBar::new(*p).desired_width(140.0).show_percentage());
                        ui.label(RichText::new(format!("{} {title}", icon::SPINNER)).small());
                    } else {
                        ui.label(RichText::new(format!("{} Idle", icon::CHECK)).small().weak());
                    }
                    ui.separator();
                    let (u, r) = self.doc().map(|d| (!d.undo.is_empty(), !d.redo.is_empty())).unwrap_or((false, false));
                    if ui.add_enabled(r, egui::Button::new(icon::ARROW_CLOCKWISE).frame(false)).on_hover_text("Redo (Ctrl+Y)").clicked() {
                        self.redo();
                    }
                    if ui.add_enabled(u, egui::Button::new(icon::ARROW_COUNTER_CLOCKWISE).frame(false)).on_hover_text("Undo (Ctrl+Z)").clicked() {
                        self.undo();
                    }
                    ui.separator();
                    let title = self.project_path.as_ref().and_then(|p| p.file_name()).map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "Untitled project".into());
                    ui.label(RichText::new(format!("{title}{}", if self.project_dirty { " •" } else { "" })).small());
                });
            });
        });
    }

    fn toolbar(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("tools").exact_width(40.0).resizable(false).show(ctx, |ui| {
            ui.add_space(4.0);
            ui.vertical_centered(|ui| {
                let big = |s: &str| RichText::new(s).size(18.0);
                if ui.add(egui::Button::new(big(icon::FOLDER_OPEN)).frame(false)).on_hover_text("Import artwork (Ctrl+O)").clicked() {
                    self.import_dialog();
                }
                ui.separator();
                for (t, i, tip) in [
                    (Tool::Hand, icon::HAND, "Hand / pan (H) — or hold Space"),
                    (Tool::Zoom, icon::MAGNIFYING_GLASS_PLUS, "Zoom (Z) — Alt-click to zoom out"),
                    (Tool::Inspect, icon::EYEDROPPER, "Inspect pixels (P)"),
                ] {
                    if ui.add(egui::Button::new(big(i)).selected(self.tool == t).frame(self.tool == t)).on_hover_text(tip).clicked() {
                        self.tool = t;
                    }
                }
                ui.separator();
                const MODS: &str = "\nShift: add to selection · Alt: subtract\nCtrl+Shift+I invert · Ctrl+D deselect";
                for (t, i, tip) in [
                    (Tool::Marquee, icon::SELECTION, "Rectangle select (M)"),
                    (Tool::Ellipse, icon::CIRCLE_DASHED, "Ellipse select (Shift+M)"),
                    (Tool::Lasso, icon::LASSO, "Lasso select (L) — drag around the area"),
                    (Tool::Brush, icon::PAINT_BRUSH, "Selection brush (B) — [ and ] change size; Alt paints out"),
                ] {
                    if ui.add(egui::Button::new(big(i)).selected(self.tool == t).frame(self.tool == t)).on_hover_text(format!("{tip}{MODS}")).clicked() {
                        self.tool = t;
                    }
                }
                ui.separator();
                if ui.add(egui::Button::new(big(icon::ARROWS_OUT)).frame(false)).on_hover_text("Fit on screen (Ctrl+0)").clicked() {
                    self.canvas.fit_pending = true;
                }
                if ui.add(egui::Button::new(big(icon::SQUARES_FOUR)).frame(false).selected(self.canvas.show_grid)).on_hover_text("Grid overlay (G)").clicked() {
                    self.canvas.show_grid = !self.canvas.show_grid;
                }
                if ui.add(egui::Button::new(big(icon::SPLIT_HORIZONTAL)).frame(false).selected(self.view == ViewMode::Split)).on_hover_text("Split comparison").clicked() {
                    self.view = if self.view == ViewMode::Split { ViewMode::Result } else { ViewMode::Split };
                }
                ui.separator();
                for (e, i, tip) in [
                    (EngineId::GridNoise, icon::GRID_NINE, "Grid Noise Studio"),
                    (EngineId::HopeNoise, icon::SHIELD_CHECKERED, "Hope:RE Noise"),
                    (EngineId::HopeStyle, icon::PALETTE, "Hope:RE Style Cloak"),
                    (EngineId::HopePoison, icon::FLASK, "Hope:RE Concept Poison"),
                    (EngineId::SignatureInk, icon::SIGNATURE, "Signature Ink"),
                ] {
                    if ui.add(egui::Button::new(big(i)).selected(self.engine == e).frame(self.engine == e)).on_hover_text(tip).clicked() {
                        self.engine = e;
                        if self.right_tab == RightTab::Provenance {
                            self.right_tab = RightTab::Properties;
                        }
                    }
                }
                ui.separator();
                if ui
                    .add(egui::Button::new(big(icon::STAMP)).selected(self.right_tab == RightTab::Provenance).frame(self.right_tab == RightTab::Provenance))
                    .on_hover_text("Provenance (Lantern registry)")
                    .clicked()
                {
                    self.right_tab = RightTab::Provenance;
                    self.show_right = true;
                }
            });
        });
    }

    fn left_panel(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("stack").default_width(220.0).width_range(170.0..=360.0).resizable(true).show(ctx, |ui| {
            theme::caption(ui, "Images");
            let mut select = None;
            let mut close = None;
            egui::ScrollArea::vertical().id_salt("docs").max_height(220.0).show(ui, |ui| {
                if self.docs.is_empty() {
                    ui.label(RichText::new("No images open").weak());
                }
                for (i, d) in self.docs.iter().enumerate() {
                    ui.horizontal(|ui| {
                        if ui.selectable_label(i == self.active, format!("{} {}", icon::IMAGE, d.name())).clicked() {
                            select = Some(i);
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button(icon::X).on_hover_text("Close").clicked() {
                                close = Some(i);
                            }
                        });
                    });
                }
            });
            if let Some(i) = select {
                self.active = i;
                self.canvas.fit_pending = true;
            }
            if let Some(i) = close {
                self.active = i;
                self.close_active();
            }
            ui.separator();
            theme::caption(ui, "Stack");
            let engine = self.engine;
            if let Some(d) = self.doc() {
                let res_name = match engine {
                    EngineId::SignatureInk => if d.still.is_some() { "Signature Ink result" } else { "Signature Ink (not applied yet)" },
                    EngineId::HopeNoise => if d.hope_still.is_some() { "Hope:RE Noise result" } else { "Hope:RE Noise (not run yet)" },
                    EngineId::HopeStyle => if d.style_still.is_some() { "Style Cloak result" } else { "Style Cloak (not run yet)" },
                    EngineId::HopePoison => if d.poison_still.is_some() { "Concept Poison result" } else { "Concept Poison (not run yet)" },
                    _ => "Grid Noise animation (live)",
                };
                ui.label(format!("{} {}", icon::STACK, res_name));
                ui.label(RichText::new(format!("   {} Source · {}×{} · {}", icon::LOCK_SIMPLE, d.source.width(), d.source.height(), d.source.info.format)).small());
                ui.label(RichText::new("   Source is read-only; results are separate.").small().weak());
            }
            ui.separator();
            theme::caption(ui, "Storyboard");
            ui.label(RichText::new("Tick 2+ images to export them as one animation, each for its own duration.").small().weak());
            for d in self.docs.iter_mut() {
                ui.horizontal(|ui| {
                    ui.checkbox(&mut d.in_storyboard, "");
                    ui.add_enabled(d.in_storyboard, egui::DragValue::new(&mut d.panel_duration).range(0.0..=60.0).speed(0.1).suffix(" s"));
                    ui.label(RichText::new(&d.source.name).small());
                });
            }
        });
    }

    fn right_panel(&mut self, ctx: &egui::Context) {
        egui::SidePanel::right("props").default_width(330.0).width_range(270.0..=520.0).resizable(true).show(ctx, |ui| {
            if self.show_monitor {
                egui::CollapsingHeader::new(RichText::new(format!("{} Effect Monitor", icon::CHART_BAR)).strong())
                    .default_open(true)
                    .show(ui, |ui| crate::monitor::show(ui, self.monitor.as_ref()));
                ui.separator();
            }
            ui.horizontal(|ui| {
                for (t, l) in [(RightTab::Properties, "Properties"), (RightTab::Presets, "Presets"), (RightTab::Export, "Export"), (RightTab::Engines, "Engines"), (RightTab::Provenance, "Provenance")] {
                    ui.selectable_value(&mut self.right_tab, t, l);
                }
            });
            ui.separator();
            egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| match self.right_tab {
                RightTab::Properties => self.properties_tab(ui),
                RightTab::Presets => self.presets_tab(ui),
                RightTab::Export => {
                    theme::caption(ui, "Still image export");
                    panels::export_options(ui, &mut self.settings.export);
                    if ui.button("Save as default").clicked() {
                        let _ = self.settings.save();
                    }
                    ui.separator();
                    let has = self.doc().is_some();
                    if ui.add_enabled(has, egui::Button::new(format!("{} Export Still…", icon::EXPORT))).clicked() {
                        self.export_still(ui.ctx(), StillMode::StaticLayers);
                    }
                    if self.engine == EngineId::GridNoise && ui.add_enabled(has, egui::Button::new(format!("{} Export Animation…", icon::FILM_STRIP))).clicked() {
                        self.export_animation(ui.ctx());
                    }
                }
                RightTab::Engines => engines_tab(ui, &self.hardware, self.ffmpeg.as_deref()),
                RightTab::Provenance => {
                    let file = self.docs.get(self.active).and_then(|d| d.source.path.clone().map(|p| (p, d.source.name.clone())));
                    let before = self.settings.lantern_enabled;
                    self.lantern.ui(ui, &mut self.settings.lantern_enabled, file);
                    if before != self.settings.lantern_enabled {
                        let _ = self.settings.save();
                    }
                }
            });
        });
    }

    fn properties_tab(&mut self, ui: &mut egui::Ui) {
        let info = engines::info(self.engine);
        ui.label(RichText::new(info.name).heading());
        ui.label(RichText::new(info.kind.label()).small().color(theme::ACCENT)).on_hover_text(info.kind.explanation());
        ui.add_space(4.0);
        let ffmpeg = self.ffmpeg.is_some();
        let engine = self.engine;
        let Some(d) = self.docs.get_mut(self.active) else {
            ui.label(RichText::new("Import an image to adjust settings.").weak());
            return;
        };
        let size = Some((d.source.width(), d.source.height()));
        ui.horizontal_wrapped(|ui| {
            let (txt, tip) = match (engine, d.selected_pct()) {
                (EngineId::SignatureInk, _) => (format!("{} Applies to: whole image", icon::SELECTION_ALL), "A signature must cover the whole image to be readable, so selections are ignored here."),
                (_, Some(p)) => (format!("{} Applies to: selection ({p:.1}%)", icon::SELECTION), "Only the selected area is changed. Ctrl+D deselects, Ctrl+Shift+I inverts."),
                (_, None) => (format!("{} Applies to: whole image", icon::SELECTION_ALL), "Optional: select an area with M (rectangle/ellipse), L (lasso) or B (brush) to protect only part of the image."),
            };
            ui.label(RichText::new(txt).small()).on_hover_text(tip);
        });
        ui.add_space(2.0);
        match engine {
            EngineId::HopeNoise | EngineId::HopeStyle | EngineId::HopePoison => {
                let avail = engines::availability(engine);
                let busy = self.hope_runs.iter().any(|(_, id, e)| *id == d.id && *e == engine);
                if let Availability::ModelDownloadRequired { detail, .. } = &avail {
                    ui.label(RichText::new(format!("Model needed: {detail}")).color(theme::WARN));
                    if ui.add_enabled(self.download.is_none(), egui::Button::new(format!("{} Download model…", icon::DOWNLOAD_SIMPLE))).clicked() {
                        self.confirm_download = Some(engine);
                    }
                    ui.separator();
                } else if !avail.is_available() {
                    ui.label(RichText::new(format!("{}: {}", avail.label(), avail.detail())).color(theme::BAD));
                }
                let has_result = d.still_for(engine).is_some();
                // Moving a slider switches the instant preview on, so the effect is always visible.
                let p = d.hope_params(engine);
                let watch = (d.id, engine, p.intensity.to_bits(), p.seed, p.style_index * 16 + p.target_index);
                if let Some(prev) = self.hope_watch {
                    if prev.0 == watch.0 && prev.1 == watch.1 && prev != watch && self.hope_preview == panels::HopePreview::Off {
                        self.hope_preview = panels::HopePreview::Simulated;
                    }
                }
                self.hope_watch = Some(watch);
                panels::hope_preview_controls(ui, &mut self.hope_preview, has_result);
                ui.separator();
                match panels::hope(ui, d.hope_params_mut(engine), size, avail.is_available(), busy) {
                    panels::HopeAction::Run => {
                        self.run_hope(ui.ctx(), engine);
                        return;
                    }
                    panels::HopeAction::NewSeed => d.hope_params_mut(engine).seed = util::fresh_seed(),
                    panels::HopeAction::None => {}
                }
                if has_result && ui.button(format!("{} Show Difference view", icon::EYE)).on_hover_text("See exactly where and how much the image was changed").clicked() {
                    self.view = ViewMode::Difference;
                }
            }
            EngineId::SignatureInk => {
                if panels::signature(ui, &mut d.sig, &mut self.sig_ui, size) {
                    let img = d.still.clone().unwrap_or_else(|| d.source.pixels.clone());
                    let key = self.sig_ui.verify_use_key.then_some(self.sig_ui.verify_key);
                    self.sig_ui.verify_result = Some(signature::extract(&img, self.sig_ui.verify_len, key));
                }
                ui.add_space(6.0);
                if ui.add_enabled(!self.still_busy, egui::Button::new(format!("{} Apply to preview", icon::PEN_NIB))).clicked() {
                    self.apply_signature_preview();
                }
            }
            _ => {
                let act = panels::gridnoise(ui, &mut d.gn, &mut self.canvas.show_grid, size, ffmpeg);
                if act.new_seed {
                    d.gn.seed = util::fresh_seed();
                }
                if act.clear_regions {
                    d.gn.region.polygons.clear();
                }
                if act.pick_audio {
                    if let Some(f) = rfd::FileDialog::new().add_filter("Audio", &["mp3", "wav", "m4a", "aac", "ogg", "flac", "opus"]).pick_file() {
                        d.gn.animation.audio_path = Some(f.display().to_string());
                    }
                }
            }
        }
        ui.add_space(8.0);
        egui::CollapsingHeader::new(RichText::new("About this method").strong()).default_open(false).show(ui, |ui| {
            ui.label(info.kind.explanation());
            ui.add_space(4.0);
            for l in info.limitations {
                ui.label(RichText::new(format!("• {l}")).small());
            }
            ui.label(RichText::new(format!("Output: {}", info.geometry)).small());
            ui.label(RichText::new(format!("Alpha: {}", info.alpha)).small());
        });
    }

    fn presets_tab(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Presets are starting points, not scientifically optimal settings.").small().weak());
        let engine = self.engine;
        let mut apply: Option<EngineParams> = None;
        let mut delete: Option<String> = None;
        theme::caption(ui, "Built-in");
        for p in self.builtin_presets.iter().filter(|p| p.params.engine() == engine) {
            if ui.button(&p.name).on_hover_text(&p.description).clicked() {
                apply = Some(p.params.clone());
            }
        }
        theme::caption(ui, "Yours");
        for p in self.user_presets.iter().filter(|p| p.params.engine() == engine) {
            ui.horizontal(|ui| {
                if ui.button(&p.name).clicked() {
                    apply = Some(p.params.clone());
                }
                if ui.small_button(icon::TRASH).on_hover_text("Delete preset").clicked() {
                    delete = Some(p.name.clone());
                }
            });
        }
        ui.separator();
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.preset_name).desired_width(150.0));
            if ui.add_enabled(self.doc().is_some(), egui::Button::new(format!("{} Save current", icon::FLOPPY_DISK))).clicked() {
                let d = self.doc().unwrap();
                let params = match engine {
                    EngineId::SignatureInk => EngineParams::Signature(d.sig.clone()),
                    EngineId::HopeNoise | EngineId::HopeStyle | EngineId::HopePoison => EngineParams::Hope(d.hope_params(engine).clone()),
                    _ => EngineParams::GridNoise(d.gn.clone()),
                };
                let p = Preset { name: self.preset_name.clone(), description: "User preset".into(), builtin: false, params };
                match presets::save_user(&p) {
                    Ok(()) => {
                        self.user_presets = presets::load_user();
                        self.info(format!("Preset '{}' saved", p.name));
                    }
                    Err(e) => self.error(format!("Could not save preset: {e}")),
                }
            }
        });
        if let Some(n) = delete {
            let _ = presets::delete_user(&n);
            self.user_presets = presets::load_user();
        }
        if let Some(p) = apply {
            if let Some(d) = self.doc_mut() {
                match p {
                    EngineParams::GridNoise(mut g) => {
                        g.region = d.gn.region.clone();
                        d.gn = g;
                    }
                    EngineParams::Signature(s) => d.sig = s,
                    EngineParams::Hope(h) => match h.algorithm {
                        hope_engine::HopeAlgorithm::Style => d.style = h,
                        hope_engine::HopeAlgorithm::Poison => d.poison = h,
                        hope_engine::HopeAlgorithm::Noise => d.hope = h,
                    },
                }
            }
        }
    }

    fn central(&mut self, ctx: &egui::Context) {
        let drop_hover = ctx.input(|i| !i.raw.hovered_files.is_empty());
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ctx, |ui| {
            // View-mode strip.
            egui::TopBottomPanel::top("viewbar").frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(8, 4)).fill(ui.visuals().panel_fill)).show_inside(ui, |ui| {
                ui.horizontal(|ui| {
                    for m in ViewMode::ALL {
                        if (m == ViewMode::Average && self.engine != EngineId::GridNoise) || (m == ViewMode::Difference && self.engine == EngineId::GridNoise) {
                            continue;
                        }
                        ui.selectable_value(&mut self.view, m, m.label());
                    }
                    if self.view == ViewMode::Difference && self.engine != EngineId::GridNoise {
                        ui.add(egui::Slider::new(&mut self.canvas.diff_gain, 1.0..=50.0).text("× amplify").logarithmic(true))
                            .on_hover_text("Shows |protected − original| per channel, multiplied by this factor. Black = unchanged.");
                        let previewing = is_hope(self.engine) && self.hope_preview != panels::HopePreview::Off;
                        if let Some((_, _, _, max, psnr)) = self.doc().filter(|d| previewing || d.still_for(self.engine).is_some()).and_then(|d| d.tex_diff.as_ref()) {
                            let what = if previewing { "Simulated" } else { "Measured" };
                            ui.label(RichText::new(format!("{what}: max change {max}/255 · PSNR {psnr:.1} dB")).small());
                        }
                    }
                    if self.view == ViewMode::Overlay {
                        ui.add(egui::Slider::new(&mut self.canvas.overlay_alpha, 0.0..=1.0).text("opacity"));
                    }
                    if self.view == ViewMode::Average {
                        if let Some(a) = self.doc().and_then(|d| d.analysis.clone()) {
                            ui.separator();
                            ui.label(RichText::new(format!(
                                "Measured over {} frames: average PSNR {:.1} dB · single frame {:.1} dB · {:.0}% of pixels changed per frame",
                                a.frames, a.average_psnr_db, a.single_frame_psnr_db, a.scrambled_fraction * 100.0
                            )).small())
                            .on_hover_text("Fidelity statistics against the source at preview size (includes blur, warp, overlays and the visible mark). Higher average PSNR = viewers see your artwork more faithfully; lower single-frame PSNR = each frame is more scrambled. Not a measure of AI resistance.");
                        }
                    }
                });
            });
            // Timeline.
            if self.engine == EngineId::GridNoise && self.doc().is_some() {
                egui::TopBottomPanel::bottom("timeline").frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(8, 4)).fill(ui.visuals().panel_fill)).show_inside(ui, |ui| {
                    self.timeline(ui);
                });
            }
            let (img_size, grid, regions, inside) = match self.doc() {
                Some(d) => {
                    let (c, r) = gridnoise::render::grid_dims(d.source.width() as usize, d.source.height() as usize, d.gn.cells);
                    ([d.source.width(), d.source.height()], (d.gn.shape == gridnoise::CellShape::Square).then_some((c, r)), d.gn.region.polygons.clone(), d.gn.region.apply_inside)
                }
                None => ([0, 0], None, vec![], true),
            };
            let previewing = is_hope(self.engine) && self.hope_preview != panels::HopePreview::Off;
            let (orig, res, diff) = self
                .doc()
                .map(|d| {
                    let has_diff = d.still_for(self.engine).is_some() || previewing;
                    (d.tex_orig.clone(), d.tex_result.clone(), if has_diff { d.tex_diff.as_ref().map(|t| t.0.clone()) } else { None })
                })
                .unwrap_or((None, None, None));
            let view = match (self.engine, self.view) {
                (EngineId::GridNoise, ViewMode::Difference) => ViewMode::Result,
                (e, ViewMode::Average) if e != EngineId::GridNoise => ViewMode::Result,
                (_, v) => v,
            };
            let (color, opacity, show_overlay) = (self.settings.selection_color, self.settings.selection_opacity, self.show_sel_overlay);
            let overlay = if show_overlay { self.doc_mut().and_then(|d| d.selection_overlay(ui.ctx(), color, opacity)) } else { None };
            let brush_radius = self.settings.brush_size / 2.0;
            let out = self.canvas.show(
                ui,
                canvas::Inputs {
                    image_size: img_size,
                    original: orig.as_ref(),
                    result: res.as_ref(),
                    diff: diff.as_ref(),
                    mode: view,
                    tool: self.tool,
                    grid,
                    regions: &regions,
                    regions_inside: inside,
                    drop_hover,
                    selection_overlay: overlay.as_ref(),
                    brush_radius,
                },
            );
            self.hover = out.hover_px;
            if !out.sel_edits.is_empty() {
                if let Some(d) = self.doc_mut() {
                    for e in &out.sel_edits {
                        d.edit_selection(e, brush_radius);
                    }
                }
            }
        });
    }

    fn timeline(&mut self, ui: &mut egui::Ui) {
        let (total, fps, cycle) = self.doc().map(|d| (d.gn.frame_count(), d.gn.animation.fps, (d.gn.balance == Balance::Cycle).then_some(d.gn.cycle_frames))).unwrap_or((1, 30, None));
        ui.horizontal(|ui| {
            let play = if self.playing { icon::PAUSE } else { icon::PLAY };
            if ui.button(RichText::new(play).size(15.0)).on_hover_text("Play / pause (Space)").clicked() {
                self.playing = !self.playing;
            }
            if ui.button(icon::SKIP_BACK).on_hover_text("First frame").clicked() {
                self.frame = 0;
            }
            if ui.button(icon::CARET_LEFT).on_hover_text("Previous frame").clicked() {
                self.frame = (self.frame + total - 1) % total;
            }
            if ui.button(icon::CARET_RIGHT).on_hover_text("Next frame").clicked() {
                self.frame = (self.frame + 1) % total;
            }
            let mut f = self.frame.min(total - 1);
            let w = (ui.available_width() - 330.0).max(120.0);
            ui.spacing_mut().slider_width = w;
            if ui.add(egui::Slider::new(&mut f, 0..=total.saturating_sub(1)).show_value(false)).changed() {
                self.frame = f;
                self.playing = false;
            }
            ui.label(RichText::new(format!("Frame {} / {total} · {:.2} s", f + 1, f as f32 / fps.max(1) as f32)).monospace());
            if let Some(c) = cycle {
                ui.label(RichText::new(format!("cycle {}/{c}", f % c + 1)).small().weak());
            }
            ui.label(RichText::new(format!("{:.0} ms/frame", self.preview_ms)).small().weak()).on_hover_text("Preview render time at preview size");
        });
    }

    fn bottom_dock(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("dock").resizable(true).default_height(120.0).height_range(80.0..=420.0).show(ctx, |ui| {
            ui.horizontal(|ui| {
                let running = self.tasks.iter().filter(|t| t.lock().unwrap().is_running()).count();
                ui.selectable_value(&mut self.bottom_tab, BottomTab::Queue, format!("Queue ({running})"));
                ui.selectable_value(&mut self.bottom_tab, BottomTab::History, "History");
                ui.selectable_value(&mut self.bottom_tab, BottomTab::Log, format!("Log ({})", self.log.len()));
            });
            ui.separator();
            egui::ScrollArea::vertical().auto_shrink([false; 2]).stick_to_bottom(self.bottom_tab == BottomTab::Log).show(ui, |ui| match self.bottom_tab {
                BottomTab::Queue => self.queue_view(ui, true),
                BottomTab::History => self.queue_view(ui, false),
                BottomTab::Log => {
                    for (t, c, m) in &self.log {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(RichText::new(format!("+{:>7.1}s", t.duration_since(self.epoch).as_secs_f32())).monospace().weak());
                            ui.label(RichText::new(m).color(*c));
                        });
                    }
                }
            });
        });
    }

    fn queue_view(&mut self, ui: &mut egui::Ui, running: bool) {
        let mut retry: Option<usize> = None;
        let mut remove: Option<usize> = None;
        let list: Vec<usize> = (0..self.tasks.len()).rev().filter(|&i| self.tasks[i].lock().unwrap().is_running() == running).collect();
        if list.is_empty() {
            ui.label(RichText::new(if running { "No jobs running. Exports and batches appear here." } else { "Finished jobs appear here." }).weak());
        }
        for i in list {
            let t = self.tasks[i].lock().unwrap();
            ui.horizontal(|ui| {
                ui.label(RichText::new(&t.title).strong());
                ui.label(RichText::new(engines::info(t.engine).short).small().color(theme::ACCENT));
                if let Some(s) = t.finished {
                    let failed = t.failed_count();
                    let done = t.items.iter().filter(|i| i.status == Status::Done).count();
                    ui.label(RichText::new(format!("{done} done · {failed} failed · {}", util::human_duration(s))).small());
                    if failed > 0 && matches!(t.work, Work::Batch { .. }) && ui.small_button("Retry failed").clicked() {
                        retry = Some(i);
                    }
                    if ui.small_button(icon::X).on_hover_text("Remove from history").clicked() {
                        remove = Some(i);
                    }
                } else {
                    ui.add(egui::ProgressBar::new(t.progress()).desired_width(160.0).show_percentage());
                    ui.label(RichText::new(util::human_duration(t.started.elapsed().as_secs_f64())).small().weak());
                    if ui.small_button(format!("{} Cancel", icon::STOP)).clicked() {
                        t.cancel.cancel();
                    }
                }
            });
            for it in &t.items {
                ui.horizontal(|ui| {
                    ui.add_space(14.0);
                    let (ic, col) = match &it.status {
                        Status::Queued => (icon::CLOCK, Color32::GRAY),
                        Status::Running => (icon::SPINNER, theme::ACCENT),
                        Status::Done => (icon::CHECK_CIRCLE, theme::OK),
                        Status::Failed(_) => (icon::X_CIRCLE, theme::BAD),
                        Status::Cancelled => (icon::MINUS_CIRCLE, theme::WARN),
                    };
                    ui.label(RichText::new(ic).color(col));
                    ui.label(&it.label);
                    match &it.status {
                        Status::Running => {
                            ui.add(egui::ProgressBar::new(it.progress).desired_width(120.0));
                            ui.label(RichText::new(&it.message).small().weak());
                        }
                        Status::Failed(e) => {
                            ui.label(RichText::new(e).small().color(theme::BAD));
                        }
                        Status::Done => {
                            ui.label(RichText::new(format!("{} · {}", it.message, util::human_duration(it.seconds))).small().weak());
                            if let Some(o) = &it.output {
                                if ui.small_button(icon::FOLDER_OPEN).on_hover_text("Show in folder").clicked() {
                                    reveal(o);
                                }
                            }
                        }
                        _ => {}
                    }
                });
            }
            ui.separator();
        }
        if let Some(i) = retry {
            let t = tasks::retry_failed(&self.tasks[i].lock().unwrap(), ui.ctx().clone());
            if let Some(t) = t {
                self.tasks.push(t);
                self.bottom_tab = BottomTab::Queue;
            }
        }
        if let Some(i) = remove {
            self.tasks.remove(i);
        }
    }

    fn status_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("status").exact_height(22.0).show(ctx, |ui| {
            ui.horizontal_centered(|ui| {
                let small = |s: String| RichText::new(s).small();
                if let Some(d) = self.doc() {
                    ui.label(small(format!("{}×{} px", d.source.width(), d.source.height())));
                    ui.separator();
                    ui.label(small(format!("{} · {}{}", d.source.info.color_type, d.source.info.format, if d.source.info.uses_transparency { " · alpha" } else { "" })));
                    ui.separator();
                    ui.label(small(format!("{:.0}%", self.canvas.zoom_percent(ctx))));
                    ui.separator();
                    match d.selected_pct() {
                        Some(p) => ui.label(small(format!("{} Selection {p:.1}%", icon::SELECTION))).on_hover_text("Protection applies only inside the selection. Ctrl+D to deselect, Ctrl+Shift+I to invert."),
                        None => ui.label(small("Whole image".into())).on_hover_text("No selection: protection applies to the whole image. Use M, L or B to select an area (optional)."),
                    };
                    if let Some((x, y)) = self.hover {
                        ui.separator();
                        let p = d.source.pixels.get_pixel(x, y);
                        ui.label(small(format!("x{x} y{y}  RGBA {} {} {} {}", p[0], p[1], p[2], p[3]))).on_hover_text("Source pixel under the cursor");
                    }
                } else {
                    ui.label(small("No image".into()));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(small(format!("{} CPU · {} threads", icon::CPU, self.hardware.cores))).on_hover_text(format!("{}\nAll processing in this build runs on the CPU; the GPU is used only to draw the interface.", self.hardware.cpu));
                    ui.separator();
                    ui.label(small(format!("Engine: {}", engines::info(self.engine).name)));
                    let running: Vec<f32> = self.tasks.iter().filter_map(|t| {
                        let t = t.lock().unwrap();
                        t.is_running().then(|| t.progress())
                    }).collect();
                    if !running.is_empty() {
                        ui.separator();
                        let avg = running.iter().sum::<f32>() / running.len() as f32;
                        ui.add(egui::ProgressBar::new(avg).desired_width(100.0).desired_height(12.0));
                    }
                });
            });
        });
    }

    // ───────── dialogs ─────────
    fn about_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_about;
        egui::Window::new("About ShieldCraft").open(&mut open).collapsible(false).resizable(false).default_width(460.0).show(ctx, |ui| {
            ui.label(RichText::new("ShieldCraft").heading());
            ui.label(format!("Version {} ({} build)", shieldcraft_core::APP_VERSION, shieldcraft_core::BUILD_PROFILE));
            ui.label("A local, private artwork-protection workstation. No accounts, no telemetry, no uploads.");
            ui.add_space(6.0);
            ui.label(RichText::new("Credits").strong());
            ui.label("Grid Noise Studio: techniques by stelmosfire13-alt (Grid Noise Animator), independently implemented.");
            ui.label("Signature Ink: blind_watermark by naganohara-yoshino, Hope:RE configuration by HopeArtOrg.");
            ui.label("UI: egui by Emil Ernerfeldt and contributors; Phosphor Icons.");
            ui.add_space(6.0);
            ui.label(RichText::new("No protection method can guarantee that artwork will not be used for AI training, copied or imitated.").small().italics());
            if ui.button("Third-Party Licenses & Credits…").clicked() {
                self.licenses.open = true;
            }
        });
        self.show_about = open;
    }

    fn prefs_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_prefs;
        let mut changed = false;
        egui::Window::new(format!("{} Preferences", icon::GEAR)).open(&mut open).collapsible(false).default_width(460.0).show(ctx, |ui| {
            egui::Grid::new("prefs").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                ui.label("Theme");
                ui.horizontal(|ui| {
                    changed |= ui.radio_value(&mut self.settings.theme, Theme::Dark, "Dark").changed();
                    changed |= ui.radio_value(&mut self.settings.theme, Theme::Light, "Light").changed();
                });
                ui.end_row();
                ui.label("Interface scale");
                changed |= ui.add(egui::Slider::new(&mut self.settings.ui_scale, 0.75..=2.0)).drag_stopped();
                ui.end_row();
                ui.label("Preview size");
                ui.add(egui::Slider::new(&mut self.settings.preview_max_side, 480..=2048).suffix(" px")).on_hover_text("Longest side of the live animation preview");
                ui.end_row();
                ui.label("Welcome screen");
                ui.checkbox(&mut self.settings.show_welcome, "Show at startup");
                ui.end_row();
                ui.label("FFmpeg");
                ui.vertical(|ui| {
                    ui.label(RichText::new(self.ffmpeg.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "Not found".into())).small());
                    ui.horizontal(|ui| {
                        if ui.button("Choose ffmpeg.exe…").clicked() {
                            if let Some(f) = rfd::FileDialog::new().add_filter("ffmpeg", &["exe"]).pick_file() {
                                self.settings.ffmpeg_path = Some(f.display().to_string());
                                self.ffmpeg = gridnoise::find_ffmpeg(Some(&f));
                            }
                        }
                        if ui.button("Re-detect").clicked() {
                            self.ffmpeg = gridnoise::find_ffmpeg(self.settings.ffmpeg_path.as_deref().map(Path::new));
                        }
                    });
                });
                ui.end_row();
            });
            ui.add_space(4.0);
            ui.label(RichText::new("Selection").strong());
            egui::Grid::new("prefs_sel").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                ui.label("Brush size");
                changed |= ui.add(egui::Slider::new(&mut self.settings.brush_size, 2.0..=2000.0).logarithmic(true).suffix(" px")).on_hover_text("Diameter in image pixels. [ and ] change it while painting.").drag_stopped();
                ui.end_row();
                ui.label("Edge feather");
                changed |= ui.add(egui::Slider::new(&mut self.settings.selection_feather, 0.0..=64.0).suffix(" px")).on_hover_text("Softens the selection edge so protected and untouched areas blend.").drag_stopped();
                ui.end_row();
                ui.label("Overlay colour");
                ui.horizontal(|ui| {
                    changed |= ui.color_edit_button_srgb(&mut self.settings.selection_color).changed();
                    ui.label(RichText::new("tints the area that will NOT be protected").small().weak());
                });
                ui.end_row();
                ui.label("Overlay opacity");
                changed |= ui.add(egui::Slider::new(&mut self.settings.selection_opacity, 0.0..=0.9)).drag_stopped();
                ui.end_row();
            });
            ui.separator();
            if ui.button("Save").clicked() {
                for d in self.docs.iter_mut() {
                    d.rebuild_preview_source(self.settings.preview_max_side);
                }
                let _ = self.settings.save();
            }
        });
        if changed {
            theme::apply(ctx, self.settings.theme, self.settings.ui_scale);
            let _ = self.settings.save();
        }
        self.show_prefs = open;
    }

    fn shortcuts_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_shortcuts;
        egui::Window::new(format!("{} Keyboard Shortcuts", icon::KEYBOARD)).open(&mut open).collapsible(false).default_width(420.0).show(ctx, |ui| {
            egui::ScrollArea::vertical().max_height(520.0).show(ui, |ui| {
                for (group, rows) in [
                    ("File", &[("Ctrl+O", "Import artwork"), ("Ctrl+S / Ctrl+Shift+S", "Save project / Save as"), ("Ctrl+E", "Export still"), ("Ctrl+Shift+E", "Export animation"), ("Ctrl+W", "Close image")][..]),
                    ("Edit", &[("Ctrl+Z", "Undo"), ("Ctrl+Y / Ctrl+Shift+Z", "Redo")][..]),
                    (
                        "Selection (optional)",
                        &[
                            ("M", "Rectangle select"),
                            ("Shift+M", "Switch rectangle / ellipse"),
                            ("L", "Lasso select"),
                            ("B", "Selection brush"),
                            ("[  /  ]", "Smaller / larger brush"),
                            ("Shift + drag", "Add to selection"),
                            ("Alt + drag", "Subtract from selection"),
                            ("Ctrl+A", "Select all"),
                            ("Ctrl+D", "Deselect (protect whole image)"),
                            ("Ctrl+Shift+I", "Invert selection"),
                            ("Ctrl+H", "Show / hide selection overlay"),
                        ][..],
                    ),
                    ("View", &[("H / Space", "Hand (pan)"), ("Z", "Zoom tool"), ("P", "Inspect pixels"), ("Ctrl+0", "Fit on screen"), ("Ctrl+1", "Actual pixels"), ("Ctrl+ + / Ctrl+ −", "Zoom in / out"), ("G", "Grid overlay"), ("Tab", "Toggle panels"), ("Space / Ctrl+Enter", "Play / pause animation"), ("F1", "This window")][..]),
                ] {
                    ui.label(RichText::new(group).strong());
                    egui::Grid::new(group).num_columns(2).spacing([18.0, 3.0]).striped(true).show(ui, |ui| {
                        for (k, what) in rows {
                            ui.label(RichText::new(*k).monospace());
                            ui.label(*what);
                            ui.end_row();
                        }
                    });
                    ui.add_space(6.0);
                }
            });
        });
        self.show_shortcuts = open;
    }

    fn info_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_info;
        let Some(d) = self.doc() else {
            self.show_info = false;
            return;
        };
        let i = d.source.info.clone();
        let (icc, exif) = (d.source.icc_profile.as_ref().map(|v| v.len()), d.source.exif.as_ref().map(|v| v.len()));
        egui::Window::new("Image Information").open(&mut open).collapsible(false).show(ctx, |ui| {
            egui::Grid::new("info").num_columns(2).striped(true).show(ui, |ui| {
                for (k, v) in [
                    ("File", d.source.name.clone()),
                    ("Format", i.format.clone()),
                    ("Dimensions", format!("{} × {}", i.width, i.height)),
                    ("Colour", i.color_type.clone()),
                    ("Alpha channel", format!("{} ({})", i.has_alpha_channel, if i.uses_transparency { "transparency used" } else { "fully opaque" })),
                    ("File size", util::human_bytes(i.file_size)),
                    ("ICC profile", icc.map(|n| format!("{n} bytes (kept on export by default)")).unwrap_or_else(|| "none".into())),
                    ("EXIF", exif.map(|n| format!("{n} bytes (stripped on export by default)")).unwrap_or_else(|| "none".into())),
                ] {
                    ui.label(RichText::new(k).weak());
                    ui.label(v);
                    ui.end_row();
                }
            });
            if !i.transformations.is_empty() {
                ui.separator();
                ui.label(RichText::new("Transformations applied").strong());
                for t in &i.transformations {
                    ui.label(format!("• {t}"));
                }
            }
        });
        self.show_info = open;
    }

    fn batch_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_batch;
        let mut start = false;
        egui::Window::new(format!("{} Batch Process", icon::STACK)).open(&mut open).default_size([560.0, 440.0]).show(ctx, |ui| {
            ui.label(RichText::new("Uses the current image's settings for the chosen method. Each file is processed independently; failures do not stop the batch. Originals are never modified.").small().weak());
            ui.horizontal(|ui| {
                if ui.button(format!("{} Add files…", icon::FILE_PLUS)).clicked() {
                    if let Some(f) = rfd::FileDialog::new().add_filter("Images", image_io::SUPPORTED_IMPORT_EXT).pick_files() {
                        for p in f {
                            if !self.batch_files.contains(&p) {
                                self.batch_files.push(p);
                            }
                        }
                    }
                }
                if ui.button(format!("{} Add folder…", icon::FOLDER_PLUS)).clicked() {
                    if let Some(d) = rfd::FileDialog::new().pick_folder() {
                        self.add_batch_folder(&d);
                    }
                }
                if ui.button("Clear").clicked() {
                    self.batch_files.clear();
                }
                ui.label(format!("{} file(s)", self.batch_files.len()));
            });
            egui::ScrollArea::vertical().max_height(160.0).auto_shrink([false, true]).show(ui, |ui| {
                let mut rm = None;
                for (i, f) in self.batch_files.iter().enumerate() {
                    ui.horizontal(|ui| {
                        if ui.small_button(icon::X).clicked() {
                            rm = Some(i);
                        }
                        ui.label(RichText::new(f.display().to_string()).small());
                    });
                }
                if let Some(i) = rm {
                    self.batch_files.remove(i);
                }
            });
            ui.separator();
            ui.horizontal_wrapped(|ui| {
                ui.label("Method");
                for m in [BatchMode::Animation, BatchMode::StaticStill, BatchMode::HopeNoise, BatchMode::HopeStyle, BatchMode::HopePoison, BatchMode::Signature] {
                    ui.radio_value(&mut self.batch_mode, m, m.label());
                }
            });
            ui.horizontal(|ui| {
                ui.label("Output folder");
                ui.label(RichText::new(self.batch_out.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "not chosen".into())).small());
                if ui.button("Choose…").clicked() {
                    self.batch_out = rfd::FileDialog::new().pick_folder();
                }
            });
            let params_ok = self.doc().is_some() || self.batch_mode == BatchMode::Signature;
            if !params_ok {
                ui.label(RichText::new("Open an image first: its settings are used for the batch (or default settings will be used).").small().color(theme::WARN));
            }
            ui.label(RichText::new(format!(
                "Output: {} · suffix '{}' · {}",
                if self.batch_mode == BatchMode::Animation { self.doc().map(|d| d.gn.animation.format.label()).unwrap_or("GIF") } else { self.settings.export.format.label() },
                self.settings.export.suffix,
                if self.settings.export.write_report { "with reports" } else { "no reports" }
            )).small());
            start = ui.add_enabled(!self.batch_files.is_empty() && self.batch_out.is_some(), egui::Button::new(RichText::new(format!("{} Start batch", icon::PLAY)).strong())).clicked();
        });
        self.show_batch = open;
        if start {
            let gn = self.doc().map(|d| d.gn.clone()).unwrap_or_default();
            let sig = self.doc().map(|d| d.sig.clone()).unwrap_or_default();
            let engine = match self.batch_mode {
                BatchMode::Signature => EngineId::SignatureInk,
                BatchMode::HopeNoise => EngineId::HopeNoise,
                BatchMode::HopeStyle => EngineId::HopeStyle,
                BatchMode::HopePoison => EngineId::HopePoison,
                _ => EngineId::GridNoise,
            };
            let hope = self.doc().map(|d| d.hope_params(engine).clone()).unwrap_or_else(|| match engine {
                EngineId::HopeStyle => default_style_params(),
                EngineId::HopePoison => default_poison_params(),
                _ => HopeParams::default(),
            });
            if is_hope(engine) && !engines::availability(engine).is_available() {
                self.error(format!("{} is not ready (model missing?). See its Properties panel.", engines::info(engine).name));
                return;
            }
            let work = Work::Batch { files: self.batch_files.clone(), mode: self.batch_mode, gn, sig, hope, out_dir: self.batch_out.clone().unwrap(), opts: self.settings.export.clone(), ffmpeg: self.ffmpeg.clone() };
            let title = format!("Batch · {} · {} file(s)", self.batch_mode.label(), self.batch_files.len());
            self.tasks.push(tasks::spawn(title, engine, work, ctx.clone()));
            self.bottom_tab = BottomTab::Queue;
            self.show_bottom = true;
            self.show_batch = false;
        }
    }
}

#[derive(PartialEq, Clone, Copy)]
enum WelcomeAction {
    Continue,
    Licenses,
}

/// Bytes of selection masks held by an undo history, counting each shared mask once.
fn undo_mask_bytes(undo: &[Snapshot]) -> usize {
    let mut seen = std::collections::HashSet::new();
    undo.iter().filter_map(|s| s.5.mask.as_ref()).filter(|m| seen.insert(Arc::as_ptr(m))).map(|m| m.as_raw().len()).sum()
}

fn engines_tab(ui: &mut egui::Ui, hw: &Hardware, ffmpeg: Option<&Path>) {
    theme::caption(ui, "Hardware");
    ui.label(RichText::new(format!("{}\n{} threads · {:.0} GB RAM · {}", hw.cpu, hw.cores, hw.ram_gb, hw.os)).small());
    ui.label(RichText::new("All engines in this build run on the CPU.").small().weak());
    ui.label(RichText::new(format!("FFmpeg: {}", ffmpeg.map(|p| p.display().to_string()).unwrap_or_else(|| "not found (MP4/WebM disabled)".into()))).small());
    ui.add_space(6.0);
    theme::caption(ui, "Engines");
    for id in EngineId::ALL {
        let info = engines::info(id);
        let a = engines::availability(id);
        let col = match &a {
            Availability::Available { .. } => theme::OK,
            _ => theme::WARN,
        };
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new("●").color(col));
                ui.label(RichText::new(info.name).strong());
                ui.label(RichText::new(a.label()).small().color(col));
            });
            ui.label(RichText::new(info.kind.label()).small().color(theme::ACCENT));
            ui.label(RichText::new(format!("By {} · {}", info.credit, info.license)).small());
            ui.label(RichText::new(a.detail()).small().weak());
            ui.hyperlink_to(RichText::new(info.repo_url).small(), info.repo_url);
        });
    }
}

fn draw_welcome_art(p: &egui::Painter, r: egui::Rect, t: f32) {
    // Original procedural artwork: a field of cells whose colours drift like temporal grid noise.
    let cols = 10;
    let rows = 16;
    let cw = r.width() / cols as f32;
    let ch = r.height() / rows as f32;
    for y in 0..rows {
        for x in 0..cols {
            let k = util::hash_keys(&[x as u64, y as u64]);
            let base = util::unit_f32(k);
            let phase = base * std::f32::consts::TAU + t * (0.4 + 0.6 * util::unit_f32(k >> 7));
            let hue = 200.0 + 60.0 * phase.sin() + 40.0 * (y as f32 / rows as f32);
            let light = 22.0 + 10.0 * (phase * 0.7).cos() + 6.0 * (x as f32 / cols as f32);
            let c = shieldcraft_core::imgproc::hsl_to_rgb(hue, 55.0, light);
            let rect = egui::Rect::from_min_size(egui::pos2(r.left() + x as f32 * cw, r.top() + y as f32 * ch), egui::vec2(cw - 1.0, ch - 1.0));
            p.rect_filled(rect, 1.0, Color32::from_rgb(c[0] as u8, c[1] as u8, c[2] as u8));
        }
    }
    p.rect_filled(egui::Rect::from_min_max(egui::pos2(r.right() - 40.0, r.top()), r.right_bottom()), 0.0, Color32::from_rgba_unmultiplied(22, 23, 27, 160));
}

fn open_in_explorer(p: &Path) {
    let _ = std::process::Command::new("explorer").arg(p).spawn();
}

fn reveal(p: &Path) {
    let _ = std::process::Command::new("explorer").arg(format!("/select,{}", p.display())).spawn();
}
