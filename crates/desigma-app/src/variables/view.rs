//! The local-variables view. Figma UI3 shows it in two forms that share one layout: a full
//! view that replaces the editor panels (the rail stays), and a floating panel over the editor.
//! Both are a Collections / Groups sidebar next to a top bar, a table (one column per mode)
//! and a bottom bar with "Create variable".

use super::widgets::{
    arrows_glyph, cross_glyph, filter_glyph, from_c32, hexagon, kind_glyph, kind_name, picker, pill, sliders_glyph, to_c32, PickItem, PickResult,
};
use super::{AliasPicker, CellPicker, EditPopover, RowDrag, VarMenu, VarMenuKind};
use crate::app::App;
use crate::icons::{self, Icon};
use crate::panels::clipped;
use crate::theme;
use crate::widgets::field;
use crate::widgets::menu::{popup_menu, MenuItem, MenuResult};
use crate::widgets::{self, fmt_num};
use desigma_doc::*;
use egui::{Align2, Color32, Context, Id, Key, Order, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, Vec2};
use std::collections::HashSet;

/// Where the view is drawn and how its parts are placed (full view or floating panel).
#[derive(Clone, Copy, Debug)]
pub struct Geom {
    pub panel: Rect,
    pub floating: bool,
    pub sidebar: bool,
}

impl Geom {
    /// Sidebar content width (the 1px border follows it in the full view, is inside it when floating).
    pub fn side_inner(&self) -> f32 {
        if self.sidebar {
            theme::VAR_SIDEBAR_W - if self.floating { 1.0 } else { 0.0 }
        } else {
            0.0
        }
    }
    /// Left edge of the main (table) area.
    pub fn main_left(&self) -> f32 {
        self.panel.left() + if self.sidebar { theme::VAR_SIDEBAR_W + if self.floating { 0.0 } else { 1.0 } } else { 0.0 }
    }
    /// Height of the top bar including its 1px bottom border.
    pub fn bar(&self) -> f32 {
        if self.floating {
            theme::VAR_FLOAT_BAR_H
        } else {
            theme::VAR_BAR_H + 1.0
        }
    }
    /// y of the table header's top edge.
    pub fn head_top(&self) -> f32 {
        self.panel.top() + self.bar()
    }
    /// y of the first row's top edge.
    pub fn rows_top(&self) -> f32 {
        self.head_top() + theme::VAR_HEAD_H + 1.0
    }
    /// y of the bottom bar's top border.
    pub fn bottom_line(&self) -> f32 {
        self.panel.bottom() - theme::VAR_BOTTOM_H
    }
    /// The horizontal scrollbar strip.
    pub fn strip(&self) -> Rect {
        let b = self.bottom_line();
        Rect::from_min_max(Pos2::new(self.main_left(), b - theme::VAR_SCROLL_H), Pos2::new(self.panel.right(), b))
    }
    pub fn rows_rect(&self) -> Rect {
        Rect::from_min_max(Pos2::new(self.main_left(), self.rows_top()), Pos2::new(self.panel.right(), self.strip().top()))
    }
    /// Vertical centre of the top bar.
    pub fn bar_cy(&self) -> f32 {
        self.panel.top() + if self.floating { 20.0 } else { 24.0 }
    }
    /// Mode column width (the 1px grid line follows it).
    pub fn mode_w(&self, modes: usize) -> f32 {
        let n = modes.max(1) as f32;
        let avail = self.panel.right() - self.main_left() - (theme::VAR_NAME_COL + 1.0) - (theme::VAR_ADD_COL + 1.0);
        ((avail - (n - 1.0)) / n).clamp(theme::VAR_MODE_MIN, theme::VAR_MODE_MAX)
    }
    /// x of the first mode column.
    pub fn modes_left(&self) -> f32 {
        self.main_left() + theme::VAR_NAME_COL + 1.0
    }
}

pub fn floating_rect(app: &App, screen: Rect) -> Rect {
    let size = Vec2::new(theme::VAR_FLOAT_W.min(screen.width()), theme::VAR_FLOAT_H.min(screen.height()));
    let pos = app.vars.float_pos.unwrap_or(Pos2::new(0.0, theme::VAR_FLOAT_Y));
    let x = pos.x.clamp(screen.left(), (screen.right() - size.x).max(screen.left()));
    let y = pos.y.clamp(screen.top(), (screen.bottom() - size.y).max(screen.top()));
    Rect::from_min_size(Pos2::new(x, y), size)
}

enum SlotKind {
    Var(String),
    Header(String),
}

struct Slot {
    top: f32,
    bottom: f32,
    kind: SlotKind,
}

/// What one frame of the view wants to change.
pub struct Frame<'a> {
    pub view: &'a VariableStore,
    pub work: VariableStore,
    pub removed_vars: Vec<String>,
    pub removed_colls: Vec<String>,
    /// Variable ids in visible row order.
    order: Vec<String>,
    slots: Vec<Slot>,
    group_hits: Vec<(Rect, Option<String>)>,
    pub close: bool,
}

enum Row {
    Header(String, usize),
    Var(String),
}

// ------------------------------------------------------------------ model helpers

fn folder_of(name: &str) -> &str {
    split_name(name).0
}

/// Whether `folder` is `group` or inside it (`None` = All).
fn in_group(folder: &str, group: Option<&str>) -> bool {
    match group {
        None => true,
        Some(g) => folder == g || folder.strip_prefix(g).is_some_and(|r| r.starts_with('/')),
    }
}

/// Group paths of a collection in tree order with their depth.
pub fn group_paths(view: &VariableStore, coll: &VariableCollection) -> Vec<(String, usize)> {
    let mut paths: Vec<String> = Vec::new();
    for id in &coll.variables {
        let Some(v) = view.variables.get(id) else { continue };
        let mut acc = String::new();
        for seg in folder_of(&v.name).split('/').filter(|s| !s.is_empty()) {
            if !acc.is_empty() {
                acc.push('/');
            }
            acc.push_str(seg);
            if !paths.contains(&acc) {
                paths.push(acc.clone());
            }
        }
    }
    fn dfs(parent: &str, depth: usize, paths: &[String], out: &mut Vec<(String, usize)>) {
        for p in paths.iter().filter(|p| folder_of(p) == parent) {
            out.push((p.clone(), depth));
            dfs(p, depth + 1, paths, out);
        }
    }
    let mut out = Vec::new();
    dfs("", 0, &paths, &mut out);
    out
}

fn count_in(view: &VariableStore, coll: &VariableCollection, group: Option<&str>) -> usize {
    coll.variables.iter().filter_map(|id| view.variables.get(id)).filter(|v| in_group(folder_of(&v.name), group)).count()
}

fn kind_hidden(app: &App, k: VariableKind) -> bool {
    app.vars.hidden_kinds.contains(&k)
}

fn folded_key(coll: &str, path: &str) -> String {
    format!("{coll}\0{path}")
}

/// Rows of the table for the selected group: variables grouped under their folder's header.
fn build_rows(app: &App, view: &VariableStore, coll: &VariableCollection) -> Vec<Row> {
    let q = app.vars.search.to_lowercase();
    let group = app.vars.group.as_deref();
    let mut folders: Vec<(String, Vec<String>)> = Vec::new();
    for id in &coll.variables {
        let Some(v) = view.variables.get(id) else { continue };
        let folder = folder_of(&v.name);
        if !in_group(folder, group) || kind_hidden(app, v.kind) || (!q.is_empty() && !v.name.to_lowercase().contains(&q)) {
            continue;
        }
        match folders.iter_mut().find(|(f, _)| f == folder) {
            Some((_, ids)) => ids.push(id.clone()),
            None => folders.push((folder.to_string(), vec![id.clone()])),
        }
    }
    // the selected group's own variables come first, then its sub-groups in tree order
    let base = group.unwrap_or("");
    folders.sort_by_key(|(f, _)| (f != base, 0));
    let mut out = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for (folder, ids) in folders {
        let rel = folder.strip_prefix(base).unwrap_or(&folder).trim_start_matches('/').to_string();
        let mut hidden = false;
        if !rel.is_empty() {
            let mut acc = base.to_string();
            for (depth, seg) in rel.split('/').enumerate() {
                if !acc.is_empty() {
                    acc.push('/');
                }
                acc.push_str(seg);
                if seen.insert(acc.clone()) {
                    out.push(Row::Header(acc.clone(), depth));
                }
                hidden |= app.vars.folded.contains(&folded_key(&coll.id, &acc));
            }
        }
        if !hidden {
            out.extend(ids.into_iter().map(Row::Var));
        }
    }
    out
}

// ------------------------------------------------------------------ entry point

