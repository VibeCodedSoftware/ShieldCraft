//! Hope:RE ONNX model files: where they come from, how large they are, how to verify them.

use crate::util::sha256_file;
use std::path::PathBuf;

pub struct ModelSpec {
    pub file: &'static str,
    pub url: &'static str,
    pub bytes: u64,
    /// SHA-256 measured from the Hope:RE v2.5.1 release assets on 2026-10-21. The noise model's
    /// hash also matches the Git LFS pointer committed in the Hope:RE repository.
    pub sha256: &'static str,
}

pub const NOISE: ModelSpec = ModelSpec {
    file: "noise_algorithm.onnx",
    url: "https://github.com/HopeArtOrg/hope-re/releases/download/v2.5.1/noise_algorithm.onnx",
    bytes: 351_470_439,
    sha256: "83db45e0fcf2ae7f95822fc2afbffd299ba71a79e01fc3f274c35de662a2e07d",
};
pub const STYLE: ModelSpec = ModelSpec {
    file: "glaze_algorithm.onnx",
    url: "https://github.com/HopeArtOrg/hope-re/releases/download/v2.5.1/glaze_algorithm.onnx",
    bytes: 351_461_680,
    sha256: "3cb3d787aa1e7a26a352b9066644b8478d7bf12b06cd66ca775c6daf7a3a137b",
};
pub const POISON: ModelSpec = ModelSpec {
    file: "nightshade_algorithm.onnx",
    url: "https://github.com/HopeArtOrg/hope-re/releases/download/v2.5.1/nightshade_algorithm.onnx",
    bytes: 351_467_826,
    sha256: "50998dcf6ea6d5455d71c13a21c98e1e25365ce63e021bc6d0caac6320a07a68",
};

pub fn model_path(spec: &ModelSpec) -> PathBuf {
    crate::paths::models_dir().join("hope-re").join(spec.file)
}

#[derive(Clone, Debug, PartialEq)]
pub enum ModelState {
    Missing,
    /// Present with the expected size (hash not yet verified — that takes a few seconds).
    Present,
    Verified,
    Corrupt(String),
}

pub fn quick_state(spec: &ModelSpec) -> ModelState {
    match std::fs::metadata(model_path(spec)) {
        Err(_) => ModelState::Missing,
        Ok(m) if m.len() == spec.bytes => ModelState::Present,
        Ok(m) if m.len() < 1024 => ModelState::Corrupt("File is a Git LFS pointer or truncated".into()),
        Ok(m) => ModelState::Corrupt(format!("Unexpected size {} bytes (expected {})", m.len(), spec.bytes)),
    }
}

pub fn verify(spec: &ModelSpec) -> ModelState {
    let path = model_path(spec);
    match quick_state(spec) {
        ModelState::Present => match sha256_file(&path) {
            Ok(h) if h == spec.sha256 => ModelState::Verified,
            Ok(h) => ModelState::Corrupt(format!("SHA-256 mismatch ({}…)", &h[..12])),
            Err(e) => ModelState::Corrupt(e.to_string()),
        },
        other => other,
    }
}

/// Download a model with progress, writing to a temp file and verifying the hash before it is
/// moved into place. Only called after the user explicitly confirms the download.
pub fn download(spec: &ModelSpec, progress: &dyn Fn(u64, u64), cancel: &crate::engine::CancelToken) -> Result<(), String> {
    use std::io::{Read, Write};
    let dest = model_path(spec);
    std::fs::create_dir_all(dest.parent().unwrap()).map_err(|e| e.to_string())?;
    let tmp = dest.with_extension("onnx.download");
    let resp = ureq::get(spec.url).call().map_err(|e| format!("Download failed: {e}"))?;
    let total = resp
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
        .unwrap_or(spec.bytes);
    let mut reader = resp.into_body().into_reader();
    let mut file = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
    let mut buf = vec![0u8; 1 << 16];
    let mut done = 0u64;
    loop {
        if cancel.is_cancelled() {
            drop(file);
            let _ = std::fs::remove_file(&tmp);
            return Err("Download cancelled".into());
        }
        let n = reader.read(&mut buf).map_err(|e| format!("Download interrupted: {e}"))?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        done += n as u64;
        progress(done, total);
    }
    drop(file);
    let hash = sha256_file(&tmp).map_err(|e| e.to_string())?;
    if hash != spec.sha256 {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("Downloaded file failed verification (SHA-256 {}…). It was deleted.", &hash[..12]));
    }
    std::fs::rename(&tmp, &dest).map_err(|e| e.to_string())
}
