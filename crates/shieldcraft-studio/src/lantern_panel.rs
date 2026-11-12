//! Provenance tab: Lantern registry lookups for the active artwork.
//!
//! Everything here is user-initiated. Hash lookups send only the SHA-256 of the original file;
//! the image check uploads the file only after an explicit confirmation.

use crossbeam_channel::{unbounded, Receiver};
use egui::{Color32, RichText, Ui};
use egui_phosphor::regular as icon;
use shieldcraft_core::lantern::{self, CheckRecord, History, ImageCheck, OptOut, Provenance, ServiceStatus};
use shieldcraft_core::util;
use std::path::PathBuf;

enum Reply {
    Lookup { sha: String, result: Result<Option<(Provenance, Option<History>)>, String> },
    Image { sha: String, result: Result<ImageCheck, String> },
    OptOut { sha: String, result: Result<OptOut, String> },
    Id { id: String, sha: String, result: Result<Option<Provenance>, String>, hash_owner: Result<Option<String>, String> },
    Status(Result<ServiceStatus, String>),
}

#[derive(Default)]
pub struct LanternUi {
    rx: Option<Receiver<Reply>>,
    busy: Option<String>,
    id_input: String,
    lookup: Option<(String, Result<Option<(Provenance, Option<History>)>, String>)>,
    image: Option<(String, Result<ImageCheck, String>)>,
    optout: Option<(String, Result<OptOut, String>)>,
    id_check: Option<(String, String, Result<Option<Provenance>, String>, Result<Option<String>, String>)>,
    pub status: Option<Result<ServiceStatus, String>>,
    pub last_ok: Option<String>,
    confirm_upload: bool,
    log: Vec<CheckRecord>,
    log_loaded: bool,
    hashed: Option<(PathBuf, String)>,
}

fn now() -> String {
    util::format_utc(util::unix_now())
}

