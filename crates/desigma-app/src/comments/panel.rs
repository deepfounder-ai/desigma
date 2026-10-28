//! The right panel in comment mode: every thread, filtered by page, status and text.

use super::model::{self, TaskStatus};
use super::ui;
use crate::app::App;
use crate::canvas::tools::Tool;
use crate::home::time::relative;
use crate::icons::Icon;
use crate::theme;
use crate::widgets::{self, hline};
use egui::{Align2, Id, Pos2, Rect, Sense, Stroke, StrokeKind, TextEdit, Ui, Vec2};

const SIDE: f32 = 12.0;
const AVATAR: f32 = 20.0;
const ROWS_MAX: usize = 3;
const SEARCH_H: f32 = 28.0;
const FOOTER_H: f32 = 40.0;

pub fn show(app: &mut App, ui: &mut Ui, rect: Rect) {
    let p = ui.painter().clone();
    p.rect_filled(rect, 0.0, theme::BG);
    widgets::vline(&p, rect.left(), rect.top(), rect.bottom(), theme::BORDER);
    let x = rect.left() + SIDE + 1.0;
    let w = rect.width() - SIDE * 2.0 - 1.0;

    // header
    p.text(
        Pos2::new(rect.left() + 16.0, rect.top() + theme::COMMENT_PANEL_HEADER_H / 2.0),
        Align2::LEFT_CENTER,
        "Comments",
        theme::font_large_strong(),
        theme::TEXT,
    );
    let close = Rect::from_min_size(Pos2::new(rect.right() - 8.0 - 24.0, rect.top() + theme::COMMENT_PANEL_HEADER_H / 2.0 - 12.0), Vec2::splat(24.0));
    if widgets::icon_button(ui, close, "comments-close", Icon::Close, false, "Back to the Move tool").clicked() {
        app.set_tool(Tool::Move);
    }
    widgets::record(ui, "comments-panel", rect);
    let mut y = rect.top() + theme::COMMENT_PANEL_HEADER_H;
    hline(&p, rect.left() + 1.0, rect.right(), y, theme::BORDER);
    y += 1.0 + 8.0;

    // filters
    let r = Rect::from_min_size(Pos2::new(x, y), Vec2::new(w, theme::COMMENT_FILTER_H));
    if let Some(i) = ui::segmented(ui, r, "cf-page", &["This page", "All pages"], app.comments.all_pages as usize) {
        app.comments.all_pages = i == 1;
        app.comments.scroll = 0.0;
    }
    widgets::record(ui, "comments-filter-pages", r);
    y += theme::COMMENT_FILTER_H + 8.0;
    let r = Rect::from_min_size(Pos2::new(x, y), Vec2::new(w, theme::COMMENT_FILTER_H));
    if let Some(i) = ui::segmented(ui, r, "cf-status", &["Open", "Resolved"], app.comments.list_resolved as usize) {
        app.comments.list_resolved = i == 1;
        app.comments.scroll = 0.0;
    }
    widgets::record(ui, "comments-filter-status", r);
    y += theme::COMMENT_FILTER_H + 8.0;

    // search
    let sr = Rect::from_min_size(Pos2::new(x, y), Vec2::new(w, SEARCH_H));
    p.rect_filled(sr, theme::cr(theme::RADIUS_MEDIUM), theme::BG_SECONDARY);
    let sid = Id::new("comments-search");
    if ui.memory(|m| m.has_focus(sid)) {
        p.rect_stroke(sr, theme::cr(theme::RADIUS_MEDIUM), Stroke::new(1.0, theme::BORDER_SELECTED), StrokeKind::Inside);
    }
    crate::icons::paint(&p, Icon::Search, Pos2::new(sr.left() + 14.0, sr.center().y), 16.0, theme::ICON_SECONDARY);
    let font = theme::font_body();
    let te = TextEdit::singleline(&mut app.comments.search)
        .id(sid)
        .font(font.clone())
        .text_color(theme::TEXT)
        .frame(egui::Frame::NONE)
        .margin(egui::Margin::ZERO)
        .vertical_align(egui::Align::Center)
        .hint_text(egui::RichText::new("Search comments").font(font.clone()).color(theme::TEXT_TERTIARY))
        .desired_width(sr.width() - 36.0);
    let search_field = Rect::from_min_max(Pos2::new(sr.left() + 28.0, sr.top()), Pos2::new(sr.right() - 8.0, sr.bottom()));
    if ui.put(search_field, te).changed() {
        app.comments.scroll = 0.0;
    }
    widgets::record(ui, "comments-search", sr);
    y += SEARCH_H + 8.0;
    hline(&p, rect.left() + 1.0, rect.right(), y, theme::BORDER);
    y += 1.0;

    // footer: show resolved pins on the canvas
    let footer = Rect::from_min_max(Pos2::new(rect.left() + 1.0, rect.bottom() - FOOTER_H), rect.max);
    hline(&p, footer.left(), footer.right(), footer.top(), theme::BORDER);
    let cb = Rect::from_min_size(Pos2::new(x, footer.top() + 8.0), Vec2::new(w, 24.0));
    let resp = ui.interact(cb, ui.id().with("show-resolved"), Sense::click());
    let box_r = Rect::from_min_size(Pos2::new(cb.left(), cb.center().y - 7.0), Vec2::splat(14.0));
    if app.comments.show_resolved {
        p.rect_filled(box_r, theme::cr(theme::RADIUS_SMALL), theme::BG_BRAND);
        widgets::paint_icon_in(&p, box_r, Icon::Check, 12.0, theme::TEXT);
    } else {
        p.rect_stroke(box_r, theme::cr(theme::RADIUS_SMALL), Stroke::new(1.0, theme::TEXT_TERTIARY), StrokeKind::Inside);
    }
    p.text(Pos2::new(box_r.right() + 8.0, cb.center().y), Align2::LEFT_CENTER, "Show resolved", font.clone(), theme::TEXT);
    widgets::record(ui, "comments-show-resolved", cb);
    if resp.clicked() {
        app.comments.show_resolved = !app.comments.show_resolved;
    }

    // list
    let list = Rect::from_min_max(Pos2::new(rect.left() + 1.0, y), Pos2::new(rect.right(), footer.top()));
    let filter = model::Filter { all_pages: app.comments.all_pages, resolved: app.comments.list_resolved, query: app.comments.search.clone() };
    let threads: Vec<_> = model::filtered(&app.doc, app.page, &app.doc.comments, &filter).into_iter().cloned().collect();
    if threads.is_empty() {
        let msg = if app.doc.comments.is_empty() { "No comments yet.\nClick the canvas to add one." } else { "No matching comments." };
        let g = p.layout(msg.to_string(), font.clone(), theme::TEXT_TERTIARY, w);
        p.galley(Pos2::new(x, list.top() + 16.0), g, theme::TEXT_TERTIARY);
        return;
    }
    let now = super::time::now_ms();
    let card_w = w;
    let text_w = card_w - theme::COMMENT_CARD_PAD * 2.0;
    struct Card {
        id: String,
        galley: std::sync::Arc<egui::Galley>,
        h: f32,
        replies: usize,
        page: String,
        task: Option<TaskStatus>,
    }
    let cards: Vec<Card> = threads
        .iter()
        .map(|t| {
            let first = &t.messages[0];
            let (task, body) = model::split_marker(&first.text);
            let mut job = ui::mention_job(body, font.clone(), theme::TEXT, text_w);
            job.wrap.max_rows = ROWS_MAX;
            job.wrap.overflow_character = Some('…');
            let galley = p.layout_job(job);
            let replies = t.messages.len() - 1;
            let page = model::thread_page(&app.doc, t);
            let page_name = if app.comments.all_pages { app.doc.get(page).map(|n| n.name.clone()).unwrap_or_default() } else { String::new() };
            let has_footer = replies > 0 || !page_name.is_empty() || task.is_some();
            let h = theme::COMMENT_CARD_PAD * 2.0 + AVATAR + 6.0 + galley.size().y + if has_footer { 18.0 } else { 0.0 };
            Card { id: t.id.clone(), galley, h, replies, page: page_name, task }
        })
        .collect();
    let content_h: f32 = cards.iter().map(|c| c.h + 4.0).sum::<f32>() + 8.0;
    crate::panels::wheel_scroll(ui, list, &mut app.comments.scroll, content_h);
    let scroll = app.comments.scroll;
    let mut goto: Option<String> = None;
    let mut mark: Option<(String, TaskStatus)> = None;
    let open = app.comments.open.clone();
    crate::panels::clipped(ui, list, |ui| {
        let p = ui.painter().clone();
        let mut cy = list.top() + 8.0 - scroll;
        for (t, c) in threads.iter().zip(&cards) {
            let r = Rect::from_min_size(Pos2::new(x, cy), Vec2::new(card_w, c.h));
            cy += c.h + 4.0;
            if r.bottom() < list.top() || r.top() > list.bottom() {
                continue;
            }
            let resp = ui.interact(r, ui.id().with(("comment-card", &c.id)), Sense::click());
            if open.as_deref() == Some(c.id.as_str()) {
                p.rect_filled(r, theme::cr(theme::RADIUS_MEDIUM), theme::BG_SELECTED);
            } else if resp.hovered() {
                p.rect_filled(r, theme::cr(theme::RADIUS_MEDIUM), theme::BG_HOVER);
            }
            let first = &t.messages[0];
            let ax = r.left() + theme::COMMENT_CARD_PAD;
            let ay = r.top() + theme::COMMENT_CARD_PAD;
            ui::avatar(&p, Pos2::new(ax + AVATAR / 2.0, ay + AVATAR / 2.0), AVATAR, &first.author);
            let name = widgets::elide(&p, &first.author, &theme::font_body_strong(), card_w * 0.45);
            let nr = widgets::text_on_baseline(&p, ax + AVATAR + 8.0, ay + AVATAR / 2.0 + 4.0, &name, theme::font_body_strong(), theme::TEXT);
            let when = model::last_activity(t);
            let stamp = if when > 0 { relative(now, when) } else { String::new() };
            let stamp = widgets::elide(&p, &stamp, &theme::font_small(), (r.right() - theme::COMMENT_CARD_PAD - nr.right() - 8.0).max(0.0));
            widgets::text_on_baseline(&p, nr.right() + 8.0, ay + AVATAR / 2.0 + 4.0, &stamp, theme::font_small(), theme::TEXT_TERTIARY);
            let ty = ay + AVATAR + 6.0;
            p.galley(Pos2::new(ax, ty), c.galley.clone(), theme::TEXT);
            // footer: status dot and label, replies, page; "Mark done" / "Reopen" on the right
            let fy = ty + c.galley.size().y + 13.0;
            let mut lx = ax;
            let mut right = r.right() - theme::COMMENT_CARD_PAD;
            let mut parts = Vec::new();
            let mut marked = false;
            if let Some(s) = c.task {
                ui::status_dot(&p, Pos2::new(ax + 4.0, fy - 4.0), s);
                lx += 14.0;
                parts.push(s.label().to_string());
                let label = if s == TaskStatus::Done { "Reopen" } else { "Mark done" };
                let lw = widgets::text_width(&p, label, &theme::font_small_strong());
                let br = Rect::from_min_max(Pos2::new(right - lw - 8.0, fy - 14.0), Pos2::new(right, fy + 5.0));
                let bresp = ui.interact(br, ui.id().with(("comment-mark", &c.id)), Sense::click());
                widgets::button_bg(&p, br, &bresp, false, theme::cr(theme::RADIUS_SMALL));
                let col = if bresp.hovered() { theme::TEXT } else { theme::TEXT_BRAND };
                widgets::text_on_baseline(&p, br.left() + 4.0, fy, label, theme::font_small_strong(), col);
                widgets::record(ui, &format!("comment-mark-{}", first.text.chars().take(24).collect::<String>()), br);
                if bresp.clicked() {
                    mark = Some((c.id.clone(), if s == TaskStatus::Done { TaskStatus::Todo } else { TaskStatus::Done }));
                    marked = true;
                }
                right = br.left() - 8.0;
            }
            if c.replies > 0 {
                parts.push(format!("{} {}", c.replies, if c.replies == 1 { "reply" } else { "replies" }));
            }
            if !c.page.is_empty() {
                parts.push(c.page.clone());
            }
            if !parts.is_empty() {
                let line = widgets::elide(&p, &parts.join("  \u{b7}  "), &theme::font_small(), (right - lx).max(0.0));
                widgets::text_on_baseline(&p, lx, fy, &line, theme::font_small(), theme::TEXT_SECONDARY);
            }
            widgets::record(ui, &format!("comment-card-{}", first.text.chars().take(24).collect::<String>()), r);
            if resp.clicked() && !marked {
                goto = Some(c.id.clone());
            }
        }
    });
    crate::panels::scrollbar(ui, list, scroll, content_h);
    if let Some(id) = goto {
        app.goto_comment(&id);
    }
    if let Some((id, status)) = mark {
        app.set_comment_task(&id, status);
    }
}
