//! shieldcraft-cli — headless tool used for testing and scripting.
//!
//!   shieldcraft-cli gridnoise <input> <output.gif|.png|.mp4|.webm> [preset-name]
//!   shieldcraft-cli still <input> <output.png>          (static layers, full resolution)
//!   shieldcraft-cli sign <input> <output.png> <text> [key]
//!   shieldcraft-cli verify <input> <byte-length> [key]
//!   shieldcraft-cli analyse <input> [frames]

use shieldcraft_core::engine::CancelToken;
use shieldcraft_core::engines::gridnoise::{self, AnimFormat, GridNoiseParams, Panel, StillMode};
use shieldcraft_core::engines::{signature, EngineParams};
use shieldcraft_core::image_io;
use std::path::Path;
use std::sync::Arc;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Err(e) = run(&args) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn preset_params(name: Option<&String>) -> Result<GridNoiseParams, String> {
    let Some(name) = name else { return Ok(GridNoiseParams::default()) };
    shieldcraft_core::presets::builtin()
        .into_iter()
        .find(|p| p.name.to_lowercase().starts_with(&name.to_lowercase()))
        .and_then(|p| match p.params {
            EngineParams::GridNoise(g) => Some(g),
            _ => None,
        })
        .ok_or_else(|| format!("No Grid Noise preset starting with '{name}'"))
}

