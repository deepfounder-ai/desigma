//! Left sidebar of the home screen: search, Recents / Starred, the project
//! tree with its folders, and the Trash.

use super::{Act, DropTo, Home, Section, Target};
use crate::icons::{self, Icon};
use crate::theme;
use crate::widgets;
use egui::{Align2, Id, Pos2, Rect, Response, Sense, Ui, Vec2};

const ROW_H: f32 = 30.0;
const INDENT: f32 = 16.0;

pub fn show(h: &mut Home, ui: &mut Ui, rect: Rect, acts: &mut Vec<Act>) {
    let p = ui.painter().clone();
    p.rect_filled(rect, 0.0, theme::BG);
    widgets::vline(&p, rect.right() - theme::BORDER_WIDTH, rect.top(), rect.bottom(), theme::BORDER);
    let x0 = rect.left();
    let w = rect.width() - theme::BORDER_WIDTH;
    let mut y = rect.top();

    // ---- brand
    icons::paint(&p, Icon::Logo, Pos2::new(x0 + 28.0, y + 28.0), 24.0, theme::ICON);
    p.text(Pos2::new(x0 + 48.0, y + 28.0), Align2::LEFT_CENTER, "desigma", theme::font_large_strong(), theme::TEXT);
    y += 56.0;

    // ---- search
    let sr = Rect::from_min_size(Pos2::new(x0 + 12.0, y), Vec2::new(w - 24.0, 32.0));
    p.rect_filled(sr, theme::cr(theme::RADIUS_MEDIUM), theme::BG_SECONDARY);
    icons::paint(&p, Icon::Search, Pos2::new(sr.left() + 16.0, sr.center().y), 16.0, theme::ICON_SECONDARY);
    let tr = Rect::from_min_max(Pos2::new(sr.left() + 32.0, sr.top()), sr.max - Vec2::new(8.0, 0.0));
    let tid = Id::new("home-search");
    let mut text = h.search.clone();
    let te = egui::TextEdit::singleline(&mut text)
        .id(tid)
        .font(theme::font_body())
        .text_color(theme::TEXT)
        .frame(egui::Frame::NONE)
        .margin(egui::Margin::ZERO)
        .vertical_align(egui::Align::Center)
        .hint_text(egui::RichText::new(crate::i18n::tr("Search files")).font(theme::font_body()).color(theme::TEXT_TERTIARY))
        .min_size(tr.size())
        .desired_width(tr.width());
    ui.put(tr, te);
    widgets::record(ui, "home-search", sr);
    if text != h.search {
        h.search = text;
        h.selected = None;
        h.scroll = 0.0;
    }
    y += 32.0 + 12.0;

    // ---- Recents / Starred
    for (icon, label, section, key) in [(Icon::Clock, "Recents", Section::Recents, "recents"), (Icon::Star, "Starred", Section::Starred, "starred")] {
        let r = Rect::from_min_size(Pos2::new(x0 + 8.0, y), Vec2::new(w - 16.0, ROW_H));
        let selected = h.section == section && !h.searching();
        if row(ui, r, Id::new(("home-nav", key)), icon, crate::i18n::tr(label), 0.0, selected, false, key).clicked() {
            acts.push(Act::Go(section));
        }
        y += ROW_H;
    }
    y += 12.0;

    // ---- Projects header
    p.text(Pos2::new(x0 + 20.0, y + 14.0), Align2::LEFT_CENTER, crate::i18n::tr("Projects"), theme::font_body_strong(), theme::TEXT_SECONDARY);
    let plus = Rect::from_min_size(Pos2::new(x0 + w - 12.0 - 24.0, y + 2.0), Vec2::splat(24.0));
    if widgets::icon_button(ui, plus, "home-new-project", Icon::Plus, false, "New project").clicked() {
        acts.push(Act::NewProject);
    }
    widgets::record(ui, "home-new-project", plus);
    y += 28.0;

    // ---- project tree (scrolls)
    let trash_h = 52.0;
    let footer_h = trash_h + ROW_H;
    let list = Rect::from_min_max(Pos2::new(x0, y), Pos2::new(x0 + w, rect.bottom() - footer_h));
    let rows = tree_rows(h);
    let content_h = rows.len() as f32 * ROW_H + 8.0;
    crate::panels::wheel_scroll(ui, list, &mut h.side_scroll, content_h);
    let scroll = h.side_scroll;
    let ptr = ui.input(|i| i.pointer.hover_pos());
    let dragging = h.drag.is_some();
    let mut to_toggle = None;
    let mut secondary: Option<(Target, Pos2)> = None;
    let mut dropzones: Vec<(Rect, DropTo)> = vec![];
    crate::panels::clipped(ui, list, |ui| {
        if rows.is_empty() {
            ui.painter().text(
                Pos2::new(x0 + 20.0, list.top() + 16.0),
                Align2::LEFT_CENTER,
                crate::i18n::tr("No projects yet"),
                theme::font_body(),
                theme::TEXT_TERTIARY,
            );
        }
        for (i, tr) in rows.iter().enumerate() {
            let r = Rect::from_min_size(Pos2::new(x0 + 8.0, list.top() + i as f32 * ROW_H - scroll), Vec2::new(w - 16.0, ROW_H));
            if r.bottom() < list.top() || r.top() > list.bottom() {
                continue;
            }
            let selected = !h.searching() && h.section == tr.section;
            let drop_hover = dragging && ptr.is_some_and(|p| r.contains(p) && list.contains(p));
            let icon = if tr.folder { Icon::Folder } else { Icon::Frame };
            let resp =
                row(ui, r, Id::new(("home-tree", &tr.key)), icon, &tr.label, tr.depth as f32 * INDENT, selected, drop_hover, &format!("project-{}", tr.label));
            // chevron for projects with folders
            if let Some(open) = tr.expanded {
                let c = Pos2::new(r.left() + 10.0 + tr.depth as f32 * INDENT, r.center().y);
                icons::paint(ui.painter(), if open { Icon::ChevronDown } else { Icon::ChevronRight }, c, 12.0, theme::ICON_TERTIARY);
            }
            let on_chevron = tr.expanded.is_some() && ptr.is_some_and(|p| p.x < r.left() + 22.0 + tr.depth as f32 * INDENT);
            if resp.clicked() {
                if on_chevron {
                    to_toggle = Some(tr.project.clone());
                } else {
                    acts.push(Act::Go(tr.section.clone()));
                }
            }
            if resp.secondary_clicked() {
                let at = ui.input(|i| i.pointer.interact_pos()).unwrap_or(r.left_bottom());
                let target = match &tr.section {
                    Section::Project(p, Some(f)) => Target::Folder(p.clone(), f.clone()),
                    _ => Target::Project(tr.project.clone()),
                };
                secondary = Some((target, at));
            }
            let to = match &tr.section {
                Section::Project(p, Some(f)) => DropTo::Folder(p.clone(), f.clone()),
                _ => DropTo::Project(tr.project.clone()),
            };
            dropzones.push((r.intersect(list), to));
        }
    });
    h.drop_rects.extend(dropzones);
    crate::panels::scrollbar(ui, list, scroll, content_h);
    if let Some(p) = to_toggle {
        if !h.collapsed.remove(&p) {
            h.collapsed.insert(p);
        }
    }
    if secondary.is_some() {
        h.menu = secondary;
    }

    // ---- Claude, then trash
    let cr = Rect::from_min_size(Pos2::new(x0 + 8.0, rect.bottom() - footer_h + 4.0), Vec2::new(w - 16.0, ROW_H));
    if row(ui, cr, Id::new("home-claude"), Icon::Link, "Claude", 0.0, false, false, "home-claude").clicked() {
        acts.push(Act::Mcp);
    }
    widgets::hline(&p, x0, x0 + w, rect.bottom() - trash_h, theme::BORDER);
    let tr = Rect::from_min_size(Pos2::new(x0 + 8.0, rect.bottom() - trash_h + 10.0), Vec2::new(w - 16.0, ROW_H));
    let n = h.snap.trash.len();
    let label = if n > 0 { format!("Trash ({n})") } else { "Trash".to_string() };
    if row(ui, tr, Id::new("home-trash"), Icon::Trash, &crate::i18n::tr_text(&label), 0.0, h.section == Section::Trash && !h.searching(), false, "trash")
        .clicked()
    {
        acts.push(Act::Go(Section::Trash));
    }
}

