//! The comment tool and the pins on the canvas.
//!
//! `process` turns a canvas click into a draft thread while the Comment tool is
//! active; `overlay` draws the pins above the canvas, opens threads when they are
//! clicked and shows the popovers.

use super::model::TaskStatus;
use super::ui::{self, PinStyle};
use super::{model, thread, Composer};
use crate::app::App;
use crate::canvas::interact;
use crate::canvas::tools::Tool;
use egui::{CursorIcon, Pos2, Rect, Response, Sense, Ui};

/// Canvas click with the Comment tool: starts a new thread at the clicked point,
/// anchored to the node under the cursor.
pub fn process(app: &mut App, rect: Rect, resp: &Response) {
    if app.tool != Tool::Comment || app.history.viewing.is_some() || !resp.clicked_by(egui::PointerButton::Primary) {
        return;
    }
    if std::mem::take(&mut app.comments.swallow_click) {
        return;
    }
    let Some(pos) = resp.interact_pointer_pos().filter(|p| rect.contains(*p)) else { return };
    if app.comments.pin_rects.iter().any(|r| r.contains(pos)) {
        return;
    }
    if app.comments.open.is_some() || app.comments.composer.is_some() {
        app.comments.close_popovers();
        return;
    }
    let point = app.view.to_page(rect.min, pos);
    let node = interact::hit_under(app, point, false);
    let anchor = model::make_anchor(&app.doc, app.page, point, node);
    app.comments.composer = Some(Composer { anchor, text: String::new(), fresh: true });
}

struct PinView {
    id: String,
    tip: Pos2,
    author: String,
    resolved: bool,
    task: Option<TaskStatus>,
}

pub fn overlay(app: &mut App, ui: &mut Ui, rect: Rect) {
    let ctx = ui.ctx().clone();
    let comment_tool = app.tool == Tool::Comment;
    if !comment_tool {
        // leaving comment mode drops an unposted draft
        app.comments.composer = None;
    }
    if app.comments.open.as_ref().is_some_and(|id| !app.doc.comments.iter().any(|t| &t.id == id)) {
        app.comments.close_popovers();
    }
    let clip = ui.painter().with_clip_rect(rect);
    let (pressed, any_down, ptr) = ctx.input(|i| (i.pointer.any_pressed(), i.pointer.any_down(), i.pointer.latest_pos()));

    let pins: Vec<PinView> = model::visible_on(&app.doc, app.page, &app.doc.comments, app.comments.show_resolved)
        .filter_map(|t| {
            let p = model::pin_point(&app.doc, &t.anchor)?;
            Some(PinView {
                id: t.id.clone(),
                tip: app.view.to_screen(rect.min, p),
                author: t.messages.first().map(|m| m.author.clone()).unwrap_or_default(),
                resolved: t.resolved,
                task: model::task_status(t),
            })
        })
        .collect();
    app.comments.pin_rects.clear();
    let mut over_pin = false;
    let mut clicked_pin: Option<String> = None;
    for pv in &pins {
        let r = ui::pin_rect(pv.tip);
        if !rect.intersects(r) {
            continue;
        }
        let hit = r.expand(crate::theme::COMMENT_PIN_HIT_PAD);
        app.comments.pin_rects.push(hit);
        let blocked = crate::widgets::pointer_over_popup(&ctx);
        let resp = ui.interact(hit, ui.id().with(("comment-pin", &pv.id)), if blocked { Sense::hover() } else { Sense::click() });
        if resp.hovered() && !blocked {
            over_pin = true;
            ctx.set_cursor_icon(CursorIcon::PointingHand);
        }
        let style = if app.comments.open.as_deref() == Some(pv.id.as_str()) {
            PinStyle::Open
        } else if pv.resolved {
            PinStyle::Resolved
        } else {
            PinStyle::Normal
        };
        ui::pin(&clip, pv.tip, &pv.author, style, pv.task);
        if resp.clicked() {
            clicked_pin = Some(pv.id.clone());
        }
    }
    if let Some(id) = clicked_pin {
        if app.comments.open.as_deref() == Some(id.as_str()) {
            app.comments.close_popovers();
        } else {
            app.comments.close_popovers();
            app.comments.open = Some(id);
            app.comments.focus_reply = true;
        }
    }

    // the pin of the draft thread
    if let Some(c) = &app.comments.composer {
        if let Some(p) = model::pin_point(&app.doc, &c.anchor) {
            let author = app.comments.author.clone().unwrap_or_default();
            ui::pin(&clip, app.view.to_screen(rect.min, p), &author, PinStyle::Open, None);
        }
    }

    // ghost pin instead of the cursor in comment mode
    let busy = app.comments.composer.is_some() || app.history.viewing.is_some();
    if comment_tool && !busy && !over_pin {
        let hover = ctx.input(|i| i.pointer.hover_pos());
        if let Some(p) = hover.filter(|p| rect.contains(*p) && !crate::widgets::pointer_over_popup(&ctx)) {
            ctx.set_cursor_icon(CursorIcon::None);
            let author = app.author();
            ui::pin(&clip, p, &author, PinStyle::Ghost, None);
        }
    }

    let shown = thread::show(app, &ctx, rect);

    // a press outside the popover (and not on a pin or one of its menus) closes it
    if pressed && (app.comments.open.is_some() || app.comments.composer.is_some()) {
        if let Some(p) = ptr {
            let in_pop = shown.as_ref().is_some_and(|s| s.rect.contains(p));
            let in_pin = app.comments.pin_rects.iter().any(|r| r.contains(p));
            let other_popup = ctx.layer_id_at(p).is_some_and(|l| l.order == egui::Order::Foreground && shown.as_ref().is_none_or(|s| s.layer != l));
            if !in_pop && !in_pin && !other_popup {
                app.comments.close_popovers();
                app.comments.swallow_click = true;
            }
        }
    }
    if !any_down {
        app.comments.swallow_click = false;
    }
}
