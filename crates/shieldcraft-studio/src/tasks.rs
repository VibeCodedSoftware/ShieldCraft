//! Background work (exports and batches). Each task runs on its own thread and shares its
//! state through a mutex the UI reads every frame. A failing batch item is recorded and the
//! batch carries on; failed items can be retried.

use shieldcraft_core::engine::{CancelToken, EngineError, EngineId};
use shieldcraft_core::engines::gridnoise::{self, GridNoiseParams, Panel, StillMode};
use shieldcraft_core::engines::hope::HopeParams;
use shieldcraft_core::selection::Mask;
use shieldcraft_core::engines::{signature, EngineParams};
use shieldcraft_core::image_io::{self, ExportOptions, SourceImage};
use shieldcraft_core::report::ProcessingReport;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Queued,
    Running,
    Done,
    Failed(String),
    Cancelled,
}

#[derive(Clone)]
pub struct Item {
    pub label: String,
    pub input: Option<PathBuf>,
    pub status: Status,
    pub progress: f32,
    pub message: String,
    pub output: Option<PathBuf>,
    pub seconds: f64,
}

pub struct Task {
    pub title: String,
    pub engine: EngineId,
    pub items: Vec<Item>,
    pub cancel: CancelToken,
    pub started: Instant,
    pub finished: Option<f64>,
    pub work: Work,
}

impl Task {
    pub fn progress(&self) -> f32 {
        if self.items.is_empty() {
            return 0.0;
        }
        self.items.iter().map(|i| if matches!(i.status, Status::Done | Status::Failed(_) | Status::Cancelled) { 1.0 } else { i.progress }).sum::<f32>() / self.items.len() as f32
    }
    pub fn is_running(&self) -> bool {
        self.finished.is_none()
    }
    pub fn failed_count(&self) -> usize {
        self.items.iter().filter(|i| matches!(i.status, Status::Failed(_))).count()
    }
}

pub type Shared = Arc<Mutex<Task>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BatchMode {
    /// Grid Noise animation per image.
    Animation,
    /// Grid Noise static layers as a still per image.
    StaticStill,
    /// Signature Ink per image.
    Signature,
    /// Hope:RE Noise per image (slow: minutes per image on the CPU).
    HopeNoise,
    /// Hope:RE Style Cloak per image (slow).
    HopeStyle,
    /// Hope:RE Concept Poison per image (slow).
    HopePoison,
}

impl BatchMode {
    pub fn label(self) -> &'static str {
        match self {
            BatchMode::Animation => "Grid Noise animation",
            BatchMode::StaticStill => "Grid Noise still (static layers)",
            BatchMode::Signature => "Signature Ink",
            BatchMode::HopeNoise => "Hope:RE Noise",
            BatchMode::HopeStyle => "Hope:RE Style Cloak",
            BatchMode::HopePoison => "Hope:RE Concept Poison",
        }
    }
}

#[derive(Clone)]
pub enum Work {
    Animation { source_name: String, source_size: (u32, u32), panels: Vec<Panel>, params: GridNoiseParams, out: PathBuf, ffmpeg: Option<PathBuf>, write_report: bool },
    Still { source: Arc<SourceImage>, params: EngineParams, mode: StillMode, out: PathBuf, opts: ExportOptions, mask: Option<Arc<Mask>> },
    Batch { files: Vec<PathBuf>, mode: BatchMode, gn: GridNoiseParams, sig: signature::SignatureParams, hope: HopeParams, out_dir: PathBuf, opts: ExportOptions, ffmpeg: Option<PathBuf> },
}

pub fn spawn(title: String, engine: EngineId, work: Work, ctx: egui::Context) -> Shared {
    let items = match &work {
        Work::Animation { source_name, .. } => vec![item(source_name, None)],
        Work::Still { source, .. } => vec![item(&source.name, source.path.clone())],
        Work::Batch { files, .. } => files.iter().map(|f| item(&f.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(), Some(f.clone()))).collect(),
    };
    let task = Arc::new(Mutex::new(Task {
        title,
        engine,
        items,
        cancel: CancelToken::new(),
        started: Instant::now(),
        finished: None,
        work: work.clone(),
    }));
    let shared = task.clone();
    std::thread::spawn(move || {
        run(&shared, work, &ctx);
        let mut t = shared.lock().unwrap();
        t.finished = Some(t.started.elapsed().as_secs_f64());
        drop(t);
        ctx.request_repaint();
    });
    task
}

