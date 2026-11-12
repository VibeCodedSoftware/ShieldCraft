//! Where ShieldCraft keeps settings, models, engine environments and logs.
//!
//! Everything lives under `%LOCALAPPDATA%\ShieldCraft` by default. A portable install can
//! override this by placing a file named `portable.flag` next to the executable, in which case
//! a `data` folder beside the executable is used instead.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

static ROOT: OnceLock<PathBuf> = OnceLock::new();

pub fn data_root() -> &'static Path {
    ROOT.get_or_init(|| {
        if let Some(dir) = std::env::var_os("SHIELDCRAFT_DATA").filter(|v| !v.is_empty()) {
            return PathBuf::from(dir);
        }
        if let Some(dir) = exe_dir() {
            if dir.join("portable.flag").exists() {
                return dir.join("data");
            }
        }
        directories::BaseDirs::new()
            .map(|b| {
                let new = b.data_local_dir().join("ShieldCraft");
                // The app was called "ArtShield Studio" before; carry its data (settings, presets,
                // ~1 GB of models) over instead of downloading again. Same drive, so this is a rename.
                let old = b.data_local_dir().join("ArtShieldStudio");
                if !new.exists() && old.is_dir() {
                    let _ = std::fs::rename(&old, &new);
                }
                new
            })
            .unwrap_or_else(|| PathBuf::from("ShieldCraft-data"))
    })
}

pub fn exe_dir() -> Option<PathBuf> {
    std::env::current_exe().ok()?.parent().map(Path::to_path_buf)
}

fn ensure(p: PathBuf) -> PathBuf {
    let _ = std::fs::create_dir_all(&p);
    p
}

pub fn settings_file() -> PathBuf {
    ensure(data_root().to_path_buf()).join("settings.json")
}
pub fn presets_dir() -> PathBuf {
    ensure(data_root().join("presets"))
}
pub fn models_dir() -> PathBuf {
    ensure(data_root().join("models"))
}
pub fn engines_dir() -> PathBuf {
    ensure(data_root().join("engines"))
}
pub fn logs_dir() -> PathBuf {
    ensure(data_root().join("logs"))
}
pub fn temp_dir() -> PathBuf {
    ensure(data_root().join("tmp"))
}
pub fn history_file() -> PathBuf {
    ensure(data_root().to_path_buf()).join("history.json")
}

/// Locate a bundled resource directory (e.g. `licenses`, `workers`). Looks next to the
/// executable first, then walks up from it so `cargo run` from the workspace also works.
pub fn resource_dir(name: &str) -> Option<PathBuf> {
    let mut dir = exe_dir()?;
    for _ in 0..5 {
        let candidate = dir.join(name);
        if candidate.is_dir() {
            return Some(candidate);
        }
        if !dir.pop() {
            break;
        }
    }
    None
}
