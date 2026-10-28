//! The Agents panel in the left column: connect row, people strip, then the Chat tab
//! (chat selector, messages, composer, import from web) or the Notes & tasks tab.

use super::client::{self, Message, Thread};
use super::{Iterate, Link, Parallel, Tab, IMPORT_WIDTHS};
use crate::app::App;
use crate::comments::ui::{button, button_width, segmented, text_box};
use crate::icons::Icon;
use crate::panels::{clipped, scrollbar, wheel_scroll};
use crate::theme;
use crate::widgets::menu::dropdown;
use crate::widgets::popover::check;
use crate::widgets::{self, hline};
use egui::text::LayoutJob;
use egui::{Align2, Color32, Id, Key, Modifiers, Pos2, Rect, Sense, Ui, Vec2};

const SIDE: f32 = 12.0;
const ROW: f32 = 24.0;
const GAP: f32 = 4.0;
const COMPOSER_H: f32 = 64.0;
const CHIP_H: f32 = 20.0;
const ROWS_MAX: usize = 12;
/// How to start a server when none answers; the one place an implementation is named.
const START_HINT: &str = "desigma-agents serve --link-file link.txt";

pub fn show(app: &mut App, ui: &mut Ui, rect: Rect) {
    let p = ui.painter().clone();
    let x = rect.left() + SIDE;
    let w = rect.width() - SIDE * 2.0;
    let mut y = rect.top();
    widgets::record(ui, "agents-panel", rect);

    // ---- title row with the two tabs
    p.text(
        Pos2::new(rect.left() + theme::PANEL_PADDING_X, y + theme::PANEL_TITLE_ROW_HEIGHT / 2.0),
        Align2::LEFT_CENTER,
        "Agents",
        theme::font_body_strong(),
        theme::TEXT,
    );
    let open_notes = app.agents.state.as_ref().map(|s| s.comments.iter().filter(|t| !t.resolved && t.status.as_deref() != Some("done")).count()).unwrap_or(0);
    let notes_label = if open_notes > 0 { format!("Notes ({open_notes})") } else { "Notes".to_string() };
    let tabs_w = 120.0;
    let tr = Rect::from_min_size(Pos2::new(rect.right() - SIDE - tabs_w, y + 8.0), Vec2::new(tabs_w, ROW));
    if let Some(i) = segmented(ui, tr, "agents-tab", &["Chat", &notes_label], app.agents.tab as usize) {
        app.agents.tab = if i == 0 { Tab::Chat } else { Tab::Notes };
    }
    widgets::record(ui, "agents-tabs", tr);
    y += theme::PANEL_TITLE_ROW_HEIGHT;

    // ---- connect row
    y = connect_row(app, ui, x, y, w);
    if !matches!(app.agents.link, Link::Up) {
        not_running(app, ui, x, y, w);
        return;
    }
    y = people_strip(app, ui, x, y, w);
    hline(&p, rect.left(), rect.right(), y, theme::BORDER);
    y += 1.0;

    let body = Rect::from_min_max(Pos2::new(rect.left(), y), rect.max);
    match app.agents.tab {
        Tab::Chat => chat_tab(app, ui, body),
        Tab::Notes => notes_tab(app, ui, body),
    }
}

/// LED + "Connected · <doc> as <name>" or the reason it is not; hover shows the URL.
fn connect_row(app: &mut App, ui: &mut Ui, x: f32, y: f32, w: f32) -> f32 {
    let p = ui.painter().clone();
    let row = Rect::from_min_size(Pos2::new(x, y), Vec2::new(w, ROW));
    let (col, text) = match (&app.agents.link, &app.agents.state) {
        (Link::Idle | Link::Connecting, _) => (theme::TEXT_TERTIARY, "Connecting…".to_string()),
        (Link::Down(_), _) => (theme::DANGER, "Not running".to_string()),
        (Link::Up, Some(s)) if s.joined => (theme::AVATAR_COLORS[2], format!("Connected · {} as {}", s.doc_name.clone().unwrap_or_default(), s.name)),
        (Link::Up, Some(s)) if s.joining => (theme::AVATAR_COLORS[1], "Connected · joining the session…".into()),
        (Link::Up, Some(s)) => (
            theme::AVATAR_COLORS[1],
            format!("Connected · not in a session{}", s.join_error.as_ref().or(s.disconnected.as_ref()).map(|e| format!(": {e}")).unwrap_or_default()),
        ),
        (Link::Up, None) => (theme::AVATAR_COLORS[1], "Connected".into()),
    };
    p.circle_filled(Pos2::new(x + 5.0, row.center().y), 4.0, col);
    let font = theme::font_body();
    let shown = widgets::elide(&p, &text, &font, w - 16.0);
    p.text(Pos2::new(x + 16.0, row.center().y), Align2::LEFT_CENTER, shown, font, theme::TEXT);
    ui.interact(row, ui.id().with("agents-link"), Sense::hover()).on_hover_text(&app.agents.url);
    widgets::record(ui, "agents-connect", row);
    y + ROW + GAP
}

