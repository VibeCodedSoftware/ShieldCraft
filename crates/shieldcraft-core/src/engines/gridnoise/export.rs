//! Animation export for Grid Noise Studio: GIF and APNG (built in), PNG frame sequences, and
//! MP4/WebM through an FFmpeg executable when one is available.

use super::params::{AnimFormat, GridNoiseParams};
use super::render::{output_size, render_frame, Prepared, RenderMode};
use crate::engine::{CancelToken, EngineError, ProgressFn};
use image::RgbaImage;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;

/// One storyboard panel: an image shown for `duration_s` seconds.
#[derive(Clone)]
pub struct Panel {
    pub image: Arc<RgbaImage>,
    pub duration_s: f32,
    /// Optional selection: only this area animates; the rest stays as the original.
    pub mask: Option<Arc<crate::selection::Mask>>,
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct ExportSummary {
    pub path: String,
    pub width: u32,
    pub height: u32,
    pub frames: u32,
    pub fps: u32,
    pub bytes: u64,
    pub warnings: Vec<String>,
}

/// Find FFmpeg: explicit setting, then next to the executable, then on PATH.
pub fn find_ffmpeg(configured: Option<&Path>) -> Option<PathBuf> {
    if let Some(p) = configured.filter(|p| p.is_file()) {
        return Some(p.to_path_buf());
    }
    if let Some(dir) = crate::paths::exe_dir() {
        for c in [dir.join("ffmpeg.exe"), dir.join("tools").join("ffmpeg.exe")] {
            if c.is_file() {
                return Some(c);
            }
        }
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|d| d.join("ffmpeg.exe")).find(|p| p.is_file())
}

fn cover_resize<P: image::Pixel<Subpixel = u8> + 'static>(img: &image::ImageBuffer<P, Vec<u8>>, w: u32, h: u32) -> image::ImageBuffer<P, Vec<u8>> {
    if img.dimensions() == (w, h) {
        return img.clone();
    }
    let s = (w as f32 / img.width() as f32).max(h as f32 / img.height() as f32);
    let rw = ((img.width() as f32 * s).ceil() as u32).max(w);
    let rh = ((img.height() as f32 * s).ceil() as u32).max(h);
    let resized = image::imageops::resize(img, rw, rh, image::imageops::FilterType::Triangle);
    image::imageops::crop_imm(&resized, (rw - w) / 2, (rh - h) / 2, w, h).to_image()
}

/// Render and encode the full animation (all panels) to `out`.
pub fn export_animation(
    panels: &[Panel],
    p: &GridNoiseParams,
    out: &Path,
    ffmpeg: Option<&Path>,
    progress: ProgressFn,
    cancel: &CancelToken,
) -> Result<ExportSummary, EngineError> {
    p.validate().map_err(EngineError::InvalidParams)?;
    let first = panels.first().ok_or_else(|| EngineError::InvalidParams("No image to animate".into()))?;
    let (mut w, mut h) = output_size(first.image.width(), first.image.height(), p.animation.max_side);
    let mut warnings = Vec::new();
    let fps = p.animation.fps;
    if p.animation.format.needs_ffmpeg() && (w % 2 == 1 || h % 2 == 1) {
        w -= w % 2;
        h -= h % 2;
        warnings.push("Video encoders need even dimensions: one pixel row/column was trimmed".into());
    }
    if p.animation.format == AnimFormat::Gif && fps > 50 {
        warnings.push("GIF timing is limited to 50 fps; frames are delayed accordingly".into());
    }
    if p.animation.format == AnimFormat::Mp4 && p.preserve_alpha {
        warnings.push("MP4/H.264 has no alpha channel: transparent areas are encoded as black".into());
    }
    if p.animation.format.needs_ffmpeg() {
        warnings.push("Video encoding uses 4:2:0 chroma subsampling, which softens per-pixel colour noise".into());
    }

    let frames_per: Vec<u32> = if panels.len() == 1 {
        vec![p.frame_count()]
    } else {
        // A panel with any duration gets at least one frame instead of silently vanishing.
        panels.iter().map(|pl| ((pl.duration_s * fps as f32).round() as u32).max(u32::from(pl.duration_s > 0.0))).collect()
    };
    let total: u32 = frames_per.iter().sum();
    if total == 0 {
        return Err(EngineError::InvalidParams("The animation has no frames (all durations are 0)".into()));
    }

    let mut sink = Sink::open(p, out, w, h, ffmpeg, total)?;
    let mut buf = vec![0u8; (w * h * 4) as usize];
    let mut global = 0u32;
    for (pi, panel) in panels.iter().enumerate() {
        if frames_per[pi] == 0 {
            continue;
        }
        let img = cover_resize(&*panel.image, w, h);
        let mask = panel.mask.as_ref().map(|m| cover_resize(&**m, w, h));
        let pr = Prepared::new(&img, p, 1.0);
        for _ in 0..frames_per[pi] {
            if cancel.is_cancelled() {
                sink.abort();
                return Err(EngineError::Cancelled);
            }
            render_frame(&pr, p, global, RenderMode::Frame, &mut buf);
            if let Some(m) = &mask {
                crate::selection::composite_raw(img.as_raw(), &mut buf, m.as_raw());
            }
            sink.push(&buf, global)?;
            global += 1;
            progress(
                global as f32 / total as f32 * 0.97,
                &format!("Rendering frame {global} / {total}{}", if panels.len() > 1 { format!(" (panel {})", pi + 1) } else { String::new() }),
            );
        }
    }
    progress(0.98, "Finalising file…");
    let bytes = sink.finish()?;
    progress(1.0, "Done");
    Ok(ExportSummary { path: out.display().to_string(), width: w, height: h, frames: total, fps, bytes, warnings })
}

