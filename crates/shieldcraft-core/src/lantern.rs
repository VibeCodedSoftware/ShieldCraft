//! Lantern (https://lantern-us.com) — external provenance registry. NOT open source: a hosted
//! service used through its documented public API, under Lantern's Terms of Service.
//!
//! Only endpoints that need no account are used. Every request is made on an explicit user
//! action; nothing runs in the background. Field names follow Lantern's published OpenAPI spec
//! (`/api/v1/verify/*`) and the docs page; for `/verify/image` and the opt-out check, which the
//! spec does not cover, only fields observed in real responses are read and the raw JSON is kept.

use serde::{Deserialize, Serialize};
use std::time::Duration;

pub const API_BASE: &str = "https://api.lantern-us.com";
pub const SITE: &str = "https://lantern-us.com";
pub const TERMS_URL: &str = "https://lantern-us.com/terms";
pub const DOCS_URL: &str = "https://lantern-us.com/docs";

#[derive(Debug, Clone, PartialEq)]
pub enum LanternError {
    /// HTTP 429: the caller must wait. Lantern sends `Retry-After` in seconds.
    RateLimited { retry_after_s: Option<u64> },
    Network(String),
    Http { status: u16, detail: String },
    Parse(String),
}

impl std::fmt::Display for LanternError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LanternError::RateLimited { retry_after_s } => match retry_after_s {
                Some(s) => write!(f, "Lantern rate limit reached: try again in {s} s"),
                None => write!(f, "Lantern rate limit reached: try again later"),
            },
            LanternError::Network(e) => write!(f, "Could not reach Lantern: {e}"),
            LanternError::Http { status, detail } => write!(f, "Lantern returned HTTP {status}: {detail}"),
            LanternError::Parse(e) => write!(f, "Unexpected response from Lantern: {e}"),
        }
    }
}