/// The explanation shown while no server answers, with a Retry button.
fn not_running(app: &mut App, ui: &mut Ui, x: f32, mut y: f32, w: f32) {
    let p = ui.painter().clone();
    let font = theme::font_body();
    let reason = match &app.agents.link {
        Link::Down(e) => e.clone(),
        _ => String::new(),
    };
    let text = format!(
        "No agent server answers at {}.{}\n\nStart one that joins this file's shared session (Share → Copy link) and offers the agents API, then connect. The URL comes from DESIGMA_AGENTS_URL or Preferences → Agent server URL.",
        app.agents.url,
        if reason.is_empty() { String::new() } else { format!("\n{reason}") }
    );
    let g = p.layout(text, font.clone(), theme::TEXT_SECONDARY, w);
    p.galley(Pos2::new(x, y + 8.0), g.clone(), theme::TEXT_SECONDARY);
    y += g.size().y + 16.0;
    let cmd = Rect::from_min_size(Pos2::new(x, y), Vec2::new(w, 28.0));
    p.rect_filled(cmd, theme::cr(theme::RADIUS_MEDIUM), theme::BG_SECONDARY);
    let mono = theme::font_mono();
    let shown = widgets::elide(&p, START_HINT, &mono, w - 16.0);
    p.text(Pos2::new(cmd.left() + 8.0, cmd.center().y), Align2::LEFT_CENTER, shown, mono, theme::TEXT);
    ui.interact(cmd, ui.id().with("agents-hint"), Sense::hover()).on_hover_text(START_HINT);
    widgets::record(ui, "agents-start-hint", cmd);
    y += 28.0 + 12.0;
    let bw = button_width(&p, "Connect");
    let br = Rect::from_min_size(Pos2::new(x, y), Vec2::new(bw, ROW));
    let busy = matches!(app.agents.link, Link::Connecting);
    if button(ui, br, "agents-retry", "Connect", true, !busy).clicked() {
        app.agents.state_due = 0.0;
        app.agents.link = Link::Connecting;
    }
    widgets::record(ui, "agents-retry", br);
}

/// Everyone in the session as "● name" chips, wrapped into as many rows as needed.
fn people_strip(app: &mut App, ui: &mut Ui, x: f32, y: f32, w: f32) -> f32 {
    let Some(state) = &app.agents.state else { return y };
    let p = ui.painter().clone();
    let font = theme::font_small_medium();
    let (mut cx, mut cy) = (x, y);
    if state.people.is_empty() {
        p.text(Pos2::new(x, y + CHIP_H / 2.0), Align2::LEFT_CENTER, "Nobody else in the session yet", theme::font_small(), theme::TEXT_TERTIARY);
        return y + CHIP_H + GAP;
    }
    for (i, person) in state.people.iter().enumerate() {
        let name = widgets::elide(&p, &person.name, &font, w - 24.0);
        let cw = 20.0 + widgets::text_width(&p, &name, &font) + 8.0;
        if cx + cw > x + w && cx > x {
            cx = x;
            cy += CHIP_H + GAP;
        }
        let chip = Rect::from_min_size(Pos2::new(cx, cy), Vec2::new(cw, CHIP_H));
        p.rect_filled(chip, theme::cr(10), theme::BG_SECONDARY);
        let col = client::color(&person.color, theme::TEXT_TERTIARY);
        p.circle_filled(Pos2::new(chip.left() + 10.0, chip.center().y), 4.0, col);
        p.text(
            Pos2::new(chip.left() + 18.0, chip.center().y),
            Align2::LEFT_CENTER,
            &name,
            font.clone(),
            if person.is_agent { theme::TEXT_SECONDARY } else { theme::TEXT },
        );
        let sel: Vec<&str> = person.selection.iter().map(|n| n.name.as_str()).collect();
        let tip = format!(
            "{}{}\npage: {}\nselection: {}",
            person.name,
            if person.is_agent { " (agent)" } else { "" },
            person.page_name.clone().unwrap_or_default(),
            if sel.is_empty() { "none".to_string() } else { sel.join(", ") }
        );
        ui.interact(chip, ui.id().with(("agents-person", i)), Sense::hover()).on_hover_text(tip);
        cx += cw + GAP;
    }
    cy + CHIP_H + 8.0
}

