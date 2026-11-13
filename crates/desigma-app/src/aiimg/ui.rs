//! The two AI image dialogs: Preferences -> AI images… and Generate / Edit.

use super::compose::{Region, ASPECTS};
use super::{Models, Target};
use crate::app::App;
use crate::comments::ui::{button, button_width};
use crate::i18n::tr;
use crate::theme;
use crate::widgets;
use egui::{Align2, Color32, Context, Id, Key, Modifiers, Order, Painter, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, Vec2};

const PAD: f32 = 20.0;
const BTN: f32 = 32.0;
const ROW: f32 = 28.0;
const LIST_ROWS: f32 = 6.0;

fn label(p: &Painter, x: f32, y: f32, s: &str) {
    p.text(Pos2::new(x, y + 8.0), Align2::LEFT_CENTER, tr(s), theme::font_small_medium(), theme::TEXT_SECONDARY);
}

/// Wrapped small text; returns its height.
fn note(p: &Painter, at: Pos2, w: f32, s: &str, color: Color32) -> f32 {
    let g = p.layout(s.to_string(), theme::font_small(), color, w);
    let h = g.size().y;
    p.galley(at, g, color);
    h
}

fn note_height(p: &Painter, w: f32, s: &str) -> f32 {
    p.layout(s.to_string(), theme::font_small(), theme::TEXT, w).size().y
}

fn field(ui: &mut Ui, rect: Rect, id: &str, text: &mut String, hint: &str, password: bool, lines: usize) -> egui::Response {
    ui.painter().rect_filled(rect, theme::cr(theme::RADIUS_MEDIUM), theme::BG_SECONDARY);
    let te = if lines > 1 { egui::TextEdit::multiline(text).desired_rows(lines) } else { egui::TextEdit::singleline(text).vertical_align(egui::Align::Center) };
    let te = te
        .id(Id::new(id))
        .font(theme::font_body())
        .text_color(theme::TEXT)
        .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(10, if lines > 1 { 8 } else { 0 })))
        .hint_text(egui::RichText::new(tr(hint)).color(theme::TEXT_TERTIARY))
        .password(password)
        .min_size(rect.size())
        .desired_width(rect.width());
    let resp = ui.put(rect, te);
    widgets::record(ui, id, rect);
    resp
}

/// The scrim and the panel of a centred dialog; returns true when the scrim was clicked.
fn chrome(ui: &mut Ui, screen: Rect, rect: Rect, title: &str) -> bool {
    let scrim = ui.allocate_rect(screen, Sense::click_and_drag());
    let p = ui.painter();
    p.rect_filled(screen, 0.0, Color32::from_black_alpha(140));
    widgets::menu_panel(p, rect, theme::MENU_RADIUS);
    p.text(Pos2::new(rect.left() + PAD, rect.top() + PAD + 12.0), Align2::LEFT_CENTER, tr(title), theme::font_large_strong(), theme::TEXT);
    scrim.clicked() && !rect.contains(scrim.interact_pointer_pos().unwrap_or(Pos2::ZERO))
}

/// Right-aligned buttons along the bottom; returns the index of the clicked one.
fn buttons(ui: &mut Ui, rect: Rect, items: &[(&str, &str, bool, bool)]) -> Option<usize> {
    let p = ui.painter().clone();
    let mut right = rect.right() - PAD;
    let mut clicked = None;
    for (i, (id, text, primary, enabled)) in items.iter().enumerate() {
        let w = button_width(&p, text) + 16.0;
        let r = Rect::from_min_size(Pos2::new(right - w, rect.bottom() - PAD - BTN), Vec2::new(w, BTN));
        if button(ui, r, *id, text, *primary, *enabled).clicked() && *enabled {
            clicked = Some(i);
        }
        widgets::record(ui, id, r);
        right = r.left() - 8.0;
    }
    clicked
}

const PRIVACY: &str = "Images are made by OpenRouter with your own key and at your cost. Your prompt and the layers you use as references are sent to OpenRouter. The key is kept in this computer's keychain.";