/// Draws the view (when open) and every popup of the variables UI. `content` is the area right
/// of the rail, used by the full view; the floating panel positions itself on the screen.
pub fn show(app: &mut App, ui: &mut Ui, content: Rect) {
    let ctx = ui.ctx().clone();
    if !app.vars.open {
        crate::panels::var_bind::bind_picker(app, &ctx);
        return;
    }
    let screen = ctx.content_rect();
    let floating = !app.vars.full;
    let g = Geom { panel: if floating { floating_rect(app, screen) } else { content }, floating, sidebar: !app.vars.sidebar_hidden };
    let view = VariableStore::from_doc(&app.doc);
    if !app.vars.collection.as_ref().is_some_and(|c| view.collections.contains_key(c)) {
        app.vars.collection = view.collections.keys().next().cloned();
        app.vars.group = None;
    }
    if let (Some(gp), Some(c)) = (app.vars.group.clone(), app.vars.collection.as_ref().and_then(|c| view.collections.get(c))) {
        if !group_paths(&view, c).iter().any(|(p, _)| *p == gp) {
            app.vars.group = None;
        }
    }
    let ids: Vec<String> = view.variables.keys().cloned().collect();
    app.vars.selected.retain(|s| ids.contains(s));
    let mut fr =
        Frame { view: &view, work: view.clone(), removed_vars: vec![], removed_colls: vec![], order: vec![], slots: vec![], group_hits: vec![], close: false };
    let popup_open = app.vars.menu.is_some() || app.vars.alias_picker.is_some() || app.vars.cell_picker.is_some() || app.vars.edit.is_some();
    let typing = ctx.egui_wants_keyboard_input();

    egui::Area::new(Id::new("vars-view")).order(Order::Middle).fixed_pos(g.panel.min).constrain(false).show(&ctx, |ui| {
        ui.set_clip_rect(g.panel.expand(40.0));
        let (_, _) = ui.allocate_exact_size(g.panel.size(), Sense::hover());
        let p = ui.painter().clone();
        if g.floating {
            widgets::elevation_400(&p, g.panel, theme::MENU_RADIUS);
        }
        let radius = if g.floating { theme::cr_each(0, theme::MENU_RADIUS, theme::MENU_RADIUS, 0) } else { theme::cr(0) };
        p.rect_filled(g.panel, radius, theme::BG);
        if g.floating {
            p.rect_stroke(g.panel, radius, Stroke::new(1.0, theme::ELEVATION_400_RING), StrokeKind::Inside);
        }
        // swallow pointer input over the panel; the controls below register after and win
        let body = ui.interact(g.panel, Id::new("vars-body"), Sense::click_and_drag());
        if g.floating && body.dragged() && ui.input(|i| i.pointer.press_origin()).is_some_and(|o| o.y < g.panel.top() + g.bar()) {
            let d = ui.input(|i| i.pointer.delta());
            app.vars.float_pos = Some(g.panel.min + d);
        }
        let p = ui.painter().clone();
        let mut body_ui = ui.new_child(egui::UiBuilder::new().max_rect(g.panel));
        body_ui.set_clip_rect(g.panel);
        let ui = &mut body_ui;
        if g.sidebar {
            sidebar(app, ui, &g, &mut fr);
        }
        main(app, ui, &g, &mut fr);
        let _ = &p;
    });

    run_popups(app, &ctx, &g, &mut fr);
    let (work, rv, rc, close) = (fr.work, fr.removed_vars, fr.removed_colls, fr.close);
    if work != view {
        app.commit_store(work, &rv, &rc);
    }
    if !popup_open && !typing {
        keys(app, &ctx, &view);
    }
    if close {
        app.close_variables();
    }
}

/// Keyboard: Enter edits the selected row's name, Delete removes, Ctrl+D duplicates, Esc clears / closes.
fn keys(app: &mut App, ctx: &Context, view: &VariableStore) {
    let events = ctx.input(|i| i.events.clone());
    for ev in events {
        let egui::Event::Key { key, pressed: true, modifiers, .. } = ev else { continue };
        let sel: Vec<String> = app.vars.selected.iter().filter(|s| view.variables.contains_key(*s)).cloned().collect();
        match key {
            Key::Escape => {
                if !sel.is_empty() {
                    app.vars.selected.clear();
                } else {
                    app.close_variables();
                    return;
                }
            }
            Key::Enter if sel.len() == 1 && !modifiers.command => {
                app.vars.renaming = Some(('v', sel[0].clone()));
                app.vars.rename_frames = 0;
            }
            Key::Delete | Key::Backspace if !sel.is_empty() => {
                app.edit_variables(|s| {
                    for id in &sel {
                        s.delete_variable(id);
                    }
                    (sel.clone(), vec![])
                });
                app.vars.selected.clear();
            }
            Key::D if modifiers.command && !sel.is_empty() => {
                let mut copies = vec![];
                app.edit_variables(|s| {
                    for id in &sel {
                        if let Some(n) = s.duplicate_variable(id) {
                            copies.push(n);
                        }
                    }
                    (vec![], vec![])
                });
                app.vars.selected = copies;
            }
            Key::Z if modifiers.command && !modifiers.shift => {
                app.undo();
            }
            Key::Z | Key::Y if modifiers.command => {
                app.redo();
            }
            _ => {}
        }
    }
}

// ------------------------------------------------------------------ small widgets

fn swatch(p: &egui::Painter, rect: Rect, color: Color32) {
    let r = theme::cr((rect.width() * 0.2).round() as u8);
    if color.a() < 255 {
        p.rect_filled(rect, r, theme::CHECKER_A);
        let h = rect.height() / 2.0;
        p.rect_filled(Rect::from_min_size(rect.min, Vec2::splat(h)), 0.0, theme::CHECKER_B);
        p.rect_filled(Rect::from_min_size(rect.min + Vec2::splat(h), Vec2::splat(h)), 0.0, theme::CHECKER_B);
    }
    p.rect_filled(rect, r, color);
    p.rect_stroke(rect, r, Stroke::new(1.0, theme::BORDER_TRANSLUCENT), StrokeKind::Inside);
}

fn text_button(ui: &mut Ui, rect: Rect, id: Id, hovered_bg: bool) -> egui::Response {
    let r = ui.interact(rect, id, Sense::click());
    if hovered_bg && (r.hovered() || r.is_pointer_button_down_on()) {
        ui.painter().rect_filled(rect, theme::cr(theme::RADIUS_MEDIUM), if r.is_pointer_button_down_on() { theme::BG_PRESSED } else { theme::BG_HOVER });
    }
    r
}

fn glyph_button(ui: &mut Ui, c: Pos2, name: &str, draw: impl FnOnce(&egui::Painter, Pos2, Color32), tip: &str) -> egui::Response {
    let rect = Rect::from_center_size(c, Vec2::splat(theme::PANEL_ACTION_BUTTON));
    let r = text_button(ui, rect, Id::new(("vars-btn", name)), true);
    draw(ui.painter(), c, theme::ICON);
    widgets::record(ui, name, rect);
    if tip.is_empty() {
        r
    } else {
        r.on_hover_text(tip)
    }
}

/// Inline rename field; returns the committed text.
fn rename_field(app: &mut App, ui: &mut Ui, rect: Rect, eid: Id, cur: &str, font: egui::FontId, pad: f32) -> Option<String> {
    if app.vars.rename_frames == 0 {
        ui.ctx().memory_mut(|m| m.request_focus(eid));
    }
    app.vars.rename_frames += 1;
    ui.painter().rect_filled(rect, theme::cr(theme::RADIUS_MEDIUM), theme::BG_SECONDARY);
    ui.painter().rect_stroke(rect, theme::cr(theme::RADIUS_MEDIUM), Stroke::new(1.0, theme::BORDER_SELECTED), StrokeKind::Inside);
    let r = field::edit_text(ui, rect, eid, cur, font, theme::TEXT, pad);
    if r.is_some() || (app.vars.rename_frames > 2 && !ui.memory(|m| m.has_focus(eid))) {
        app.vars.renaming = None;
    }
    r
}

fn cell_chrome(ui: &Ui, rect: Rect, id: Id) {
    let focused = ui.memory(|m| m.has_focus(id));
    let hovered = ui.rect_contains_pointer(rect);
    let p = ui.painter();
    if focused {
        p.rect_filled(rect, theme::cr(theme::RADIUS_MEDIUM), theme::BG_SECONDARY);
        p.rect_stroke(rect, theme::cr(theme::RADIUS_MEDIUM), Stroke::new(1.0, theme::BORDER_SELECTED), StrokeKind::Inside);
    } else if hovered {
        p.rect_filled(rect, theme::cr(theme::RADIUS_MEDIUM), theme::BG_SECONDARY);
    }
}

// ------------------------------------------------------------------ sidebar

