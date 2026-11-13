//! The two AI image dialogs: Preferences -> AI images… and Generate / Edit.

use super::compose::ASPECTS;
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
/// Height of a picture in the versions strip.
const THUMB: f32 = 48.0;

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
    if d.running {
        // the dialog steps aside: the canvas shows where the image is being made
        let cancel = running_pill(ctx, app.now - d.started);
        app.ai.prompt = Some(d);
        if cancel {
            app.ai_cancel();
        }
        return;
    }
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
        Target::Edit { .. } if d.region().is_some() => "Only the painted part will change.".to_string(),
        Target::Edit { .. } => "Paint over the part to change. With nothing painted the whole image is regenerated.".to_string(),
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
    let mut restore = None;
    let strip_h = if d.versions.len() > 1 { 16.0 + THUMB + 12.0 } else { 0.0 };
    egui::Area::new(Id::new("ai-prompt")).order(Order::Foreground).fixed_pos(screen.min).interactable(true).show(ctx, |ui| {
        let p = ui.painter().clone();
        let error_h = d.error.as_ref().map_or(0.0, |e| note_height(&p, text_w, tr(e)) + 10.0);
        let info_h = note_height(&p, text_w, tr(&info));
        let h = PAD
            + 24.0
            + 12.0
            + preview_size.map_or(0.0, |s| s.y + 8.0 + 30.0 + 12.0)
            + strip_h
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

        if let (Some(tex), Some(size)) = (d.preview.clone(), preview_size) {
            let r = Rect::from_min_size(Pos2::new(x0 + (text_w - size.x) / 2.0, y), size);
            p.image(tex.id(), r, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
            let resp = ui.interact(r, Id::new("ai-region"), Sense::click_and_drag());
            widgets::record(ui, "ai-region", r);
            let radius = super::BRUSHES[d.brush.min(super::BRUSHES.len() - 1)];
            if let Some(mask) = d.mask.as_mut() {
                // the brush: every pointer position while the button is down joins the stroke
                let (mw, mh) = (mask.w as f32, mask.h as f32);
                let cell = |pos: Pos2| ((pos.x - r.left()) / r.width() * mw, (pos.y - r.top()) / r.height() * mh);
                let down = resp.is_pointer_button_down_on();
                let at = if down { ui.input(|i| i.pointer.interact_pos()) } else { None };
                match at.map(cell) {
                    Some(now) => {
                        // a new stroke starts where the button went down
                        let from = d.last.or(ui.input(|i| i.pointer.press_origin()).map(cell)).unwrap_or(now);
                        mask.stroke(from, now, radius * mw / r.width());
                        d.last = Some(now);
                        d.mask_dirty = true;
                    }
                    None => d.last = None,
                }
                if d.mask_dirty {
                    let tint = theme::BG_BRAND;
                    let px: Vec<u8> = mask.data.iter().flat_map(|m| [tint.r(), tint.g(), tint.b(), if *m == 0 { 0 } else { 105 }]).collect();
                    let image = egui::ColorImage::from_rgba_unmultiplied([mask.w as usize, mask.h as usize], &px);
                    d.mask_tex = Some(ui.ctx().load_texture("ai-edit-mask", image, egui::TextureOptions::LINEAR));
                    d.mask_dirty = false;
                }
                if let Some(mt) = &d.mask_tex {
                    p.image(mt.id(), r, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
                }
                if let Some(pos) = resp.hover_pos() {
                    let ring = p.with_clip_rect(r);
                    ring.circle_stroke(pos, radius, Stroke::new(1.5, Color32::WHITE));
                    ring.circle_stroke(pos, radius + 1.0, Stroke::new(1.0, Color32::from_black_alpha(120)));
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
                }
            }
            y += size.y + 8.0;

            // brush sizes and Clear
            label(&p, x0, y + 7.0, "Brush");
            let mut x = x0 + widgets::text_width(&p, tr("Brush"), &theme::font_small_medium()) + 10.0;
            for (i, size) in super::BRUSHES.iter().enumerate() {
                let br = Rect::from_min_size(Pos2::new(x, y), Vec2::new(34.0, 30.0));
                let resp = ui.interact(br, Id::new(("ai-brush", i)), Sense::click());
                let on = d.brush == i;
                p.rect_filled(
                    br,
                    theme::cr(theme::RADIUS_MEDIUM),
                    if on {
                        theme::BG_SELECTED
                    } else if resp.hovered() {
                        theme::BG_HOVER
                    } else {
                        theme::BG_SECONDARY
                    },
                );
                p.circle_filled(br.center(), 2.0 + size / 6.0, theme::TEXT);
                if resp.clicked() {
                    d.brush = i;
                }
                widgets::record(ui, &format!("ai-brush-{i}"), br);
                x += 34.0 + 6.0;
            }
            let painted = d.mask.as_ref().is_some_and(|m| !m.is_empty());
            let cw = button_width(&p, "Clear") + 16.0;
            let cr = Rect::from_min_size(Pos2::new(x0 + text_w - cw, y), Vec2::new(cw, 30.0));
            if button(ui, cr, "ai-clear", "Clear", false, painted).clicked() && painted {
                if let Some(m) = d.mask.as_mut() {
                    m.clear();
                    d.mask_dirty = true;
                }
            }
            widgets::record(ui, "ai-clear", cr);
            y += 30.0 + 12.0;
        }

        // the layer's earlier pictures: a click brings one back
        if d.versions.len() > 1 {
            label(&p, x0, y - 4.0, "Versions");
            let mut x = x0;
            for (i, v) in d.versions.iter().enumerate() {
                let s = v.tex.size_vec2();
                let w = (s.x / s.y * THUMB).clamp(24.0, 96.0);
                let tr_ = Rect::from_min_size(Pos2::new(x, y + 16.0), Vec2::new(w, THUMB));
                if tr_.right() > x0 + text_w {
                    break;
                }
                let resp = ui.interact(tr_, Id::new(("ai-version", v.index)), Sense::click());
                p.image(v.tex.id(), tr_, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
                let ring = if v.current {
                    Stroke::new(2.0, theme::BG_BRAND)
                } else if resp.hovered() {
                    Stroke::new(1.5, theme::TEXT)
                } else {
                    Stroke::new(1.0, theme::BORDER)
                };
                p.rect_stroke(tr_, 2.0, ring, StrokeKind::Outside);
                let tip = if v.prompt.is_empty() { tr("Original").to_string() } else { v.prompt.clone() };
                if resp.on_hover_text(tip).clicked() && !v.current {
                    restore = Some(v.index);
                }
                widgets::record(ui, &format!("ai-version-{i}"), tr_);
                x += w + 8.0;
            }
            y += strip_h;
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
    } else if let Some(index) = restore {
        app.ai_restore_version(ctx, index);
    } else if submit {
        app.ai_submit(ctx);
    }
}

/// The small bar shown while a request runs: a spinner, the elapsed time and Cancel.
/// Returns true when the request should be cancelled.
fn running_pill(ctx: &Context, elapsed: f64) -> bool {
    let screen = ctx.content_rect();
    let mut cancel = ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
    let secs = elapsed.max(0.0) as u64;
    let text = format!("{} {}:{:02}", tr("Generating image\u{2026}"), secs / 60, secs % 60);
    egui::Area::new(Id::new("ai-running")).order(Order::Foreground).fixed_pos(screen.min).interactable(true).show(ctx, |ui| {
        let p = ui.painter().clone();
        let cw = button_width(&p, "Cancel") + 16.0;
        let tw = widgets::text_width(&p, &text, &theme::font_body_strong());
        let w = 14.0 + 18.0 + 10.0 + tw + 14.0 + cw + 6.0;
        let rect = Rect::from_min_size(Pos2::new(screen.center().x - w / 2.0, screen.bottom() - 132.0), Vec2::new(w, 44.0));
        ui.allocate_rect(rect, Sense::click());
        widgets::menu_panel(&p, rect, 12);
        spinner(&p, Pos2::new(rect.left() + 14.0 + 9.0, rect.center().y), 8.0, ui.input(|i| i.time));
        p.text(Pos2::new(rect.left() + 14.0 + 18.0 + 10.0, rect.center().y), Align2::LEFT_CENTER, &text, theme::font_body_strong(), theme::TEXT);
        let cr = Rect::from_min_size(Pos2::new(rect.right() - 6.0 - cw, rect.top() + 6.0), Vec2::new(cw, 32.0));
        if button(ui, cr, "ai-cancel", "Cancel", false, true).clicked() {
            cancel = true;
        }
        widgets::record(ui, "ai-cancel", cr);
    });
    ctx.request_repaint();
    cancel
}

/// A rotating arc.
fn spinner(p: &Painter, c: Pos2, r: f32, time: f64) {
    let start = (time * 4.5) as f32;
    let pts: Vec<Pos2> = (0..=18).map(|i| start + i as f32 / 18.0 * 4.4).map(|a| c + Vec2::new(a.cos(), a.sin()) * r).collect();
    p.circle_stroke(c, r, Stroke::new(2.0, theme::BORDER));
    p.add(egui::Shape::line(pts, Stroke::new(2.0, theme::BG_BRAND)));
}

/// Where on the screen the running request's image will appear: the layer being filled or
/// edited (the marked part of it), or the middle of the view for a new image.
fn target_rect(app: &App, d: &super::Prompt, canvas: Rect) -> Option<Rect> {
    let of_node = |id| {
        let b = crate::panels::inspect::page_aabb(&app.doc, id)?;
        let s = |x: f32, y: f32| app.view.to_screen(canvas.min, desigma_doc::Vec2::new(x, y));
        Some(Rect::from_two_pos(s(b.left(), b.top()), s(b.right(), b.bottom())))
    };
    match &d.target {
        Target::Fill(id) => of_node(*id),
        Target::Edit { node, .. } => {
            let r = of_node(*node)?;
            Some(match d.region() {
                Some(g) => Rect::from_min_max(
                    Pos2::new(r.left() + g.x0 * r.width(), r.top() + g.y0 * r.height()),
                    Pos2::new(r.left() + g.x1 * r.width(), r.top() + g.y1 * r.height()),
                ),
                None => r,
            })
        }
        Target::Place => {
            let ratio = ASPECTS[d.aspect.min(ASPECTS.len() - 1)].split_once(':').and_then(|(w, h)| Some((w.parse::<f32>().ok()?, h.parse::<f32>().ok()?)));
            let (w, h) = ratio.unwrap_or((1.0, 1.0));
            let k = (360.0 / w).min(280.0 / h);
            Some(Rect::from_center_size(canvas.center(), Vec2::new(w * k, h * k)))
        }
    }
}

/// While a request runs: the place the image will appear glows, with a band of light sweeping
/// across it, twinkling points and a pulsing outline. Drawn over the canvas.
pub fn canvas_overlay(app: &App, ui: &Ui, canvas: Rect) {
    let Some(d) = app.ai.prompt.as_ref().filter(|d| d.running) else { return };
    let Some(r) = target_rect(app, d, canvas) else { return };
    if r.width() < 2.0 || r.height() < 2.0 {
        return;
    }
    let t = ui.input(|i| i.time) as f32;
    let p = ui.painter().with_clip_rect(canvas.intersect(r.expand(3.0)));
    let brand = theme::BG_BRAND;
    let tint = |a: f32| Color32::from_rgba_unmultiplied(brand.r(), brand.g(), brand.b(), (a.clamp(0.0, 1.0) * 255.0) as u8);
    let pulse = 0.5 + 0.5 * (t * 2.6).sin();
    p.rect_filled(r, 2.0, tint(0.16 + 0.06 * pulse));

    // the sweep: a slanted band, clear at its edges and bright in the middle
    let inner = p.with_clip_rect(canvas.intersect(r));
    let band = (r.width() * 0.35).clamp(40.0, 260.0);
    let slant = r.height() * 0.35;
    let travel = r.width() + band + slant;
    let x = r.left() - band + travel * (t * 0.55).fract();
    let clear = Color32::from_rgba_unmultiplied(255, 255, 255, 0);
    let bright = Color32::from_rgba_unmultiplied(255, 255, 255, 70);
    let mut mesh = egui::Mesh::default();
    for (i, (dx, c)) in [(0.0, clear), (band / 2.0, bright), (band, clear)].into_iter().enumerate() {
        mesh.colored_vertex(Pos2::new(x + dx, r.top()), c);
        mesh.colored_vertex(Pos2::new(x + dx - slant, r.bottom()), c);
        if i > 0 {
            let v = (i as u32) * 2;
            mesh.add_triangle(v - 2, v - 1, v);
            mesh.add_triangle(v - 1, v, v + 1);
        }
    }
    inner.add(egui::Shape::mesh(mesh));

    // twinkles at fixed places, each on its own beat
    let count = ((r.area() / 9000.0) as usize).clamp(4, 28);
    for i in 0..count {
        let h = |k: f32| ((i as f32 * 12.9898 + k * 78.233).sin() * 43_758.547).fract().abs();
        let at = Pos2::new(r.left() + h(1.0) * r.width(), r.top() + h(2.0) * r.height());
        let glow = (0.5 + 0.5 * (t * (1.6 + h(3.0) * 2.4) + h(4.0) * std::f32::consts::TAU).sin()).powi(3);
        let size = 1.5 + 3.5 * glow;
        let c = Color32::from_rgba_unmultiplied(255, 255, 255, (glow * 220.0) as u8);
        inner.line_segment([at - Vec2::new(size, 0.0), at + Vec2::new(size, 0.0)], Stroke::new(1.2, c));
        inner.line_segment([at - Vec2::new(0.0, size), at + Vec2::new(0.0, size)], Stroke::new(1.2, c));
    }

    p.rect_stroke(r, 2.0, Stroke::new(1.5, tint(0.55 + 0.45 * pulse)), StrokeKind::Outside);
    ui.ctx().request_repaint();
}