pub fn settings_dialog(app: &mut App, ctx: &Context) {
    let Some(mut s) = app.ai.settings.take() else { return };
    let screen = ctx.content_rect();
    let esc = ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
    const W: f32 = 480.0;
    let text_w = W - PAD * 2.0;
    let from_env = app.ai_key_from_env();
    let saved = app.ai_key_saved();
    // the field searches the list until it holds a listed model's id
    let exact = matches!(&s.models, Models::Loaded(all) if all.iter().any(|m| m.id == s.model.trim()));
    let filter = if exact { String::new() } else { s.model.trim().to_lowercase() };
    let rows: Vec<(String, String, bool)> = match &s.models {
        Models::Loaded(all) => {
            let with_refs = all.iter().any(|m| m.references);
            all.iter()
                // models that take reference images come first; the rest cannot edit an image
                .filter(|m| !with_refs || m.references || m.id == s.model)
                .filter(|m| filter.is_empty() || m.id == s.model || m.id.to_lowercase().contains(&filter) || m.name.to_lowercase().contains(&filter))
                .map(|m| (m.id.clone(), m.name.clone(), m.references))
                .collect()
        }
        _ => Vec::new(),
    };
    let status = match &s.models {
        Models::Unknown => Some(("Load the models to choose one, or type a model id.".to_string(), theme::TEXT_SECONDARY)),
        Models::Loading => Some((tr("Loading models\u{2026}").to_string(), theme::TEXT_SECONDARY)),
        Models::Failed(e) => Some((e.clone(), theme::DANGER)),
        Models::Loaded(_) if rows.is_empty() => Some(("No model matches.".to_string(), theme::TEXT_SECONDARY)),
        Models::Loaded(_) => None,
    };
    let mut close = esc;
    let mut save = false;
    let mut load = false;
    let mut remove = false;
    egui::Area::new(Id::new("ai-settings")).order(Order::Foreground).fixed_pos(screen.min).interactable(true).show(ctx, |ui| {
        let p = ui.painter().clone();
        let privacy_h = note_height(&p, text_w, tr(PRIVACY));
        let error_h = s.error.as_ref().map_or(0.0, |e| note_height(&p, text_w, tr(e)) + 10.0);
        let list_h = ROW * LIST_ROWS;
        let h = PAD + 24.0 + 12.0 + privacy_h + 14.0 + 54.0 + 54.0 + list_h + 10.0 + error_h + BTN + PAD;
        let rect = Rect::from_center_size(screen.center() - Vec2::new(0.0, 24.0), Vec2::new(W, h));
        if chrome(ui, screen, rect, "AI images") {
            close = true;
        }
        let x0 = rect.left() + PAD;
        let mut y = rect.top() + PAD + 24.0 + 12.0;
        y += note(&p, Pos2::new(x0, y), text_w, tr(PRIVACY), theme::TEXT_SECONDARY) + 14.0;

        label(&p, x0, y, "OpenRouter API key");
        let key_r = Rect::from_min_size(Pos2::new(x0, y + 20.0), Vec2::new(text_w, 30.0));
        if from_env {
            p.rect_filled(key_r, theme::cr(theme::RADIUS_MEDIUM), theme::BG_SECONDARY);
            p.text(
                key_r.left_center() + Vec2::new(10.0, 0.0),
                Align2::LEFT_CENTER,
                tr("Using OPENROUTER_API_KEY from the environment"),
                theme::font_body(),
                theme::TEXT_SECONDARY,
            );
        } else {
            let hint = if saved { "Saved in the keychain. Type to replace it" } else { "sk-or-\u{2026}" };
            let resp = field(ui, key_r, "ai-key", &mut s.key, hint, true, 1);
            if s.fresh && !saved {
                resp.request_focus();
            }
        }
        y += 54.0;

        label(&p, x0, y, "Model");
        let load_w = button_width(&p, "Load models") + 16.0;
        let model_r = Rect::from_min_size(Pos2::new(x0, y + 20.0), Vec2::new(text_w - load_w - 8.0, 30.0));
        let resp = field(ui, model_r, "ai-model", &mut s.model, "Search, or type a model id", false, 1);
        if s.fresh && saved {
            resp.request_focus();
        }
        if resp.changed() {
            s.scroll = 0.0;
        }
        let load_r = Rect::from_min_size(Pos2::new(model_r.right() + 8.0, model_r.top()), Vec2::new(load_w, 30.0));
        let can_load = !matches!(s.models, Models::Loading);
        if button(ui, load_r, "ai-load", "Load models", false, can_load).clicked() && can_load {
            load = true;
        }
        widgets::record(ui, "ai-load", load_r);
        y += 54.0;

        let list = Rect::from_min_size(Pos2::new(x0, y), Vec2::new(text_w, list_h));
        p.rect_stroke(list, theme::cr(theme::RADIUS_MEDIUM), Stroke::new(1.0, theme::BORDER), StrokeKind::Inside);
        if let Some((text, color)) = &status {
            note(&p, list.min + Vec2::new(10.0, 8.0), text_w - 20.0, tr(text), *color);
        } else {
            crate::panels::wheel_scroll(ui, list, &mut s.scroll, rows.len() as f32 * ROW);
            let lp = p.with_clip_rect(list.shrink(1.0));
            let first = (s.scroll / ROW).floor() as usize;
            for (i, (id, name, refs)) in rows.iter().enumerate().skip(first).take(LIST_ROWS as usize + 1) {
                let r = Rect::from_min_size(Pos2::new(list.left(), list.top() + i as f32 * ROW - s.scroll), Vec2::new(list.width(), ROW));
                let hit = r.intersect(list);
                let resp = ui.interact(hit, Id::new(("ai-model-row", id)), Sense::click());
                let picked = *id == s.model;
                if picked || resp.hovered() {
                    lp.rect_filled(r.shrink2(Vec2::new(2.0, 1.0)), theme::cr(theme::RADIUS_MEDIUM), if picked { theme::BG_SELECTED } else { theme::BG_HOVER });
                }
                lp.text(r.left_center() + Vec2::new(10.0, 0.0), Align2::LEFT_CENTER, name, theme::font_body(), theme::TEXT);
                if !*refs {
                    lp.text(r.right_center() - Vec2::new(10.0, 0.0), Align2::RIGHT_CENTER, tr("no references"), theme::font_small(), theme::TEXT_TERTIARY);
                }
                if resp.clicked() {
                    s.model = id.clone();
                }
                if i < 3 {
                    widgets::record(ui, &format!("ai-model-row-{i}"), hit);
                }
            }
        }
        y += list_h + 10.0;
        if let Some(e) = &s.error {
            note(&p, Pos2::new(x0, y), text_w, tr(e), theme::DANGER);
        }

        let can_save = !s.model.trim().is_empty() || !s.key.trim().is_empty();
        match buttons(ui, rect, &[("ai-save", "Save", true, can_save), ("ai-cancel", "Cancel", false, true)]) {
            Some(0) => save = true,
            Some(_) => close = true,
            None => {}
        }
        if saved && !from_env {
            let w = button_width(&p, "Remove key") + 16.0;
            let r = Rect::from_min_size(Pos2::new(x0, rect.bottom() - PAD - BTN), Vec2::new(w, BTN));
            if button(ui, r, "ai-remove-key", "Remove key", false, true).clicked() {
                remove = true;
            }
            widgets::record(ui, "ai-remove-key", r);
        }
    });
    s.fresh = false;
    if close {
        app.ai.listing = None;
        return;
    }
    app.ai.settings = Some(s);
    if remove {
        app.ai_remove_key();
    }
    if load {
        app.ai_load_models(ctx);
    }
    if save {
        app.ai_save_settings();
    }
}