fn run(a: &[String]) -> Result<(), String> {
    let arg = |i: usize| a.get(i).ok_or_else(|| "missing argument (run without arguments for help)".to_string());
    match a.first().map(String::as_str) {
        Some("hope-worker") => {
            // Isolated worker for Hope:RE Noise (see engines/worker.rs for the protocol).
            let req_path = a.iter().position(|x| x == "--request").and_then(|i| a.get(i + 1)).ok_or("--request <file> is required")?;
            if let Err(e) = hope_worker(Path::new(req_path)) {
                println!("ERROR {e}");
                std::process::exit(2);
            }
        }
        Some("lantern") => {
            // shieldcraft-cli lantern status
            // shieldcraft-cli lantern lookup <file>          (sends only the SHA-256)
            // shieldcraft-cli lantern check <file>           (UPLOADS the image to Lantern)
            // shieldcraft-cli lantern id <LNTN-xxxxxxxx>
            use shieldcraft_core::lantern;
            let c = lantern::Client::default();
            match a.get(1).map(String::as_str) {
                Some("status") => {
                    let s = c.status().map_err(|e| e.to_string())?;
                    println!("healthy: {}\nhealth: {}\nfeatures: {}", s.healthy, s.health_raw, s.features_raw);
                }
                Some("lookup") => {
                    let bytes = std::fs::read(arg(2)?).map_err(|e| e.to_string())?;
                    let sha = lantern::sha256_bytes(&bytes);
                    println!("sha256 {sha}");
                    match c.lookup_hash(&sha).map_err(|e| e.to_string())? {
                        Some(p) => println!("REGISTERED: {} status={} dispute={} registered_at={} chain_verified={} {}", p.lantern_id, p.status, p.dispute_status, p.registered_at, p.chain.verified, p.verify_url),
                        None => println!("not in the Lantern registry (exact file hash)"),
                    }
                    let o = c.optout(&sha).map_err(|e| e.to_string())?;
                    println!("opt-out registry: {}", if o.found { "listed" } else { "not listed" });
                }
                Some("check") => {
                    let path = Path::new(arg(2)?);
                    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
                    let r = c.check_image(&bytes, &path.file_name().unwrap_or_default().to_string_lossy()).map_err(|e| e.to_string())?;
                    let verdict = if r.inconclusive() { "INCONCLUSIVE" } else if r.no_match() { "no match" } else { "MATCH/candidates" };
                    println!("{verdict}: [{}] {} — {} (match_mode={}, exact={}, similar={})", r.badge_label, r.title, r.helper_text.unwrap_or_default(), r.match_mode, r.exact_found, r.similar_count);
                }
                Some("id") => match c.lookup_id(arg(2)?).map_err(|e| e.to_string())? {
                    Some(p) => println!("{}", serde_json::to_string_pretty(&p).unwrap()),
                    None => println!("unknown Lantern ID"),
                },
                _ => println!("usage: lantern status | lookup <file> | check <file> | id <LNTN-xxxxxxxx>"),
            }
        }
        Some("hope") => {
            // Convenience: shieldcraft-cli hope <in> <out.png> [iterations] [intensity%]
            let src = image_io::load_image(Path::new(arg(1)?)).map_err(|e| e.to_string())?;
            let mut p = shieldcraft_core::engines::hope::HopeParams::default();
            if let Some(i) = a.get(3).and_then(|s| s.parse().ok()) {
                p.iterations = i;
            }
            if let Some(i) = a.get(4).and_then(|s: &String| s.parse::<f32>().ok()) {
                p.intensity = i / 100.0;
            }
            // Optional: noise | style[:index] | poison[:index]
            if let Some(mode) = a.get(5) {
                let (m, idx) = mode.split_once(':').unwrap_or((mode.as_str(), "0"));
                let idx: u32 = idx.parse().unwrap_or(0);
                match m {
                    "style" => {
                        p.algorithm = shieldcraft_core::engines::hope::HopeAlgorithm::Style;
                        p.style_index = idx;
                    }
                    "poison" => {
                        p.algorithm = shieldcraft_core::engines::hope::HopeAlgorithm::Poison;
                        p.target_index = idx;
                    }
                    _ => {}
                }
            }
            if let Some(seed) = a.get(6).and_then(|s| s.parse().ok()) {
                p.seed = seed;
            }
            if a.iter().any(|x| x == "--no-auto") {
                p.auto_iterate = false;
            }
            let mask = match a.iter().position(|x| x == "--mask").and_then(|i| a.get(i + 1)) {
                Some(m) => Some(image::open(m).map_err(|e| format!("mask: {e}"))?.to_luma8()),
                None => None,
            };
            let (img, info) = shieldcraft_core::engines::worker::run_hope(&src.pixels, &p, mask.as_ref(), &|f, m| {
                if ((f * 100.0) as u32).is_multiple_of(10) {
                    eprintln!("{:3.0}% {m}", f * 100.0);
                }
            }, &CancelToken::new())
            .map_err(|e| e.to_string())?;
            save(&img, &src, Path::new(arg(2)?))?;
            println!("{} tiles ({} skipped outside the selection), {} evaluations, {:.1}s on {}", info.tiles, info.tiles_skipped, info.evaluations, info.seconds, info.execution_provider);
            println!("surrogate loss per tile: {:.4} -> {:.4}", info.loss_before, info.loss_after);
            println!("auto-iterate: {} tile(s) extended, {} tile(s) not improved, {} iterations in total", info.tiles_extended, info.tiles_unconverged, info.total_iterations);
            let diff = img.pixels().zip(src.pixels.pixels()).map(|(x, y)| (0..3).map(|c| (x[c] as i32 - y[c] as i32).abs()).max().unwrap()).max().unwrap_or(0);
            println!("max per-channel change: {diff}/255; PSNR vs source: {:.1} dB", image_io::psnr(&img, &src.pixels).unwrap_or(0.0));
        }
        Some("gridnoise") => {
            let src = image_io::load_image(Path::new(arg(1)?)).map_err(|e| e.to_string())?;
            let out = Path::new(arg(2)?);
            let mut p = preset_params(a.get(3))?;
            p.animation.format = match out.extension().and_then(|e| e.to_str()).unwrap_or("") {
                "gif" => AnimFormat::Gif,
                "png" => AnimFormat::Apng,
                "mp4" => AnimFormat::Mp4,
                "webm" => AnimFormat::WebM,
                _ => AnimFormat::PngSequence,
            };
            let ff = gridnoise::find_ffmpeg(None);
            let t = std::time::Instant::now();
            let s = gridnoise::export_animation(
                &[Panel { image: src.pixels.clone(), duration_s: p.animation.duration_s, mask: None }],
                &p,
                out,
                ff.as_deref(),
                &|f, m| {
                    let pct = (f * 100.0) as u32;
                    if (pct.is_multiple_of(25) && m.starts_with("Rendering")) || f >= 1.0 {
                        eprintln!("{pct:3}% {m}");
                    }
                },
                &CancelToken::new(),
            )
            .map_err(|e| e.to_string())?;
            println!("{} frames {}x{} @{}fps -> {} ({} bytes, {:.1}s)", s.frames, s.width, s.height, s.fps, s.path, s.bytes, t.elapsed().as_secs_f32());
            for w in s.warnings {
                println!("warning: {w}");
            }
        }
        Some("still") => {
            let src = image_io::load_image(Path::new(arg(1)?)).map_err(|e| e.to_string())?;
            let mut p = GridNoiseParams::default();
            p.text_tile.enabled = true;
            let img = gridnoise::render_still(&src.pixels, &p, StillMode::StaticLayers);
            save(&img, &src, Path::new(arg(2)?))?;
        }
        Some("sign") => {
            let src = image_io::load_image(Path::new(arg(1)?)).map_err(|e| e.to_string())?;
            let text = arg(3)?.clone();
            let seed = a.get(4).and_then(|s| s.parse().ok());
            let img = signature::embed(&src.pixels, &signature::SignatureParams { text: text.clone(), seed })?;
            save(&img, &src, Path::new(arg(2)?))?;
            println!("signed; read back with byte length {}", text.len());
        }
        Some("verify") => {
            let src = image_io::load_image(Path::new(arg(1)?)).map_err(|e| e.to_string())?;
            let len: usize = arg(2)?.parse().map_err(|_| "byte length must be a number")?;
            let seed = a.get(3).and_then(|s| s.parse().ok());
            println!("{}", signature::extract(&src.pixels, len, seed)?);
        }
        Some("analyse") => {
            let src = image_io::load_image(Path::new(arg(1)?)).map_err(|e| e.to_string())?;
            let frames = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(24);
            let p = GridNoiseParams::default();
            let (w, h) = gridnoise::output_size(src.width(), src.height(), p.animation.max_side);
            let img = image::imageops::resize(&*src.pixels, w, h, image::imageops::FilterType::Lanczos3);
            let pr = gridnoise::Prepared::new(&img, &p, 1.0);
            let (an, _) = gridnoise::analyse(&pr, &p, 0, frames);
            println!("{}", serde_json::to_string_pretty(&an).unwrap());
        }
        _ => {
            println!("shieldcraft-cli {}\n  gridnoise <in> <out.gif|.png|.mp4|.webm> [preset]\n  still <in> <out.png>\n  sign <in> <out.png> <text> [key]\n  verify <in> <byte-length> [key]\n  analyse <in> [frames]", shieldcraft_core::APP_VERSION);
        }
    }
    Ok(())
}