impl LanternUi {
    fn start(&mut self, label: &str, ctx: &egui::Context, job: impl FnOnce() -> Reply + Send + 'static) {
        let (tx, rx) = unbounded();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(job());
            ctx.request_repaint();
        });
        self.rx = Some(rx);
        self.busy = Some(label.to_string());
    }

    pub fn check_status(&mut self, ctx: &egui::Context) {
        self.start("Checking Lantern status…", ctx, || Reply::Status(lantern::Client::default().status().map_err(|e| e.to_string())));
    }

    fn record(&mut self, kind: &str, file: &str, sha: &str, outcome: String, id: Option<String>, response: serde_json::Value) {
        let rec = CheckRecord { at_utc: now(), kind: kind.into(), file_name: file.into(), sha256: sha.into(), outcome, lantern_id: id, response };
        let _ = lantern::append_log(rec.clone());
        self.log.push(rec);
    }

    /// Collect finished replies; returns a message for the app log when something completed.
    pub fn poll(&mut self, file_name: &str) -> Option<(bool, String)> {
        let reply = self.rx.as_ref()?.try_recv().ok()?;
        self.rx = None;
        self.busy = None;
        let mut msg = None;
        match reply {
            Reply::Status(r) => {
                if r.as_ref().map(|s| s.healthy).unwrap_or(false) {
                    self.last_ok = Some(now());
                }
                self.status = Some(r);
            }
            Reply::Lookup { sha, result } => {
                let (ok, outcome, id, resp) = match &result {
                    Ok(Some((p, _))) => (true, format!("Registered on Lantern as {} ({})", p.lantern_id, p.status), Some(p.lantern_id.clone()), serde_json::to_value(p).unwrap_or_default()),
                    Ok(None) => (true, "This exact file is not in the Lantern registry".into(), None, serde_json::Value::Null),
                    Err(e) => (false, format!("Lookup failed: {e}"), None, serde_json::Value::Null),
                };
                if ok {
                    self.last_ok = Some(now());
                }
                self.record("hash_lookup", file_name, &sha, outcome.clone(), id, resp);
                msg = Some((ok, format!("Lantern lookup: {outcome}")));
                self.lookup = Some((sha, result));
            }
            Reply::Image { sha, result } => {
                let (ok, outcome, resp) = match &result {
                    Ok(c) if c.inconclusive() => (true, format!("Inconclusive — Lantern could not run matching: {}", c.title), c.raw.clone()),
                    Ok(c) if c.no_match() => (true, "No match in the Lantern registry".to_string(), c.raw.clone()),
                    Ok(c) => (true, format!("Lantern reported a possible match: {} ({})", c.title, c.badge_label), c.raw.clone()),
                    Err(e) => (false, format!("Image check failed: {e}"), serde_json::Value::Null),
                };
                if ok {
                    self.last_ok = Some(now());
                }
                self.record("image_check", file_name, &sha, outcome.clone(), None, resp);
                msg = Some((ok, format!("Lantern image check: {outcome}")));
                self.image = Some((sha, result));
            }
            Reply::OptOut { sha, result } => {
                let (ok, outcome, resp) = match &result {
                    Ok(o) => (true, if o.found { "Listed in Lantern's training opt-out registry".into() } else { "Not listed in Lantern's training opt-out registry".to_string() }, o.raw.clone()),
                    Err(e) => (false, format!("Opt-out check failed: {e}"), serde_json::Value::Null),
                };
                if ok {
                    self.last_ok = Some(now());
                }
                self.record("optout_check", file_name, &sha, outcome.clone(), None, resp);
                msg = Some((ok, format!("Lantern opt-out registry: {outcome}")));
                self.optout = Some((sha, result));
            }
            Reply::Id { id, sha, result, hash_owner } => {
                let outcome = match (&result, &hash_owner) {
                    (Ok(Some(_)), Ok(Some(owner))) if owner == &id => format!("{id} exists and is registered for exactly this file"),
                    (Ok(Some(_)), Ok(Some(owner))) => format!("{id} exists, but this file is registered as {owner}, a different record"),
                    (Ok(Some(_)), Ok(None)) => format!("{id} exists, but this exact file is not its registered image (it may be a different export of the work)"),
                    (Ok(None), _) => format!("{id} was not found on Lantern"),
                    (Err(e), _) | (_, Err(e)) => format!("Could not check {id}: {e}"),
                };
                let ok = result.is_ok() && hash_owner.is_ok();
                if ok {
                    self.last_ok = Some(now());
                }
                let resp = result.as_ref().ok().and_then(|r| r.as_ref()).map(|p| serde_json::to_value(p).unwrap_or_default()).unwrap_or_default();
                self.record("id_lookup", file_name, &sha, outcome.clone(), Some(id.clone()), resp);
                msg = Some((ok, format!("Lantern ID check: {outcome}")));
                self.id_check = Some((id, sha, result, hash_owner));
            }
        }
        msg
    }

    /// Draws the Provenance tab. `enabled` mirrors the user's network consent setting.
    pub fn ui(&mut self, ui: &mut Ui, enabled: &mut bool, file: Option<(PathBuf, String)>) {
        if !self.log_loaded {
            self.log = lantern::load_log();
            self.log_loaded = true;
        }
        ui.label(RichText::new(format!("{} Provenance · Lantern", icon::STAMP)).heading());
        ui.label(RichText::new(
            "Lantern is a public registry where artists register their work. Registration records a fingerprint (a unique code computed from the \
             image) with a timestamp on a public blockchain. This can support an ownership claim, but it does not by itself establish legal copyright \
             or prove anyone infringed it.",
        ).small());
        ui.horizontal_wrapped(|ui| {
            ui.hyperlink_to(RichText::new("lantern-us.com").small(), lantern::SITE);
            ui.label(RichText::new("· external service, not open source ·").small().weak());
            ui.hyperlink_to(RichText::new("Terms").small(), lantern::TERMS_URL);
        });
        ui.add_space(4.0);

        if !*enabled {
            ui.label(RichText::new("Lantern is off. Turning it on lets ShieldCraft contact api.lantern-us.com when you click a Lantern button. Nothing is sent automatically.").color(crate::theme::WARN));
            if ui.button(format!("{} Turn on Lantern lookups", icon::GLOBE)).clicked() {
                *enabled = true;
            }
            return;
        }
        let Some((path, name)) = file else {
            ui.label(RichText::new("Open an image file to check it. (Lantern fingerprints the original file, so images without a file on disk can't be checked.)").weak());
            return;
        };
        let sha = match &self.hashed {
            Some((p, h)) if *p == path => h.clone(),
            _ => match std::fs::read(&path) {
                Ok(b) => {
                    let h = lantern::sha256_bytes(&b);
                    self.hashed = Some((path.clone(), h.clone()));
                    h
                }
                Err(e) => {
                    ui.label(RichText::new(format!("Can't read the original file: {e}")).color(crate::theme::BAD));
                    return;
                }
            },
        };
        ui.label(RichText::new(format!("{name}\nSHA-256 {sha}")).small().monospace()).on_hover_text("The file's exact fingerprint. Any change to the file, even re-saving it, gives a different value.");
        if let Some(b) = &self.busy {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(b.as_str());
            });
        }
        let idle = self.busy.is_none();
        let ctx = ui.ctx().clone();

        crate::theme::caption(ui, "Is this artwork registered?");
        if ui.add_enabled(idle, egui::Button::new(format!("{} Look up this exact file", icon::MAGNIFYING_GLASS))).on_hover_text("Sends only the SHA-256 fingerprint, not the image.").clicked() {
            let s = sha.clone();
            self.start("Looking up on Lantern…", &ctx, move || {
                let c = lantern::Client::default();
                let result = c.lookup_hash(&s).map_err(|e| e.to_string()).map(|p| {
                    p.map(|p| {
                        let h = c.history(&p.lantern_id).ok().flatten();
                        (p, h)
                    })
                });
                Reply::Lookup { sha: s, result }
            });
        }
        if let Some((s, r)) = &self.lookup {
            if *s == sha {
                match r {
                    Ok(Some((p, h))) => provenance_card(ui, p, h.as_ref()),
                    Ok(None) => {
                        ui.label(RichText::new("Not registered: this exact file isn't in the Lantern registry. A different export of the same artwork would have a different fingerprint.").small());
                    }
                    Err(e) => {
                        ui.label(RichText::new(e).color(crate::theme::BAD));
                    }
                }
            }
        }

        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.id_input).hint_text("LNTN-xxxxxxxx").desired_width(130.0));
            if ui.add_enabled(idle && lantern::normalize_id(&self.id_input).is_some(), egui::Button::new("Check ID against this file")).clicked() {
                let id = lantern::normalize_id(&self.id_input).unwrap();
                let s = sha.clone();
                self.start("Checking Lantern ID…", &ctx, move || {
                    let c = lantern::Client::default();
                    let result = c.lookup_id(&id).map_err(|e| e.to_string());
                    let hash_owner = c.lookup_hash(&s).map(|p| p.map(|p| p.lantern_id)).map_err(|e| e.to_string());
                    Reply::Id { id, sha: s, result, hash_owner }
                });
            }
        });
        if let Some((id, s, r, owner)) = &self.id_check {
            if *s == sha {
                let (txt, col) = match (r, owner) {
                    (Ok(Some(_)), Ok(Some(o))) if o == id => (format!("{id} is registered for exactly this file."), crate::theme::OK),
                    (Ok(Some(_)), Ok(Some(o))) => (format!("{id} exists, but this file belongs to a different record ({o})."), crate::theme::WARN),
                    (Ok(Some(_)), Ok(None)) => (format!("{id} exists, but this exact file isn't its registered image."), crate::theme::WARN),
                    (Ok(None), _) => (format!("{id} was not found."), crate::theme::WARN),
                    (Err(e), _) | (_, Err(e)) => (e.clone(), crate::theme::BAD),
                };
                ui.label(RichText::new(txt).small().color(col));
                if let Ok(Some(p)) = r {
                    provenance_card(ui, p, None);
                }
            }
        }
        ui.horizontal_wrapped(|ui| {
            if ui.button(format!("{} Register on Lantern…", icon::ARROW_SQUARE_OUT)).on_hover_text("Opens lantern-us.com in your browser. Registering needs a Lantern account with a verified social-media link.").clicked() {
                let _ = std::process::Command::new("explorer").arg(format!("{}/dashboard", lantern::SITE)).spawn();
            }
            ui.label(RichText::new("then use Look up to confirm it here.").small().weak());
        });

        crate::theme::caption(ui, "Does it match anything registered?");
        ui.label(RichText::new("Lantern compares the image with works registered on Lantern only, including resized, cropped or re-compressed copies. It does not search the web.").small().weak());
        if ui.add_enabled(idle, egui::Button::new(format!("{} Check image with Lantern matching…", icon::UPLOAD_SIMPLE))).clicked() {
            self.confirm_upload = true;
        }
        if self.confirm_upload {
            egui::Modal::new(egui::Id::new("lantern_upload")).show(&ctx, |ui| {
                ui.set_max_width(420.0);
                ui.heading("Upload this image to Lantern?");
                ui.label(format!("“{name}” will be sent to api.lantern-us.com to compare it with registered works. Lantern's Terms and Privacy Policy apply. Only do this for images you are entitled to share."));
                ui.horizontal(|ui| {
                    if ui.button(RichText::new("Upload and check").strong()).clicked() {
                        self.confirm_upload = false;
                        let (p, s, n) = (path.clone(), sha.clone(), name.clone());
                        self.start("Lantern is matching the image…", &ctx, move || {
                            let result = std::fs::read(&p).map_err(|e| e.to_string()).and_then(|b| lantern::Client::default().check_image(&b, &n).map_err(|e| e.to_string()));
                            Reply::Image { sha: s, result }
                        });
                    }
                    if ui.button("Cancel").clicked() {
                        self.confirm_upload = false;
                    }
                });
            });
        }
        if let Some((s, r)) = &self.image {
            if *s == sha {
                match r {
                    Ok(c) => {
                        let (verdict, col) = if c.inconclusive() {
                            ("Inconclusive: Lantern couldn't run matching. This is not a 'no match'; try again later.", crate::theme::WARN)
                        } else if c.no_match() {
                            ("No match among works registered on Lantern.", ui.visuals().text_color())
                        } else {
                            ("Possible match reported by Lantern; review it on Lantern before acting.", crate::theme::WARN)
                        };
                        ui.label(RichText::new(verdict).color(col));
                        ui.label(RichText::new(format!("Lantern says: [{}] {}", c.badge_label, c.title)).small());
                        if let Some(h) = &c.helper_text {
                            ui.label(RichText::new(h).small().weak());
                        }
                        if let Some(l) = &c.link {
                            ui.hyperlink_to(RichText::new("Open on Lantern").small(), l);
                        }
                    }
                    Err(e) => {
                        ui.label(RichText::new(e).color(crate::theme::BAD));
                    }
                }
            }
        }

        crate::theme::caption(ui, "AI training");
        if ui.add_enabled(idle, egui::Button::new(format!("{} Check training opt-out registry", icon::PROHIBIT))).on_hover_text("Sends only the SHA-256. This checks whether the file is listed as opted out of AI training, not whether it is in a dataset.").clicked() {
            let s = sha.clone();
            self.start("Checking opt-out registry…", &ctx, move || Reply::OptOut { result: lantern::Client::default().optout(&s).map_err(|e| e.to_string()), sha: s });
        }
        if let Some((s, r)) = &self.optout {
            if *s == sha {
                let t = match r {
                    Ok(o) if o.found => "Listed as opted out of AI training in Lantern's registry.".to_string(),
                    Ok(_) => "Not listed in Lantern's training opt-out registry.".to_string(),
                    Err(e) => e.clone(),
                };
                ui.label(RichText::new(t).small());
            }
        }
        ui.label(RichText::new(
            "Dataset check: Lantern can check a registered work's exact fingerprint against LAION-5B, CC-12M and Common Crawl, but only for its \
             signed-in owner. ShieldCraft doesn't sign in to Lantern, so use your Lantern dashboard for this. A 'no match' there never proves an \
             image wasn't used for training, and a match doesn't prove a particular company trained on it.",
        ).small().weak());

        crate::theme::caption(ui, "Connection");
        ui.horizontal_wrapped(|ui| {
            if ui.add_enabled(idle, egui::Button::new("Check Lantern status")).clicked() {
                self.check_status(&ctx);
            }
            match &self.status {
                Some(Ok(s)) if s.healthy => ui.label(RichText::new("Lantern is reachable.").small().color(crate::theme::OK)),
                Some(Ok(_)) => ui.label(RichText::new("Lantern answered but reports a problem.").small().color(crate::theme::WARN)),
                Some(Err(e)) => ui.label(RichText::new(e).small().color(crate::theme::BAD)),
                None => ui.label(RichText::new("Not checked yet.").small().weak()),
            };
        });
        if let Some(t) = &self.last_ok {
            ui.label(RichText::new(format!("Last successful request: {t}")).small().weak());
        }
        ui.label(RichText::new("Available without an account: lookups, image matching, opt-out registry. Needs a Lantern account (not supported in ShieldCraft): registering through the API and the dataset check.").small().weak());
        if ui.small_button("Turn Lantern off").clicked() {
            *enabled = false;
        }

        crate::theme::caption(ui, "Evidence log");
        let mine: Vec<&CheckRecord> = self.log.iter().filter(|r| r.sha256 == sha).collect();
        if mine.is_empty() {
            ui.label(RichText::new("No Lantern checks recorded for this file yet.").small().weak());
        } else {
            for r in mine.iter().rev().take(8) {
                ui.label(RichText::new(format!("{} · {} · {}", r.at_utc, r.kind.replace('_', " "), r.outcome)).small());
            }
            if ui.button(format!("{} Save evidence report…", icon::FILE_HTML)).clicked() {
                let records: Vec<CheckRecord> = mine.iter().map(|r| (*r).clone()).collect();
                if let Some(out) = rfd::FileDialog::new().set_file_name(format!("{}_lantern_report.html", name.rsplit_once('.').map(|x| x.0).unwrap_or(&name))).add_filter("HTML", &["html"]).save_file() {
                    let html = report_html(&name, &sha, &records);
                    let _ = shieldcraft_core::image_io::write_atomic(&out, html.as_bytes(), Some(&path), false);
                }
            }
        }
    }
}