// ───────────────────────── documented response types ─────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LinkedPlatform {
    pub platform: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Creator {
    pub display_name: Option<String>,
    #[serde(default)]
    pub verified_platforms: Vec<LinkedPlatform>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Chain {
    pub verified: bool,
    pub code: String,
    pub tx: Option<String>,
    pub explorer_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Display {
    pub severity: String,
    pub label: String,
    pub helper_text: Option<String>,
}

/// `PlatformVerifyResponse` from Lantern's OpenAPI spec.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Provenance {
    pub lantern_id: String,
    /// `active` or `revoked`.
    pub status: String,
    /// `none`, `open`, `upheld` or `rejected`.
    pub dispute_status: String,
    #[serde(default)]
    pub creator: Creator,
    pub registered_at: String,
    #[serde(default)]
    pub chain: Chain,
    #[serde(default)]
    pub display: Display,
    pub verify_url: String,
    /// `pending`, `confirmed`, `not_found` or `error`.
    pub source_status: String,
    pub source_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HistoryEvent {
    pub event_type: String,
    pub occurred_at: String,
    pub actor_display_name: Option<String>,
    pub chain_tx: Option<String>,
    pub explorer_url: Option<String>,
    pub reason_category: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct History {
    pub lantern_id: String,
    #[serde(default)]
    pub events: Vec<HistoryEvent>,
}

/// Result of an image check (`POST /verify/image`). Only fields seen in real responses are
/// typed; everything else is in `raw`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ImageCheck {
    /// e.g. `none`; other values are reported as Lantern sends them.
    pub match_mode: String,
    pub sha256: Option<String>,
    pub dhash: Option<String>,
    pub badge_label: String,
    /// e.g. `not-found`, `warning` (matching could not run), `verified`.
    pub badge_severity: String,
    pub title: String,
    pub helper_text: Option<String>,
    pub link: Option<String>,
    pub exact_found: bool,
    pub similar_count: usize,
    pub raw: serde_json::Value,
}

impl ImageCheck {
    /// Lantern reports "Couldn't check" with a warning badge when matching did not run. That is
    /// not a negative answer and must never be shown as "no match".
    pub fn inconclusive(&self) -> bool {
        self.badge_severity == "warning" || self.badge_severity == "error"
    }
    pub fn no_match(&self) -> bool {
        !self.inconclusive() && self.match_mode == "none" && !self.exact_found && self.similar_count == 0
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OptOut {
    pub found: bool,
    pub raw: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ServiceStatus {
    pub healthy: bool,
    pub health_raw: String,
    pub features_raw: String,
}

// ───────────────────────── client ─────────────────────────

pub struct Client {
    agent: ureq::Agent,
    base: String,
}

impl Default for Client {
    fn default() -> Self {
        Self::new(API_BASE)
    }
}

impl Client {
    pub fn new(base: &str) -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(45)))
            .http_status_as_error(false)
            .user_agent(format!("ShieldCraft/{} (+local desktop app)", crate::APP_VERSION))
            .build()
            .new_agent();
        Self { agent, base: base.trim_end_matches('/').to_string() }
    }

    fn handle(resp: Result<ureq::http::Response<ureq::Body>, ureq::Error>) -> Result<Option<String>, LanternError> {
        let mut resp = resp.map_err(|e| LanternError::Network(e.to_string()))?;
        let status = resp.status().as_u16();
        let retry_after = resp.headers().get("retry-after").and_then(|v| v.to_str().ok()).and_then(|v| v.parse().ok());
        let body = resp.body_mut().read_to_string().unwrap_or_default();
        match status {
            200..=299 => Ok(Some(body)),
            404 => Ok(None),
            429 => Err(LanternError::RateLimited { retry_after_s: retry_after }),
            _ => {
                // Lantern errors: {"detail": {"error": "...", "error_description": "..."}} or {"detail": "..."}
                let detail = serde_json::from_str::<serde_json::Value>(&body)
                    .ok()
                    .and_then(|v| {
                        let d = v.get("detail")?;
                        Some(d.get("error_description").or(d.get("error")).and_then(|x| x.as_str()).map(String::from).unwrap_or_else(|| d.to_string()))
                    })
                    .unwrap_or_else(|| body.chars().take(200).collect());
                Err(LanternError::Http { status, detail })
            }
        }
    }

    fn get(&self, path: &str) -> Result<Option<String>, LanternError> {
        Self::handle(self.agent.get(&format!("{}{path}", self.base)).call())
    }

    pub fn status(&self) -> Result<ServiceStatus, LanternError> {
        let health = self.get("/health")?.unwrap_or_default();
        let features = self.get("/config/features")?.unwrap_or_default();
        let healthy = serde_json::from_str::<serde_json::Value>(&health).ok().and_then(|v| v.get("status").and_then(|s| s.as_str()).map(|s| s == "ok")).unwrap_or(false);
        Ok(ServiceStatus { healthy, health_raw: health, features_raw: features })
    }

    /// Exact lookup by SHA-256 of the file bytes. Sends only the hash.
    pub fn lookup_hash(&self, sha256_hex: &str) -> Result<Option<Provenance>, LanternError> {
        if sha256_hex.len() != 64 || !sha256_hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(LanternError::Parse("not a SHA-256 hex digest".into()));
        }
        match self.get(&format!("/api/v1/verify/hash/sha256:{}", sha256_hex.to_ascii_lowercase()))? {
            None => Ok(None),
            Some(b) => serde_json::from_str(&b).map(Some).map_err(|e| LanternError::Parse(e.to_string())),
        }
    }

    pub fn lookup_id(&self, lantern_id: &str) -> Result<Option<Provenance>, LanternError> {
        let id = normalize_id(lantern_id).ok_or_else(|| LanternError::Parse("Lantern IDs look like LNTN-xxxxxxxx".into()))?;
        match self.get(&format!("/api/v1/verify/{id}"))? {
            None => Ok(None),
            Some(b) => serde_json::from_str(&b).map(Some).map_err(|e| LanternError::Parse(e.to_string())),
        }
    }

    pub fn history(&self, lantern_id: &str) -> Result<Option<History>, LanternError> {
        let id = normalize_id(lantern_id).ok_or_else(|| LanternError::Parse("Lantern IDs look like LNTN-xxxxxxxx".into()))?;
        match self.get(&format!("/api/v1/verify/{id}/history"))? {
            None => Ok(None),
            Some(b) => serde_json::from_str(&b).map(Some).map_err(|e| LanternError::Parse(e.to_string())),
        }
    }

    /// Training opt-out registry lookup (Spawning-compatible). Not a dataset-membership check.
    pub fn optout(&self, sha256_hex: &str) -> Result<OptOut, LanternError> {
        let body = self.get(&format!("/api/v1/optout/check/{}", sha256_hex.to_ascii_lowercase()))?.unwrap_or_else(|| "{}".into());
        let raw: serde_json::Value = serde_json::from_str(&body).map_err(|e| LanternError::Parse(e.to_string()))?;
        Ok(OptOut { found: raw.get("found").and_then(|f| f.as_bool()).unwrap_or(false), raw })
    }

    /// Upload an image to Lantern's matcher (exact, perceptual and neural). UPLOADS THE IMAGE:
    /// only call this after the user has explicitly agreed.
    pub fn check_image(&self, bytes: &[u8], filename: &str) -> Result<ImageCheck, LanternError> {
        let mime = match filename.rsplit('.').next().map(|e| e.to_ascii_lowercase()).as_deref() {
            Some("png") => "image/png",
            Some("jpg") | Some("jpeg") => "image/jpeg",
            Some("webp") => "image/webp",
            Some("gif") => "image/gif",
            Some("tif") | Some("tiff") => "image/tiff",
            _ => return Err(LanternError::Parse("Lantern accepts PNG, JPEG, WebP, GIF or TIFF".into())),
        };
        if bytes.len() > 50 * 1024 * 1024 {
            return Err(LanternError::Parse("Lantern accepts files up to 50 MB".into()));
        }
        let boundary = format!("----shieldcraft{:016x}", crate::util::fresh_seed());
        let safe_name: String = filename.chars().filter(|c| c.is_ascii_alphanumeric() || ".-_".contains(*c)).collect();
        let mut body = Vec::with_capacity(bytes.len() + 512);
        body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"image\"; filename=\"{safe_name}\"\r\nContent-Type: {mime}\r\n\r\n").as_bytes());
        body.extend_from_slice(bytes);
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
        let resp = self
            .agent
            .post(&format!("{}/verify/image", self.base))
            .header("Content-Type", &format!("multipart/form-data; boundary={boundary}"))
            .send(&body[..]);
        let text = Self::handle(resp)?.ok_or_else(|| LanternError::Http { status: 404, detail: "verify endpoint not found".into() })?;
        parse_image_check(&text)
    }
}