fn item(label: &str, input: Option<PathBuf>) -> Item {
    Item { label: label.into(), input, status: Status::Queued, progress: 0.0, message: String::new(), output: None, seconds: 0.0 }
}

/// Build a new task that re-runs only the failed items of a finished batch.
pub fn retry_failed(task: &Task, ctx: egui::Context) -> Option<Shared> {
    let Work::Batch { mode, gn, sig, hope, out_dir, opts, ffmpeg, .. } = &task.work else { return None };
    let files: Vec<PathBuf> = task.items.iter().filter(|i| matches!(i.status, Status::Failed(_))).filter_map(|i| i.input.clone()).collect();
    if files.is_empty() {
        return None;
    }
    let work = Work::Batch { files, mode: *mode, gn: gn.clone(), sig: sig.clone(), hope: hope.clone(), out_dir: out_dir.clone(), opts: opts.clone(), ffmpeg: ffmpeg.clone() };
    Some(spawn(format!("Retry: {}", task.title), task.engine, work, ctx))
}

fn run(shared: &Shared, work: Work, ctx: &egui::Context) {
    let cancel = shared.lock().unwrap().cancel.clone();
    let n = shared.lock().unwrap().items.len();
    for idx in 0..n {
        if cancel.is_cancelled() {
            let mut t = shared.lock().unwrap();
            for it in t.items.iter_mut().filter(|i| i.status == Status::Queued) {
                it.status = Status::Cancelled;
            }
            break;
        }
        {
            let mut t = shared.lock().unwrap();
            t.items[idx].status = Status::Running;
        }
        let started = Instant::now();
        let progress = |f: f32, m: &str| {
            let mut t = shared.lock().unwrap();
            t.items[idx].progress = f.clamp(0.0, 1.0);
            t.items[idx].message = m.to_string();
            drop(t);
            ctx.request_repaint();
        };
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run_item(&work, idx, &progress, &cancel)))
            .unwrap_or_else(|_| Err(EngineError::Failed("Internal error while processing (see logs\\crash.log)".into())));
        let mut t = shared.lock().unwrap();
        let it = &mut t.items[idx];
        it.seconds = started.elapsed().as_secs_f64();
        match result {
            Ok(out) => {
                it.status = Status::Done;
                it.progress = 1.0;
                it.message = format!("Saved {}", out.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default());
                it.output = Some(out);
            }
            Err(EngineError::Cancelled) => it.status = Status::Cancelled,
            Err(e) => it.status = Status::Failed(e.to_string()),
        }
        drop(t);
        ctx.request_repaint();
    }
}

