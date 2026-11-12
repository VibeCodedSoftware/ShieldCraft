//! Persistent user preferences (JSON in the data folder).

use crate::image_io::ExportOptions;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Theme {
    Dark,
    Light,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub show_welcome: bool,
    pub theme: Theme,
    pub ui_scale: f32,
    pub export: ExportOptions,
    pub last_output_dir: Option<String>,
    pub last_open_dir: Option<String>,
    pub ffmpeg_path: Option<String>,
    /// Longest side of the live preview; larger previews look sharper but play slower.
    pub preview_max_side: u32,
    pub recent_files: Vec<String>,
    /// Allow requests to Lantern (api.lantern-us.com). Off until the user turns it on.
    pub lantern_enabled: bool,
    /// Selection brush diameter in screen pixels at 100% zoom (= source pixels).
    pub brush_size: f32,
    /// Soften selection edges by this many source pixels when protecting.
    pub selection_feather: f32,
    /// Tint for the unselected area.
    pub selection_color: [u8; 3],
    pub selection_opacity: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            show_welcome: true,
            theme: Theme::Dark,
            ui_scale: 1.0,
            export: ExportOptions::default(),
            last_output_dir: None,
            last_open_dir: None,
            ffmpeg_path: None,
            preview_max_side: 960,
            recent_files: vec![],
            lantern_enabled: false,
            brush_size: 60.0,
            selection_feather: 2.0,
            selection_color: [220, 50, 60],
            selection_opacity: 0.45,
        }
    }
}

impl Settings {
    pub fn load() -> (Settings, Option<String>) {
        let path = crate::paths::settings_file();
        match std::fs::read(&path) {
            Err(_) => (Settings::default(), None),
            Ok(bytes) => match serde_json::from_slice(&bytes) {
                Ok(s) => (s, None),
                Err(e) => {
                    let backup = path.with_extension("json.bad");
                    let _ = std::fs::rename(&path, &backup);
                    (Settings::default(), Some(format!("Settings file was unreadable ({e}); defaults restored, old file kept as settings.json.bad")))
                }
            },
        }
    }

    pub fn save(&self) -> Result<(), String> {
        let bytes = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        crate::image_io::write_atomic(&crate::paths::settings_file(), &bytes, None, false)
    }

    pub fn push_recent(&mut self, path: &str) {
        self.recent_files.retain(|p| p != path);
        self.recent_files.insert(0, path.to_string());
        self.recent_files.truncate(10);
    }
}