fn chat_tab(app: &mut App, ui: &mut Ui, body: Rect) {
    let x = body.left() + SIDE;
    let w = body.width() - SIDE * 2.0;
    let mut y = body.top() + 8.0;

    // ---- chat selector + New
    let chats: Vec<client::ChatSummary> = app.agents.state.as_ref().map(|s| s.chats.clone()).unwrap_or_default();
    // a leading emoji (a server's thread marker) has no glyph in the UI font
    let titles: Vec<String> = chats
        .iter()
        .map(|c| format!("{}{}", if c.active { "● " } else { "" }, c.title.trim_start_matches(|ch: char| ch as u32 > 0xFFFF).trim_start()))
        .collect();
    let current = chats.iter().position(|c| Some(c.id.as_str()) == app.agents.chat_id.as_deref());
    let label = current.map(|i| titles[i].clone()).unwrap_or_else(|| if chats.is_empty() { "New chat".into() } else { "Pick a chat".into() });
    let dr = Rect::from_min_size(Pos2::new(x, y), Vec2::new(w - ROW - GAP, ROW));
    if let Some(i) = dropdown(ui, dr, "agents-chat", &label, &titles) {
        app.agents.open_chat(Some(chats[i].id.clone()));
    }
    widgets::record(ui, "agents-chat-select", dr);
    let nr = Rect::from_min_size(Pos2::new(dr.right() + GAP, y), Vec2::splat(ROW));
    if widgets::icon_button(ui, nr, "agents-chat-new", Icon::Plus, false, "New chat").clicked() {
        app.agents_new_chat();
    }
    widgets::record(ui, "agents-chat-new", nr);
    y += ROW + 8.0;

    // ---- composer block at the bottom; the messages take what is left
    let import_h = 1.0 + 8.0 + ROW + GAP + ROW + 8.0;
    let composer_h = 1.0 + 8.0 + (ROW + GAP) * 4.0 + COMPOSER_H + GAP + ROW + 8.0 + import_h;
    let list = Rect::from_min_max(Pos2::new(body.left(), y), Pos2::new(body.right(), body.bottom() - composer_h));
    messages(app, ui, list);
    composer(app, ui, Rect::from_min_max(Pos2::new(body.left(), list.bottom()), body.max));
}

/// The messages of the open chat, newest at the bottom.
fn messages(app: &mut App, ui: &mut Ui, list: Rect) {
    let p = ui.painter().clone();
    let x = list.left() + SIDE;
    let w = list.width() - SIDE * 2.0;
    let font = theme::font_body();
    let Some(chat) = app.agents.chat.clone() else {
        let hint = if app.agents.chat_id.is_some() { "Loading…" } else { "Design anything. Your selection is sent as context when the toggle is on." };
        let g = p.layout(hint.into(), font, theme::TEXT_TERTIARY, w);
        p.galley(Pos2::new(x, list.top() + 8.0), g, theme::TEXT_TERTIARY);
        return;
    };
    if chat.messages.is_empty() {
        let g = p.layout("No messages yet.".into(), font, theme::TEXT_TERTIARY, w);
        p.galley(Pos2::new(x, list.top() + 8.0), g, theme::TEXT_TERTIARY);
        return;
    }
    // layout every line first so the content height (and the scroll clamp) is known
    let lines: Vec<Line> = chat.messages.iter().map(|m| line(&p, app.agents.state.as_ref(), m, w)).collect();
    let content_h: f32 = lines.iter().map(|l| l.h + 6.0).sum::<f32>() + 16.0;
    if app.agents.scroll_to_end {
        app.agents.messages_scroll = (content_h - list.height()).max(0.0);
        app.agents.scroll_to_end = false;
    }
    wheel_scroll(ui, list, &mut app.agents.messages_scroll, content_h);
    let scroll = app.agents.messages_scroll;
    clipped(ui, list, |ui| {
        let p = ui.painter().clone();
        let mut cy = list.top() + 8.0 - scroll;
        for l in &lines {
            if cy + l.h >= list.top() && cy <= list.bottom() {
                if let Some((name, col)) = &l.who {
                    p.circle_filled(Pos2::new(x + 5.0, cy + 7.0), 4.0, *col);
                    widgets::text_on_baseline(&p, x + 14.0, cy + 11.0, name, theme::font_small_strong(), *col);
                    p.galley(Pos2::new(x, cy + 16.0), l.galley.clone(), l.color);
                } else {
                    p.galley(Pos2::new(x, cy), l.galley.clone(), l.color);
                }
            }
            cy += l.h + 6.0;
        }
    });
    scrollbar(ui, list, scroll, content_h);
    widgets::record(ui, "agents-messages", list);
}

