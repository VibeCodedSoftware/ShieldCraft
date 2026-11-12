//! Grid Noise Studio settings. Every field maps to an implemented behaviour in `render.rs`.
//! Pixel-sized settings are expressed in *output* pixels (the animation's render size); the
//! preview scales them so it stays a faithful miniature of the export.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CellShape {
    /// Regular rectangular grid.
    Square,
    /// Irregular Voronoi cells from jittered seeds ("enhanced" mode).
    Voronoi,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Balance {
    /// Independent uniform draws per cell and frame (white noise). Zero-mean on average.
    Random,
    /// Checkerboard cells move in opposite phase each frame (blue-noise-like, smoother look).
    Antiphase,
    /// Deterministic phase cycle: every N consecutive frames sum exactly to zero (before clipping).
    Cycle,
}

impl Balance {
    pub fn label(self) -> &'static str {
        match self {
            Balance::Random => "Random (white)",
            Balance::Antiphase => "Antiphase (blue)",
            Balance::Cycle => "Exact cycle",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AnimFormat {
    Gif,
    Apng,
    Mp4,
    WebM,
    PngSequence,
}

impl AnimFormat {
    pub const ALL: [AnimFormat; 5] = [AnimFormat::Mp4, AnimFormat::WebM, AnimFormat::Gif, AnimFormat::Apng, AnimFormat::PngSequence];
    pub fn label(self) -> &'static str {
        match self {
            AnimFormat::Gif => "GIF (built-in, 256 colours)",
            AnimFormat::Apng => "APNG (built-in, lossless, alpha)",
            AnimFormat::Mp4 => "MP4 H.264 (requires FFmpeg)",
            AnimFormat::WebM => "WebM VP9 (requires FFmpeg, alpha)",
            AnimFormat::PngSequence => "PNG frame sequence (built-in)",
        }
    }
    pub fn ext(self) -> &'static str {
        match self {
            AnimFormat::Gif => "gif",
            AnimFormat::Apng => "png",
            AnimFormat::Mp4 => "mp4",
            AnimFormat::WebM => "webm",
            AnimFormat::PngSequence => "",
        }
    }
    pub fn needs_ffmpeg(self) -> bool {
        matches!(self, AnimFormat::Mp4 | AnimFormat::WebM)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Corner {
    BottomLeft,
    BottomRight,
    TopLeft,
    TopRight,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Toggle {
    pub enabled: bool,
    /// 0..=100 %
    pub strength: f32,
}

impl Default for Toggle {
    fn default() -> Self {
        Self { enabled: false, strength: 30.0 }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DetailSplit {
    pub enabled: bool,
    /// Blur radius separating the colour layer from the line/edge layer (output px).
    pub radius: f32,
    /// Random per-cell gain applied to the edge layer, ±%.
    pub edge_strength: f32,
}

impl Default for DetailSplit {
    fn default() -> Self {
        Self { enabled: false, radius: 25.0, edge_strength: 10.0 }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Grating {
    pub enabled: bool,
    /// Peak amplitude of the summed gratings in 0..255 units.
    pub amplitude: f32,
    /// Number of sinusoidal components per channel.
    pub components: u32,
    /// Grating period range in output pixels.
    pub min_period: f32,
    pub max_period: f32,
}

impl Default for Grating {
    fn default() -> Self {
        Self { enabled: true, amplitude: 6.0, components: 5, min_period: 2.0, max_period: 5.0 }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TextTile {
    pub enabled: bool,
    pub text: String,
    /// Contrast in 0..255 units (2..60).
    pub contrast: f32,
    pub dark: bool,
}

impl Default for TextTile {
    fn default() -> Self {
        Self { enabled: false, text: "NO-AI-TRAINING".into(), contrast: 22.0, dark: false }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct VisibleMark {
    pub enabled: bool,
    pub text: String,
    pub corner: Corner,
    /// Background box opacity 0..1.
    pub opacity: f32,
    /// Text height as % of the shorter image side.
    pub size_pct: f32,
}

impl Default for VisibleMark {
    fn default() -> Self {
        Self { enabled: true, text: "No AI Training".into(), corner: Corner::BottomLeft, opacity: 0.55, size_pct: 2.8 }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Region {
    /// Closed polygons in normalised (0..1) image coordinates.
    pub polygons: Vec<Vec<[f32; 2]>>,
    /// If true, noise is applied inside the polygons; otherwise outside them.
    pub apply_inside: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AnimationOut {
    pub fps: u32,
    pub duration_s: f32,
    pub format: AnimFormat,
    /// Longest output side in pixels; the source is downscaled to fit (never upscaled).
    pub max_side: u32,
    /// Optional audio file muxed into MP4/WebM (FFmpeg only).
    pub audio_path: Option<String>,
    pub mp4_crf: u32,
}

impl Default for AnimationOut {
    fn default() -> Self {
        Self { fps: 30, duration_s: 4.0, format: AnimFormat::Gif, max_side: 1280, audio_path: None, mp4_crf: 16 }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GridNoiseParams {
    pub seed: u64,
    pub shape: CellShape,
    /// Number of cells along the longer image side.
    pub cells: u32,
    /// Voronoi seeds are re-generated every N frames (0 = never).
    pub voronoi_reseed: u32,
    pub balance: Balance,
    pub cycle_frames: u32,
    /// Hue variance ±degrees (applied as a zero-sum colour-axis offset).
    pub hue_deg: f32,
    /// Lightness variance ± percentage points.
    pub lightness: f32,
    /// Saturation variance ± percentage points.
    pub saturation: f32,
    pub warp_enabled: bool,
    /// Per-cell sampling displacement, ± output px.
    pub warp_px: f32,
    pub blur_enabled: bool,
    /// Gaussian blur sigma applied to each frame (output px).
    pub blur_px: f32,
    pub detail: DetailSplit,
    pub iridescent: Toggle,
    pub sparkle: Toggle,
    pub cell_rainbow: Toggle,
    pub grating: Grating,
    pub text_tile: TextTile,
    pub visible_mark: VisibleMark,
    pub region: Region,
    pub animation: AnimationOut,
    /// Keep the source's transparency in formats that support it.
    pub preserve_alpha: bool,
}

impl Default for GridNoiseParams {
    fn default() -> Self {
        Self {
            seed: 20_261_021,
            shape: CellShape::Square,
            cells: 32,
            voronoi_reseed: 10,
            balance: Balance::Random,
            cycle_frames: 4,
            hue_deg: 15.0,
            lightness: 5.0,
            saturation: 5.0,
            warp_enabled: true,
            warp_px: 2.0,
            blur_enabled: true,
            blur_px: 1.0,
            detail: DetailSplit::default(),
            iridescent: Toggle::default(),
            sparkle: Toggle::default(),
            cell_rainbow: Toggle::default(),
            grating: Grating::default(),
            text_tile: TextTile::default(),
            visible_mark: VisibleMark::default(),
            region: Region::default(),
            animation: AnimationOut::default(),
            preserve_alpha: true,
        }
    }
}

impl GridNoiseParams {
    pub fn validate(&self) -> Result<(), String> {
        if !(2..=512).contains(&self.cells) {
            return Err("Grid cells must be between 2 and 512".into());
        }
        if self.balance == Balance::Cycle && !(2..=60).contains(&self.cycle_frames) {
            return Err("Cycle length must be between 2 and 60 frames".into());
        }
        if !(1..=120).contains(&self.animation.fps) {
            return Err("Frame rate must be between 1 and 120 fps".into());
        }
        if !(0.1..=600.0).contains(&self.animation.duration_s) {
            return Err("Duration must be between 0.1 and 600 seconds".into());
        }
        if !(16..=8192).contains(&self.animation.max_side) {
            return Err("Output size must be between 16 and 8192 px".into());
        }
        for v in [self.hue_deg, self.lightness, self.saturation, self.warp_px, self.blur_px, self.grating.amplitude] {
            if !v.is_finite() || v < 0.0 {
                return Err("Numeric settings must be non-negative".into());
            }
        }
        if self.grating.min_period < 1.0 || self.grating.max_period < self.grating.min_period {
            return Err("Grating period range is invalid".into());
        }
        Ok(())
    }

    /// Total number of frames for the configured duration.
    pub fn frame_count(&self) -> u32 {
        ((self.animation.duration_s * self.animation.fps as f32).round() as u32).max(1)
    }
}