fn sidebar(app: &mut App, ui: &mut Ui, g: &Geom, fr: &mut Frame) {
    let p = ui.painter().clone();
    let left = g.panel.left();
    let top = g.panel.top();
    let inner_w = g.side_inner();
    let border_x = left + if g.floating { theme::VAR_SIDEBAR_W - 1.0 } else { theme::VAR_SIDEBAR_W };
    widgets::vline(&p, border_x, top, g.panel.bottom(), theme::BORDER);
    let base = g.head_top();
    widgets::hline(&p, left, left + inner_w, base - 1.0, theme::BORDER);

    // ---- header: file name (full view) or "Variables" (floating panel), sidebar toggle
    let cy = g.bar_cy();
    if g.floating {
        p.text(Pos2::new(left + 14.0, cy), Align2::LEFT_CENTER, "Variables", theme::font_body_strong(), theme::TEXT);
    } else {
        let name = if app.doc.name.is_empty() { "Untitled".to_string() } else { app.doc.name.clone() };
        let shown = widgets::elide(&p, &name, &theme::font_large_strong(), inner_w - 16.0 - 49.0);
        p.text(Pos2::new(left + 16.0, cy), Align2::LEFT_CENTER, shown, theme::font_large_strong(), theme::TEXT);
    }
    let tg = Pos2::new(left + inner_w - if g.floating { 20.0 } else { 24.0 }, cy);
    let r = glyph_button(ui, tg, "vars-sidebar-toggle", |p, c, col| icons::paint(p, Icon::Sidebar, c, 16.0, col), "Hide sidebar");
    if r.clicked() {
        app.vars.sidebar_hidden = true;
    }

    // ---- collections
    let ty = base + 20.0;
    p.text(Pos2::new(left + 15.0, ty), Align2::LEFT_CENTER, "Collections", theme::font_body_semibold(), theme::TEXT);
    let plus = Pos2::new(left + inner_w - 20.0, ty);
    let slid = Pos2::new(left + inner_w - 47.0, ty);
    let r = glyph_button(ui, slid, "vars-settings", sliders_glyph, "Collection settings");
    if r.clicked() {
        let rect = Rect::from_center_size(slid, Vec2::splat(24.0));
        app.vars.menu = Some(VarMenu { kind: VarMenuKind::Settings, pos: rect.left_bottom() + Vec2::new(0.0, 4.0), anchor: rect });
    }
    let r = glyph_button(ui, plus, "vars-add-collection", |p, c, col| icons::paint(p, Icon::Plus, c, 16.0, col), "Create collection");
    if r.clicked() {
        let n = fr.work.collections.len() + 1;
        let id = fr.work.add_collection(&format!("Collection {n}"));
        app.vars.collection = Some(id.clone());
        app.vars.group = None;
        app.vars.renaming = Some(('c', id));
        app.vars.rename_frames = 0;
        app.vars.scroll = 0.0;
    }
    let mut y = base + 44.0;
    let ids: Vec<String> = fr.view.collections.keys().cloned().collect();
    for id in &ids {
        let c = &fr.view.collections[id];
        let inset = if g.floating { 8.0 } else { 10.0 };
        let row = Rect::from_min_size(Pos2::new(left + inset, y), Vec2::new(224.0, theme::VAR_PILL_H));
        y += theme::VAR_SIDE_ROW;
        widgets::record(ui, &format!("vars-coll:{}", c.name), row);
        let resp = ui.interact(row, Id::new(("vars-coll", id)), Sense::click());
        let selected = app.vars.collection.as_deref() == Some(id.as_str());
        if selected {
            p.rect_filled(row, theme::cr(theme::RADIUS_MEDIUM), theme::BG_SECONDARY);
        } else if resp.hovered() {
            p.rect_filled(row, theme::cr(theme::RADIUS_MEDIUM), theme::BG_HOVER);
        }
        let name_rect = Rect::from_min_max(Pos2::new(left + 15.0, row.top()), Pos2::new(left + 190.0, row.bottom()));
        if app.vars.renaming == Some(('c', id.clone())) {
            let eid = Id::new(("vars-coll-name", id));
            if let Some(s) = rename_field(app, ui, name_rect.expand2(Vec2::new(4.0, 0.0)), eid, &c.name, theme::font_body_strong(), 4.0) {
                if !s.trim().is_empty() {
                    fr.work.rename_collection(id, s.trim());
                }
            }
        } else {
            let nm = widgets::elide(&p, &c.name, &theme::font_body_semibold(), name_rect.width());
            p.text(Pos2::new(name_rect.left(), row.center().y), Align2::LEFT_CENTER, nm, theme::font_body_semibold(), theme::TEXT);
            p.text(Pos2::new(left + 225.0, row.center().y), Align2::RIGHT_CENTER, c.variables.len().to_string(), theme::font_body(), theme::TEXT_SECONDARY);
        }
        if resp.clicked() && !selected {
            app.vars.collection = Some(id.clone());
            app.vars.group = None;
            app.vars.scroll = 0.0;
            app.vars.scroll_x = 0.0;
            app.vars.selected.clear();
        }
        if resp.double_clicked() {
            app.vars.renaming = Some(('c', id.clone()));
            app.vars.rename_frames = 0;
        }
        if resp.secondary_clicked() {
            let pos = ui.input(|i| i.pointer.interact_pos()).unwrap_or(row.left_bottom());
            app.vars.menu = Some(VarMenu { kind: VarMenuKind::Collection(id.clone()), pos, anchor: Rect::from_min_size(pos, Vec2::ZERO) });
        }
    }
    if ids.is_empty() {
        p.text(Pos2::new(left + 15.0, y + 12.0), Align2::LEFT_CENTER, "No collections yet", theme::font_body(), theme::TEXT_TERTIARY);
    }

    // ---- groups
    let sep_y = base + 48.0 + ids.len().max(1) as f32 * theme::VAR_SIDE_ROW;
    widgets::hline(&p, left, left + inner_w, sep_y, theme::BORDER);
    let Some(cid) = app.vars.collection.clone() else { return };
    let Some(coll) = fr.view.collections.get(&cid) else { return };
    p.text(Pos2::new(left + 16.0, sep_y + 21.0), Align2::LEFT_CENTER, "Groups", theme::font_body_semibold(), theme::TEXT);
    let mut y = sep_y + 45.0;
    let mut entries: Vec<(Option<String>, usize, bool)> = vec![(None, 0, false)];
    let paths = group_paths(fr.view, coll);
    for (i, (path, depth)) in paths.iter().enumerate() {
        let has_kids = paths.get(i + 1).is_some_and(|(_, d)| d > depth);
        entries.push((Some(path.clone()), depth + 1, has_kids));
    }
    let area = Rect::from_min_max(Pos2::new(left, y), Pos2::new(left + inner_w, g.panel.bottom()));
    clipped(ui, area, |ui| {
        let p = ui.painter().clone();
        let mut hidden_below: Option<usize> = None;
        for (path, depth, has_kids) in entries {
            if let Some(d) = hidden_below {
                if depth > d {
                    continue;
                }
                hidden_below = None;
            }
            let row = Rect::from_min_size(Pos2::new(left + 8.0, y), Vec2::new(224.0, theme::VAR_PILL_H));
            y += theme::VAR_SIDE_ROW;
            let key = path.clone().unwrap_or_default();
            let fkey = format!("sg\0{}", folded_key(&cid, &key));
            let folded = app.vars.folded.contains(&fkey);
            if folded {
                hidden_below = Some(depth);
            }
            let label = match &path {
                None => "All".to_string(),
                Some(pth) => split_name(pth).1.to_string(),
            };
            let resp = ui.interact(row, Id::new(("vars-group", &cid, &key)), Sense::click());
            widgets::record(ui, &format!("vars-group:{}", path.as_deref().unwrap_or("All")), row);
            fr.group_hits.push((row, path.clone()));
            let selected = app.vars.group == path;
            let drop_hot = app.vars.drag.is_some() && ui.rect_contains_pointer(row);
            if selected {
                p.rect_filled(row, theme::cr(theme::RADIUS_MEDIUM), theme::BG_SELECTED);
            } else if resp.hovered() || drop_hot {
                p.rect_filled(row, theme::cr(theme::RADIUS_MEDIUM), theme::BG_HOVER);
            }
            if drop_hot {
                p.rect_stroke(row, theme::cr(theme::RADIUS_MEDIUM), Stroke::new(1.0, theme::BORDER_SELECTED), StrokeKind::Inside);
            }
            let tx = row.left() + 8.0 + depth as f32 * 16.0;
            if has_kids {
                icons::paint(&p, if folded { Icon::ChevronRight } else { Icon::ChevronDown }, Pos2::new(tx - 8.0, row.center().y), 12.0, theme::ICON_TERTIARY);
            }
            let w = (row.right() - 40.0 - tx).max(20.0);
            let shown = widgets::elide(&p, &label, &theme::font_body_semibold(), w);
            p.text(Pos2::new(tx, row.center().y), Align2::LEFT_CENTER, shown, theme::font_body_semibold(), theme::TEXT);
            let n = count_in(fr.view, coll, path.as_deref());
            p.text(Pos2::new(left + 225.0, row.center().y), Align2::RIGHT_CENTER, n.to_string(), theme::font_body(), theme::TEXT_SECONDARY);
            if resp.clicked() {
                let chev = Rect::from_center_size(Pos2::new(tx - 8.0, row.center().y), Vec2::splat(16.0));
                let pos = ui.input(|i| i.pointer.interact_pos());
                if has_kids && pos.is_some_and(|p| chev.contains(p)) {
                    if folded {
                        app.vars.folded.remove(&fkey);
                    } else {
                        app.vars.folded.insert(fkey);
                    }
                } else {
                    app.vars.group = path.clone();
                    app.vars.scroll = 0.0;
                }
            }
            if resp.secondary_clicked() && path.is_some() {
                let pos = ui.input(|i| i.pointer.interact_pos()).unwrap_or(row.left_bottom());
                app.vars.menu = Some(VarMenu { kind: VarMenuKind::Folder(cid.clone(), key.clone()), pos, anchor: Rect::from_min_size(pos, Vec2::ZERO) });
            }
        }
    });
}

// ------------------------------------------------------------------ main area