/// One laid-out chat line: an optional "● name" header, then the wrapped text.
struct Line {
    who: Option<(String, Color32)>,
    galley: std::sync::Arc<egui::Galley>,
    color: Color32,
    h: f32,
}

fn line(p: &egui::Painter, state: Option<&client::ServerState>, m: &Message, w: f32) -> Line {
    let (who, text, color, font) = match (m.role.as_str(), m.kind.as_str()) {
        ("user", _) => (Some((m.author.clone().unwrap_or_else(|| "You".into()), theme::TEXT_BRAND)), m.text.clone(), theme::TEXT, theme::font_body()),
        ("system", "run") => {
            let names: Vec<&str> = m.agents.iter().map(|a| a.name.as_str()).collect();
            let text = if names.is_empty() || names.iter().any(|n| m.text.contains(n)) { m.text.clone() } else { format!("{}: {}", m.text, names.join(", ")) };
            (None, text, theme::TEXT_SECONDARY, theme::font_small())
        }
        ("system", _) => (None, m.text.clone(), theme::TEXT_SECONDARY, theme::font_small()),
        (_, "tool") => {
            (None, format!("⚙ {}", m.summary.clone().or_else(|| m.tool.clone()).unwrap_or_else(|| "tool".into())), theme::TEXT_TERTIARY, theme::font_small())
        }
        (_, "exit") => {
            let text = match m.exit_code {
                Some(0) | None => "✓ finished".to_string(),
                Some(c) => format!("✗ exited with code {c}"),
            };
            (None, text, theme::TEXT_TERTIARY, theme::font_small())
        }
        (_, "result") => {
            let cost = m.cost_usd.map(|c| format!(" · ${c:.3}")).unwrap_or_default();
            let text = if m.is_error { format!("✗ {}", m.text) } else { format!("✓ done{cost}") };
            (None, text, if m.is_error { theme::DANGER } else { theme::TEXT_TERTIARY }, theme::font_small())
        }
        (_, "error") => (None, format!("✗ {}", m.text), theme::DANGER, theme::font_small()),
        _ => {
            let name = m.agent.clone().unwrap_or_else(|| "agent".into());
            let col = color_of(state, &name);
            (Some((name, col)), m.text.clone(), theme::TEXT, theme::font_body())
        }
    };
    let mut job = LayoutJob::simple(text, font, color, w);
    job.wrap.max_rows = ROWS_MAX;
    job.wrap.overflow_character = Some('…');
    let galley = p.layout_job(job);
    let h = galley.size().y + if who.is_some() { 16.0 } else { 0.0 };
    Line { who, galley, color, h }
}

/// The presence colour of `name` ("Phantom · sonnet" matches "Phantom"), else a stable fallback.
fn color_of(state: Option<&client::ServerState>, name: &str) -> Color32 {
    let base = |n: &str| n.split(" · ").next().unwrap_or(n).to_lowercase();
    let fallback = crate::comments::ui::avatar_color(name);
    let Some(s) = state else { return fallback };
    s.people
        .iter()
        .find(|p| p.name == name)
        .or_else(|| s.people.iter().find(|p| base(&p.name) == base(name)))
        .map(|p| client::color(&p.color, fallback))
        .unwrap_or(fallback)
}

