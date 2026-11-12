//! Background renderer for the live Grid Noise preview. Requests are coalesced: only the most
//! recent one is rendered, so dragging a slider never queues up stale frames.

use shieldcraft_core::engines::gridnoise::{self, Analysis, GridNoiseParams, Prepared, RenderMode};
use crossbeam_channel::{unbounded, Receiver, Sender};
use image::RgbaImage;
use std::sync::Arc;
use std::time::Instant;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PreviewKind {
    Frame,
    /// Temporal average over N frames, with measured statistics.
    Average(u32),
}

pub struct Request {
    pub doc: u64,
    pub source: Arc<RgbaImage>,
    pub params: GridNoiseParams,
    pub scale: f32,
    pub frame: u32,
    pub kind: PreviewKind,
    /// Selection at preview size; outside it the original is shown.
    pub mask: Option<Arc<shieldcraft_core::selection::Mask>>,
}

pub struct Rendered {
    pub doc: u64,
    pub kind: PreviewKind,
    pub image: RgbaImage,
    pub analysis: Option<Analysis>,
    pub millis: f32,
}

pub struct PreviewRenderer {
    tx: Sender<Request>,
    pub rx: Receiver<Rendered>,
    pub busy: bool,
}

impl PreviewRenderer {
    pub fn start(ctx: egui::Context) -> Self {
        let (tx, req_rx) = unbounded::<Request>();
        let (res_tx, rx) = unbounded::<Rendered>();
        std::thread::Builder::new()
            .name("preview".into())
            .spawn(move || {
                let mut cache: Option<(u64, usize, u64, f32, Prepared)> = None;
                while let Ok(mut req) = req_rx.recv() {
                    while let Ok(newer) = req_rx.try_recv() {
                        req = newer;
                    }
                    let t = Instant::now();
                    let sig = gridnoise::static_signature(&req.params);
                    let ptr = Arc::as_ptr(&req.source) as usize;
                    let fresh = !matches!(&cache, Some((d, p, s, sc, _)) if *d == req.doc && *p == ptr && *s == sig && *sc == req.scale);
                    if fresh {
                        cache = Some((req.doc, ptr, sig, req.scale, Prepared::new(&req.source, &req.params, req.scale)));
                    }
                    let pr = &cache.as_ref().unwrap().4;
                    let (mut image, analysis) = match req.kind {
                        PreviewKind::Frame => (gridnoise::render_image(pr, &req.params, req.frame, RenderMode::Frame), None),
                        PreviewKind::Average(n) => {
                            let (a, img) = gridnoise::analyse(pr, &req.params, req.frame, n);
                            (img, Some(a))
                        }
                    };
                    if let Some(m) = &req.mask {
                        shieldcraft_core::selection::composite(&req.source, &mut image, m);
                    }
                    let out = Rendered { doc: req.doc, kind: req.kind, image, analysis, millis: t.elapsed().as_secs_f32() * 1000.0 };
                    if res_tx.send(out).is_err() {
                        break;
                    }
                    ctx.request_repaint();
                }
            })
            .expect("spawn preview thread");
        Self { tx, rx, busy: false }
    }

    pub fn request(&mut self, r: Request) {
        if self.tx.send(r).is_ok() {
            self.busy = true;
        }
    }
}