fn main(app: &mut App, ui: &mut Ui, g: &Geom, fr: &mut Frame) {
    let p = ui.painter().clone();
    let x0 = g.main_left();
    let right = g.panel.right();
    let cy = g.bar_cy();
    let bar_line = g.head_top() - 1.0;
    widgets::hline(&p, x0, right, bar_line, theme::BORDER);

    // ---- top bar controls (right to left)
    let mut search_left;
    if g.floating {
        let x = glyph_button(ui, Pos2::new(right - 20.0, cy), "vars-close", |p, c, col| cross_glyph(p, c, 4.5, col), "Close");
        if x.clicked() {
            fr.close = true;
        }
        let e = glyph_button(ui, Pos2::new(right - 48.0, cy), "vars-expand", |p, c, col| arrows_glyph(p, c, col, false), "Expand to full view");
        if e.clicked() {
            app.vars.full = true;
        }
        search_left = right - 268.0;
    } else {
        let e = glyph_button(ui, Pos2::new(right - 90.0, cy), "vars-collapse", |p, c, col| arrows_glyph(p, c, col, true), "Collapse to a floating panel");
        if e.clicked() {
            app.vars.full = false;
        }
        search_left = right - 313.0;
    }
    if !g.sidebar {
        // show the sidebar toggle in the bar when the sidebar is hidden
        let r = glyph_button(ui, Pos2::new(x0 + 20.0, cy), "vars-sidebar-show", |p, c, col| icons::paint(p, Icon::Sidebar, c, 16.0, col), "Show sidebar");
        if r.clicked() {
            app.vars.sidebar_hidden = false;
        }
    }
    search_left = search_left.max(x0 + 100.0);
    let search = Rect::from_min_size(Pos2::new(search_left, cy - 12.0), Vec2::new(theme::VAR_SEARCH_W, 24.0));
    let sr = theme::cr_each(theme::RADIUS_MEDIUM, 0, 0, theme::RADIUS_MEDIUM);
    p.rect_filled(search, sr, theme::BG_SECONDARY);
    icons::paint(&p, Icon::Search, Pos2::new(search.left() + 11.0, cy), 16.0, theme::ICON_SECONDARY);
    let te = egui::TextEdit::singleline(&mut app.vars.search)
        .id(Id::new("vars-search"))
        .hint_text("Search")
        .font(theme::font_body())
        .text_color(theme::TEXT)
        .frame(egui::Frame::NONE)
        .margin(egui::Margin::symmetric(4, 0))
        .vertical_align(egui::Align::Center);
    ui.put(Rect::from_min_max(search.min + Vec2::new(24.0, -1.0), search.max + Vec2::new(0.0, -1.0)), te);
    widgets::record(ui, "vars-search", search);
    let filt = Rect::from_min_size(search.right_top() + Vec2::new(1.0, 0.0), Vec2::new(theme::VAR_FILTER_W - 1.0, 24.0));
    let fr_resp = ui.interact(filt, Id::new("vars-filter"), Sense::click());
    p.rect_filled(
        filt,
        theme::cr_each(0, theme::RADIUS_MEDIUM, theme::RADIUS_MEDIUM, 0),
        if fr_resp.hovered() { theme::BG_TERTIARY } else { theme::BG_SECONDARY },
    );
    let active = !app.vars.hidden_kinds.is_empty();
    filter_glyph(&p, filt.center(), if active { theme::ICON_SELECTED } else { theme::ICON_SECONDARY });
    widgets::record(ui, "vars-filter", filt);
    if fr_resp.clicked() {
        app.vars.menu = Some(VarMenu { kind: VarMenuKind::Filter, pos: filt.left_bottom() + Vec2::new(0.0, 4.0), anchor: filt });
    }

    // ---- title (full view): collection name, double-click renames
    let Some(cid) = app.vars.collection.clone() else {
        let r = Rect::from_min_max(Pos2::new(x0, g.rows_top()), g.panel.max);
        p.text(r.center() - Vec2::new(0.0, 16.0), Align2::CENTER_CENTER, "No variables yet", theme::font_large_strong(), theme::TEXT);
        p.text(
            r.center() + Vec2::new(0.0, 8.0),
            Align2::CENTER_CENTER,
            "Create a collection to start defining variables.",
            theme::font_body(),
            theme::TEXT_SECONDARY,
        );
        return;
    };
    let Some(coll) = fr.view.collections.get(&cid).cloned() else { return };
    if !g.floating {
        let tx = x0 + 20.0;
        let title_rect = Rect::from_min_max(Pos2::new(tx - 4.0, cy - 12.0), Pos2::new((search_left - 16.0).max(tx + 40.0), cy + 12.0));
        if app.vars.renaming == Some(('t', cid.clone())) {
            let eid = Id::new(("vars-title", &cid));
            if let Some(s) = rename_field(app, ui, title_rect, eid, &coll.name, theme::font_large(), 4.0) {
                if !s.trim().is_empty() {
                    fr.work.rename_collection(&cid, s.trim());
                }
            }
        } else {
            let shown = widgets::elide(&p, &coll.name, &theme::font_large(), title_rect.width() - 8.0);
            p.text(Pos2::new(tx, cy), Align2::LEFT_CENTER, shown, theme::font_large(), theme::TEXT);
            let r = ui.interact(title_rect, Id::new(("vars-title-hit", &cid)), Sense::click());
            widgets::record(ui, "vars-title", title_rect);
            if r.double_clicked() {
                app.vars.renaming = Some(('t', cid.clone()));
                app.vars.rename_frames = 0;
            }
        }
    }

    table(app, ui, g, fr, &coll);
    bottom_bar(app, ui, g, &coll, fr);
}

fn bottom_bar(app: &mut App, ui: &mut Ui, g: &Geom, coll: &VariableCollection, fr: &mut Frame) {
    let p = ui.painter().clone();
    let x0 = g.main_left();
    widgets::hline(&p, x0, g.panel.right(), g.bottom_line(), theme::BORDER);
    let w = 34.0 + widgets::text_width(&p, "Create variable", &theme::font_body()) + 8.0;
    let b = Rect::from_min_size(Pos2::new(x0 + 8.0, g.panel.bottom() - 8.0 - theme::VAR_FIELD_H), Vec2::new(w, theme::VAR_FIELD_H));
    let open = app.vars.menu.as_ref().is_some_and(|m| m.kind == VarMenuKind::Create);
    let r = ui.interact(b, Id::new("vars-create"), Sense::click());
    if open || r.hovered() {
        p.rect_filled(b, theme::cr(theme::RADIUS_MEDIUM), if r.is_pointer_button_down_on() || open { theme::BG_PRESSED } else { theme::BG_HOVER });
    }
    icons::paint(&p, Icon::Plus, Pos2::new(b.left() + 14.0, b.center().y), 16.0, theme::ICON);
    p.text(Pos2::new(b.left() + 34.0, b.center().y), Align2::LEFT_CENTER, "Create variable", theme::font_body(), theme::TEXT);
    widgets::record(ui, "vars-create", b);
    if r.clicked() {
        let h = theme::MENU_PADDING * 2.0 + 4.0 * theme::MENU_ITEM_HEIGHT;
        app.vars.menu = Some(VarMenu { kind: VarMenuKind::Create, pos: Pos2::new(b.left(), b.top() - h - 4.0), anchor: b });
    }
    let _ = (coll, fr);
}

// ------------------------------------------------------------------ table

fn table(app: &mut App, ui: &mut Ui, g: &Geom, fr: &mut Frame, coll: &VariableCollection) {
    let p = ui.painter().clone();
    let x0 = g.main_left();
    let right = g.panel.right();
    let n = coll.modes.len().max(1);
    let mw = g.mode_w(n);
    let modes_left = g.modes_left();
    let hy = g.head_top();
    let rows_rect = g.rows_rect();
    let strip = g.strip();

    // ---- horizontal scroll (mode columns only; the name column is sticky)
    let drawn_w = (theme::VAR_NAME_COL + 1.0) + n as f32 * (mw + 1.0) + theme::VAR_ADD_COL;
    let max_sx = (drawn_w - (right - x0)).max(0.0);
    let hscroll_area = Rect::from_min_max(Pos2::new(modes_left, hy), Pos2::new(right, strip.bottom()));
    if ui.rect_contains_pointer(hscroll_area) {
        let (dx, shift) = ui.input(|i| (i.smooth_scroll_delta.x, i.modifiers.shift));
        let dy = if shift { ui.input(|i| i.smooth_scroll_delta.y) } else { 0.0 };
        app.vars.scroll_x -= dx + dy;
    }
    app.vars.scroll_x = app.vars.scroll_x.clamp(0.0, max_sx);
    let sx = app.vars.scroll_x;
    let col_left = |i: usize| modes_left - sx + i as f32 * (mw + 1.0);

    // ---- rows
    let rows = build_rows(app, fr.view, coll);
    fr.order = rows.iter().filter_map(|r| if let Row::Var(id) = r { Some(id.clone()) } else { None }).collect();
    let content_h = rows.len() as f32 * theme::VAR_ROW_PITCH;
    let mut scroll = app.vars.scroll;
    crate::panels::wheel_scroll(ui, rows_rect, &mut scroll, content_h);
    app.vars.scroll = scroll;

    // header background and rows are clipped to their areas
    let ptr = ui.input(|i| i.pointer.hover_pos());
    let over_popup = widgets::pointer_over_popup(ui.ctx());
    clipped(ui, rows_rect, |ui| {
        let p = ui.painter().clone();
        let mut y = rows_rect.top() - scroll;
        for row in &rows {
            let r = Rect::from_min_size(Pos2::new(x0, y), Vec2::new(right - x0, theme::VAR_ROW_H));
            let slot_bottom = y + theme::VAR_ROW_PITCH;
            y = slot_bottom;
            if slot_bottom < rows_rect.top() || r.top() > rows_rect.bottom() {
                // keep drag targets consistent even off-screen
                continue;
            }
            match row {
                Row::Header(path, depth) => {
                    fr.slots.push(Slot { top: r.top(), bottom: slot_bottom, kind: SlotKind::Header(path.clone()) });
                    header_row(app, ui, g, r, &coll.id, path, *depth, fr);
                }
                Row::Var(id) => {
                    fr.slots.push(Slot { top: r.top(), bottom: slot_bottom, kind: SlotKind::Var(id.clone()) });
                    let Some(v) = fr.view.variables.get(id) else { continue };
                    let hot = ptr.is_some_and(|pp| r.contains(pp)) && !over_popup;
                    variable_row(app, ui, g, r, coll, v, hot, mw, sx, fr);
                }
            }
            widgets::hline(&p, x0, right, r.bottom(), theme::BORDER);
        }
        if rows.is_empty() {
            let msg = if app.vars.search.is_empty() { "No variables in this group" } else { "No matching variables" };
            p.text(Pos2::new(x0 + 20.0, rows_rect.top() + 20.0), Align2::LEFT_CENTER, msg, theme::font_body(), theme::TEXT_TERTIARY);
        }
        drag_overlay(app, ui, g, fr, rows_rect);
    });

    // ---- grid: vertical lines through header and rows
    let v_top = hy;
    let v_bot = strip.top().min(g.rows_top() + rows.len() as f32 * theme::VAR_ROW_PITCH - scroll).max(hy + theme::VAR_HEAD_H + 1.0);
    widgets::vline(&p, x0 + theme::VAR_NAME_COL, v_top, v_bot, theme::BORDER);
    clipped(ui, Rect::from_min_max(Pos2::new(modes_left, hy), Pos2::new(right, v_bot)), |ui| {
        let p = ui.painter().clone();
        for i in 0..n {
            widgets::vline(&p, col_left(i) + mw, v_top, v_bot, theme::BORDER);
        }
    });

    // ---- header row
    p.rect_filled(Rect::from_min_size(Pos2::new(x0, hy), Vec2::new(right - x0, theme::VAR_HEAD_H)), 0.0, theme::BG);
    widgets::hline(&p, x0, right, hy + theme::VAR_HEAD_H, theme::BORDER);
    widgets::vline(&p, x0 + theme::VAR_NAME_COL, hy, hy + theme::VAR_HEAD_H, theme::BORDER);
    let hcy = hy + theme::VAR_HEAD_H / 2.0;
    p.text(Pos2::new(x0 + 16.0, hcy), Align2::LEFT_CENTER, "Name", theme::font_body_semibold(), theme::TEXT);
    clipped(ui, Rect::from_min_max(Pos2::new(modes_left, hy), Pos2::new(right, hy + theme::VAR_HEAD_H + 1.0)), |ui| {
        let p = ui.painter().clone();
        for (i, mode) in coll.modes.iter().enumerate() {
            let cl = col_left(i);
            widgets::vline(&p, cl + mw, hy, hy + theme::VAR_HEAD_H, theme::BORDER);
            let cell = Rect::from_min_size(Pos2::new(cl, hy), Vec2::new(mw, theme::VAR_HEAD_H));
            let label = if coll.modes.len() == 1 && mode.name == "Mode 1" { "Value".to_string() } else { mode.name.clone() };
            let name_r = Rect::from_min_max(Pos2::new(cl + 8.0, hy + 8.0), Pos2::new(cl + mw - 8.0, hy + 32.0));
            let hit = ui.interact(cell, Id::new(("vars-mode-hit", &coll.id, &mode.id)), Sense::click());
            widgets::record(ui, &format!("vars-mode:{}", label), name_r);
            if app.vars.renaming == Some(('m', mode.id.clone())) {
                let eid = Id::new(("vars-mode-name", &coll.id, &mode.id));
                if let Some(s) = rename_field(app, ui, name_r, eid, &mode.name, theme::font_body_strong(), 8.0) {
                    if !s.trim().is_empty() {
                        fr.work.rename_mode(&coll.id, &mode.id, s.trim());
                    }
                }
            } else {
                let shown = widgets::elide(&p, &label, &theme::font_body_semibold(), mw - 32.0);
                p.text(Pos2::new(cl + 16.0, hcy), Align2::LEFT_CENTER, shown, theme::font_body_semibold(), theme::TEXT);
                if coll.default_mode == mode.id && coll.modes.len() > 1 {
                    let w = widgets::text_width(&p, &label, &theme::font_body_strong());
                    if w + 80.0 < mw {
                        p.text(Pos2::new(cl + 16.0 + w + 6.0, hcy), Align2::LEFT_CENTER, "default", theme::font_small(), theme::TEXT_TERTIARY);
                    }
                }
            }
            if hit.double_clicked() {
                app.vars.renaming = Some(('m', mode.id.clone()));
                app.vars.rename_frames = 0;
            }
            if hit.secondary_clicked() {
                let pos = ui.input(|i| i.pointer.interact_pos()).unwrap_or(cell.left_bottom());
                app.vars.menu = Some(VarMenu { kind: VarMenuKind::Mode(coll.id.clone(), mode.id.clone()), pos, anchor: Rect::from_min_size(pos, Vec2::ZERO) });
            }
        }
        // add mode column
        let al = col_left(n);
        let ab = Rect::from_min_size(Pos2::new(al, hy), Vec2::new(theme::VAR_ADD_COL, theme::VAR_HEAD_H));
        let r = ui.interact(ab, Id::new("vars-add-mode"), Sense::click());
        widgets::record(ui, "vars-add-mode", ab);
        icons::paint(&p, Icon::Plus, ab.center(), 16.0, theme::ICON);
        if r.clicked() {
            fr.work.add_mode(&coll.id, &format!("Mode {}", coll.modes.len() + 1));
        }
        r.on_hover_text("Add mode");
    });

    // ---- scrollbar strip (vertical thumb lives at the right edge of the rows)
    crate::panels::scrollbar(ui, rows_rect, scroll, content_h);
    hscrollbar(ui, strip, sx, drawn_w, n, right - x0, g.floating);
    let _ = p;
}