fn provenance_card(ui: &mut Ui, p: &Provenance, h: Option<&History>) {
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_width(ui.available_width());
        let ok = p.status == "active" && p.chain.verified;
        ui.label(RichText::new(format!("{} {}", if ok { icon::SEAL_CHECK } else { icon::WARNING }, p.display.label)).strong().color(if ok { crate::theme::OK } else { crate::theme::WARN }));
        egui::Grid::new(("prov", &p.lantern_id)).num_columns(2).spacing([10.0, 2.0]).show(ui, |ui| {
            let row = |ui: &mut Ui, k: &str, v: String| {
                ui.label(RichText::new(k).small().weak());
                ui.label(RichText::new(v).small());
                ui.end_row();
            };
            row(ui, "Lantern ID", p.lantern_id.clone());
            row(ui, "Status", p.status.clone());
            row(ui, "Registered", p.registered_at.clone());
            row(ui, "Disputes", p.dispute_status.clone());
            row(ui, "Blockchain record", format!("{} ({})", if p.chain.verified { "verified" } else { "not verified" }, p.chain.code));
            let platforms: Vec<&str> = p.creator.verified_platforms.iter().map(|x| x.platform.as_str()).collect();
            row(ui, "Creator", format!("{} · verified on {}", p.creator.display_name.clone().unwrap_or_else(|| "(pseudonymous)".into()), if platforms.is_empty() { "no platforms".into() } else { platforms.join(", ") }));
            row(ui, "Source link", p.source_status.clone());
        });
        ui.horizontal_wrapped(|ui| {
            ui.hyperlink_to(RichText::new("Lantern record").small(), &p.verify_url);
            if let Some(x) = &p.chain.explorer_url {
                ui.hyperlink_to(RichText::new("Blockchain transaction").small(), x);
            }
        });
        if let Some(h) = h {
            for e in &h.events {
                ui.label(RichText::new(format!("{} · {}", e.occurred_at, e.event_type.replace('_', " "))).small().color(Color32::GRAY));
            }
        }
    });
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// A self-contained HTML evidence report built only from recorded Lantern responses.
pub fn report_html(name: &str, sha: &str, records: &[CheckRecord]) -> String {
    let mut rows = String::new();
    for r in records {
        let raw = serde_json::to_string_pretty(&r.response).unwrap_or_default();
        rows.push_str(&format!(
            "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td><details><summary>response</summary><pre>{}</pre></details></td></tr>",
            esc(&r.at_utc),
            esc(&r.kind.replace('_', " ")),
            esc(&r.outcome),
            esc(r.lantern_id.as_deref().unwrap_or("")),
            esc(&raw)
        ));
    }
    format!(
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8"><title>Lantern evidence report — {n}</title>
<style>body{{font:14px/1.5 system-ui,sans-serif;max-width:960px;margin:24px auto;padding:0 16px;color:#1d1f24}}
table{{border-collapse:collapse;width:100%}}td,th{{border:1px solid #ccd;padding:6px;vertical-align:top;text-align:left}}
pre{{white-space:pre-wrap;font-size:12px}}.note{{background:#f4f5f8;padding:10px 14px;border-left:3px solid #4a7bd0}}</style></head><body>
<h1>Lantern evidence report</h1>
<p><b>Artwork file:</b> {n}<br><b>SHA-256:</b> <code>{s}</code><br><b>Report generated:</b> {t} by ShieldCraft {v}</p>
<div class="note"><b>What this report shows.</b> Each row is a request ShieldCraft made to Lantern's public API (api.lantern-us.com),
with the time and Lantern's response as received. A Lantern registration is a timestamped record that can support an ownership
claim; it does not by itself establish legal copyright or prove infringement. "No match" means none was found among works
registered on Lantern at that time; it is not a search of the internet, and it never proves an image was not copied or used for
AI training. Lantern is a third-party service; ShieldCraft is not affiliated with it.</div>
<h2>Recorded checks</h2>
<table><tr><th>Time (UTC)</th><th>Check</th><th>Outcome</th><th>Lantern ID</th><th>Raw response</th></tr>{rows}</table>
</body></html>"#,
        n = esc(name),
        s = esc(sha),
        t = esc(&now()),
        v = shieldcraft_core::APP_VERSION,
        rows = rows
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_escapes_and_states_limits() {
        let rec = CheckRecord {
            at_utc: "2026-10-21T00:00:00Z".into(),
            kind: "hash_lookup".into(),
            file_name: "a<b>.png".into(),
            sha256: "ab".into(),
            outcome: "Not in the registry <script>".into(),
            lantern_id: None,
            response: serde_json::Value::Null,
        };
        let html = report_html("a<b>.png", "ab", &[rec]);
        assert!(!html.contains("<script>") && html.contains("&lt;script&gt;"));
        assert!(html.contains("not a search of the internet"));
        assert!(html.contains("does not by itself establish legal copyright"));
    }
}
