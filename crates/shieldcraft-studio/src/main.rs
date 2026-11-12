#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod branding;
mod canvas;
mod lantern_panel;
mod licenses_window;
mod monitor;
mod panels;
mod preview;
mod tasks;
mod theme;

fn main() -> eframe::Result<()> {
    // Write panics to a log file, since release builds have no console.
    std::panic::set_hook(Box::new(|info| {
        let path = shieldcraft_core::paths::logs_dir().join("crash.log");
        let msg = format!("{} panic: {info}\n", shieldcraft_core::util::format_utc(shieldcraft_core::util::unix_now()));
        let _ = std::fs::OpenOptions::new().create(true).append(true).open(path).and_then(|mut f| std::io::Write::write_all(&mut f, msg.as_bytes()));
        eprintln!("{msg}");
    }));

    let (settings, settings_warning) = shieldcraft_core::settings::Settings::load();
    let welcome = settings.show_welcome;
    let viewport = if welcome {
        egui::ViewportBuilder::default().with_inner_size([780.0, 470.0]).with_decorations(false).with_resizable(false)
    } else {
        egui::ViewportBuilder::default().with_inner_size([1440.0, 900.0]).with_min_inner_size([960.0, 600.0]).with_maximized(true)
    }
    .with_title("ShieldCraft")
    .with_app_id("ShieldCraft")
    .with_icon(std::sync::Arc::new({
        let img = branding::icon(256);
        egui::IconData { width: img.width(), height: img.height(), rgba: img.into_raw() }
    }));

    let options = eframe::NativeOptions { viewport, centered: true, renderer: eframe::Renderer::Glow, ..Default::default() };
    eframe::run_native("ShieldCraft", options, Box::new(move |cc| Ok(Box::new(app::App::new(cc, settings, settings_warning)))))
}