fn hscrollbar(ui: &mut Ui, strip: Rect, sx: f32, drawn_w: f32, modes: usize, view_w: f32, floating: bool) {
    let p = ui.painter();
    p.rect_filled(strip, 0.0, theme::BG);
    // nominal content width: Figma's 280px mode columns plus a trailing gutter
    let nominal = (theme::VAR_NAME_COL + 1.0) + modes as f32 * (theme::VAR_MODE_MAX + 1.0) + theme::VAR_ADD_COL + if floating { 0.0 } else { 40.0 };
    let content = nominal.max(drawn_w);
    let track = Rect::from_min_max(strip.min + Vec2::new(10.0, 3.0), Pos2::new(strip.right() - 10.0, strip.bottom() - 1.0));
    let ratio = (view_w / content).clamp(0.1, 1.0);
    let thumb_w = track.width() * ratio;
    let max_sx = (drawn_w - view_w).max(0.0);
    let t = if max_sx > 0.0 { sx / max_sx } else { 0.0 };
    let tl = track.left() + (track.width() - thumb_w) * t;
    p.rect_filled(Rect::from_min_size(Pos2::new(tl, track.top()), Vec2::new(thumb_w, track.height())), theme::cr(3), theme::VAR_SCROLL_THUMB);
    for (x, dir) in [(strip.left() + 3.5, -1.0f32), (strip.right() - 3.5, 1.0)] {
        let c = Pos2::new(x, strip.center().y + 1.0);
        let pts = vec![c + Vec2::new(dir * 2.0, 0.0), c + Vec2::new(-dir * 2.0, -3.0), c + Vec2::new(-dir * 2.0, 3.0)];
        p.add(egui::Shape::convex_polygon(pts, theme::TEXT_TERTIARY, Stroke::NONE));
    }
}

#[allow(clippy::too_many_arguments)]
fn header_row(app: &mut App, ui: &mut Ui, g: &Geom, r: Rect, coll: &str, path: &str, depth: usize, fr: &mut Frame) {
    let p = ui.painter().clone();
    let name_cell = Rect::from_min_size(r.min, Vec2::new(theme::VAR_NAME_COL, r.height()));
    let key = folded_key(coll, path);
    let resp = ui.interact(Rect::from_min_max(r.min, Pos2::new(r.right(), r.bottom())), Id::new(("vars-header", &key)), Sense::click());
    if resp.hovered() {
        p.rect_filled(r, 0.0, theme::BG_HOVER);
    }
    let open = !app.vars.folded.contains(&key);
    let x = r.left() + 16.0 + depth as f32 * 16.0;
    icons::paint(&p, if open { Icon::ChevronDown } else { Icon::ChevronRight }, Pos2::new(x + 4.0, r.center().y), 12.0, theme::ICON_SECONDARY);
    let label = split_name(path).1;
    let renaming = app.vars.renaming == Some(('f', key.clone()));
    let text_r = Rect::from_min_max(Pos2::new(x + 20.0, r.top() + 8.0), Pos2::new(name_cell.right() - 8.0, r.bottom() - 8.0));
    widgets::record(ui, &format!("vars-header:{path}"), text_r);
    if renaming {
        let eid = Id::new(("vars-folder-name", &key));
        if let Some(s) = rename_field(app, ui, text_r, eid, label, theme::font_body_strong(), 6.0) {
            let (parent, _) = split_name(path);
            let new = if parent.is_empty() { s.trim().to_string() } else { format!("{parent}/{}", s.trim()) };
            fr.work.rename_folder(coll, path, &new);
        }
    } else {
        let shown = widgets::elide(&p, label, &theme::font_body_strong(), text_r.width());
        p.text(Pos2::new(text_r.left(), r.center().y), Align2::LEFT_CENTER, shown, theme::font_body_strong(), theme::TEXT);
    }
    if resp.clicked() {
        if open {
            app.vars.folded.insert(key.clone());
        } else {
            app.vars.folded.remove(&key);
        }
    }
    if resp.double_clicked() {
        app.vars.folded.remove(&key);
        app.vars.renaming = Some(('f', key.clone()));
        app.vars.rename_frames = 0;
    }
    if resp.secondary_clicked() {
        let pos = ui.input(|i| i.pointer.interact_pos()).unwrap_or(r.left_bottom());
        app.vars.menu = Some(VarMenu { kind: VarMenuKind::Folder(coll.to_string(), path.to_string()), pos, anchor: Rect::from_min_size(pos, Vec2::ZERO) });
    }
    let _ = g;
}