/// Options, the prompt box with Send / Stop, and the import row.
fn composer(app: &mut App, ui: &mut Ui, rect: Rect) {
    let p = ui.painter().clone();
    hline(&p, rect.left(), rect.right(), rect.top(), theme::BORDER);
    let x = rect.left() + SIDE;
    let w = rect.width() - SIDE * 2.0;
    let mut y = rect.top() + 1.0 + 8.0;

    // selection context toggle, with what is selected
    let r = Rect::from_min_size(Pos2::new(x, y), Vec2::new(w, ROW));
    if let Some(v) = check(ui, r, "agents-use-selection", app.agents.use_selection) {
        app.agents.use_selection = v;
    }
    let sel: Vec<String> =
        app.agents.state.as_ref().and_then(|s| s.selection.as_ref()).map(|s| s.nodes.iter().map(|n| n.name.clone()).collect()).unwrap_or_default();
    let sel_text = if sel.is_empty() { "Use selection as context".to_string() } else { format!("Use selection: {}", sel.join(", ")) };
    let shown = widgets::elide(&p, &sel_text, &theme::font_body(), w - 22.0);
    p.text(Pos2::new(x + 22.0, r.center().y), Align2::LEFT_CENTER, shown, theme::font_body(), theme::TEXT);
    widgets::record(ui, "agents-use-selection", r);
    y += ROW + GAP;

    // model
    let models = app.agents.models();
    let labels: Vec<String> = models.iter().map(|m| m.label.clone()).collect();
    let picked = app.agents.model_id();
    let label = models.iter().find(|m| Some(&m.id) == picked.as_ref()).map(|m| m.label.clone()).unwrap_or_else(|| "Model".into());
    let r = Rect::from_min_size(Pos2::new(x, y), Vec2::new(w, ROW));
    if let Some(i) = dropdown(ui, r, "agents-model", &label, &labels) {
        app.agents.model = Some(models[i].id.clone());
    }
    widgets::record(ui, "agents-model", r);
    y += ROW + GAP;

    // parallel agents: count + mode
    let cw = 56.0;
    let counts: Vec<String> = (1..=6).map(|n| format!("{n}x")).collect();
    let r = Rect::from_min_size(Pos2::new(x, y), Vec2::new(cw, ROW));
    if let Some(i) = dropdown(ui, r, "agents-parallel", &counts[(app.agents.parallel_count.clamp(1, 6) - 1) as usize], &counts) {
        app.agents.parallel_count = i as u32 + 1;
    }
    widgets::record(ui, "agents-parallel", r);
    let sr = Rect::from_min_size(Pos2::new(x + cw + GAP, y), Vec2::new(w - cw - GAP, ROW));
    if let Some(i) = segmented(ui, sr, "agents-parallel-mode", &["Split work", "Side by side"], app.agents.parallel as usize) {
        app.agents.parallel = if i == 0 { Parallel::Split } else { Parallel::SideBySide };
    }
    y += ROW + GAP;

    // iterate: variants + mode
    let variants: Vec<String> = (1..=6).map(|n| if n == 1 { "Once".to_string() } else { format!("{n} var.") }).collect();
    let r = Rect::from_min_size(Pos2::new(x, y), Vec2::new(cw, ROW));
    if let Some(i) = dropdown(ui, r, "agents-variants", &variants[(app.agents.variants.clamp(1, 6) - 1) as usize], &variants) {
        app.agents.variants = i as u32 + 1;
    }
    widgets::record(ui, "agents-variants", r);
    let sr = Rect::from_min_size(Pos2::new(x + cw + GAP, y), Vec2::new(w - cw - GAP, ROW));
    if let Some(i) = segmented(ui, sr, "agents-iterate-mode", &["Layout", "Style"], app.agents.iterate as usize) {
        app.agents.iterate = if i == 0 { Iterate::Layout } else { Iterate::Style };
    }
    y += ROW + GAP;

    // prompt: Enter sends, Shift+Enter breaks the line
    let id = Id::new("agents-prompt");
    let focused = ui.memory(|m| m.has_focus(id));
    let enter = focused && ui.ctx().input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter));
    let tb = Rect::from_min_size(Pos2::new(x, y), Vec2::new(w, COMPOSER_H));
    let hint = if app.agents.chat.as_ref().is_some_and(|c| c.thread_id.is_some()) { "Reply in the thread…" } else { "Ask the agents…" };
    text_box(ui, tb, id, &mut app.agents.prompt, hint, false);
    widgets::record(ui, "agents-prompt", tb);
    y += COMPOSER_H + GAP;

    // Stop (while a run is going) … Send
    let send_w = button_width(&p, "Send");
    let sr = Rect::from_min_size(Pos2::new(x + w - send_w, y), Vec2::new(send_w, ROW));
    let can_send = !app.agents.prompt.trim().is_empty() && !app.agents.sending;
    let send = button(ui, sr, "agents-send", "Send", true, can_send).clicked();
    widgets::record(ui, "agents-send", sr);
    if app.agents.run_active() {
        let stop_w = button_width(&p, "Stop");
        let st = Rect::from_min_size(Pos2::new(x, y), Vec2::new(stop_w, ROW));
        if button(ui, st, "agents-stop", "Stop", false, true).clicked() {
            app.agents_stop();
        }
        p.text(Pos2::new(st.right() + 8.0, st.center().y), Align2::LEFT_CENTER, "running…", theme::font_small(), theme::TEXT_TERTIARY);
        widgets::record(ui, "agents-stop", st);
    } else {
        p.text(Pos2::new(x, sr.center().y), Align2::LEFT_CENTER, "Enter to send", theme::font_small(), theme::TEXT_TERTIARY);
    }
    if (send || enter) && can_send {
        app.agents_send();
        ui.memory_mut(|m| m.request_focus(id));
    }
    y += ROW + 8.0;

    // ---- import from web
    hline(&p, rect.left(), rect.right(), y, theme::BORDER);
    y += 1.0 + 8.0;
    let import_w = button_width(&p, "Import");
    let ur = Rect::from_min_size(Pos2::new(x, y), Vec2::new(w - import_w - GAP, ROW));
    if let Some(t) = crate::widgets::field::text_input_ph(ui, ur, "agents-import-url", &app.agents.import_url, "Import from web: https://…") {
        app.agents.import_url = t;
    }
    widgets::record(ui, "agents-import-url", ur);
    let ir = Rect::from_min_size(Pos2::new(x + w - import_w, y), Vec2::new(import_w, ROW));
    if button(ui, ir, "agents-import", "Import", false, !app.agents.importing && !app.agents.import_url.trim().is_empty()).clicked() {
        app.agents_import();
    }
    widgets::record(ui, "agents-import", ir);
    y += ROW + GAP;
    let mut cx = x;
    for (i, width) in IMPORT_WIDTHS.iter().enumerate() {
        let label = width.to_string();
        let cw = 20.0 + widgets::text_width(&p, &label, &theme::font_small());
        let r = Rect::from_min_size(Pos2::new(cx, y), Vec2::new(cw, ROW));
        if let Some(v) = check(ui, r, ("agents-width", i), app.agents.import_widths[i]) {
            app.agents.import_widths[i] = v;
        }
        p.text(Pos2::new(cx + 18.0, r.center().y), Align2::LEFT_CENTER, label, theme::font_small(), theme::TEXT);
        cx += cw + 8.0;
    }
    p.text(Pos2::new(cx, y + ROW / 2.0), Align2::LEFT_CENTER, "widths", theme::font_small(), theme::TEXT_TERTIARY);
}

