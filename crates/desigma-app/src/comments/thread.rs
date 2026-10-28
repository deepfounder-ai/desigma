//! The popovers next to a pin: the composer for a new thread and the open thread
//! (messages, reply field, per-message "…" menu).

use super::{model, ui};
use crate::app::App;
use crate::home::time::relative;
use crate::icons::Icon;
use crate::theme;
use crate::widgets::menu::{popup_menu, MenuItem, MenuResult};
use crate::widgets::{self, hline};
use egui::{Align2, Area, Context, Id, LayerId, Order, Pos2, Rect, Vec2};

const COMPOSER_W: f32 = 280.0;
const AVATAR_GAP: f32 = 8.0;
const ROW_GAP: f32 = 8.0;

/// A popover that is on screen this frame.
pub struct Shown {
    pub rect: Rect,
    pub layer: LayerId,
}

fn measure_painter(ctx: &Context) -> egui::Painter {
    ctx.layer_painter(LayerId::new(Order::Foreground, Id::new("comment-measure")))
}

/// Puts a popover of `size` right of the pin (left of it when there is no room), inside `bounds`.
fn place(tip: Pos2, size: Vec2, bounds: Rect) -> Rect {
    let mut x = tip.x + theme::COMMENT_PIN + theme::COMMENT_POP_GAP;
    if x + size.x > bounds.right() - 8.0 {
        x = tip.x - theme::COMMENT_POP_GAP - size.x;
    }
    let x = x.clamp(bounds.left() + 8.0, (bounds.right() - size.x - 8.0).max(bounds.left() + 8.0));
    let y = (tip.y - theme::COMMENT_PIN).clamp(bounds.top() + 8.0, (bounds.bottom() - size.y - 8.0).max(bounds.top() + 8.0));
    Rect::from_min_size(Pos2::new(x, y), size)
}

pub fn show(app: &mut App, ctx: &Context, canvas: Rect) -> Option<Shown> {
    if app.comments.composer.is_some() {
        return composer(app, ctx, canvas);
    }
    let id = app.comments.open.clone()?;
    open_thread(app, ctx, canvas, &id)
}

fn pop_frame(p: &egui::Painter, rect: Rect) {
    widgets::menu_panel(p, rect, theme::RADIUS_LARGE);
}

// ------------------------------------------------------------------ composer

fn composer(app: &mut App, ctx: &Context, canvas: Rect) -> Option<Shown> {
    let author = app.author();
    let c = app.comments.composer.as_ref()?;
    let tip = app.view.to_screen(canvas.min, model::pin_point(&app.doc, &c.anchor)?);
    let inner_w = COMPOSER_W - theme::COMMENT_PAD * 2.0;
    let box_h = ui::box_height(&measure_painter(ctx), &c.text, inner_w);
    let h = theme::COMMENT_PAD * 2.0 + theme::COMMENT_AVATAR + ROW_GAP + box_h + ROW_GAP + theme::COMMENT_BTN_H;
    let rect = place(tip, Vec2::new(COMPOSER_W, h), canvas);
    let (post_key, esc) = ui::submit_keys(ctx);
    let mut text = std::mem::take(&mut app.comments.composer.as_mut()?.text);
    let fresh = std::mem::take(&mut app.comments.composer.as_mut()?.fresh);
    let (mut post, cancel) = (post_key, esc);
    let area = Area::new(Id::new("comment-composer")).order(Order::Foreground).fixed_pos(rect.min).show(ctx, |ui| {
        ui.allocate_rect(rect, egui::Sense::hover());
        let p = ui.painter().clone();
        pop_frame(&p, rect);
        let x = rect.left() + theme::COMMENT_PAD;
        let mut y = rect.top() + theme::COMMENT_PAD;
        ui::avatar(&p, Pos2::new(x + theme::COMMENT_AVATAR / 2.0, y + theme::COMMENT_AVATAR / 2.0), theme::COMMENT_AVATAR, &author);
        p.text(
            Pos2::new(x + theme::COMMENT_AVATAR + AVATAR_GAP, y + theme::COMMENT_AVATAR / 2.0),
            Align2::LEFT_CENTER,
            &author,
            theme::font_body_strong(),
            theme::TEXT,
        );
        y += theme::COMMENT_AVATAR + ROW_GAP;
        let box_r = Rect::from_min_size(Pos2::new(x, y), Vec2::new(inner_w, box_h));
        ui::text_box(ui, box_r, Id::new("comment-composer-text"), &mut text, "Add a comment", fresh);
        widgets::record(ui, "comment-composer-box", box_r);
        y += box_h + ROW_GAP;
        let bw = ui::button_width(&p, "Post").max(56.0);
        let br = Rect::from_min_size(Pos2::new(rect.right() - theme::COMMENT_PAD - bw, y), Vec2::new(bw, theme::COMMENT_BTN_H));
        p.text(Pos2::new(x, y + theme::COMMENT_BTN_H / 2.0), Align2::LEFT_CENTER, "Ctrl+Enter to post", theme::font_small(), theme::TEXT_TERTIARY);
        let enabled = !text.trim().is_empty();
        if ui::button(ui, br, "composer-post", "Post", true, enabled).clicked() {
            post = true;
        }
        widgets::record(ui, "comment-composer-post", br);
    });
    let shown = Shown { rect, layer: area.response.layer_id };
    if let Some(c) = app.comments.composer.as_mut() {
        c.text = text.clone();
    }
    if cancel {
        app.comments.composer = None;
        return None;
    }
    if post && !text.trim().is_empty() {
        let anchor = app.comments.composer.as_ref()?.anchor;
        app.comments.composer = None;
        app.post_comment(anchor, &text);
        return None;
    }
    Some(shown)
}

