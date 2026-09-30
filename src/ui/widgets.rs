//! Installer-specific widgets drawn with the shared palette.

use crate::report::{Level, LogLine};
use crate::shared::theme::{Palette, alpha, mix};
use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Margin, Pos2, Rect, Response, RichText, Sense, Stroke,
    StrokeKind, TextureHandle, Vec2, pos2, vec2,
};
use egui_phosphor::regular as ph;

// ---------------------------------------------------------------------------
// Textures
// ---------------------------------------------------------------------------

pub struct Icons {
    pub zenless: TextureHandle,
    pub dm: TextureHandle,
    pub torrent: TextureHandle,
    pub setup: TextureHandle,
}

impl Icons {
    pub fn load(ctx: &egui::Context) -> Self {
        let tex = |name: &str, rgba: &[u8]| {
            ctx.load_texture(
                name,
                egui::ColorImage::from_rgba_unmultiplied([128, 128], rgba),
                egui::TextureOptions::LINEAR,
            )
        };
        Self {
            zenless: tex("zenless", include_bytes!("../../assets/zenless-128.rgba")),
            dm: tex("dm", include_bytes!("../../assets/download-manager-128.rgba")),
            torrent: tex("torrent", include_bytes!("../../assets/torrent-128.rgba")),
            setup: tex("setup", include_bytes!("../../assets/icon-128.rgba")),
        }
    }
}