fn run_item(work: &Work, idx: usize, progress: &(dyn Fn(f32, &str) + Sync), cancel: &CancelToken) -> Result<PathBuf, EngineError> {
    let t0 = Instant::now();
    match work {
        Work::Animation { source_name, source_size, panels, params, out, ffmpeg, write_report } => {
            let s = gridnoise::export_animation(panels, params, out, ffmpeg.as_deref(), progress, cancel)?;
            if *write_report {
                let mut r = ProcessingReport::new(EngineId::GridNoise, source_name, *source_size, serde_json::to_value(params).unwrap_or_default());
                r.output_size = [s.width, s.height];
                r.seconds = t0.elapsed().as_secs_f64();
                r.execution = format!("CPU, {} frames at {} fps, {} storyboard panel(s)", s.frames, s.fps, panels.len());
                r.warnings = s.warnings.clone();
                r.output_file = out.file_name().map(|n| n.to_string_lossy().to_string());
                write_report_file(out, &r);
            }
            Ok(out.clone())
        }
        Work::Still { source, params, mode, out, opts, mask } => {
            progress(0.01, "Processing…");
            let (img, note) = process_still(source, params, *mode, mask.as_deref(), progress, cancel)?;
            cancel.check()?;
            progress(0.95, "Encoding…");
            save_still(&img, source, params, out, opts, t0, &note)?;
            Ok(out.clone())
        }
        Work::Batch { files, mode, gn, sig, hope, out_dir, opts, ffmpeg } => {
            let path = &files[idx];
            progress(0.02, "Loading…");
            let src = image_io::load_image(path).map_err(|e| EngineError::Failed(e.to_string()))?;
            let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "image".into());
            match mode {
                BatchMode::Animation => {
                    let mut p = gn.clone();
                    if p.animation.format.needs_ffmpeg() && ffmpeg.is_none() {
                        return Err(EngineError::Unavailable("FFmpeg not found for MP4/WebM".into()));
                    }
                    let out = if p.animation.format == gridnoise::AnimFormat::PngSequence {
                        image_io::plan_output_path(out_dir, &stem, &format!("{}_frames", opts.suffix), "", false).with_extension("")
                    } else {
                        image_io::plan_output_path(out_dir, &stem, &opts.suffix, p.animation.format.ext(), opts.allow_overwrite)
                    };
                    p.seed = p.seed.wrapping_add(idx as u64);
                    let w = Work::Animation {
                        source_name: src.name.clone(),
                        source_size: (src.width(), src.height()),
                        panels: vec![Panel { image: src.pixels.clone(), duration_s: p.animation.duration_s, mask: None }],
                        params: p,
                        out,
                        ffmpeg: ffmpeg.clone(),
                        write_report: opts.write_report,
                    };
                    run_item(&w, 0, progress, cancel)
                }
                BatchMode::StaticStill | BatchMode::Signature | BatchMode::HopeNoise | BatchMode::HopeStyle | BatchMode::HopePoison => {
                    let params = match mode {
                        BatchMode::Signature => EngineParams::Signature(sig.clone()),
                        BatchMode::HopeNoise | BatchMode::HopeStyle | BatchMode::HopePoison => EngineParams::Hope(hope.clone()),
                        _ => EngineParams::GridNoise(gn.clone()),
                    };
                    let src = Arc::new(src);
                    progress(0.02, "Processing…");
                    let (img, note) = process_still(&src, &params, StillMode::StaticLayers, None, progress, cancel)?;
                    cancel.check()?;
                    let out = image_io::plan_output_path(out_dir, &stem, &opts.suffix, opts.format.ext(), opts.allow_overwrite);
                    progress(0.95, "Encoding…");
                    save_still(&img, &src, &params, &out, opts, t0, &note)?;
                    Ok(out)
                }
            }
        }
    }
}

