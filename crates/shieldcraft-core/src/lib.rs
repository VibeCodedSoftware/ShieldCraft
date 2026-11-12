//! ShieldCraft core: image model, engines, settings, presets and license inventory.

pub mod engine;
pub mod engines;
pub mod hardware;
pub mod image_io;
pub mod imgproc;
pub mod lantern;
pub mod licenses;
pub mod paths;
pub mod presets;
pub mod report;
pub mod selection;
pub mod settings;
pub mod text;
pub mod util;

pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const BUILD_PROFILE: &str = if cfg!(debug_assertions) { "debug" } else { "release" };