enum Sink {
    Gif { enc: image::codecs::gif::GifEncoder<std::io::BufWriter<std::fs::File>>, tmp: PathBuf, out: PathBuf, w: u32, h: u32, delay_ms: u32 },
    Apng { writer: Option<png::Writer<std::io::BufWriter<std::fs::File>>>, tmp: PathBuf, out: PathBuf },
    Seq { dir: PathBuf, w: u32, h: u32 },
    Ffmpeg { child: std::process::Child, tmp: PathBuf, out: PathBuf },
}

impl Sink {
    fn open(p: &GridNoiseParams, out: &Path, w: u32, h: u32, ffmpeg: Option<&Path>, total: u32) -> Result<Sink, EngineError> {
        let io = |e: std::io::Error| EngineError::Failed(format!("Cannot write {}: {e}", out.display()));
        let tmp = out.with_extension(format!("{}.partial", p.animation.format.ext()));
        match p.animation.format {
            AnimFormat::Gif => {
                let f = std::fs::File::create(&tmp).map_err(io)?;
                let mut enc = image::codecs::gif::GifEncoder::new_with_speed(std::io::BufWriter::new(f), 10);
                enc.set_repeat(image::codecs::gif::Repeat::Infinite).map_err(|e| EngineError::Failed(e.to_string()))?;
                let delay_ms = (1000 / p.animation.fps.clamp(1, 50)).max(20);
                Ok(Sink::Gif { enc, tmp, out: out.to_path_buf(), w, h, delay_ms })
            }
            AnimFormat::Apng => {
                let f = std::fs::File::create(&tmp).map_err(io)?;
                let mut enc = png::Encoder::new(std::io::BufWriter::new(f), w, h);
                enc.set_color(png::ColorType::Rgba);
                enc.set_depth(png::BitDepth::Eight);
                enc.set_animated(total, 0).map_err(|e| EngineError::Failed(e.to_string()))?;
                enc.set_frame_delay(1, p.animation.fps.min(u16::MAX as u32) as u16).map_err(|e| EngineError::Failed(e.to_string()))?;
                let writer = enc.write_header().map_err(|e| EngineError::Failed(e.to_string()))?;
                Ok(Sink::Apng { writer: Some(writer), tmp, out: out.to_path_buf() })
            }
            AnimFormat::PngSequence => {
                std::fs::create_dir_all(out).map_err(io)?;
                Ok(Sink::Seq { dir: out.to_path_buf(), w, h })
            }
            AnimFormat::Mp4 | AnimFormat::WebM => {
                let exe = ffmpeg.ok_or_else(|| {
                    EngineError::Unavailable(
                        "MP4/WebM export needs FFmpeg. Install it (or place ffmpeg.exe next to ShieldCraft) \
                         and set its path in Preferences, or choose GIF/APNG/PNG sequence."
                            .into(),
                    )
                })?;
                let mut cmd = Command::new(exe);
                cmd.args(["-hide_banner", "-loglevel", "error", "-y", "-f", "rawvideo", "-pix_fmt", "rgba"])
                    .args(["-s", &format!("{w}x{h}"), "-r", &p.animation.fps.to_string(), "-i", "-"]);
                if let Some(a) = p.animation.audio_path.as_deref().filter(|a| Path::new(a).is_file()) {
                    cmd.args(["-i", a, "-shortest"]);
                }
                if p.animation.format == AnimFormat::Mp4 {
                    cmd.args(["-c:v", "libx264", "-preset", "slow", "-crf", &p.animation.mp4_crf.to_string(), "-pix_fmt", "yuv420p", "-movflags", "+faststart", "-c:a", "aac", "-f", "mp4"]);
                } else {
                    let pix = if p.preserve_alpha { "yuva420p" } else { "yuv420p" };
                    cmd.args(["-c:v", "libvpx-vp9", "-b:v", "0", "-crf", "24", "-pix_fmt", pix, "-c:a", "libopus", "-f", "webm"]);
                }
                cmd.arg(&tmp).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::piped());
                #[cfg(windows)]
                {
                    use std::os::windows::process::CommandExt;
                    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
                }
                let child = cmd.spawn().map_err(|e| EngineError::Failed(format!("Could not start FFmpeg: {e}")))?;
                Ok(Sink::Ffmpeg { child, tmp, out: out.to_path_buf() })
            }
        }
    }

    fn push(&mut self, rgba: &[u8], index: u32) -> Result<(), EngineError> {
        let fail = |e: String| EngineError::Failed(format!("Encoding failed: {e}"));
        match self {
            Sink::Gif { enc, w, h, delay_ms, .. } => {
                let img = RgbaImage::from_raw(*w, *h, rgba.to_vec()).ok_or_else(|| fail("bad frame".into()))?;
                let frame = image::Frame::from_parts(img, 0, 0, image::Delay::from_numer_denom_ms(*delay_ms, 1));
                enc.encode_frame(frame).map_err(|e| fail(e.to_string()))
            }
            Sink::Apng { writer, .. } => writer.as_mut().unwrap().write_image_data(rgba).map_err(|e| fail(e.to_string())),
            Sink::Seq { dir, w, h } => {
                let img = RgbaImage::from_raw(*w, *h, rgba.to_vec()).ok_or_else(|| fail("bad frame".into()))?;
                img.save(dir.join(format!("frame_{:05}.png", index + 1))).map_err(|e| fail(e.to_string()))
            }
            Sink::Ffmpeg { child, .. } => {
                let stdin = child.stdin.as_mut().ok_or_else(|| fail("FFmpeg input closed".into()))?;
                stdin.write_all(rgba).map_err(|e| fail(format!("FFmpeg stopped accepting frames ({e})")))
            }
        }
    }

    fn abort(self) {
        match self {
            Sink::Gif { tmp, .. } | Sink::Apng { tmp, .. } => {
                let _ = std::fs::remove_file(tmp);
            }
            Sink::Ffmpeg { mut child, tmp, .. } => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = std::fs::remove_file(tmp);
            }
            Sink::Seq { .. } => {}
        }
    }

    fn finish(self) -> Result<u64, EngineError> {
        let finalize = |tmp: &Path, out: &Path| -> Result<u64, EngineError> {
            if out.exists() {
                let _ = std::fs::remove_file(out);
            }
            std::fs::rename(tmp, out).map_err(|e| EngineError::Failed(format!("Cannot finalise {}: {e}", out.display())))?;
            Ok(std::fs::metadata(out).map(|m| m.len()).unwrap_or(0))
        };
        match self {
            Sink::Gif { enc, tmp, out, .. } => {
                drop(enc);
                finalize(&tmp, &out)
            }
            Sink::Apng { mut writer, tmp, out } => {
                writer.take().unwrap().finish().map_err(|e| EngineError::Failed(e.to_string()))?;
                finalize(&tmp, &out)
            }
            Sink::Seq { dir, .. } => Ok(std::fs::read_dir(&dir)
                .map(|it| it.flatten().filter_map(|e| e.metadata().ok()).map(|m| m.len()).sum())
                .unwrap_or(0)),
            Sink::Ffmpeg { mut child, tmp, out } => {
                drop(child.stdin.take());
                let output = child.wait_with_output().map_err(|e| EngineError::Failed(e.to_string()))?;
                if !output.status.success() {
                    let _ = std::fs::remove_file(&tmp);
                    return Err(EngineError::Failed(format!(
                        "FFmpeg failed: {}",
                        String::from_utf8_lossy(&output.stderr).trim()
                    )));
                }
                finalize(&tmp, &out)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gif_and_apng_export() {
        let img = Arc::new(RgbaImage::from_fn(40, 30, |x, y| image::Rgba([x as u8 * 5, y as u8 * 7, 90, 255])));
        let dir = std::env::temp_dir().join(format!("shieldcraft-anim-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for fmt in [AnimFormat::Gif, AnimFormat::Apng] {
            let mut p = GridNoiseParams::default();
            p.animation.format = fmt;
            p.animation.fps = 10;
            p.animation.duration_s = 0.5;
            let out = dir.join(format!("t.{}", fmt.ext()));
            let s = export_animation(&[Panel { image: img.clone(), duration_s: 1.0, mask: None }], &p, &out, None, &|_, _| {}, &CancelToken::new()).unwrap();
            assert_eq!(s.frames, 5);
            assert!(out.is_file() && s.bytes > 0);
            let decoded = image::open(&out).unwrap();
            assert_eq!(decoded.width(), 40);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