pub fn parse_image_check(text: &str) -> Result<ImageCheck, LanternError> {
    let raw: serde_json::Value = serde_json::from_str(text).map_err(|e| LanternError::Parse(e.to_string()))?;
    let s = |v: Option<&serde_json::Value>| v.and_then(|x| x.as_str()).map(String::from);
    let display = raw.get("display");
    let badge = display.and_then(|d| d.get("badge"));
    Ok(ImageCheck {
        match_mode: s(raw.get("match_mode")).unwrap_or_else(|| "unknown".into()),
        sha256: s(raw.get("sha256")),
        dhash: s(raw.get("dhash")),
        badge_label: s(badge.and_then(|b| b.get("label"))).unwrap_or_default(),
        badge_severity: s(badge.and_then(|b| b.get("severity"))).unwrap_or_default(),
        title: s(display.and_then(|d| d.get("title"))).unwrap_or_default(),
        helper_text: s(display.and_then(|d| d.get("helper_text"))),
        link: s(display.and_then(|d| d.get("link"))),
        exact_found: raw.get("exact").map(|e| !e.is_null()).unwrap_or(false),
        similar_count: raw.get("similar").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0),
        raw,
    })
}

/// Accepts `LNTN-xxxxxxxx` (case-insensitive prefix) and returns it in canonical form.
pub fn normalize_id(id: &str) -> Option<String> {
    let id = id.trim();
    let rest = id.get(..5).filter(|p| p.eq_ignore_ascii_case("LNTN-")).map(|_| &id[5..])?;
    (rest.len() == 8 && rest.chars().all(|c| c.is_ascii_alphanumeric())).then(|| format!("LNTN-{rest}"))
}