/// Comment threads as tasks: status dot, text, where it is pinned, Run and Resolve.
fn notes_tab(app: &mut App, ui: &mut Ui, body: Rect) {
    let p = ui.painter().clone();
    let x = body.left() + SIDE;
    let w = body.width() - SIDE * 2.0;
    let font = theme::font_body();
    let footer = Rect::from_min_max(Pos2::new(body.left(), body.bottom() - 40.0), body.max);
    hline(&p, footer.left(), footer.right(), footer.top(), theme::BORDER);
    let cb = Rect::from_min_size(Pos2::new(x, footer.top() + 8.0), Vec2::new(w, ROW));
    if let Some(v) = check(ui, cb, "agents-show-resolved", app.agents.show_resolved) {
        app.agents.show_resolved = v;
    }
    p.text(Pos2::new(x + 22.0, cb.center().y), Align2::LEFT_CENTER, "Show resolved", font.clone(), theme::TEXT);

    let list = Rect::from_min_max(body.min, Pos2::new(body.right(), footer.top()));
    let Some(state) = app.agents.state.clone() else { return };
    let threads: Vec<&Thread> = state.comments.iter().filter(|t| app.agents.show_resolved || !t.resolved).collect();
    if threads.is_empty() {
        let g = p.layout(
            "No notes yet. Write a comment in the file (C) like \"[ ] @claude make the title bolder\" and run it from here.".into(),
            font,
            theme::TEXT_TERTIARY,
            w,
        );
        p.galley(Pos2::new(x, list.top() + 16.0), g, theme::TEXT_TERTIARY);
        return;
    }
    struct Card {
        galley: std::sync::Arc<egui::Galley>,
        meta: String,
        h: f32,
    }
    let cards: Vec<Card> = threads
        .iter()
        .map(|t| {
            let mut job = LayoutJob::simple(t.text.clone(), font.clone(), theme::TEXT, w - 16.0);
            job.wrap.max_rows = 3;
            job.wrap.overflow_character = Some('…');
            let galley = p.layout_job(job);
            let mut parts = vec![t.anchor.node_name.clone().or_else(|| t.anchor.page_name.clone()).unwrap_or_default(), t.author.clone()];
            if let Some(f) = state.flows.iter().find(|f| f.thread_id == t.thread_id) {
                parts.push(format!("● {} working", f.agent));
            }
            if t.messages.len() > 1 {
                parts.push(format!("{} replies", t.messages.len() - 1));
            }
            if t.resolved {
                parts.push("resolved".into());
            }
            let meta = parts.into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join("  ·  ");
            Card { h: 8.0 + galley.size().y + 18.0 + ROW + 8.0, galley, meta }
        })
        .collect();
    let content_h: f32 = cards.iter().map(|c| c.h + 4.0).sum::<f32>() + 16.0;
    wheel_scroll(ui, list, &mut app.agents.notes_scroll, content_h);
    let scroll = app.agents.notes_scroll;
    let mut run: Option<String> = None;
    let mut resolve: Option<(String, bool)> = None;
    clipped(ui, list, |ui| {
        let p = ui.painter().clone();
        let mut cy = list.top() + 8.0 - scroll;
        for (t, c) in threads.iter().zip(&cards) {
            let r = Rect::from_min_size(Pos2::new(x, cy), Vec2::new(w, c.h));
            cy += c.h + 4.0;
            if r.bottom() < list.top() || r.top() > list.bottom() {
                continue;
            }
            p.rect_filled(r, theme::cr(theme::RADIUS_MEDIUM), theme::BG_SECONDARY);
            let dot = match t.status.as_deref() {
                Some("doing") => theme::AVATAR_COLORS[1],
                Some("done") => theme::AVATAR_COLORS[2],
                Some("todo") => theme::TEXT_SECONDARY,
                _ => theme::TEXT_TERTIARY,
            };
            p.circle_filled(Pos2::new(r.left() + 8.0, r.top() + 8.0 + 7.0), 4.0, dot);
            p.galley(Pos2::new(r.left() + 16.0, r.top() + 8.0), c.galley.clone(), theme::TEXT);
            let my = r.top() + 8.0 + c.galley.size().y + 4.0;
            let meta = widgets::elide(&p, &c.meta, &theme::font_small(), w - 16.0);
            p.text(Pos2::new(r.left() + 16.0, my + 7.0), Align2::LEFT_CENTER, meta, theme::font_small(), theme::TEXT_SECONDARY);
            let by = r.bottom() - 8.0 - ROW;
            let working = state.flows.iter().any(|f| f.thread_id == t.thread_id);
            let run_w = button_width(&p, "Run");
            let rr = Rect::from_min_size(Pos2::new(r.left() + 8.0, by), Vec2::new(run_w, ROW));
            if button(ui, rr, ("agents-run", &t.thread_id), "Run", true, !working && !t.resolved).clicked() {
                run = Some(t.thread_id.clone());
            }
            widgets::record(ui, &format!("agents-run-{}", t.text.chars().take(16).collect::<String>()), rr);
            let label = if t.resolved { "Reopen" } else { "Resolve" };
            let rw = button_width(&p, label);
            let xr = Rect::from_min_size(Pos2::new(r.right() - 8.0 - rw, by), Vec2::new(rw, ROW));
            if button(ui, xr, ("agents-resolve", &t.thread_id), label, false, true).clicked() {
                resolve = Some((t.thread_id.clone(), !t.resolved));
            }
        }
    });
    scrollbar(ui, list, scroll, content_h);
    widgets::record(ui, "agents-notes", list);
    if let Some(id) = run {
        app.agents_run_thread(&id);
        app.show_toast("Starting an agent on the note…", false);
    }
    if let Some((id, v)) = resolve {
        app.agents_resolve(&id, v);
    }
}