#[allow(clippy::too_many_arguments)]
fn variable_row(app: &mut App, ui: &mut Ui, g: &Geom, r: Rect, coll: &VariableCollection, v: &Variable, hot: bool, mw: f32, sx: f32, fr: &mut Frame) {
    let p = ui.painter().clone();
    let selected = app.vars.selected.contains(&v.id);
    let dragging = app.vars.drag.as_ref().is_some_and(|d| d.var == v.id);
    if selected {
        p.rect_filled(r, 0.0, theme::BG_SELECTED);
    } else if hot || dragging {
        p.rect_filled(r, 0.0, theme::BG_HOVER);
    }
    let x0 = g.main_left();
    let modes_left = g.modes_left();

    // ---- value cells (scroll horizontally under the sticky name cell)
    let area = Rect::from_min_max(Pos2::new(modes_left, r.top()), r.max);
    clipped(ui, area, |ui| {
        for (i, mode) in coll.modes.iter().enumerate() {
            let cell = Rect::from_min_size(Pos2::new(modes_left - sx + i as f32 * (mw + 1.0), r.top()), Vec2::new(mw, r.height()));
            if cell.right() < area.left() || cell.left() > area.right() {
                continue;
            }
            value_cell(app, ui, cell, v, &mode.id, hot, true, fr);
        }
    });

    // ---- name cell (sticky)
    let name_cell = Rect::from_min_size(r.min, Vec2::new(theme::VAR_NAME_COL, r.height()));
    clipped(ui, name_cell, |ui| {
        let p = ui.painter().clone();
        p.rect_filled(name_cell, 0.0, theme::BG);
        if selected {
            p.rect_filled(name_cell, 0.0, theme::BG_SELECTED);
        } else if hot || dragging {
            p.rect_filled(name_cell, 0.0, theme::BG_HOVER);
        }
        let resp = ui.interact(name_cell, Id::new(("vars-rowname", &v.id)), Sense::click_and_drag());
        widgets::record(ui, &format!("vars-row:{}", v.name), name_cell);
        kind_glyph(&p, Pos2::new(name_cell.left() + theme::VAR_ICON_X + 8.0, r.center().y), v.kind, theme::ICON);
        let in_folder = !folder_of(&v.name).is_empty();
        let leaf = if in_folder { split_name(&v.name).1.to_string() } else { v.name.clone() };
        let text_rect =
            Rect::from_min_max(Pos2::new(name_cell.left() + theme::VAR_NAME_X - 6.0, r.top() + 8.0), Pos2::new(name_cell.right() - 8.0, r.bottom() - 8.0));
        if app.vars.focus_name.as_deref() == Some(v.id.as_str()) {
            app.vars.focus_name = None;
            app.vars.renaming = Some(('v', v.id.clone()));
            app.vars.rename_frames = 0;
        }
        if app.vars.renaming == Some(('v', v.id.clone())) {
            let eid = Id::new(("vars-name", &v.id));
            if let Some(s) = rename_field(app, ui, text_rect, eid, &leaf, theme::font_body(), 6.0) {
                let (folder, _) = split_name(&v.name);
                let full = if in_folder && !s.contains('/') { format!("{folder}/{}", s.trim()) } else { s.trim().to_string() };
                fr.work.rename_variable(&v.id, &full);
            }
        } else {
            let shown = widgets::elide(&p, &leaf, &theme::font_body(), text_rect.width() - 6.0);
            p.text(Pos2::new(name_cell.left() + theme::VAR_NAME_X, r.center().y), Align2::LEFT_CENTER, shown, theme::font_body(), theme::TEXT);
        }
        // selection
        if resp.clicked() {
            let m = ui.input(|i| i.modifiers);
            select_row(app, fr, &v.id, m);
        }
        if resp.double_clicked() {
            app.vars.selected = vec![v.id.clone()];
            app.vars.renaming = Some(('v', v.id.clone()));
            app.vars.rename_frames = 0;
        }
        if resp.drag_started() {
            if !app.vars.selected.contains(&v.id) {
                app.vars.selected = vec![v.id.clone()];
                app.vars.anchor = Some(v.id.clone());
            }
            app.vars.drag = Some(RowDrag { var: v.id.clone() });
        }
        if resp.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        }
    });
    row_context(app, ui, r, v, fr);
    let _ = x0;
}

/// Right click anywhere on a row opens the row menu (selecting the row when it is not selected).
fn row_context(app: &mut App, ui: &mut Ui, r: Rect, v: &Variable, fr: &mut Frame) {
    let pos = ui.input(|i| if i.pointer.secondary_clicked() { i.pointer.interact_pos() } else { None });
    let Some(pos) = pos else { return };
    if !r.contains(pos) || !ui.clip_rect().contains(pos) || widgets::pointer_over_popup(ui.ctx()) {
        return;
    }
    if !app.vars.selected.contains(&v.id) {
        app.vars.selected = vec![v.id.clone()];
        app.vars.anchor = Some(v.id.clone());
    }
    let _ = fr;
    app.vars.menu = Some(VarMenu { kind: VarMenuKind::Row(v.id.clone()), pos, anchor: Rect::from_min_size(pos, Vec2::ZERO) });
}

fn select_row(app: &mut App, fr: &Frame, id: &str, m: egui::Modifiers) {
    if m.shift {
        let anchor = app.vars.anchor.clone().unwrap_or_else(|| id.to_string());
        let a = fr.order.iter().position(|x| *x == anchor);
        let b = fr.order.iter().position(|x| x == id);
        if let (Some(a), Some(b)) = (a, b) {
            let (lo, hi) = (a.min(b), a.max(b));
            app.vars.selected = fr.order[lo..=hi].to_vec();
            return;
        }
    }
    if m.command {
        if let Some(i) = app.vars.selected.iter().position(|x| x == id) {
            app.vars.selected.remove(i);
        } else {
            app.vars.selected.push(id.to_string());
        }
        app.vars.anchor = Some(id.to_string());
        return;
    }
    app.vars.selected = vec![id.to_string()];
    app.vars.anchor = Some(id.to_string());
}

/// Drop indicator while dragging rows and the drop itself.
fn drag_overlay(app: &mut App, ui: &mut Ui, g: &Geom, fr: &mut Frame, rows_rect: Rect) {
    if app.vars.drag.is_none() {
        return;
    }
    let Some(ptr) = ui.input(|i| i.pointer.latest_pos()) else { return };
    let released = ui.input(|i| i.pointer.any_released());
    let x0 = g.main_left();
    // target: (move before this variable, indicator rect)
    enum T {
        Before(Option<String>),
        Group(Option<String>),
    }
    let mut target: Option<(T, Rect)> = None;
    if let Some((rect, path)) = fr.group_hits.iter().find(|(r, _)| r.contains(ptr)) {
        target = Some((T::Group(path.clone()), *rect));
    } else if ptr.x >= x0 && rows_rect.contains(ptr) {
        let n = fr.slots.len();
        for (i, s) in fr.slots.iter().enumerate() {
            if ptr.y < s.bottom {
                let line = |y: f32| Rect::from_min_size(Pos2::new(x0, y - 1.0), Vec2::new(g.panel.right() - x0, 2.0));
                match &s.kind {
                    SlotKind::Header(path) => {
                        target = Some((T::Group(Some(path.clone())), Rect::from_min_max(Pos2::new(x0, s.top), Pos2::new(g.panel.right(), s.bottom - 1.0))))
                    }
                    SlotKind::Var(id) => {
                        if ptr.y < (s.top + s.bottom) / 2.0 {
                            target = Some((T::Before(Some(id.clone())), line(s.top)));
                        } else {
                            let next = fr.slots[i + 1..].iter().find_map(|x| if let SlotKind::Var(i2) = &x.kind { Some(i2.clone()) } else { None });
                            target = Some((T::Before(next), line(s.bottom - 1.0)));
                        }
                    }
                }
                break;
            }
            if i + 1 == n {
                if let SlotKind::Var(_) = &s.kind {
                    target = Some((T::Before(None), Rect::from_min_size(Pos2::new(x0, s.bottom - 2.0), Vec2::new(g.panel.right() - x0, 2.0))));
                }
            }
        }
    }
    if let Some((t, rect)) = &target {
        let p = ui.ctx().layer_painter(egui::LayerId::new(Order::Middle, Id::new("vars-drop"))).with_clip_rect(g.panel);
        match t {
            T::Before(_) => {
                p.rect_filled(*rect, 1.0, theme::DROP_INDICATOR);
            }
            T::Group(_) => {
                p.rect_stroke(*rect, 0.0, Stroke::new(1.0, theme::BORDER_SELECTED), StrokeKind::Inside);
            }
        }
    }
    if released {
        let dragged: Vec<String> = {
            let sel = &app.vars.selected;
            let d = app.vars.drag.as_ref().map(|d| d.var.clone()).unwrap_or_default();
            if sel.contains(&d) {
                fr.view.collections.values().flat_map(|c| c.variables.iter()).filter(|id| sel.contains(id)).cloned().collect()
            } else {
                vec![d]
            }
        };
        if let Some((t, _)) = target {
            match t {
                T::Before(before) => {
                    for id in &dragged {
                        fr.work.move_variable(id, before.as_deref());
                    }
                }
                T::Group(path) => {
                    for id in &dragged {
                        move_into_group(&mut fr.work, id, path.as_deref());
                    }
                }
            }
        }
        app.vars.drag = None;
    }
}

/// Moves a variable into `group` (end of the group; `None` = All, i.e. out of any group).
fn move_into_group(store: &mut VariableStore, id: &str, group: Option<&str>) {
    let Some(v) = store.variables.get(id) else { return };
    let leaf = split_name(&v.name).1.to_string();
    let coll = v.collection.clone();
    let name = match group {
        Some(g) if !g.is_empty() => format!("{g}/{leaf}"),
        _ => leaf,
    };
    store.rename_variable(id, &name);
    // place after the last variable of the group
    let last = group.and_then(|g| {
        store.collections.get(&coll).and_then(|c| {
            c.variables.iter().rev().find(|x| *x != id && store.variables.get(*x).is_some_and(|v| in_group(folder_of(&v.name), Some(g)))).cloned()
        })
    });
    let next =
        last.and_then(|l| store.collections.get(&coll).and_then(|c| c.variables.iter().position(|x| *x == l).and_then(|i| c.variables.get(i + 1).cloned())));
    match (group, next) {
        (Some(_), Some(n)) if n != id => {
            // moving "before next" would adopt next's folder; keep our name instead
            let keep = store.variables.get(id).map(|v| v.name.clone());
            store.move_variable(id, Some(&n));
            if let (Some(k), Some(v)) = (keep, store.variables.get_mut(id)) {
                v.name = k;
            }
        }
        (Some(_), _) => store.move_variable(id, None),
        (None, _) => {}
    }
}

// ------------------------------------------------------------------ value cells