// ------------------------------------------------------------------ thread

enum Act {
    Resolve(bool),
    Task(model::TaskStatus),
    Close,
    Reply(String),
    OpenMenu(String, Rect),
    SaveEdit,
    CancelEdit,
}

fn open_thread(app: &mut App, ctx: &Context, canvas: Rect, id: &str) -> Option<Shown> {
    let thread = app.doc.comments.iter().find(|t| t.id == id)?.clone();
    let tip = app.view.to_screen(canvas.min, model::pin_point(&app.doc, &thread.anchor)?);
    let now = super::time::now_ms();
    let w = theme::COMMENT_POP_W;
    let pad = theme::COMMENT_PAD;
    let inner_w = w - pad * 2.0;
    let text_x = theme::COMMENT_AVATAR + AVATAR_GAP;
    let text_w = inner_w - text_x;
    let mp = measure_painter(ctx);

    // ---- measure
    let editing = app.comments.editing.as_ref().filter(|e| e.thread == id).map(|e| (e.comment.clone(), e.text.clone()));
    let mut galleys = Vec::new();
    let mut heights = Vec::new();
    let mut edit_box_h = 0.0;
    let task = model::task_status(&thread);
    for (i, m) in thread.messages.iter().enumerate() {
        let is_edit = editing.as_ref().is_some_and(|(c, _)| *c == m.id);
        if is_edit {
            edit_box_h = ui::box_height(&mp, editing.as_ref().map(|e| e.1.as_str()).unwrap_or(""), inner_w);
            galleys.push(None);
            heights.push(theme::COMMENT_AVATAR + ROW_GAP + edit_box_h + ROW_GAP + theme::COMMENT_BTN_H + pad);
        } else {
            // the opening message shows its task marker as the header status, not as text
            let text = if i == 0 { model::split_marker(&m.text).1 } else { m.text.as_str() };
            let g = mp.layout_job(ui::mention_job(text, theme::font_body(), theme::TEXT, text_w));
            heights.push(22.0 + g.size().y + pad);
            galleys.push(Some(g));
        }
    }
    let reply_box_h = ui::box_height(&mp, &app.comments.reply, inner_w);
    let reply_h = pad + reply_box_h + ROW_GAP + theme::COMMENT_BTN_H + pad;
    let total = theme::COMMENT_HEADER_H + pad / 2.0 + heights.iter().sum::<f32>() + 1.0 + reply_h;
    let rect = place(tip, Vec2::new(w, total), canvas);

    let (post_key, esc) = ui::submit_keys(ctx);
    let mut reply = std::mem::take(&mut app.comments.reply);
    let mut edit_text = app.comments.editing.as_mut().filter(|e| e.thread == id).map(|e| std::mem::take(&mut e.text));
    let edit_fresh = app.comments.editing.as_mut().map(|e| std::mem::take(&mut e.fresh)).unwrap_or(false);
    let focus_reply = std::mem::take(&mut app.comments.focus_reply);
    let mut acts: Vec<Act> = Vec::new();
    if esc {
        acts.push(if edit_text.is_some() { Act::CancelEdit } else { Act::Close });
    }

    let area = Area::new(Id::new("comment-thread")).order(Order::Foreground).fixed_pos(rect.min).show(ctx, |ui| {
        ui.allocate_rect(rect, egui::Sense::hover());
        let p = ui.painter().clone();
        pop_frame(&p, rect);
        let left = rect.left() + pad;
        // header: state on the left, resolve and close on the right
        let hr = Rect::from_min_size(rect.min, Vec2::new(w, theme::COMMENT_HEADER_H));
        if thread.resolved {
            p.text(Pos2::new(left, hr.center().y), Align2::LEFT_CENTER, "Resolved", theme::font_body(), theme::TEXT_SECONDARY);
        } else if let Some(s) = task {
            // task status; a click moves it on (to do → in progress → done)
            let lw = widgets::text_width(&p, s.label(), &theme::font_body());
            let sr = Rect::from_min_size(Pos2::new(left - 4.0, hr.center().y - 12.0), Vec2::new(lw + 26.0, 24.0));
            let resp = ui.interact(sr, ui.id().with("thread-task"), egui::Sense::click());
            widgets::button_bg(&p, sr, &resp, false, theme::cr(theme::RADIUS_MEDIUM));
            ui::status_dot(&p, Pos2::new(sr.left() + 10.0, sr.center().y), s);
            p.text(Pos2::new(sr.left() + 18.0, sr.center().y), Align2::LEFT_CENTER, s.label(), theme::font_body(), theme::TEXT);
            widgets::record(ui, "comment-task-status", sr);
            if resp.on_hover_text(format!("Mark {}", s.next().label().to_lowercase())).clicked() {
                acts.push(Act::Task(s.next()));
            }
        }
        let btn = |i: f32| Rect::from_min_size(Pos2::new(rect.right() - 8.0 - 24.0 * (i + 1.0) - 4.0 * i, hr.center().y - 12.0), Vec2::splat(24.0));
        if widgets::icon_button(ui, btn(0.0), "thread-close", Icon::Close, false, "Close").clicked() {
            acts.push(Act::Close);
        }
        let tip_text = if thread.resolved { "Mark as unresolved" } else { "Mark as resolved" };
        let rb = btn(1.0);
        if widgets::icon_button(ui, rb, "thread-resolve", Icon::Check, thread.resolved, tip_text).clicked() {
            acts.push(Act::Resolve(!thread.resolved));
        }
        widgets::record(ui, "comment-resolve", rb);
        hline(&p, rect.left(), rect.right(), rect.top() + theme::COMMENT_HEADER_H - 1.0, theme::BORDER_MENU);

        // messages
        let mut y = rect.top() + theme::COMMENT_HEADER_H + pad / 2.0;
        for (i, m) in thread.messages.iter().enumerate() {
            let row_h = heights[i];
            let av = Pos2::new(left + theme::COMMENT_AVATAR / 2.0, y + theme::COMMENT_AVATAR / 2.0);
            ui::avatar(&p, av, theme::COMMENT_AVATAR, &m.author);
            let nx = left + text_x;
            let name_r = widgets::text_on_baseline(&p, nx, y + 12.0, &m.author, theme::font_body_strong(), theme::TEXT);
            let when = super::time::parse_rfc3339(&m.created).map(|t| relative(now, t)).unwrap_or_default();
            let stamp = if m.edited.is_some() { format!("{when} (edited)") } else { when };
            let dots = Rect::from_min_size(Pos2::new(rect.right() - pad - theme::COMMENT_ICON_BTN, y), Vec2::splat(theme::COMMENT_ICON_BTN));
            let avail = (dots.left() - 4.0 - (name_r.right() + 6.0)).max(0.0);
            let stamp = widgets::elide(&p, &stamp, &theme::font_small(), avail);
            widgets::text_on_baseline(&p, name_r.right() + 6.0, y + 12.0, &stamp, theme::font_small(), theme::TEXT_TERTIARY);
            if editing.as_ref().is_none_or(|(c, _)| *c != m.id) && ui::dots_button(ui, dots, ("msg-dots", i), "").clicked() {
                acts.push(Act::OpenMenu(m.id.clone(), dots));
            }
            widgets::record(ui, &format!("comment-dots-{i}"), dots);
            if let (Some(txt), Some((c, _))) = (edit_text.as_mut(), editing.as_ref()) {
                if *c == m.id {
                    let by = y + theme::COMMENT_AVATAR + ROW_GAP;
                    let br = Rect::from_min_size(Pos2::new(left, by), Vec2::new(inner_w, edit_box_h));
                    ui::text_box(ui, br, Id::new("comment-edit-text"), txt, "Edit comment", edit_fresh);
                    let sy = by + edit_box_h + ROW_GAP;
                    let sw = ui::button_width(&p, "Save").max(56.0);
                    let save_r = Rect::from_min_size(Pos2::new(rect.right() - pad - sw, sy), Vec2::new(sw, theme::COMMENT_BTN_H));
                    let cw = ui::button_width(&p, "Cancel").max(56.0);
                    let cancel_r = Rect::from_min_size(Pos2::new(save_r.left() - 4.0 - cw, sy), Vec2::new(cw, theme::COMMENT_BTN_H));
                    if ui::button(ui, save_r, "edit-save", "Save", true, !txt.trim().is_empty()).clicked() {
                        acts.push(Act::SaveEdit);
                    }
                    if ui::button(ui, cancel_r, "edit-cancel", "Cancel", false, true).clicked() {
                        acts.push(Act::CancelEdit);
                    }
                    if post_key {
                        acts.push(Act::SaveEdit);
                    }
                }
            } else if let Some(g) = &galleys[i] {
                p.galley(Pos2::new(nx, y + 22.0), g.clone(), theme::TEXT);
            }
            y += row_h;
        }

        // reply
        hline(&p, rect.left(), rect.right(), y, theme::BORDER_MENU);
        y += 1.0 + pad;
        let box_r = Rect::from_min_size(Pos2::new(left, y), Vec2::new(inner_w, reply_box_h));
        let reply_focus = focus_reply && edit_text.is_none();
        ui::text_box(ui, box_r, Id::new("comment-reply-text"), &mut reply, "Reply", reply_focus);
        widgets::record(ui, "comment-reply-box", box_r);
        y += reply_box_h + ROW_GAP;
        let bw = ui::button_width(&p, "Post").max(56.0);
        let br = Rect::from_min_size(Pos2::new(rect.right() - pad - bw, y), Vec2::new(bw, theme::COMMENT_BTN_H));
        p.text(Pos2::new(left, y + theme::COMMENT_BTN_H / 2.0), Align2::LEFT_CENTER, "Ctrl+Enter to post", theme::font_small(), theme::TEXT_TERTIARY);
        let can_reply = !reply.trim().is_empty();
        if ui::button(ui, br, "reply-post", "Reply", true, can_reply).clicked() || (post_key && edit_text.is_none() && can_reply) {
            acts.push(Act::Reply(reply.clone()));
        }
        widgets::record(ui, "comment-reply-post", br);
    });
    let shown = Shown { rect, layer: area.response.layer_id };

    // put the drafts back, then run what was asked
    app.comments.reply = reply;
    if let (Some(t), Some(e)) = (edit_text, app.comments.editing.as_mut()) {
        e.text = t;
    }
    for a in acts {
        match a {
            Act::Close => {
                app.comments.close_popovers();
                return None;
            }
            Act::Resolve(r) => {
                app.resolve_comment_thread(id, r);
                app.comments.close_popovers();
                return None;
            }
            Act::Task(s) => {
                app.set_comment_task(id, s);
            }
            Act::Reply(text) => {
                if app.reply_comment(id, &text) {
                    app.comments.reply.clear();
                    app.comments.focus_reply = true;
                }
            }
            Act::OpenMenu(comment, anchor) => app.comments.menu = Some(super::MsgMenu { thread: id.to_string(), comment, anchor }),
            Act::SaveEdit => {
                if let Some(e) = app.comments.editing.take() {
                    app.edit_comment_text(&e.thread, &e.comment, &e.text);
                }
            }
            Act::CancelEdit => app.comments.editing = None,
        }
    }
    message_menu(app, ctx);
    Some(shown)
}

