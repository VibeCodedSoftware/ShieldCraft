//! The central image canvas.

use egui::{pos2, vec2, Color32, Pos2, Rect, Sense, Stroke, TextureHandle, Vec2};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewMode {
    Result,
    Original,
    Split,
    SideBySide,
    Overlay,
    Average,
    /// Amplified |protected − original| — makes invisible perturbations visible.
    Difference,
}

impl ViewMode {
    pub const ALL: [ViewMode; 7] =
        [ViewMode::Result, ViewMode::Original, ViewMode::Split, ViewMode::SideBySide, ViewMode::Overlay, ViewMode::Average, ViewMode::Difference];
    pub fn label(self) -> &'static str {
        match self {
            ViewMode::Result => "Protected",
            ViewMode::Original => "Original",
            ViewMode::Split => "Split",
            ViewMode::SideBySide => "Side by side",
            ViewMode::Overlay => "Overlay",
            ViewMode::Average => "Frame average",
            ViewMode::Difference => "Difference",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Hand,
    Zoom,
    /// Rectangular selection (M).
    Marquee,
    /// Elliptical selection (Shift+M toggles with Marquee).
    Ellipse,
    Lasso,
    /// Paint the selection; Alt erases (B).
    Brush,
    Inspect,
}

impl Tool {
    pub fn selects(self) -> bool {
        matches!(self, Tool::Marquee | Tool::Ellipse | Tool::Lasso | Tool::Brush)
    }
}

/// A change to the selection, in source-image pixel coordinates (polygons normalised 0..1).
#[derive(Clone, Debug)]
pub enum SelEdit {
    Rect((f32, f32), (f32, f32), shieldcraft_core::selection::Op),
    Ellipse((f32, f32), (f32, f32), shieldcraft_core::selection::Op),
    Polygon(Vec<[f32; 2]>, shieldcraft_core::selection::Op),
    /// One brush segment while dragging. `first` is true for the first segment of a stroke.
    Brush { from: (f32, f32), to: (f32, f32), erase: bool },
}

fn op_from(m: &egui::Modifiers) -> shieldcraft_core::selection::Op {
    use shieldcraft_core::selection::Op;
    if m.alt {
        Op::Subtract
    } else if m.shift {
        Op::Add
    } else {
        Op::Replace
    }
}

pub struct CanvasState {
    pub zoom: f32,
    pub offset: Vec2,
    pub fit_pending: bool,
    pub split: f32,
    pub overlay_alpha: f32,
    pub show_grid: bool,
    /// Amplification for the Difference view.
    pub diff_gain: f32,
    lasso: Vec<[f32; 2]>,
    drag_start: Option<(f32, f32)>,
    drag_op: Option<shieldcraft_core::selection::Op>,
    brush_last: Option<(f32, f32)>,
    checker: Option<TextureHandle>,
    /// Keep fitting to the window until the user zooms or pans.
    auto_fit: bool,
    last_size: Vec2,
}

impl Default for CanvasState {
    fn default() -> Self {
        Self { zoom: 1.0, offset: Vec2::ZERO, fit_pending: true, split: 0.5, overlay_alpha: 0.5, show_grid: false, diff_gain: 10.0, lasso: vec![], drag_start: None, drag_op: None, brush_last: None, checker: None, auto_fit: true, last_size: Vec2::ZERO }
    }
}

pub struct Inputs<'a> {
    pub image_size: [u32; 2],
    pub original: Option<&'a TextureHandle>,
    pub result: Option<&'a TextureHandle>,
    pub diff: Option<&'a TextureHandle>,
    pub mode: ViewMode,
    pub tool: Tool,
    pub grid: Option<(usize, usize)>,
    pub regions: &'a [Vec<[f32; 2]>],
    pub regions_inside: bool,
    pub drop_hover: bool,
    /// Tinted overlay marking the unselected area.
    pub selection_overlay: Option<&'a TextureHandle>,
    /// Brush radius in source pixels.
    pub brush_radius: f32,
}

