//! Searchable Third-Party Licenses & Credits window.

use shieldcraft_core::licenses::{license_text, Status, COMPONENTS, STARTUP_STATEMENT};
use egui::RichText;

#[derive(Default)]
pub struct LicensesWindow {
    pub open: bool,
    search: String,
    selected: usize,
    text_cache: Option<(String, String)>,
}

impl LicensesWindow {
    pub fn show(&mut self, ctx: &egui::Context) {
        if !self.open {
            return;
        }
        let mut open = self.open;
        egui::Window::new(format!("{}  Third-Party Licenses & Credits", egui_phosphor::regular::SCROLL))
            .open(&mut open)
            .default_size([900.0, 600.0])
            .min_size([640.0, 360.0])
            .collapsible(false)
            .show(ctx, |ui| {
                ui.label(RichText::new(STARTUP_STATEMENT).small());
                ui.label(
                    RichText::new("This window is informational. It does not change, waive or override any third-party license terms.")
                        .small()
                        .italics(),
                );
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label(egui_phosphor::regular::MAGNIFYING_GLASS);
                    ui.add(egui::TextEdit::singleline(&mut self.search).hint_text("Search projects, authors, licenses…").desired_width(280.0));
                });
                let q = self.search.to_lowercase();
                let matches: Vec<usize> = COMPONENTS
                    .iter()
                    .enumerate()
                    .filter(|(_, c)| q.is_empty() || [c.name, c.author, c.license, c.notes, c.status.label()].iter().any(|s| s.to_lowercase().contains(&q)))
                    .map(|(i, _)| i)
                    .collect();
                if !matches.contains(&self.selected) {
                    if let Some(&f) = matches.first() {
                        self.selected = f;
                    }
                }
                egui::SidePanel::left("lic_list").resizable(true).default_width(280.0).show_inside(ui, |ui| {
                    egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
                        for status in [Status::ThisProject, Status::IndependentImplementation, Status::Included, Status::ExternalService, Status::DesignReference] {
                            let group: Vec<usize> = matches.iter().copied().filter(|&i| COMPONENTS[i].status == status).collect();
                            if group.is_empty() {
                                continue;
                            }
                            crate::theme::caption(ui, status.label());
                            for i in group {
                                let c = &COMPONENTS[i];
                                let r = ui.selectable_label(self.selected == i, format!("{}\nby {}", c.name, c.author));
                                if r.clicked() {
                                    self.selected = i;
                                }
                            }
                            ui.add_space(4.0);
                        }
                    });
                });
                egui::CentralPanel::default().show_inside(ui, |ui| {
                    let Some(c) = COMPONENTS.get(self.selected) else { return };
                    egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
                        ui.heading(c.name);
                        egui::Grid::new("lic_meta").num_columns(2).spacing([12.0, 4.0]).show(ui, |ui| {
                            ui.label(RichText::new("Created by").weak());
                            ui.label(RichText::new(c.author).strong());
                            ui.end_row();
                            ui.label(RichText::new("Project").weak());
                            if !c.url.is_empty() {
                                    ui.hyperlink(c.url);
                                }
                            ui.end_row();
                            ui.label(RichText::new("Status").weak());
                            ui.label(c.status.label());
                            ui.end_row();
                            ui.label(RichText::new("License").weak());
                            ui.label(c.license);
                            ui.end_row();
                        });
                        ui.add_space(6.0);
                        ui.label(c.notes);
                        ui.add_space(8.0);
                        if let Some(file) = c.license_file {
                            crate::theme::caption(ui, &format!("Full text — {file}"));
                            if self.text_cache.as_ref().map(|(f, _)| f != file).unwrap_or(true) {
                                let t = license_text(file).unwrap_or_else(|| {
                                    format!("The file licenses/{file} was not found next to the application. It ships in the release package and in the project's licenses folder.")
                                });
                                self.text_cache = Some((file.to_string(), t));
                            }
                            let mut text = self.text_cache.as_ref().unwrap().1.as_str();
                            ui.add(egui::TextEdit::multiline(&mut text).font(egui::TextStyle::Monospace).desired_width(f32::INFINITY).desired_rows(18));
                        } else {
                            ui.label(RichText::new("No license text is shipped for this entry because no code or assets from it are included in ShieldCraft.").weak());
                        }
                    });
                });
            });
        self.open = open;
    }
}