struct TreeRow {
    key: String,
    project: String,
    label: String,
    depth: usize,
    folder: bool,
    /// `Some(open)` when the row can be expanded.
    expanded: Option<bool>,
    section: Section,
}

/// Projects and (unless collapsed) their folder trees, flattened.
fn tree_rows(h: &Home) -> Vec<TreeRow> {
    fn walk(out: &mut Vec<TreeRow>, p: &desigma_library::Project, parent: Option<&str>, depth: usize) {
        let mut kids: Vec<_> = p.folders.iter().filter(|f| f.parent.as_deref() == parent).collect();
        kids.sort_by_key(|f| f.name.to_lowercase());
        for f in kids {
            out.push(TreeRow {
                key: format!("{}/{}", p.id, f.id),
                project: p.id.clone(),
                label: f.name.clone(),
                depth,
                folder: true,
                expanded: None,
                section: Section::Project(p.id.clone(), Some(f.id.clone())),
            });
            walk(out, p, Some(&f.id), depth + 1);
        }
    }
    let mut out = vec![];
    for p in &h.snap.projects {
        let open = !h.collapsed.contains(&p.id);
        out.push(TreeRow {
            key: p.id.clone(),
            project: p.id.clone(),
            label: p.name.clone(),
            depth: 0,
            folder: false,
            expanded: (!p.folders.is_empty()).then_some(open),
            section: Section::Project(p.id.clone(), None),
        });
        if open {
            walk(&mut out, p, None, 1);
        }
    }
    out
}

/// One sidebar row: icon, label, hover / selected / drop-target highlight.
#[allow(clippy::too_many_arguments)]
fn row(ui: &mut Ui, r: Rect, id: Id, icon: Icon, label: &str, indent: f32, selected: bool, drop_hover: bool, record: &str) -> Response {
    let resp = ui.interact(r, id, Sense::click());
    let p = ui.painter();
    if drop_hover {
        p.rect_filled(r, theme::cr(theme::RADIUS_MEDIUM), theme::BG_SELECTED_HOVER);
    } else if selected {
        p.rect_filled(r, theme::cr(theme::RADIUS_MEDIUM), theme::BG_SELECTED);
    } else if resp.hovered() {
        p.rect_filled(r, theme::cr(theme::RADIUS_MEDIUM), theme::BG_HOVER);
    }
    let ix = r.left() + 12.0 + indent;
    icons::paint(p, icon, Pos2::new(ix + 14.0, r.center().y), 16.0, if selected { theme::ICON } else { theme::ICON_SECONDARY });
    let font = if selected { theme::font_body_strong() } else { theme::font_body() };
    let shown = widgets::elide(p, label, &font, r.right() - (ix + 30.0) - 8.0);
    p.text(Pos2::new(ix + 30.0, r.center().y), Align2::LEFT_CENTER, shown, font, theme::TEXT);
    widgets::record(ui, &format!("home-{record}"), r);
    resp
}