/// The "…" menu of a message: Edit, Delete message, Delete thread, Mark resolved.
fn message_menu(app: &mut App, ctx: &Context) {
    let Some(m) = app.comments.menu.as_ref() else { return };
    let (thread, comment, anchor) = (m.thread.clone(), m.comment.clone(), m.anchor);
    let Some(t) = app.doc.comments.iter().find(|t| t.id == thread) else {
        app.comments.menu = None;
        return;
    };
    let resolved = t.resolved;
    let items = [
        MenuItem::new("Edit"),
        MenuItem::new("Delete message"),
        MenuItem::new("Delete thread").sep(),
        MenuItem::new(if resolved { "Mark unresolved" } else { "Mark resolved" }),
    ];
    match popup_menu(ctx, Id::new("comment-msg-menu"), anchor.left_bottom() + Vec2::new(0.0, 2.0), 160.0, &items, Some(anchor)) {
        MenuResult::Open => {}
        MenuResult::Closed => app.comments.menu = None,
        MenuResult::Picked(i) => {
            app.comments.menu = None;
            match i {
                0 => {
                    let text =
                        app.doc.comments.iter().find(|t| t.id == thread).and_then(|t| t.messages.iter().find(|m| m.id == comment)).map(|m| m.text.clone());
                    if let Some(text) = text {
                        app.comments.editing = Some(super::Editing { thread, comment, text, fresh: true });
                    }
                }
                1 => {
                    app.delete_comment_message(&thread, &comment);
                }
                2 => {
                    app.delete_comment_thread(&thread);
                    app.comments.close_popovers();
                }
                _ => {
                    app.resolve_comment_thread(&thread, !resolved);
                    app.comments.close_popovers();
                }
            }
        }
    }
}