pub fn prompt_dialog(app: &mut App, ctx: &Context) {
    let Some(mut d) = app.ai.prompt.take() else { return };
    let screen = ctx.content_rect();
    let (esc, send) = ctx.input_mut(|i| (i.consume_key(Modifiers::NONE, Key::Escape), i.consume_key(Modifiers::COMMAND, Key::Enter)));
    const W: f32 = 520.0;
    let text_w = W - PAD * 2.0;
    let editing = matches!(d.target, Target::Edit { .. });
    let (title, hint) = if editing {
        ("Edit image with AI", "What should change? For example: replace the sky with a sunset")
    } else {
        ("Generate image", "Describe the image")
    };
    let info = match &d.target {
        Target::Edit { .. } if d.region.is_some_and(|r| !r.is_tiny()) => "Only the selected part will change.".to_string(),
        Target::Edit { .. } => "Drag over the part to change. Without a selection the whole image is regenerated.".to_string(),
        Target::Fill(_) => "The image fills the selected layer.".to_string(),
        Target::Place if d.references.is_empty() => "The image is placed in the middle of the view.".to_string(),
        Target::Place => format!("{} {}", d.references.len(), tr("selected layers are sent as references.")),
    };
    // the preview keeps the picture's proportions inside a fixed box
    let preview_size = d.preview.as_ref().map(|t| {
        let s = t.size_vec2();
        s * (text_w / s.x).min(260.0 / s.y).min(1.0)
    });
    let mut cancel = esc;
    let mut submit = send;
    egui::Area::new(Id::new("ai-prompt")).order(Order::Foreground).fixed_pos(screen.min).interactable(true).show(ctx, |ui| {
        let p = ui.painter().clone();
        let error_h = d.error.as_ref().map_or(0.0, |e| note_height(&p, text_w, tr(e)) + 10.0);
        let info_h = note_height(&p, text_w, tr(&info));
        let h = PAD
            + 24.0
            + 12.0
            + preview_size.map_or(0.0, |s| s.y + 12.0)
            + 96.0
            + 10.0
            + if editing { 0.0 } else { 30.0 + 10.0 }
            + info_h
            + 12.0
            + error_h
            + BTN
            + PAD;
        let rect = Rect::from_center_size(screen.center() - Vec2::new(0.0, 24.0), Vec2::new(W, h));
        if chrome(ui, screen, rect, title) && !d.running {
            cancel = true;
        }
        let x0 = rect.left() + PAD;
        let mut y = rect.top() + PAD + 24.0 + 12.0;

        if let (Some(tex), Some(size)) = (&d.preview, preview_size) {
            let r = Rect::from_min_size(Pos2::new(x0 + (text_w - size.x) / 2.0, y), size);
            p.image(tex.id(), r, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
            let resp = ui.interact(r, Id::new("ai-region"), if d.running { Sense::hover() } else { Sense::click_and_drag() });
            widgets::record(ui, "ai-region", r);
            let frac = |pos: Pos2| ((pos.x - r.left()) / r.width(), (pos.y - r.top()) / r.height());
            if resp.drag_started() {
                // where the button went down, not where the drag was recognized
                d.drag = ui.input(|i| i.pointer.press_origin()).or(resp.interact_pointer_pos()).map(frac);
            }
            if let (Some(a), Some(pos)) = (d.drag, resp.interact_pointer_pos()) {
                if resp.dragged() || resp.drag_stopped() {
                    d.region = Some(Region::new(a, frac(pos)));
                }
            }
            if resp.drag_stopped() {
                d.drag = None;
                d.region = d.region.filter(|r| !r.is_tiny());
            }
            if resp.clicked() {
                d.region = None;
            }
            if let Some(g) = d.region {
                let sel = Rect::from_min_max(
                    Pos2::new(r.left() + g.x0 * r.width(), r.top() + g.y0 * r.height()),
                    Pos2::new(r.left() + g.x1 * r.width(), r.top() + g.y1 * r.height()),
                );
                // dim what stays, outline what changes
                let dim = Color32::from_black_alpha(110);
                for part in [
                    Rect::from_min_max(r.min, Pos2::new(r.right(), sel.top())),
                    Rect::from_min_max(Pos2::new(r.left(), sel.bottom()), r.max),
                    Rect::from_min_max(Pos2::new(r.left(), sel.top()), Pos2::new(sel.left(), sel.bottom())),
                    Rect::from_min_max(Pos2::new(sel.right(), sel.top()), Pos2::new(r.right(), sel.bottom())),
                ] {
                    p.rect_filled(part, 0.0, dim);
                }
                p.rect_stroke(sel, 0.0, Stroke::new(1.5, theme::BG_BRAND), StrokeKind::Inside);
            }
            y += size.y + 12.0;
        }

        let prompt_r = Rect::from_min_size(Pos2::new(x0, y), Vec2::new(text_w, 96.0));
        ui.add_enabled_ui(!d.running, |ui| {
            let resp = field(ui, prompt_r, "ai-prompt-text", &mut d.text, hint, false, 4);
            if d.fresh {
                resp.request_focus();
            }
        });
        y += 96.0 + 10.0;

        if !editing {
            let mut x = x0;
            for (i, a) in ASPECTS.iter().enumerate() {
                let w = widgets::text_width(&p, tr(a), &theme::font_body()) + 20.0;
                let r = Rect::from_min_size(Pos2::new(x, y), Vec2::new(w, 30.0));
                let resp = ui.interact(r, Id::new(("ai-aspect", i)), if d.running { Sense::hover() } else { Sense::click() });
                let on = d.aspect == i;
                p.rect_filled(
                    r,
                    theme::cr(theme::RADIUS_MEDIUM),
                    if on {
                        theme::BG_SELECTED
                    } else if resp.hovered() {
                        theme::BG_HOVER
                    } else {
                        theme::BG_SECONDARY
                    },
                );
                p.text(r.center(), Align2::CENTER_CENTER, tr(a), theme::font_body(), theme::TEXT);
                if resp.clicked() {
                    d.aspect = i;
                }
                widgets::record(ui, &format!("ai-aspect-{a}"), r);
                x += w + 6.0;
            }
            y += 30.0 + 10.0;
        }

        y += note(&p, Pos2::new(x0, y), text_w, tr(&info), theme::TEXT_SECONDARY) + 12.0;
        if let Some(e) = &d.error {
            note(&p, Pos2::new(x0, y), text_w, tr(e), theme::DANGER);
        }

        if d.running {
            p.text(
                Pos2::new(x0, rect.bottom() - PAD - BTN / 2.0),
                Align2::LEFT_CENTER,
                tr("Generating\u{2026} this can take a minute"),
                theme::font_body(),
                theme::TEXT_SECONDARY,
            );
            if buttons(ui, rect, &[("ai-cancel", "Cancel", false, true)]).is_some() {
                cancel = true;
            }
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
        } else {
            let can = !d.text.trim().is_empty();
            match buttons(ui, rect, &[("ai-generate", "Generate", true, can), ("ai-cancel", "Cancel", false, true)]) {
                Some(0) => submit = true,
                Some(_) => cancel = true,
                None => {}
            }
        }
    });
    d.fresh = false;
    app.ai.prompt = Some(d);
    if cancel {
        app.ai_cancel();
    } else if submit {
        app.ai_submit(ctx);
    }
}
