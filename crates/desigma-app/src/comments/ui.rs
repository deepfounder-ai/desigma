//! Small painted widgets shared by the comment pins, popovers and the panel.

use super::model::{self, TaskStatus};
use crate::icons::{self, Icon};
use crate::theme;
use crate::widgets;
use egui::text::{LayoutJob, TextFormat};
use egui::{Align, Align2, Color32, FontId, Id, Margin, Painter, Pos2, Rect, Response, Sense, Stroke, StrokeKind, TextEdit, Ui, Vec2};

pub fn avatar_color(name: &str) -> Color32 {
    theme::AVATAR_COLORS[model::avatar_index(name, theme::AVATAR_COLORS.len())]
}

/// Round avatar with the author's initial.
pub fn avatar(p: &Painter, center: Pos2, diameter: f32, name: &str) {
    p.circle_filled(center, diameter / 2.0, avatar_color(name));
    let font = if diameter >= 24.0 { theme::font_body_strong() } else { theme::font_small_strong() };
    p.text(center, Align2::CENTER_CENTER, model::initial(name), font, theme::TEXT);
}

/// How a comment bubble is drawn.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PinStyle {
    Normal,
    /// The thread is open: ringed in the brand colour.
    Open,
    Resolved,
    /// Follows the cursor in comment mode.
    Ghost,
}

/// The pin's hit/paint rect for a tip (the bottom-left corner of the bubble).
pub fn pin_rect(tip: Pos2) -> Rect {
    Rect::from_min_max(Pos2::new(tip.x, tip.y - theme::COMMENT_PIN), Pos2::new(tip.x + theme::COMMENT_PIN, tip.y))
}

/// Fill of a task's pin or status dot; `None` for a to-do task keeps the author colour.
pub fn task_color(status: TaskStatus) -> Option<Color32> {
    match status {
        TaskStatus::Todo => None,
        TaskStatus::Doing => Some(theme::COMMENT_TASK_DOING),
        TaskStatus::Done => Some(theme::COMMENT_TASK_DONE),
    }
}

/// A round bubble with its bottom-left corner squared off to the tip. Task threads
/// take the status colour and a checkbox glyph instead of the author's initial;
/// resolved threads look the same whether or not they are tasks.
pub fn pin(p: &Painter, tip: Pos2, author: &str, style: PinStyle, task: Option<TaskStatus>) {
    let d = theme::COMMENT_PIN;
    let c = Pos2::new(tip.x + d / 2.0, tip.y - d / 2.0);
    let ring = if style == PinStyle::Open { theme::BG_BRAND } else { theme::TEXT };
    let ring_w = theme::COMMENT_PIN_RING;
    let ghost = |col: Color32| if style == PinStyle::Ghost { col.gamma_multiply(0.7) } else { col };
    let task = task.filter(|_| style != PinStyle::Resolved);
    let fill = if style == PinStyle::Resolved { theme::COMMENT_PIN_RESOLVED } else { task.and_then(task_color).unwrap_or_else(|| avatar_color(author)) };
    // outline: a slightly larger circle and corner square underneath
    p.circle_filled(c, d / 2.0 + ring_w, ghost(ring));
    p.rect_filled(Rect::from_min_max(Pos2::new(tip.x - ring_w, c.y), Pos2::new(c.x, tip.y + ring_w)), 0.0, ghost(ring));
    p.circle_filled(c, d / 2.0, ghost(fill));
    p.rect_filled(Rect::from_min_max(Pos2::new(tip.x, c.y), Pos2::new(c.x, tip.y)), 0.0, ghost(fill));
    if style == PinStyle::Resolved {
        icons::paint(p, Icon::Check, c, 16.0, theme::TEXT);
    } else if let Some(t) = task {
        checkbox(p, c, t, ghost(theme::TEXT), ghost(fill));
    } else {
        p.text(c, Align2::CENTER_CENTER, model::initial(author), theme::font_body_strong(), ghost(theme::TEXT));
    }
}

/// A 14px checkbox: empty for to do, half filled in progress, ticked when done.
fn checkbox(p: &Painter, center: Pos2, status: TaskStatus, ink: Color32, bg: Color32) {
    let r = Rect::from_center_size(center, Vec2::splat(14.0));
    let radius = theme::cr(theme::RADIUS_SMALL);
    match status {
        TaskStatus::Todo => {
            p.rect_stroke(r, radius, Stroke::new(2.0, ink), StrokeKind::Inside);
        }
        TaskStatus::Doing => {
            p.rect_stroke(r, radius, Stroke::new(2.0, ink), StrokeKind::Inside);
            let half = Rect::from_min_max(r.min + Vec2::splat(3.0), Pos2::new(r.center().x, r.max.y - 3.0));
            p.rect_filled(half, 0.0, ink);
        }
        TaskStatus::Done => {
            p.rect_filled(r, radius, ink);
            icons::paint(p, Icon::Check, center, 12.0, bg);
        }
    }
}

/// The small status dot used in the panel and the thread header.
pub fn status_dot(p: &Painter, center: Pos2, status: TaskStatus) {
    match task_color(status) {
        Some(col) => p.circle_filled(center, 4.0, col),
        None => p.circle_stroke(center, 3.5, Stroke::new(1.0, theme::ICON_SECONDARY)),
    };
}

/// `text` in `font` and `color` with its `@mentions` in the brand colour.
pub fn mention_job(text: &str, font: FontId, color: Color32, wrap_width: f32) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = wrap_width;
    let plain = TextFormat { font_id: font.clone(), color, ..Default::default() };
    let accent = TextFormat { font_id: font, color: theme::TEXT_BRAND, ..Default::default() };
    let mut at = 0;
    for (a, b) in model::mention_spans(text) {
        job.append(&text[at..a], 0.0, plain.clone());
        job.append(&text[a..b], 0.0, accent.clone());
        at = b;
    }
    job.append(&text[at..], 0.0, plain);
    job
}