#[derive(Default)]
pub struct Output {
    pub hover_px: Option<(u32, u32)>,
    pub sel_edits: Vec<SelEdit>,
}

impl CanvasState {
    pub fn zoom_percent(&self, ctx: &egui::Context) -> f32 {
        self.zoom * ctx.pixels_per_point() * 100.0
    }
    pub fn set_actual_pixels(&mut self, ctx: &egui::Context) {
        self.zoom = 1.0 / ctx.pixels_per_point();
        self.offset = Vec2::ZERO;
        self.auto_fit = false;
    }
    pub fn zoom_by(&mut self, factor: f32) {
        self.zoom = (self.zoom * factor).clamp(0.02, 64.0);
        self.auto_fit = false;
    }

    pub fn show(&mut self, ui: &mut egui::Ui, inp: Inputs) -> Output {
        let mut out = Output::default();
        let (rect, resp) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, ui.visuals().extreme_bg_color);

        let [iw, ih] = inp.image_size;
        if iw == 0 || inp.original.is_none() {
            empty_state(&painter, rect, inp.drop_hover);
            return out;
        }
        let side = inp.mode == ViewMode::SideBySide;
        let content_w = if side { iw as f32 * 2.0 + 24.0 / self.zoom.max(0.01) } else { iw as f32 };
        if self.fit_pending {
            self.auto_fit = true;
        }
        if self.auto_fit && (self.last_size - rect.size()).length() > 1.0 {
            self.fit_pending = true;
        }
        self.last_size = rect.size();
        if self.fit_pending {
            self.zoom =((rect.width() - 40.0) / content_w).min((rect.height() - 40.0) / ih as f32).max(0.02);
            self.offset = Vec2::ZERO;
            self.fit_pending = false;
        }