#[allow(clippy::too_many_arguments)]
pub(super) fn value_cell(app: &mut App, ui: &mut Ui, cell: Rect, v: &Variable, mode: &str, hot: bool, opacity: bool, fr: &mut Frame) {
    let id = Id::new(("vars-cell", &v.id, mode, opacity));
    let value = v.values.get(mode).cloned().unwrap_or_else(|| default_value(v.kind));
    let p = ui.painter().clone();
    let cy = cell.center().y;
    let fh = theme::VAR_FIELD_H;
    widgets::record(ui, &format!("vars-cell:{}:{}", v.name, mode), cell);
    if let VariableValue::Alias(target) = &value {
        let tv = fr.view.variables.get(target);
        let swatch_c = match fr.view.resolve(&v.id, mode) {
            Some(VariableValue::Color(c)) if v.kind == VariableKind::Color => Some(to_c32(c)),
            _ => None,
        };
        let label = tv.map(|t| t.name.clone()).unwrap_or_else(|| "Missing variable".into());
        let tw = widgets::text_width(&p, &label, &theme::font_body());
        let w = (tw + 12.0 + 12.0 + if swatch_c.is_some() { 18.0 } else { 0.0 } + 8.0 + 22.0).min(cell.width() - 16.0);
        let pr_rect = Rect::from_min_size(Pos2::new(cell.left() + 8.0, cy - fh / 2.0), Vec2::new(w, fh));
        widgets::record(ui, &format!("vars-alias:{}:{}", v.name, mode), pr_rect);
        let pr = pill(ui, pr_rect, id, &label, swatch_c, true);
        if pr.detach {
            fr.work.detach_alias(&v.id, mode);
        } else if pr.body.clicked() {
            app.vars.alias_picker = Some(AliasPicker { var: v.id.clone(), mode: mode.to_string(), anchor: pr_rect, query: String::new() });
        }
        return;
    }
    let bind_rect = Rect::from_center_size(Pos2::new(cell.right() - 16.0, cy), Vec2::splat(20.0));
    let mut field_right = cell.right() - 8.0;
    if hot && opacity {
        field_right = bind_rect.left() - 4.0;
    }
    match &value {
        VariableValue::Color(c) => {
            let sw = Rect::from_min_size(Pos2::new(cell.left() + 17.0, cy - theme::VAR_SWATCH / 2.0), Vec2::splat(theme::VAR_SWATCH));
            let hex_w = theme::VAR_HEX_W.min((cell.width() - theme::VAR_HEX_X - 8.0).max(40.0));
            let hex_r = Rect::from_min_size(Pos2::new(cell.left() + theme::VAR_HEX_X, cy - fh / 2.0), Vec2::new(hex_w, fh));
            let eid = id.with("hex");
            let oid = id.with("op");
            let op_r = Rect::from_min_size(
                Pos2::new((cell.right() - theme::VAR_OPACITY_RIGHT).max(hex_r.left() + 60.0), cy - fh / 2.0),
                Vec2::new(theme::VAR_OPACITY_W, fh),
            );
            let op_shown = opacity && (hot || ui.memory(|m| m.has_focus(oid)) || ui.memory(|m| m.has_focus(eid)));
            let hex_r = if op_shown { Rect::from_min_max(hex_r.min, Pos2::new(hex_r.right().min(op_r.left() - 4.0), hex_r.bottom())) } else { hex_r };
            cell_chrome(ui, hex_r, eid);
            let hex_text = Rect::from_min_max(Pos2::new(cell.left() + 36.0, hex_r.top()), hex_r.max);
            let swr = ui.interact(sw.expand(2.0), id.with("sw"), Sense::click());
            swatch(&p, sw, to_c32(*c));
            widgets::record(ui, &format!("vars-swatch:{}:{}", v.name, mode), sw);
            widgets::record(ui, &format!("vars-hex:{}:{}", v.name, mode), hex_r);
            if swr.clicked() {
                app.vars.cell_picker = Some(CellPicker { var: v.id.clone(), mode: mode.to_string(), color: to_c32(*c), anchor: sw, before: fr.view.clone() });
            }
            if let Some(s) = field::edit_text(ui, hex_text, eid, &c.to_hex(), theme::font_body(), theme::TEXT, 0.0) {
                if let Some(nc) = Color::from_hex(&s) {
                    fr.work.set_value(&v.id, mode, VariableValue::Color(Color { a: if s.trim_start_matches('#').len() == 8 { nc.a } else { c.a }, ..nc }));
                }
            }
            if op_shown {
                let op_r = Rect::from_min_size(Pos2::new(field_right - theme::VAR_OPACITY_W, op_r.top()), op_r.size());
                p.rect_filled(op_r, theme::cr(theme::RADIUS_MEDIUM), theme::BG_SECONDARY);
                if ui.memory(|m| m.has_focus(oid)) {
                    p.rect_stroke(op_r, theme::cr(theme::RADIUS_MEDIUM), Stroke::new(1.0, theme::BORDER_SELECTED), StrokeKind::Inside);
                }
                widgets::record(ui, &format!("vars-opacity:{}:{}", v.name, mode), op_r);
                let shown = format!("{}%", fmt_num((c.a * 100.0).round()));
                if let Some(s) = field::edit_text(ui, op_r, oid, &shown, theme::font_body(), theme::TEXT, 8.0) {
                    if let Some(x) = field::parse_number(&s) {
                        fr.work.set_value(&v.id, mode, VariableValue::Color(Color { a: (x / 100.0).clamp(0.0, 1.0), ..*c }));
                    }
                }
            }
        }
        VariableValue::Number(n) => {
            let eid = id.with("num");
            let edit_r = Rect::from_min_max(Pos2::new(cell.left() + 8.0, cy - fh / 2.0), Pos2::new(field_right.min(cell.left() + 8.0 + 140.0), cy + fh / 2.0));
            cell_chrome(ui, edit_r, eid);
            widgets::record(ui, &format!("vars-num:{}:{}", v.name, mode), edit_r);
            let text_r = Rect::from_min_max(edit_r.min + Vec2::new(8.0, 0.0), edit_r.max);
            if let Some(s) = field::edit_text(ui, text_r, eid, &fmt_num(*n), theme::font_body(), theme::TEXT, 0.0) {
                if let Some(x) = field::parse_number(&s) {
                    fr.work.set_value(&v.id, mode, VariableValue::Number(x));
                }
            }
        }
        VariableValue::String(s0) => {
            let eid = id.with("str");
            let edit_r = Rect::from_min_max(Pos2::new(cell.left() + 8.0, cy - fh / 2.0), Pos2::new(field_right.min(cell.left() + 8.0 + 140.0), cy + fh / 2.0));
            cell_chrome(ui, edit_r, eid);
            widgets::record(ui, &format!("vars-str:{}:{}", v.name, mode), edit_r);
            let text_r = Rect::from_min_max(edit_r.min + Vec2::new(8.0, 0.0), edit_r.max);
            if let Some(s) = field::edit_text(ui, text_r, eid, s0, theme::font_body(), theme::TEXT, 0.0) {
                fr.work.set_value(&v.id, mode, VariableValue::String(s));
            }
        }
        VariableValue::Boolean(b) => {
            let sw = Rect::from_min_size(Pos2::new(cell.left() + 16.0, cy - 8.0), Vec2::new(28.0, 16.0));
            let resp = ui.interact(Rect::from_min_max(cell.min, Pos2::new(field_right, cell.bottom())), id.with("tog"), Sense::click());
            widgets::record(ui, &format!("vars-bool:{}:{}", v.name, mode), sw);
            p.rect_filled(sw, theme::cr(255), if *b { theme::BG_BRAND } else { theme::BG_TERTIARY });
            let kx = if *b { sw.right() - 8.0 } else { sw.left() + 8.0 };
            p.circle_filled(Pos2::new(kx, sw.center().y), 6.0, Color32::WHITE);
            p.text(Pos2::new(sw.right() + 8.0, cy), Align2::LEFT_CENTER, if *b { "True" } else { "False" }, theme::font_body(), theme::TEXT);
            if resp.clicked() {
                fr.work.set_value(&v.id, mode, VariableValue::Boolean(!b));
            }
        }
        VariableValue::Alias(_) => {}
    }
    if hot && opacity {
        let br = ui.interact(bind_rect, id.with("bind"), Sense::click());
        if br.hovered() {
            p.rect_filled(bind_rect, theme::cr(theme::RADIUS_MEDIUM), theme::BG_PRESSED);
        }
        hexagon(&p, bind_rect.center(), 5.0, theme::ICON_SECONDARY, false);
        widgets::record(ui, &format!("vars-bind:{}:{}", v.name, mode), bind_rect);
        if br.clicked() {
            app.vars.alias_picker = Some(AliasPicker { var: v.id.clone(), mode: mode.to_string(), anchor: bind_rect, query: String::new() });
        }
        br.on_hover_text("Create alias");
    }
}

// ------------------------------------------------------------------ popups

fn run_popups(app: &mut App, ctx: &Context, g: &Geom, fr: &mut Frame) {
    run_menu(app, ctx, fr);
    run_alias_picker(app, ctx, fr);
    run_cell_picker(app, ctx);
    super::popover::show(app, ctx, g, fr);
    crate::panels::var_bind::bind_picker(app, ctx);
}

fn unique_group_name(fr: &Frame, coll: &str, parent: &str) -> String {
    let existing: Vec<String> = fr.work.collections.get(coll).map(|c| group_paths(&fr.work, c).into_iter().map(|(p, _)| p).collect()).unwrap_or_default();
    let mut n = 1;
    loop {
        let name = if parent.is_empty() { format!("Group {n}") } else { format!("{parent}/Group {n}") };
        if !existing.contains(&name) {
            return name;
        }
        n += 1;
    }
}