fn hope_worker(req_path: &Path) -> Result<(), String> {
    use shieldcraft_core::engines::{hope, worker::{HopeRequest, HopeResponse}};
    use std::io::Write;
    let req: HopeRequest = serde_json::from_slice(&std::fs::read(req_path).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let dll = shieldcraft_core::engines::worker::runtime_dll().ok_or("cannot locate onnxruntime.dll")?;
    let env = ort::init_from(&dll).map_err(|e| format!("Could not load ONNX Runtime from {}: {e}", dll.display()))?;
    env.with_name("shieldcraft").commit();
    let model = hope::models::model_path(req.params.algorithm.model());
    if !model.is_file() {
        return Err(format!("Model not found at {}", model.display()));
    }
    let img = image::open(&req.input).map_err(|e| e.to_string())?.to_rgba8();
    let t = std::time::Instant::now();
    println!("PROGRESS 0.01 Loading model…");
    let mut session = hope::spsa::open_session(&model, &req.params)?;
    let out = std::sync::Mutex::new(std::io::stdout());
    let progress = |f: f32, m: &str| {
        let mut o = out.lock().unwrap();
        let _ = writeln!(o, "PROGRESS {f:.4} {m}");
        let _ = o.flush();
    };
    let mask = match &req.mask {
        Some(p) => Some(image::open(p).map_err(|e| format!("Could not read the selection mask: {e}"))?.to_luma8()),
        None => None,
    };
    let (result, loss) = hope::spsa::protect(&img, &mut session, &req.params, mask.as_ref(), &progress, &CancelToken::new()).map_err(|e| e.to_string())?;
    result.save(&req.output).map_err(|e| e.to_string())?;
    let resp = HopeResponse {
        execution_provider: "CPU (ONNX Runtime 1.24.2)".into(),
        tiles: hope::spsa::tile_regions(img.width(), img.height()).len() as u32,
        evaluations: loss.total_iterations * hope::spsa::DIRECTIONS_PER_ITER as u64 * 2,
        seconds: t.elapsed().as_secs_f64(),
        loss_before: loss.before,
        loss_after: loss.after,
        tiles_extended: loss.tiles_extended,
        tiles_unconverged: loss.tiles_unconverged,
        tiles_skipped: loss.tiles_skipped,
        total_iterations: loss.total_iterations,
    };
    println!("RESULT {}", serde_json::to_string(&resp).unwrap());
    Ok(())
}

fn save(img: &image::RgbaImage, src: &image_io::SourceImage, out: &Path) -> Result<(), String> {
    let opts = image_io::ExportOptions::default();
    let (bytes, warnings) = image_io::encode(img, &opts, src.icc_profile.as_deref(), None)?;
    image_io::write_atomic(out, &bytes, src.path.as_deref(), true)?;
    for w in warnings {
        println!("warning: {w}");
    }
    println!("wrote {}", out.display());
    let _ = Arc::new(());
    Ok(())
}