        // Navigation.
        if resp.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y + (i.zoom_delta() - 1.0) * 400.0);
            if scroll != 0.0 {
                if let Some(p) = resp.hover_pos() {
                    let before = (p - rect.center() - self.offset) / self.zoom;
                    self.zoom_by((scroll * 0.0018).exp());
                    self.offset = p - rect.center() - before * self.zoom;
                }
            }
        }
        let panning = inp.tool == Tool::Hand || ui.input(|i| i.key_down(egui::Key::Space) || i.pointer.middle_down());
        if resp.dragged() && (panning || resp.dragged_by(egui::PointerButton::Middle)) {
            self.offset += resp.drag_delta();
            self.auto_fit = false;
        }
        if inp.tool == Tool::Zoom && resp.clicked() {
            let f = if ui.input(|i| i.modifiers.alt) { 0.5 } else { 2.0 };
            if let Some(p) = resp.interact_pointer_pos() {
                let before = (p - rect.center() - self.offset) / self.zoom;
                self.zoom_by(f);
                self.offset = p - rect.center() - before * self.zoom;
            }
        }

        let size = vec2(iw as f32, ih as f32) * self.zoom;
        let gap = if side { 24.0 } else { 0.0 };
        let total = vec2(if side { size.x * 2.0 + gap } else { size.x }, size.y);
        let origin = rect.center() + self.offset - total / 2.0;
        let a_rect = Rect::from_min_size(origin, size);
        let b_rect = if side { Rect::from_min_size(origin + vec2(size.x + gap, 0.0), size) } else { a_rect };
        let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));

        let checker = self.checker(ui.ctx());
        for r in if side { vec![a_rect, b_rect] } else { vec![a_rect] } {
            let tiles = r.size() / 16.0;
            painter.image(checker.id(), r, Rect::from_min_max(Pos2::ZERO, pos2(tiles.x, tiles.y)), Color32::WHITE);
        }
        let orig = inp.original.unwrap();
        let res = inp.result.unwrap_or(orig);
        match inp.mode {
            ViewMode::Result | ViewMode::Average => {
                painter.image(res.id(), a_rect, uv, Color32::WHITE);
            }
            ViewMode::Difference => match inp.diff {
                Some(d) => {
                    painter.image(d.id(), a_rect, uv, Color32::WHITE);
                }
                None => {
                    painter.rect_filled(a_rect, 0.0, Color32::BLACK);
                    painter.text(a_rect.center(), egui::Align2::CENTER_CENTER, "No result yet — run the engine first", egui::FontId::proportional(14.0), Color32::GRAY);
                }
            },
            ViewMode::Original => {
                painter.image(orig.id(), a_rect, uv, Color32::WHITE);
            }
            ViewMode::SideBySide => {
                painter.image(orig.id(), a_rect, uv, Color32::WHITE);
                painter.image(res.id(), b_rect, uv, Color32::WHITE);
                let font = egui::FontId::proportional(11.0);
                painter.text(a_rect.left_top() + vec2(0.0, -4.0), egui::Align2::LEFT_BOTTOM, "Original", font.clone(), ui.visuals().weak_text_color());
                painter.text(b_rect.left_top() + vec2(0.0, -4.0), egui::Align2::LEFT_BOTTOM, "Protected", font, ui.visuals().weak_text_color());
            }
            ViewMode::Overlay => {
                painter.image(orig.id(), a_rect, uv, Color32::WHITE);
                painter.image(res.id(), a_rect, uv, Color32::from_white_alpha((self.overlay_alpha * 255.0) as u8));
            }
            ViewMode::Split => {
                let x = a_rect.left() + a_rect.width() * self.split;
                painter.image(orig.id(), a_rect, uv, Color32::WHITE);
                let right = Rect::from_min_max(pos2(x, a_rect.top()), a_rect.max);
                let ruv = Rect::from_min_max(pos2(self.split, 0.0), pos2(1.0, 1.0));
                painter.image(res.id(), right, ruv, Color32::WHITE);
                painter.line_segment([pos2(x, a_rect.top()), pos2(x, a_rect.bottom())], Stroke::new(1.5, Color32::WHITE));
                let handle = Rect::from_center_size(pos2(x, a_rect.center().y), vec2(14.0, 34.0));
                painter.rect_filled(handle, 3.0, Color32::from_rgb(230, 232, 238));
                let id = ui.id().with("split");
                let hr = ui.interact(handle.expand(6.0), id, Sense::drag());
                if hr.dragged() {
                    self.split = ((self.split * a_rect.width() + hr.drag_delta().x) / a_rect.width()).clamp(0.0, 1.0);
                }
                if hr.hovered() || hr.dragged() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                }
            }
        }
        painter.rect_stroke(a_rect, 0.0, Stroke::new(1.0, Color32::from_black_alpha(160)), egui::StrokeKind::Outside);

        // Grid overlay.
        if self.show_grid {
            if let Some((cols, rows)) = inp.grid {
                let stroke = Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 60));
                for c in 1..cols {
                    let x = a_rect.left() + a_rect.width() * c as f32 / cols as f32;
                    painter.line_segment([pos2(x, a_rect.top()), pos2(x, a_rect.bottom())], stroke);
                }
                for r in 1..rows {
                    let y = a_rect.top() + a_rect.height() * r as f32 / rows as f32;
                    painter.line_segment([pos2(a_rect.left(), y), pos2(a_rect.right(), y)], stroke);
                }
            }
        }
        // Pixel grid at very high zoom.
        if self.zoom_percent(ui.ctx()) >= 1600.0 {
            let stroke = Stroke::new(1.0, Color32::from_black_alpha(70));
            let vis = a_rect.intersect(rect);
            let x0 = ((vis.left() - a_rect.left()) / self.zoom).floor() as i32;
            let x1 = ((vis.right() - a_rect.left()) / self.zoom).ceil() as i32;
            let y0 = ((vis.top() - a_rect.top()) / self.zoom).floor() as i32;
            let y1 = ((vis.bottom() - a_rect.top()) / self.zoom).ceil() as i32;
            for x in x0..=x1 {
                let sx = a_rect.left() + x as f32 * self.zoom;
                painter.line_segment([pos2(sx, vis.top()), pos2(sx, vis.bottom())], stroke);
            }
            for y in y0..=y1 {
                let sy = a_rect.top() + y as f32 * self.zoom;
                painter.line_segment([pos2(vis.left(), sy), pos2(vis.right(), sy)], stroke);
            }
        }

        // Regions.
        let to_screen = |p: [f32; 2]| pos2(a_rect.left() + p[0] * a_rect.width(), a_rect.top() + p[1] * a_rect.height());
        let region_col = if inp.regions_inside { Color32::from_rgb(90, 200, 255) } else { Color32::from_rgb(255, 180, 70) };
        for poly in inp.regions {
            let pts: Vec<Pos2> = poly.iter().map(|p| to_screen(*p)).collect();
            painter.add(egui::Shape::closed_line(pts, Stroke::new(1.5, region_col)));
        }

        // Selection overlay: tints the unselected area.
        if let Some(ov) = inp.selection_overlay {
            painter.image(ov.id(), a_rect, uv, Color32::WHITE);
        }

        // Selection tools.
        let to_img = |p: Pos2| (((p.x - a_rect.left()) / self.zoom).clamp(0.0, iw as f32), ((p.y - a_rect.top()) / self.zoom).clamp(0.0, ih as f32));
        let img_to_screen = |q: (f32, f32)| pos2(a_rect.left() + q.0 * self.zoom, a_rect.top() + q.1 * self.zoom);
        let outline = Stroke::new(1.5, Color32::WHITE);
        let shadow = Stroke::new(3.0, Color32::from_black_alpha(160));
        if inp.tool.selects() && !panning {
            if resp.drag_started() {
                self.drag_op = Some(op_from(&ui.input(|i| i.modifiers)));
                self.drag_start = resp.interact_pointer_pos().map(to_img);
                self.lasso.clear();
                self.brush_last = None;
            }
            let op = self.drag_op.unwrap_or(shieldcraft_core::selection::Op::Replace);
            match inp.tool {
                Tool::Marquee | Tool::Ellipse => {
                    if let (Some(a), Some(p)) = (self.drag_start, resp.interact_pointer_pos()) {
                        let b = to_img(p);
                        if resp.dragged() {
                            let r = Rect::from_two_pos(img_to_screen(a), img_to_screen(b));
                            if inp.tool == Tool::Marquee {
                                painter.rect_stroke(r, 0.0, shadow, egui::StrokeKind::Middle);
                                painter.rect_stroke(r, 0.0, outline, egui::StrokeKind::Middle);
                            } else {
                                let c = r.center();
                                let pts: Vec<Pos2> = (0..96).map(|i| {
                                    let t = i as f32 / 96.0 * std::f32::consts::TAU;
                                    pos2(c.x + r.width() / 2.0 * t.cos(), c.y + r.height() / 2.0 * t.sin())
                                }).collect();
                                painter.add(egui::Shape::closed_line(pts.clone(), shadow));
                                painter.add(egui::Shape::closed_line(pts, outline));
                            }
                        }
                        if resp.drag_stopped() && (a.0 - b.0).abs() >= 1.0 && (a.1 - b.1).abs() >= 1.0 {
                            out.sel_edits.push(if inp.tool == Tool::Marquee { SelEdit::Rect(a, b, op) } else { SelEdit::Ellipse(a, b, op) });
                        }
                    }
                }
                Tool::Lasso => {
                    let to_norm = |p: Pos2| [((p.x - a_rect.left()) / a_rect.width()).clamp(0.0, 1.0), ((p.y - a_rect.top()) / a_rect.height()).clamp(0.0, 1.0)];
                    if resp.dragged() {
                        if let Some(p) = resp.interact_pointer_pos() {
                            let n = to_norm(p);
                            if self.lasso.last().map(|l| (l[0] - n[0]).abs() + (l[1] - n[1]).abs() > 0.002).unwrap_or(true) {
                                self.lasso.push(n);
                            }
                        }
                    }
                    if !self.lasso.is_empty() {
                        let pts: Vec<Pos2> = self.lasso.iter().map(|p| to_screen(*p)).collect();
                        painter.add(egui::Shape::line(pts.clone(), shadow));
                        painter.add(egui::Shape::line(pts, outline));
                    }
                    if resp.drag_stopped() && self.lasso.len() >= 3 {
                        out.sel_edits.push(SelEdit::Polygon(std::mem::take(&mut self.lasso), op));
                    }
                }
                Tool::Brush => {
                    if resp.dragged() || resp.drag_started() {
                        if let Some(p) = resp.interact_pointer_pos() {
                            let cur = to_img(p);
                            let from = self.brush_last.unwrap_or(cur);
                            out.sel_edits.push(SelEdit::Brush { from, to: cur, erase: op == shieldcraft_core::selection::Op::Subtract });
                            self.brush_last = Some(cur);
                        }
                    }
                }
                _ => {}
            }
            if resp.drag_stopped() {
                self.drag_start = None;
                self.drag_op = None;
                self.brush_last = None;
                self.lasso.clear();
            }
        }
        // Brush size cursor.
        if inp.tool == Tool::Brush {
            if let Some(p) = resp.hover_pos() {
                let r = inp.brush_radius * self.zoom;
                painter.circle_stroke(p, r, Stroke::new(2.5, Color32::from_black_alpha(150)));
                painter.circle_stroke(p, r, Stroke::new(1.0, Color32::WHITE));
            }
        }

        // Hover pixel.
        if let Some(p) = resp.hover_pos() {
            if a_rect.contains(p) {
                let x = ((p.x - a_rect.left()) / self.zoom) as u32;
                let y = ((p.y - a_rect.top()) / self.zoom) as u32;
                out.hover_px = Some((x.min(iw - 1), y.min(ih - 1)));
            }
            let cursor = match inp.tool {
                _ if panning => egui::CursorIcon::Grab,
                Tool::Zoom => egui::CursorIcon::ZoomIn,
                Tool::Brush => egui::CursorIcon::None,
                Tool::Lasso | Tool::Inspect | Tool::Marquee | Tool::Ellipse => egui::CursorIcon::Crosshair,
                Tool::Hand => egui::CursorIcon::Grab,
            };
            ui.ctx().set_cursor_icon(cursor);
        }
        if inp.drop_hover {
            painter.rect_stroke(rect.shrink(4.0), 6.0, Stroke::new(2.0, crate::theme::ACCENT), egui::StrokeKind::Inside);
        }
        out
    }

    fn checker(&mut self, ctx: &egui::Context) -> TextureHandle {
        self.checker
            .get_or_insert_with(|| {
                let (a, b) = (Color32::from_gray(204), Color32::from_gray(160));
                let img = egui::ColorImage::new([2, 2], vec![a, b, b, a]);
                let opts = egui::TextureOptions { magnification: egui::TextureFilter::Nearest, minification: egui::TextureFilter::Nearest, wrap_mode: egui::TextureWrapMode::Repeat, mipmap_mode: None };
                ctx.load_texture("checker", img, opts)
            })
            .clone()
    }
}

fn empty_state(painter: &egui::Painter, rect: Rect, hover: bool) {
    let c = rect.center();
    let col = if hover { crate::theme::ACCENT } else { Color32::from_gray(120) };
    painter.text(c - vec2(0.0, 26.0), egui::Align2::CENTER_CENTER, egui_phosphor::regular::IMAGE_SQUARE, egui::FontId::proportional(42.0), col);
    painter.text(c + vec2(0.0, 12.0), egui::Align2::CENTER_CENTER, "Drop artwork here, or use File › Import (Ctrl+O)", egui::FontId::proportional(14.0), col);
    painter.text(c + vec2(0.0, 32.0), egui::Align2::CENTER_CENTER, "PNG · JPEG · WebP · TIFF · BMP · GIF — originals are never modified", egui::FontId::proportional(11.0), Color32::from_gray(100));
}