/// Height a text box needs for `text` at `width`, within the min/max of the design.
pub fn box_height(p: &Painter, text: &str, width: f32) -> f32 {
    let g = p.layout(if text.is_empty() { " ".to_string() } else { text.to_string() }, theme::font_body(), theme::TEXT, (width - 20.0).max(10.0));
    (g.size().y + 20.0).clamp(theme::COMMENT_BOX_MIN_H, theme::COMMENT_BOX_MAX_H)
}

/// A multi-line text field with the standard field background. Returns the edit response.
pub fn text_box(ui: &mut Ui, rect: Rect, id: Id, text: &mut String, hint: &str, focus: bool) -> Response {
    let focused = ui.memory(|m| m.has_focus(id));
    ui.painter().rect_filled(rect, theme::cr(theme::RADIUS_MEDIUM), theme::BG_SECONDARY);
    if focused {
        ui.painter().rect_stroke(rect, theme::cr(theme::RADIUS_MEDIUM), Stroke::new(1.0, theme::BORDER_SELECTED), StrokeKind::Inside);
    }
    let font = theme::font_body();
    let te = TextEdit::multiline(text)
        .id(id)
        .font(font.clone())
        .text_color(theme::TEXT)
        .frame(egui::Frame::NONE.inner_margin(Margin::symmetric(10, 10)))
        .vertical_align(Align::Min)
        .hint_text(egui::RichText::new(hint).font(font).color(theme::TEXT_TERTIARY))
        .desired_rows(1)
        .desired_width(rect.width());
    let resp = crate::panels::clipped(ui, rect, |ui| ui.put(rect, te));
    if focus {
        resp.request_focus();
    }
    resp
}

/// Ctrl/Cmd+Enter and Escape for a comment text box, consumed so the field never sees them.
pub fn submit_keys(ctx: &egui::Context) -> (bool, bool) {
    ctx.input_mut(|i| (i.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter), i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)))
}

/// A text button: brand-filled when `primary`, otherwise a plain hover tile.
pub fn button(ui: &mut Ui, rect: Rect, id: impl widgets::Key, label: &str, primary: bool, enabled: bool) -> Response {
    let resp = ui.interact(rect, ui.id().with(id), if enabled { Sense::click() } else { Sense::hover() });
    let p = ui.painter();
    let bg = match (primary, enabled) {
        (_, false) => theme::BG_TERTIARY,
        (true, true) if resp.is_pointer_button_down_on() => theme::BG_BRAND_PRESSED,
        (true, true) if resp.hovered() => theme::BG_BRAND_HOVER,
        (true, true) => theme::BG_BRAND,
        (false, true) if resp.hovered() => theme::BG_HOVER,
        (false, true) => Color32::TRANSPARENT,
    };
    if bg != Color32::TRANSPARENT {
        p.rect_filled(rect, theme::cr(theme::RADIUS_MEDIUM), bg);
    }
    let col = if !enabled { theme::TEXT_DISABLED } else { theme::TEXT };
    p.text(rect.center(), Align2::CENTER_CENTER, label, theme::font_body_strong(), col);
    resp
}

/// Width of a [`button`] for `label`.
pub fn button_width(p: &Painter, label: &str) -> f32 {
    widgets::text_width(p, label, &theme::font_body_strong()) + 24.0
}

/// The "…" button.
pub fn dots_button(ui: &mut Ui, rect: Rect, id: impl widgets::Key, tip: &str) -> Response {
    let resp = ui.interact(rect, ui.id().with(id), Sense::click());
    let p = ui.painter();
    widgets::button_bg(p, rect, &resp, false, theme::cr(theme::RADIUS_MEDIUM));
    let col = if resp.hovered() { theme::ICON } else { theme::ICON_SECONDARY };
    for dx in [-4.0, 0.0, 4.0] {
        p.circle_filled(rect.center() + Vec2::new(dx, 0.0), 1.2, col);
    }
    if tip.is_empty() {
        resp
    } else {
        resp.on_hover_text(tip)
    }
}

/// A row of mutually exclusive options; returns the clicked index.
pub fn segmented(ui: &mut Ui, rect: Rect, id: impl widgets::Key + Copy, labels: &[&str], selected: usize) -> Option<usize> {
    let p = ui.painter().clone();
    p.rect_filled(rect, theme::cr(theme::RADIUS_MEDIUM), theme::BG_SECONDARY);
    let w = rect.width() / labels.len() as f32;
    let mut out = None;
    for (i, l) in labels.iter().enumerate() {
        let r = Rect::from_min_size(Pos2::new(rect.left() + i as f32 * w, rect.top()), Vec2::new(w, rect.height()));
        let resp = ui.interact(r, ui.id().with((id, i)), Sense::click());
        if i == selected {
            p.rect_filled(r.shrink(2.0), theme::cr(theme::RADIUS_MEDIUM), theme::BG_TERTIARY);
        } else if resp.hovered() {
            p.rect_filled(r.shrink(2.0), theme::cr(theme::RADIUS_MEDIUM), theme::BG_HOVER);
        }
        p.text(r.center(), Align2::CENTER_CENTER, l, if i == selected { theme::font_body_strong() } else { theme::font_body() }, theme::TEXT);
        if resp.clicked() {
            out = Some(i);
        }
    }
    out
}