/// Run a still-image engine. Returns the image and a note describing how it was executed.
/// Run a still-image engine. Returns the image and a note describing how it was executed.
/// With a selection `mask`, only the selected area is changed (Signature Ink ignores it).
pub fn process_still(
    src: &SourceImage,
    params: &EngineParams,
    mode: StillMode,
    mask: Option<&Mask>,
    progress: &(dyn Fn(f32, &str) + Sync),
    cancel: &CancelToken,
) -> Result<(image::RgbaImage, String), EngineError> {
    let sel_note = |note: String| match mask {
        Some(m) => format!("{note} · applied to the selection ({:.0}% of the image)", shieldcraft_core::selection::selected_fraction(m) * 100.0),
        None => note,
    };
    match params {
        EngineParams::GridNoise(p) => {
            p.validate().map_err(EngineError::InvalidParams)?;
            let mut img = gridnoise::render_still(&src.pixels, p, mode);
            if let Some(m) = mask {
                shieldcraft_core::selection::composite(&src.pixels, &mut img, m);
            }
            Ok((img, sel_note("CPU".into())))
        }
        EngineParams::Signature(p) => signature::embed(&src.pixels, p).map(|i| (i, "CPU".into())).map_err(EngineError::Failed),
        EngineParams::Hope(p) => {
            let (img, r) = shieldcraft_core::engines::worker::run_hope(&src.pixels, p, mask, &|f, m| progress(0.02 + f * 0.9, m), cancel)?;
            let target = match p.algorithm {
                shieldcraft_core::engines::hope::HopeAlgorithm::Style => {
                    format!("decoy style {} · ", shieldcraft_core::engines::hope::STYLES[p.style_index as usize])
                }
                shieldcraft_core::engines::hope::HopeAlgorithm::Poison => {
                    format!("decoy concept {} · ", shieldcraft_core::engines::hope::TARGETS[p.target_index as usize])
                }
                _ => String::new(),
            };
            let mut note = format!(
                "{target}{} in an isolated worker · {} tiles · {} model evaluations · {:.0} s · surrogate loss per tile {:.4} → {:.4} · max change ±{:.0}/255",
                r.execution_provider, r.tiles, r.evaluations, r.seconds, r.loss_before, r.loss_after, p.budget_255()
            );
            if p.auto_iterate {
                note.push_str(&format!(
                    " · auto-iterate: {} of {} tiles needed extra rounds, {} iterations in total (cap {} per tile)",
                    r.tiles_extended, r.tiles, r.total_iterations, p.max_iterations()
                ));
            }
            if r.loss_after >= r.loss_before {
                note.push_str(" · WARNING: the surrogate loss did not improve, so this result is not effective. Run again with more iterations per tile.");
            } else if r.tiles_unconverged > 0 {
                note.push_str(&format!(
                    " · WARNING: {} of {} tiles still did not improve at the iteration cap. Those areas are not effectively protected; raise the iterations per tile and run again.",
                    r.tiles_unconverged, r.tiles
                ));
            }
            if r.tiles_skipped > 0 {
                note.push_str(&format!(" · {} tiles outside the selection were skipped", r.tiles_skipped));
            }
            Ok((img, sel_note(note)))
        }
    }
}

fn save_still(img: &image::RgbaImage, src: &SourceImage, params: &EngineParams, out: &Path, opts: &ExportOptions, t0: Instant, execution: &str) -> Result<(), EngineError> {
    let (bytes, warnings) = image_io::encode(img, opts, src.icc_profile.as_deref(), src.exif.as_deref()).map_err(EngineError::Failed)?;
    image_io::write_atomic(out, &bytes, src.path.as_deref(), true).map_err(EngineError::Failed)?;
    if opts.side_by_side {
        let sbs = image_io::side_by_side(&src.pixels, img);
        let p = out.with_file_name(format!("{}_compare.png", out.file_stem().unwrap_or_default().to_string_lossy()));
        let (b, _) = image_io::encode(&sbs, &ExportOptions { format: image_io::ExportFormat::Png, ..opts.clone() }, None, None).map_err(EngineError::Failed)?;
        image_io::write_atomic(&p, &b, src.path.as_deref(), true).map_err(EngineError::Failed)?;
    }
    if opts.write_report {
        let mut r = ProcessingReport::new(params.engine(), &src.name, (src.width(), src.height()), serde_json::to_value(params).unwrap_or_default());
        r.output_size = [img.width(), img.height()];
        r.seconds = t0.elapsed().as_secs_f64();
        r.execution = execution.to_string();
        r.warnings = warnings;
        if let EngineParams::Signature(s) = params {
            r.warnings.push(format!("To read the signature back use byte length {} {}", s.text.len(), if s.seed.is_some() { "and the same key" } else { "(no key)" }));
        }
        r.output_file = out.file_name().map(|n| n.to_string_lossy().to_string());
        write_report_file(out, &r);
    }
    Ok(())
}

fn write_report_file(out: &Path, r: &ProcessingReport) {
    let name = format!("{}.report.json", out.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "output".into()));
    let path = out.with_file_name(name);
    if let Ok(b) = serde_json::to_vec_pretty(r) {
        let _ = image_io::write_atomic(&path, &b, None, false);
    }
}