pub fn sha256_bytes(bytes: &[u8]) -> String {
    use sha2::Digest;
    format!("{:x}", sha2::Sha256::digest(bytes))
}

// ───────────────────────── local evidence log ─────────────────────────

/// One recorded check, kept so results stay traceable (what was asked, when, and the answer).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckRecord {
    pub at_utc: String,
    /// `hash_lookup`, `image_check`, `optout_check`, `id_lookup`.
    pub kind: String,
    pub file_name: String,
    pub sha256: String,
    /// Short human-readable outcome, exactly as determined from Lantern's answer.
    pub outcome: String,
    pub lantern_id: Option<String>,
    pub response: serde_json::Value,
}

pub fn log_path() -> std::path::PathBuf {
    crate::paths::data_root().join("lantern-checks.json")
}

pub fn load_log() -> Vec<CheckRecord> {
    std::fs::read(log_path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

pub fn append_log(rec: CheckRecord) -> Result<(), String> {
    let mut all = load_log();
    all.push(rec);
    let bytes = serde_json::to_vec_pretty(&all).map_err(|e| e.to_string())?;
    crate::image_io::write_atomic(&log_path(), &bytes, None, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Recorded from real Lantern responses on 2026-10-21 (synthetic test image, not artwork).
    // Test fixtures only — never shown to users as findings.
    const NOT_FOUND: &str = r#"{"match_mode":"none","sha256":"sha256:dc2ef61e3e16461a619397af5f961dc9c936619ce3a8b55736e79e57aa9a9c46","dhash":"fcfdfbf3e7efdf9f","exact":null,"similar":[],"display":{"badge":{"label":"Not Found","severity":"not-found"},"title":"No match yet","subtitle":null,"helper_text":"This image isn't in the Lantern registry.","link":null,"claimable":false},"cacheable_for_seconds":300}"#;
    const COULD_NOT_CHECK: &str = r#"{"match_mode":"none","sha256":"sha256:dc2e","dhash":"fcfd","exact":null,"similar":[],"display":{"badge":{"label":"Couldn't check","severity":"warning"},"title":"Couldn't check right now","subtitle":null,"helper_text":"Image matching didn't respond in time.","link":null,"claimable":false},"cacheable_for_seconds":5}"#;

    #[test]
    fn not_found_is_a_negative_answer() {
        let c = parse_image_check(NOT_FOUND).unwrap();
        assert!(c.no_match() && !c.inconclusive());
        assert_eq!(c.dhash.as_deref(), Some("fcfdfbf3e7efdf9f"));
    }

    #[test]
    fn timeout_is_inconclusive_not_no_match() {
        let c = parse_image_check(COULD_NOT_CHECK).unwrap();
        assert!(c.inconclusive());
        assert!(!c.no_match(), "a matcher timeout must never read as 'no match'");
    }

    #[test]
    fn provenance_schema_parses() {
        let j = r#"{"lantern_id":"LNTN-abc12345","status":"active","dispute_status":"none","creator":{"display_name":null,"verified_platforms":[{"platform":"pixiv"}]},"registered_at":"2026-01-01T00:00:00Z","chain":{"verified":true,"code":"confirmed","tx":"0x1","explorer_url":null},"display":{"severity":"verified","label":"Verified","helper_text":null},"verify_url":"https://lantern-us.com/verify/LNTN-abc12345","source_status":"confirmed","source_url":null}"#;
        let p: Provenance = serde_json::from_str(j).unwrap();
        assert_eq!(p.creator.verified_platforms[0].platform, "pixiv");
        assert!(p.chain.verified);
    }

    #[test]
    fn id_normalisation() {
        assert_eq!(normalize_id(" lntn-AbC12345 ").as_deref(), Some("LNTN-AbC12345"));
        assert_eq!(normalize_id("LNTN-abc"), None);
        assert_eq!(normalize_id("XXXX-abc12345"), None);
    }
}