fn run_menu(app: &mut App, ctx: &Context, fr: &mut Frame) {
    let Some(menu) = app.vars.menu.clone() else { return };
    let cid = app.vars.collection.clone().unwrap_or_default();
    let items: Vec<MenuItem> = match &menu.kind {
        VarMenuKind::Create => ["Color", "Number", "String", "Boolean"].iter().map(|s| MenuItem::new(*s)).collect(),
        VarMenuKind::Row(_) => {
            vec![
                MenuItem::new("Edit variable").sep(),
                MenuItem::new("Duplicate").shortcut("Ctrl+D"),
                MenuItem::new("New group with selection").sep(),
                MenuItem::new("Delete").shortcut("Del"),
            ]
        }
        VarMenuKind::Mode(coll, mode) => {
            let multi = fr.view.collections.get(coll).is_some_and(|c| c.modes.len() > 1);
            let is_default = fr.view.collections.get(coll).is_some_and(|c| &c.default_mode == mode);
            vec![
                MenuItem::new("Rename mode"),
                MenuItem::new("Duplicate mode"),
                MenuItem::new("Set as default").disabled(is_default),
                MenuItem::new("Delete mode").disabled(!multi),
            ]
        }
        VarMenuKind::Collection(_) => vec![MenuItem::new("Rename"), MenuItem::new("Duplicate"), MenuItem::new("Delete")],
        VarMenuKind::Folder(..) => vec![MenuItem::new("Rename group")],
        VarMenuKind::Settings => vec![MenuItem::new("Import variables..."), MenuItem::new("Export variables...")],
        VarMenuKind::Filter => [VariableKind::Color, VariableKind::Number, VariableKind::String, VariableKind::Boolean]
            .iter()
            .map(|k| MenuItem::new(kind_name(*k)).checked(!app.vars.hidden_kinds.contains(k)))
            .collect(),
    };
    match popup_menu(ctx, Id::new("vars-menu"), menu.pos, 168.0, &items, Some(menu.anchor)) {
        MenuResult::Open => {}
        MenuResult::Closed => app.vars.menu = None,
        MenuResult::Picked(i) => {
            app.vars.menu = None;
            match (&menu.kind, i) {
                (VarMenuKind::Create, i) => {
                    let kind = [VariableKind::Color, VariableKind::Number, VariableKind::String, VariableKind::Boolean][i];
                    // inside a selected group the new variable joins it
                    let name = match app.vars.group.as_deref() {
                        Some(g) => format!("{g}/{}", kind_name(kind)),
                        None => kind_name(kind).to_string(),
                    };
                    if let Some(id) = fr.work.add_variable(&cid, &name, kind) {
                        app.vars.focus_name = Some(id.clone());
                        app.vars.selected = vec![id];
                        app.vars.scroll = f32::MAX;
                        app.vars.search.clear();
                    }
                }
                (VarMenuKind::Row(id), 0) => {
                    app.vars.edit = Some(EditPopover { var: id.clone(), anchor: Rect::from_min_size(menu.pos, Vec2::ZERO), scroll: 0.0 });
                }
                (VarMenuKind::Row(_), 1) => {
                    let sel = app.vars.selected.clone();
                    let mut copies = vec![];
                    for id in &sel {
                        if let Some(n) = fr.work.duplicate_variable(id) {
                            copies.push(n);
                        }
                    }
                    app.vars.selected = copies;
                }
                (VarMenuKind::Row(id), 2) => {
                    let sel = if app.vars.selected.contains(id) { app.vars.selected.clone() } else { vec![id.clone()] };
                    let parent = sel.first().and_then(|s| fr.work.variables.get(s)).map(|v| folder_of(&v.name).to_string()).unwrap_or_default();
                    let group = unique_group_name(fr, &cid, &parent);
                    for s in &sel {
                        let leaf = fr.work.variables.get(s).map(|v| split_name(&v.name).1.to_string()).unwrap_or_default();
                        fr.work.rename_variable(s, &format!("{group}/{leaf}"));
                    }
                    app.vars.renaming = Some(('f', folded_key(&cid, &group)));
                    app.vars.rename_frames = 0;
                    app.vars.group = None;
                }
                (VarMenuKind::Row(id), _) => {
                    let sel = if app.vars.selected.contains(id) { app.vars.selected.clone() } else { vec![id.clone()] };
                    for s in &sel {
                        fr.work.delete_variable(s);
                        fr.removed_vars.push(s.clone());
                    }
                    app.vars.selected.clear();
                }
                (VarMenuKind::Mode(c, m), 0) => {
                    app.vars.renaming = Some(('m', m.clone()));
                    app.vars.rename_frames = 0;
                    let _ = c;
                }
                (VarMenuKind::Mode(c, m), 1) => {
                    fr.work.duplicate_mode(c, m);
                }
                (VarMenuKind::Mode(c, m), 2) => fr.work.set_default_mode(c, m),
                (VarMenuKind::Mode(c, m), _) => {
                    fr.work.delete_mode(c, m);
                }
                (VarMenuKind::Collection(id), 0) => {
                    app.vars.collection = Some(id.clone());
                    app.vars.renaming = Some(('c', id.clone()));
                    app.vars.rename_frames = 0;
                }
                (VarMenuKind::Collection(id), 1) => {
                    if let Some(n) = fr.work.duplicate_collection(id) {
                        app.vars.collection = Some(n);
                        app.vars.group = None;
                    }
                }
                (VarMenuKind::Collection(id), _) => {
                    let gone = fr.work.delete_collection(id);
                    fr.removed_vars.extend(gone);
                    fr.removed_colls.push(id.clone());
                    app.vars.collection = fr.work.collections.keys().find(|k| *k != id).cloned();
                    app.vars.group = None;
                }
                (VarMenuKind::Folder(c, f), _) => {
                    app.vars.renaming = Some(('f', folded_key(c, f)));
                    app.vars.rename_frames = 0;
                }
                (VarMenuKind::Settings, 0) => {
                    app.pick_file_async(
                        || rfd::FileDialog::new().add_filter("Variables JSON", &["json"]).pick_file(),
                        |app, path| app.import_variables_file(&path),
                    );
                }
                (VarMenuKind::Settings, _) => {
                    app.pick_file_async(
                        || rfd::FileDialog::new().add_filter("Variables JSON", &["json"]).set_file_name("variables.json").save_file(),
                        |app, path| app.export_variables_file(&path),
                    );
                }
                (VarMenuKind::Filter, i) => {
                    let k = [VariableKind::Color, VariableKind::Number, VariableKind::String, VariableKind::Boolean][i];
                    if let Some(pos) = app.vars.hidden_kinds.iter().position(|x| *x == k) {
                        app.vars.hidden_kinds.remove(pos);
                    } else {
                        app.vars.hidden_kinds.push(k);
                    }
                    // keep the menu open for further toggles
                    app.vars.menu = Some(menu.clone());
                }
            }
        }
    }
}

fn run_alias_picker(app: &mut App, ctx: &Context, fr: &mut Frame) {
    let Some(mut ap) = app.vars.alias_picker.take() else { return };
    let Some(var) = fr.work.variables.get(&ap.var).cloned() else { return };
    let current = match var.values.get(&ap.mode) {
        Some(VariableValue::Alias(t)) => Some(t.clone()),
        _ => None,
    };
    let items: Vec<PickItem> = fr
        .work
        .collections
        .values()
        .flat_map(|c| c.variables.iter())
        .filter_map(|id| fr.work.variables.get(id))
        .filter(|t| fr.work.can_alias(&var.id, &t.id))
        .map(|t| store_item(&fr.work, t))
        .collect();
    let res = picker(
        ctx,
        Id::new("vars-alias-picker"),
        ap.anchor.left_bottom() + Vec2::new(0.0, 4.0),
        Some(ap.anchor),
        "Alias of",
        &mut ap.query,
        &items,
        current.as_deref(),
        current.is_some(),
    );
    match res {
        PickResult::Open => app.vars.alias_picker = Some(ap),
        PickResult::Closed => {}
        PickResult::Picked(t) => {
            fr.work.set_alias(&ap.var, &ap.mode, &t);
        }
        PickResult::Detach => {
            fr.work.detach_alias(&ap.var, &ap.mode);
        }
    }
}

/// Picker item straight from a store (previews resolve in each default mode).
pub fn store_item(store: &VariableStore, v: &Variable) -> PickItem {
    let coll = store.collections.get(&v.collection);
    let preview = match (v.kind, coll) {
        (VariableKind::Color, Some(c)) => match store.resolve(&v.id, &c.default_mode) {
            Some(VariableValue::Color(col)) => Some(to_c32(col)),
            _ => None,
        },
        _ => None,
    };
    PickItem { id: v.id.clone(), group: coll.map(|c| c.name.clone()).unwrap_or_default(), name: v.name.clone(), kind: v.kind, preview }
}

fn run_cell_picker(app: &mut App, ctx: &Context) {
    let Some(mut pk) = app.vars.cell_picker.take() else { return };
    let screen = ctx.content_rect();
    let size = Vec2::new(224.0, 250.0);
    let pos = Pos2::new(pk.anchor.left().min(screen.right() - size.x - 8.0), (pk.anchor.bottom() + 6.0).min(screen.bottom() - size.y - 8.0));
    let mut changed = false;
    let area = egui::Area::new(Id::new("vars-color-picker")).order(Order::Foreground).fixed_pos(pos).show(ctx, |ui| {
        let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
        widgets::menu_panel(ui.painter(), rect, theme::MENU_RADIUS);
        ui.scope_builder(egui::UiBuilder::new().max_rect(rect.shrink(8.0)), |ui| {
            ui.spacing_mut().slider_width = 180.0;
            changed = egui::color_picker::color_picker_color32(ui, &mut pk.color, egui::color_picker::Alpha::OnlyBlend);
        });
        rect
    });
    if changed {
        let c = from_c32(pk.color);
        if let Some(v) = app.doc.variables.get_mut(&pk.var) {
            v.values.insert(pk.mode.clone(), VariableValue::Color(c));
        }
        app.changed();
    }
    let (pressed, ptr, esc) = ctx.input(|i| (i.pointer.any_pressed(), i.pointer.interact_pos(), i.key_pressed(Key::Escape)));
    let outside = pressed && ptr.is_some_and(|p| !area.inner.contains(p) && !pk.anchor.contains(p));
    if outside || esc {
        app.finish_cell_picker(pk);
    } else {
        app.vars.cell_picker = Some(pk);
    }
}