pub fn paint_texture(painter: &egui::Painter, tex: &TextureHandle, rect: Rect) {
    painter.image(tex.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
}

pub fn image(ui: &mut egui::Ui, tex: &TextureHandle, size: f32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    paint_texture(ui.painter(), tex, rect);
    resp
}

// ---------------------------------------------------------------------------
// Painting helpers
// ---------------------------------------------------------------------------

/// Horizontal two-color gradient.
pub fn gradient_rect(painter: &egui::Painter, rect: Rect, left: Color32, right: Color32) {
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(rect.left_top(), left);
    mesh.colored_vertex(rect.right_top(), right);
    mesh.colored_vertex(rect.right_bottom(), right);
    mesh.colored_vertex(rect.left_bottom(), left);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(egui::Shape::mesh(mesh));
}

/// Soft radial glow (concentric translucent circles).
pub fn glow(painter: &egui::Painter, center: Pos2, radius: f32, color: Color32, strength: f32) {
    let steps = 14;
    for i in 0..steps {
        let t = i as f32 / steps as f32;
        let r = radius * (1.0 - t);
        painter.circle_filled(center, r, alpha(color, strength * 0.1 * (0.25 + t)));
    }
}

/// A rounded pill painted at `pos` (left-top). Returns its rect.
pub fn paint_pill(painter: &egui::Painter, pos: Pos2, text: &str, color: Color32) -> Rect {
    let galley = painter.layout_no_wrap(text.to_owned(), FontId::proportional(11.5), color);
    let pad = vec2(8.0, 3.0);
    let rect = Rect::from_min_size(pos, galley.size() + pad * 2.0);
    painter.rect_filled(rect, CornerRadius::same(255), alpha(color, 0.15));
    painter.galley(rect.min + pad, galley, color);
    rect
}

/// Round check mark used by selectable cards.
pub fn check_mark(painter: &egui::Painter, center: Pos2, r: f32, t: f32, p: &Palette) {
    painter.circle_filled(center, r, mix(p.surface2, p.accent, t));
    painter.circle_stroke(center, r, Stroke::new(1.2, mix(p.border, p.accent, t)));
    if t > 0.05 {
        painter.text(center, Align2::CENTER_CENTER, ph::CHECK, FontId::proportional(r * 1.25), alpha(p.accent_fg, t));
    }
}

// ---------------------------------------------------------------------------
// Buttons
// ---------------------------------------------------------------------------

const BTN_SIZE: Vec2 = vec2(124.0, 36.0);

pub fn primary(ui: &mut egui::Ui, p: &Palette, text: &str, enabled: bool) -> Response {
    let btn = egui::Button::new(RichText::new(text).color(p.accent_fg).strong().size(14.5))
        .fill(p.accent)
        .stroke(Stroke::NONE)
        .corner_radius(CornerRadius::same(9))
        .min_size(BTN_SIZE);
    ui.add_enabled(enabled, btn).on_hover_cursor(egui::CursorIcon::PointingHand)
}

pub fn danger(ui: &mut egui::Ui, p: &Palette, text: &str, enabled: bool) -> Response {
    let fg = if p.danger.r() as u32 + p.danger.g() as u32 + p.danger.b() as u32 > 520 { Color32::BLACK } else { Color32::WHITE };
    let btn = egui::Button::new(RichText::new(text).color(fg).strong().size(14.5))
        .fill(p.danger)
        .stroke(Stroke::NONE)
        .corner_radius(CornerRadius::same(9))
        .min_size(BTN_SIZE);
    ui.add_enabled(enabled, btn).on_hover_cursor(egui::CursorIcon::PointingHand)
}

pub fn secondary(ui: &mut egui::Ui, p: &Palette, text: &str) -> Response {
    let btn = egui::Button::new(RichText::new(text).color(p.text).size(14.0))
        .fill(p.surface2)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(9))
        .min_size(vec2(96.0, 36.0));
    ui.add(btn).on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Small outlined action button, e.g. "Open folder".
pub fn action(ui: &mut egui::Ui, p: &Palette, icon: &str, text: &str) -> Response {
    let btn = egui::Button::new(RichText::new(format!("{icon}  {text}")).color(p.text).size(13.0))
        .fill(p.surface2)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(7))
        .min_size(vec2(0.0, 30.0));
    ui.add(btn).on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Frameless text button (links, "Cancel").
pub fn ghost(ui: &mut egui::Ui, color: Color32, text: &str) -> Response {
    ui.add(egui::Button::new(RichText::new(text).color(color).size(14.0)).frame(false).min_size(vec2(0.0, 36.0)))
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

// ---------------------------------------------------------------------------
// Toggle rows
// ---------------------------------------------------------------------------

/// A full-width row with a switch, a title and an optional description.
pub fn toggle_row(ui: &mut egui::Ui, p: &Palette, on: &mut bool, title: &str, subtitle: &str, enabled: bool) -> Response {
    let text_w = ui.available_width() - 70.0;
    let title_color = if enabled { p.text } else { alpha(p.text_dim, 0.7) };
    let sub_color = if enabled { p.text_dim } else { alpha(p.text_dim, 0.6) };
    let title_g = ui.painter().layout(title.to_owned(), FontId::proportional(14.0), title_color, text_w);
    let sub_g = (!subtitle.is_empty())
        .then(|| ui.painter().layout(subtitle.to_owned(), FontId::proportional(12.0), sub_color, text_w));
    let text_h = title_g.size().y + sub_g.as_ref().map_or(0.0, |g| g.size().y + 2.0);
    let h = (text_h + 12.0).max(32.0);
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, mut resp) = ui.allocate_exact_size(vec2(ui.available_width(), h), sense);
    if enabled && resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let t = ui.ctx().animate_bool_with_time(resp.id, *on && enabled, 0.12);
    let painter = ui.painter();
    if enabled && resp.hovered() {
        painter.rect_filled(rect, CornerRadius::same(8), alpha(p.accent, 0.06));
    }
    let sw = Rect::from_min_size(pos2(rect.left() + 8.0, rect.center().y - 10.0), vec2(38.0, 20.0));
    let track = if enabled { mix(p.surface2, p.accent, t) } else { mix(p.surface2, p.bg, 0.5) };
    painter.rect(sw, CornerRadius::same(10), track, Stroke::new(1.0, mix(p.border, p.accent, t)), StrokeKind::Inside);
    let knob = egui::lerp(sw.left() + 10.0..=sw.right() - 10.0, t);
    let knob_color = if !enabled { alpha(p.text_dim, 0.5) } else { mix(p.text_dim, p.accent_fg, t) };
    painter.circle_filled(pos2(knob, sw.center().y), 7.0, knob_color);

    let x = sw.right() + 12.0;
    let top = rect.center().y - text_h / 2.0;
    let title_h = title_g.size().y;
    painter.galley(pos2(x, top), title_g, title_color);
    if let Some(g) = sub_g {
        painter.galley(pos2(x, top + title_h + 2.0), g, sub_color);
    }
    if enabled { resp.on_hover_cursor(egui::CursorIcon::PointingHand) } else { resp }
}

/// A smaller square checkbox with a label (e.g. "Launch Download Manager").
pub fn check_row(ui: &mut egui::Ui, p: &Palette, on: &mut bool, label: &str) -> Response {
    let galley = ui.painter().layout_no_wrap(label.to_owned(), FontId::proportional(14.0), p.text);
    let size = vec2(24.0 + galley.size().x + 10.0, 30.0);
    let (rect, mut resp) = ui.allocate_exact_size(size, Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let t = ui.ctx().animate_bool_with_time(resp.id, *on, 0.1);
    let painter = ui.painter();
    let bx = Rect::from_center_size(pos2(rect.left() + 10.0, rect.center().y), Vec2::splat(18.0));
    painter.rect(bx, CornerRadius::same(5), mix(p.surface2, p.accent, t), Stroke::new(1.0, mix(p.border, p.accent, t)), StrokeKind::Inside);
    if t > 0.05 {
        painter.text(bx.center(), Align2::CENTER_CENTER, ph::CHECK, FontId::proportional(13.0), alpha(p.accent_fg, t));
    }
    painter.galley(pos2(bx.right() + 10.0, rect.center().y - galley.size().y / 2.0), galley, p.text);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

// ---------------------------------------------------------------------------
// Cards
// ---------------------------------------------------------------------------

/// An app icon with an optional small glyph badge (e.g. the Chrome logo).
pub struct CardIcon<'a> {
    pub tex: &'a TextureHandle,
    pub badge: Option<&'a str>,
}

impl CardIcon<'_> {
    /// Paints the icon into `rect`; the badge sits on its bottom-right corner.
    pub fn paint(&self, painter: &egui::Painter, p: &Palette, rect: Rect) {
        paint_texture(painter, self.tex, rect);
        if let Some(badge) = self.badge {
            let r = (rect.width() * 0.24).max(8.0);
            let c = rect.right_bottom() - vec2(r * 0.3, r * 0.3);
            painter.circle_filled(c, r, p.surface2);
            painter.circle_stroke(c, r, Stroke::new(1.0, p.border));
            painter.text(c, Align2::CENTER_CENTER, badge, FontId::proportional(r * 1.2), p.text);
        }
    }
}

pub struct CardTag {
    pub text: String,
    pub color: Color32,
}

/// Selectable component card: icon, title, description, tags and a check.
#[allow(clippy::too_many_arguments)]
pub fn component_card(
    ui: &mut egui::Ui,
    p: &Palette,
    icon: CardIcon,
    title: &str,
    desc: &str,
    tags: &[CardTag],
    selected: bool,
    size: Vec2,
) -> Response {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let t = ui.ctx().animate_bool_with_time(resp.id, selected, 0.12);
    let hovered = resp.hovered();
    let painter = ui.painter();

    let fill = mix(p.surface, p.accent, 0.04 * t + if hovered { 0.025 } else { 0.0 });
    let stroke = if selected {
        Stroke::new(1.5, mix(p.border, p.accent, 0.75 * t))
    } else if hovered {
        Stroke::new(1.0, mix(p.border, p.accent, 0.4))
    } else {
        Stroke::new(1.0, p.border)
    };
    painter.rect(rect, CornerRadius::same(12), fill, stroke, StrokeKind::Inside);

    let icon_rect = Rect::from_min_size(rect.min + vec2(16.0, 16.0), Vec2::splat(44.0));
    icon.paint(painter, p, icon_rect);
    check_mark(painter, pos2(rect.right() - 26.0, rect.top() + 27.0), 10.0, t, p);

    let x = icon_rect.right() + 14.0;
    let title_g = painter.layout_no_wrap(title.to_owned(), FontId::proportional(16.0), p.text);
    let title_h = title_g.size().y;
    painter.galley(pos2(x, rect.top() + 16.0), title_g, p.text);
    let desc_w = rect.right() - x - 18.0;
    let desc_g = painter.layout(desc.to_owned(), FontId::proportional(12.5), p.text_dim, desc_w);
    painter.galley(pos2(x, rect.top() + 20.0 + title_h), desc_g, p.text_dim);

    let mut tag_x = rect.left() + 16.0;
    for tag in tags {
        let r = paint_pill(painter, pos2(tag_x, rect.bottom() - 32.0), &tag.text, tag.color);
        tag_x = r.right() + 6.0;
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Large clickable choice ("Modify / Update", "Uninstall").
pub fn choice_card(ui: &mut egui::Ui, p: &Palette, glyph: &str, color: Color32, title: &str, desc: &str, size: Vec2) -> Response {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let hovered = resp.hovered();
    let t = ui.ctx().animate_bool_with_time(resp.id, hovered, 0.12);
    let painter = ui.painter();
    painter.rect(
        rect,
        CornerRadius::same(12),
        mix(p.surface, color, 0.05 * t),
        Stroke::new(1.0 + 0.5 * t, mix(p.border, color, 0.2 + 0.6 * t)),
        StrokeKind::Inside,
    );
    let icon = Rect::from_min_size(rect.min + vec2(18.0, 18.0), Vec2::splat(44.0));
    painter.rect_filled(icon, CornerRadius::same(12), alpha(color, 0.15));
    painter.text(icon.center(), Align2::CENTER_CENTER, glyph, FontId::proportional(22.0), color);
    painter.text(pos2(rect.left() + 18.0, icon.bottom() + 16.0), Align2::LEFT_TOP, title, FontId::proportional(16.0), p.text);
    let g = painter.layout(desc.to_owned(), FontId::proportional(12.5), p.text_dim, rect.width() - 36.0);
    painter.galley(pos2(rect.left() + 18.0, icon.bottom() + 40.0), g, p.text_dim);
    painter.text(
        pos2(rect.right() - 18.0 + 3.0 * t, rect.top() + 40.0),
        Align2::RIGHT_CENTER,
        ph::ARROW_RIGHT,
        FontId::proportional(18.0),
        mix(p.text_dim, color, t),
    );
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

// ---------------------------------------------------------------------------
// Sidebar steps
// ---------------------------------------------------------------------------

pub fn steps(ui: &mut egui::Ui, p: &Palette, names: &[&str], current: usize) {
    let row_h = 40.0;
    for (i, name) in names.iter().enumerate() {
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), row_h), Sense::hover());
        let painter = ui.painter();
        let c = pos2(rect.left() + 22.0, rect.center().y);
        if i == current {
            painter.rect_filled(rect.shrink2(vec2(6.0, 3.0)), CornerRadius::same(9), alpha(p.accent, 0.09));
        }
        if i + 1 < names.len() {
            let color = if i < current { alpha(p.accent, 0.55) } else { p.border };
            painter.line_segment([c + vec2(0.0, 12.0), c + vec2(0.0, row_h - 12.0)], Stroke::new(1.5, color));
        }
        let r = 11.0;
        if i < current {
            painter.circle_filled(c, r, alpha(p.accent, 0.16));
            painter.circle_stroke(c, r, Stroke::new(1.2, p.accent));
            painter.text(c, Align2::CENTER_CENTER, ph::CHECK, FontId::proportional(12.0), p.accent);
        } else if i == current {
            painter.circle_filled(c, r, p.accent);
            painter.text(c, Align2::CENTER_CENTER, format!("{}", i + 1), FontId::proportional(12.0), p.accent_fg);
        } else {
            painter.circle_stroke(c, r, Stroke::new(1.2, p.border));
            painter.text(c, Align2::CENTER_CENTER, format!("{}", i + 1), FontId::proportional(11.5), p.text_dim);
        }
        let color = if i == current {
            p.text
        } else if i < current {
            mix(p.text, p.text_dim, 0.35)
        } else {
            p.text_dim
        };
        painter.text(pos2(c.x + 22.0, c.y), Align2::LEFT_CENTER, *name, FontId::proportional(14.0), color);
    }
}

// ---------------------------------------------------------------------------
// Log
// ---------------------------------------------------------------------------

pub fn log_view(ui: &mut egui::Ui, p: &Palette, lines: &[LogLine], height: f32) {
    egui::Frame::new()
        .fill(p.input)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_height(height);
            egui::ScrollArea::vertical()
                .stick_to_bottom(true)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 3.0;
                    let wrap = ui.available_width();
                    for line in lines {
                        let (icon, color) = match line.level {
                            Level::Info => (ph::DOT_OUTLINE, p.text_dim),
                            Level::Ok => (ph::CHECK, p.success),
                            Level::Warn => (ph::WARNING, p.warning),
                            Level::Error => (ph::X_CIRCLE, p.danger),
                        };
                        let text_color = match line.level {
                            Level::Info => mix(p.text, p.text_dim, 0.4),
                            Level::Ok => p.text,
                            Level::Warn => p.warning,
                            Level::Error => p.danger,
                        };
                        let mut job = egui::text::LayoutJob::default();
                        job.wrap.max_width = wrap;
                        job.append(
                            icon,
                            0.0,
                            egui::TextFormat { font_id: FontId::proportional(12.5), color, ..Default::default() },
                        );
                        job.append(
                            &line.text,
                            8.0,
                            egui::TextFormat { font_id: FontId::monospace(12.0), color: text_color, ..Default::default() },
                        );
                        ui.label(job);
                    }
                });
        });
}

/// Colored banner with an icon (errors, warnings, notes).
pub fn banner(ui: &mut egui::Ui, p: &Palette, icon: &str, color: Color32, title: &str, body: &str) {
    egui::Frame::new()
        .fill(alpha(color, 0.10))
        .stroke(Stroke::new(1.0, alpha(color, 0.45)))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::symmetric(14, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                ui.label(RichText::new(icon).size(18.0).color(color));
                ui.vertical(|ui| {
                    ui.label(RichText::new(title).color(p.text).size(14.0).strong());
                    if !body.is_empty() {
                        ui.label(RichText::new(body).color(p.text_dim).size(12.5));
                    }
                });
            });
        });
}
