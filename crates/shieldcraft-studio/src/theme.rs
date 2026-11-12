//! Restrained, information-dense visual style (dark charcoal by default, light optional).

use shieldcraft_core::settings::Theme;
use egui::{Color32, CornerRadius, FontFamily, FontId, Stroke, TextStyle, Visuals};

pub const ACCENT: Color32 = Color32::from_rgb(64, 140, 240);
pub const OK: Color32 = Color32::from_rgb(96, 190, 120);
pub const WARN: Color32 = Color32::from_rgb(226, 170, 64);
pub const BAD: Color32 = Color32::from_rgb(226, 92, 84);

pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    ctx.set_fonts(fonts);
}

pub fn apply(ctx: &egui::Context, theme: Theme, scale: f32) {
    ctx.set_pixels_per_point(ctx.native_pixels_per_point().unwrap_or(1.0) * scale.clamp(0.75, 2.0));
    let mut v = match theme {
        Theme::Dark => {
            let mut v = Visuals::dark();
            v.panel_fill = Color32::from_rgb(38, 39, 43);
            v.window_fill = Color32::from_rgb(44, 45, 50);
            v.extreme_bg_color = Color32::from_rgb(24, 25, 28);
            v.faint_bg_color = Color32::from_rgb(46, 47, 52);
            v.code_bg_color = Color32::from_rgb(30, 31, 35);
            v.widgets.noninteractive.bg_fill = Color32::from_rgb(38, 39, 43);
            v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(56, 58, 64));
            v.widgets.inactive.bg_fill = Color32::from_rgb(56, 58, 64);
            v.widgets.inactive.weak_bg_fill = Color32::from_rgb(52, 54, 60);
            v.widgets.hovered.bg_fill = Color32::from_rgb(68, 70, 78);
            v.widgets.hovered.weak_bg_fill = Color32::from_rgb(64, 66, 74);
            v.widgets.active.bg_fill = Color32::from_rgb(58, 96, 150);
            v
        }
        Theme::Light => {
            let mut v = Visuals::light();
            v.panel_fill = Color32::from_rgb(236, 237, 240);
            v.window_fill = Color32::from_rgb(244, 245, 247);
            v.extreme_bg_color = Color32::from_rgb(214, 216, 220);
            v.faint_bg_color = Color32::from_rgb(228, 229, 233);
            v
        }
    };
    v.selection.bg_fill = ACCENT.linear_multiply(0.8);
    v.selection.stroke = Stroke::new(1.0, Color32::WHITE);
    v.hyperlink_color = ACCENT;
    v.window_corner_radius = CornerRadius::same(4);
    v.menu_corner_radius = CornerRadius::same(3);
    for w in [&mut v.widgets.noninteractive, &mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
        w.corner_radius = CornerRadius::same(2);
    }
    v.slider_trailing_fill = true;
    ctx.set_visuals(v);
    ctx.style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(6.0, 4.0);
        s.spacing.button_padding = egui::vec2(6.0, 2.0);
        s.spacing.interact_size.y = 20.0;
        s.spacing.slider_width = 140.0;
        s.text_styles = [
            (TextStyle::Small, FontId::new(10.5, FontFamily::Proportional)),
            (TextStyle::Body, FontId::new(12.5, FontFamily::Proportional)),
            (TextStyle::Button, FontId::new(12.5, FontFamily::Proportional)),
            (TextStyle::Heading, FontId::new(15.0, FontFamily::Proportional)),
            (TextStyle::Monospace, FontId::new(11.5, FontFamily::Monospace)),
        ]
        .into();
    });
}

/// Small uppercase section caption.
pub fn caption(ui: &mut egui::Ui, text: &str) {
    ui.add_space(2.0);
    ui.label(egui::RichText::new(text.to_uppercase()).small().strong().color(ui.visuals().weak_text_color()));
}
