//! Right panel content: page settings when nothing is selected, otherwise the
//! inspector (alignment, position, layout, appearance, fill, stroke, effects).

use super::{clipped, scrollbar, var_bind, wheel_scroll};
use crate::app::App;
use crate::edit::Tx;
use crate::icons::{self, Icon};
use crate::ops::{apply_resize, Align};
use crate::theme;
use crate::util::v2;
use crate::widgets::field::{self, Prefix};
use crate::widgets::menu::dropdown;
use crate::widgets::{self, fmt_num};
use desigma_doc::Vec2 as DVec2;
use desigma_doc::*;
use egui::{Align2, Color32, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, Vec2};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickTarget {
    Fill(usize),
    Stroke(usize),
    PageBg,
    Shadow(usize),
    /// A colour from the Selection colors list (rgb of the colour being replaced).
    SelColor(u32),
    /// The colour of text (the edited character range, or every run of the selected text layers).
    TextFill,
    /// Colour of the i-th layout grid of a frame.
    GridColor(usize),
    /// Colour (or second colour) of the i-th effect, a noise effect.
    NoiseColor(usize, bool),
}

pub struct Picker {
    pub target: PickTarget,
    pub tx: Tx,
    pub anchor: Rect,
    pub ids: Vec<NodeId>,
    /// The paint being edited (solid colour with opacity as alpha, or a gradient).
    pub paint: Paint,
    pub state: crate::widgets::picker::State,
    /// Swatch sources gathered when the picker opened.
    pub sources: Vec<(String, Vec<Color32>)>,
    /// Waiting for a click on the canvas to sample a colour.
    pub eyedrop: bool,
}

#[derive(Default)]
pub struct PropsState {
    pub scroll: f32,
    pub content_h: f32,
    pub picker: Option<Picker>,
    /// Collapsed style folders in the Styles section.
    pub folded: std::collections::HashSet<String>,
    /// Page background opacity before the visibility toggle hid it.
    pub page_alpha_prev: f32,
    /// Open settings popover (effect or stroke).
    pub pop: Option<super::effects::PopState>,
    /// The Effects "+" menu is open.
    pub fx_menu: bool,
    /// The paint styles menu (4-dots button) is open: Some(for_stroke).
    pub styles_menu: Option<bool>,
}

pub(crate) struct Pane {
    pub(crate) x0: f32,
    pub(crate) oy: f32,
    pub(crate) y: f32,
}

impl Pane {
    pub(crate) fn rect(&self, x: f32, dy: f32, w: f32, h: f32) -> Rect {
        Rect::from_min_size(Pos2::new(self.x0 + x, self.oy + self.y + dy), Vec2::new(w, h))
    }
    pub(crate) fn col(&self, i: usize, dy: f32) -> Rect {
        self.rect(theme::FIELD_COL_X[i], dy, theme::FIELD_WIDTH_HALF, theme::FIELD_HEIGHT)
    }
    pub(crate) fn full(&self, dy: f32) -> Rect {
        self.rect(theme::SECTION_CONTENT_X, dy, theme::SECTION_CONTENT_W, theme::FIELD_HEIGHT)
    }
}

pub(crate) fn to_c32(c: Color, opacity: f32) -> Color32 {
    let f = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    Color32::from_rgba_unmultiplied(f(c.r), f(c.g), f(c.b), f(c.a * opacity))
}

#[cfg(test)]
fn from_c32(c: Color32) -> Color {
    let [r, g, b, a] = c.to_srgba_unmultiplied();
    Color::rgba(r, g, b, a)
}

pub(crate) fn section_sep(ui: &Ui, pane: &mut Pane, first: bool) {
    if !first {
        widgets::hline(ui.painter(), pane.x0, pane.x0 + theme::RIGHT_PANEL_WIDTH, pane.oy + pane.y, theme::BORDER);
        pane.y += 1.0;
    }
}

/// Section title row; returns true when its action button was clicked.
pub(crate) fn title(ui: &mut Ui, pane: &mut Pane, text: &str, action: Option<Icon>, dim: bool) -> bool {
    let row = pane.rect(0.0, 0.0, theme::RIGHT_PANEL_WIDTH - 1.0, theme::SECTION_TITLE_HEIGHT);
    ui.painter().text(
        Pos2::new(row.left() + theme::SECTION_CONTENT_X, row.center().y),
        Align2::LEFT_CENTER,
        crate::i18n::tr(text),
        theme::font_body_strong(),
        if dim { theme::TEXT_SECONDARY } else { theme::TEXT },
    );
    let mut clicked = false;
    if let Some(ic) = action {
        let b = pane.rect(theme::SECTION_TRAILING_X, 8.0, theme::PANEL_ACTION_BUTTON, theme::PANEL_ACTION_BUTTON);
        clicked = widgets::icon_button(ui, b, ("title-action", text), ic, false, "").clicked();
    }
    pane.y += theme::SECTION_TITLE_HEIGHT;
    clicked
}

/// Fill / Stroke section header: title, the styles button (4 dots) and "+". Returns true when
/// "+" was clicked.
pub(crate) fn paint_title(app: &mut App, ui: &mut Ui, pane: &mut Pane, sel: &[NodeId], text: &str, stroke: bool, dim: bool) -> bool {
    title(ui, pane, text, None, dim);
    let y = -theme::SECTION_TITLE_HEIGHT + 8.0;
    let plus = pane.rect(theme::SECTION_TRAILING_X, y, theme::PANEL_ACTION_BUTTON, theme::PANEL_ACTION_BUTTON);
    let styles = pane.rect(theme::SECTION_TRAILING_X - 28.0, y, theme::PANEL_ACTION_BUTTON, theme::PANEL_ACTION_BUTTON);
    let clicked = widgets::icon_button(ui, plus, ("title-action", text), Icon::Plus, false, "").clicked();
    let open = app.props.styles_menu == Some(stroke);
    if widgets::icon_button(ui, styles, ("title-styles", text), Icon::Styles, open, "Apply style").clicked() {
        app.props.styles_menu = (!open).then_some(stroke);
    }
    if open {
        let want = if stroke { StyleProperty::Stroke } else { StyleProperty::Fill };
        let entries: Vec<(String, String)> = crate::styles::style_entries(&app.doc, false)
            .into_iter()
            .filter(|e| app.doc.style_meta.get(&e.key).map(|m| m.property).unwrap_or(StyleProperty::Fill) == want)
            .map(|e| (e.key.clone(), app.doc.style_name(&e.key).to_string()))
            .collect();
        let items: Vec<crate::widgets::menu::MenuItem> = if entries.is_empty() {
            vec![crate::widgets::menu::MenuItem::new("No color styles").disabled(true)]
        } else {
            entries.iter().map(|(_, n)| crate::widgets::menu::MenuItem::new(n.clone())).collect()
        };
        let pos = Pos2::new(styles.right() - 168.0, styles.bottom() + 4.0);
        match crate::widgets::menu::popup_menu(ui.ctx(), egui::Id::new(("styles-menu", stroke)), pos, 168.0, &items, Some(styles)) {
            crate::widgets::menu::MenuResult::Picked(i) => {
                app.props.styles_menu = None;
                if let Some((key, _)) = entries.get(i) {
                    app.apply_style(sel, key);
                }
            }
            crate::widgets::menu::MenuResult::Closed => app.props.styles_menu = None,
            crate::widgets::menu::MenuResult::Open => {}
        }
    }
    clicked
}

pub(crate) fn sublabel(ui: &Ui, pane: &mut Pane, text: &str) {
    let r = pane.rect(theme::SECTION_CONTENT_X, 0.0, theme::SECTION_CONTENT_W, theme::SUBLABEL_HEIGHT);
    ui.painter().text(Pos2::new(r.left(), r.center().y), Align2::LEFT_CENTER, crate::i18n::tr(text), theme::font_small_medium(), theme::TEXT_SECONDARY);
    pane.y += theme::SUBLABEL_HEIGHT + 4.0;
}

/// Row advance after a 24px field row.
pub(crate) fn row_end(pane: &mut Pane) {
    pane.y += theme::FIELD_HEIGHT + 4.0;
}

pub(crate) fn segmented(ui: &mut Ui, pane: &Pane, x: f32, key: &str, items: &[Icon], selected: Option<usize>) -> Option<usize> {
    let mut out = None;
    let n = items.len();
    for (i, icon) in items.iter().enumerate() {
        let r = pane.rect(x + i as f32 * theme::SEGMENT_W, 0.0, theme::SEGMENT_W, theme::FIELD_HEIGHT);
        let radius = match (i == 0, i + 1 == n) {
            (true, true) => theme::cr(theme::RADIUS_MEDIUM),
            (true, false) => theme::cr_each(theme::RADIUS_MEDIUM, 0, 0, theme::RADIUS_MEDIUM),
            (false, true) => theme::cr_each(0, theme::RADIUS_MEDIUM, theme::RADIUS_MEDIUM, 0),
            _ => theme::cr(0),
        };
        let resp = ui.interact(r, ui.id().with(("seg", key, i)), Sense::click());
        let sel = selected == Some(i);
        let bg = if sel {
            theme::BG_TERTIARY
        } else if resp.hovered() {
            theme::BG_TERTIARY.gamma_multiply(0.6)
        } else {
            theme::BG_SECONDARY
        };
        ui.painter().rect_filled(r, radius, bg);
        icons::paint(ui.painter(), *icon, r.center(), 16.0, if sel { theme::ICON } else { theme::ICON_SECONDARY });
        if i > 0 {
            // 1px gap between neighbouring segments.
            widgets::vline(ui.painter(), r.left(), r.top(), r.bottom(), theme::BG);
        }
        if resp.clicked() {
            out = Some(i);
        }
    }
    out
}

pub(crate) fn checkbox(ui: &mut Ui, pane: &mut Pane, key: &str, label: &str, value: bool) -> Option<bool> {
    let r = pane.rect(theme::SECTION_CONTENT_X, 0.0, theme::SECTION_CONTENT_W, theme::FIELD_HEIGHT);
    let resp = ui.interact(r, ui.id().with(("chk", key)), Sense::click());
    let b = Rect::from_min_size(Pos2::new(r.left(), r.center().y - 7.0), Vec2::splat(14.0));
    let p = ui.painter();
    if value {
        p.rect_filled(b, theme::cr(theme::RADIUS_SMALL), theme::BG_BRAND);
        icons::paint(p, Icon::Check, b.center(), 12.0, theme::TEXT);
    } else {
        p.rect_stroke(b, theme::cr(theme::RADIUS_SMALL), Stroke::new(1.0, theme::TEXT_TERTIARY), StrokeKind::Inside);
    }
    p.text(Pos2::new(b.right() + 8.0, r.center().y), Align2::LEFT_CENTER, label, theme::font_body(), theme::TEXT);
    pane.y += theme::FIELD_HEIGHT + 4.0;
    resp.clicked().then_some(!value)
}

/// Common value of `f` over all selected nodes, or None if they differ.
fn common<T: PartialEq>(app: &App, sel: &[NodeId], f: impl Fn(&Node) -> T) -> Option<T> {
    let mut it = sel.iter().filter_map(|id| app.doc.get(*id)).map(f);
    let first = it.next()?;
    it.all(|v| v == first).then_some(first)
}

pub fn show(app: &mut App, ui: &mut Ui, region: Rect) {
    let mut scroll = app.props.scroll;
    wheel_scroll(ui, region, &mut scroll, app.props.content_h);
    app.props.scroll = scroll;
    let sel = app.selection.clone();
    let end = clipped(ui, region, |ui| {
        // Spec x values are relative to the panel's outer left edge (border at x=0).
        let mut pane = Pane { x0: region.left() - theme::BORDER_WIDTH, oy: region.top() - scroll, y: 0.0 };
        if sel.is_empty() {
            page_section(app, ui, &mut pane);
            styles_section(app, ui, &mut pane);
        } else {
            selection_ui(app, ui, &mut pane, &sel);
        }
        pane.y + 24.0
    });
    app.props.content_h = end;
    scrollbar(ui, region, scroll, end);
    picker_popup(app, ui, region);
    super::effects::popover_ui(app, ui, region);
}

// ---------------------------------------------------------------- no selection
/// Page background row: swatch, hex and opacity in one 184x24 field group plus a visibility toggle.
fn page_section(app: &mut App, ui: &mut Ui, pane: &mut Pane) {
    let settings = title(ui, pane, "Page", Some(Icon::Opacity), false);
    let bg = match app.doc.get(app.page).map(|n| &n.kind) {
        Some(NodeKind::Page { background }) => *background,
        _ => Color::WHITE,
    };
    let g = pane.rect(theme::SECTION_CONTENT_X, theme::PAGE_FIELD_DY, theme::SECTION_CONTENT_W, theme::FIELD_HEIGHT);
    let hex_id = ui.id().with(("txt", "page-hex"));
    let op_id = ui.id().with(("num", "page-op"));
    let focused = ui.memory(|m| m.has_focus(hex_id) || m.has_focus(op_id));
    {
        let p = ui.painter();
        p.rect_filled(g, theme::cr(theme::RADIUS_MEDIUM), theme::BG_SECONDARY);
        if focused {
            p.rect_stroke(g, theme::cr(theme::RADIUS_MEDIUM), Stroke::new(1.0, theme::BORDER_SELECTED), StrokeKind::Inside);
        } else if ui.rect_contains_pointer(g) {
            p.rect_stroke(g, theme::cr(theme::RADIUS_MEDIUM), Stroke::new(1.0, theme::BORDER), StrokeKind::Inside);
        }
    }
    let sw = Rect::from_center_size(Pos2::new(g.left() + theme::PAGE_SWATCH_CX, g.center().y), Vec2::splat(theme::PAGE_SWATCH));
    let swr = ui.interact(sw.expand(2.0), ui.id().with("page-swatch"), Sense::click());
    widgets::swatch(ui.painter(), sw, to_c32(bg, 1.0));
    if swr.clicked() || settings {
        open_picker(app, PickTarget::PageBg, to_c32(bg, 1.0), sw, vec![app.page]);
    }
    let hex_r = Rect::from_min_max(Pos2::new(g.left() + theme::PAGE_HEX_X, g.top()), Pos2::new(g.left() + theme::PAGE_DIVIDER_X, g.bottom()));
    if let Some(s) = field::edit_text(ui, hex_r, hex_id, &bg.to_hex(), theme::font_body(), theme::TEXT, 0.0) {
        if let Some(c) = Color::from_hex(&s) {
            let page = app.page;
            app.edit_nodes(&[page], |n| n.kind = NodeKind::Page { background: Color { a: bg.a, ..c } });
        }
    }
    widgets::vline(ui.painter(), g.left() + theme::PAGE_DIVIDER_X, g.top(), g.bottom(), theme::BG);
    let op_r = Rect::from_min_max(Pos2::new(g.left() + theme::PAGE_DIVIDER_X + 1.0, g.top()), Pos2::new(g.right() - theme::PAGE_PERCENT_W, g.bottom()));
    let op_txt = fmt_num((bg.a * 100.0).round());
    if let Some(s) = field::edit_text(ui, op_r, op_id, &op_txt, theme::font_body(), theme::TEXT, theme::PAGE_OPACITY_PAD) {
        if let Ok(v) = s.trim().trim_end_matches('%').trim().parse::<f32>() {
            let page = app.page;
            let a = (v / 100.0).clamp(0.0, 1.0);
            app.edit_nodes(&[page], |n| n.kind = NodeKind::Page { background: Color { a, ..bg } });
        }
    }
    ui.painter().text(Pos2::new(g.right() - theme::PAGE_PERCENT_RIGHT, g.center().y), Align2::RIGHT_CENTER, "%", theme::font_body(), theme::TEXT);
    // Visibility toggle: hiding sets the background opacity to 0 and remembers the old value.
    let eye = pane.rect(theme::SECTION_TRAILING_X, theme::PAGE_FIELD_DY, theme::PANEL_ACTION_BUTTON, theme::PANEL_ACTION_BUTTON);
    let hidden = bg.a <= 0.0;
    if widgets::icon_button(
        ui,
        eye,
        "page-visible",
        if hidden { Icon::EyeOff } else { Icon::Eye },
        false,
        if hidden { "Show background" } else { "Hide background" },
    )
    .clicked()
    {
        let page = app.page;
        let a = if hidden { app.props.page_alpha_prev.max(0.01) } else { 0.0 };
        if !hidden {
            app.props.page_alpha_prev = bg.a;
        }
        app.edit_nodes(&[page], |n| n.kind = NodeKind::Page { background: Color { a, ..bg } });
    }
    pane.y += theme::PAGE_FIELD_DY + theme::FIELD_HEIGHT + theme::PAGE_BOTTOM_PAD;
    section_sep(ui, pane, false);
}

fn styles_section(app: &mut App, ui: &mut Ui, pane: &mut Pane) {
    title(ui, pane, "Styles", Some(Icon::Plus), false);
    let before = pane.y;
    styles_list(app, ui, pane, &[]);
    if pane.y > before {
        pane.y += theme::STYLES_BOTTOM_PAD;
    }
    section_sep(ui, pane, false);
    title(ui, pane, "Export", Some(Icon::Plus), true);
    section_sep(ui, pane, false);
}

/// Colours of a paint (one for solids, the stops for gradients).
fn paint_colors(p: &Paint) -> Vec<Color32> {
    match &p.kind {
        PaintKind::Solid(c) => vec![to_c32(*c, p.opacity)],
        PaintKind::LinearGradient { stops, .. }
        | PaintKind::RadialGradient { stops, .. }
        | PaintKind::AngularGradient { stops, .. }
        | PaintKind::DiamondGradient { stops, .. } => stops.iter().map(|g| to_c32(g.color, p.opacity)).collect(),
        PaintKind::Image { .. } => vec![Color32::GRAY],
    }
}

/// Text and colour styles grouped in folders by their "/" prefix. Clicking a
/// row applies the style to `target`.
fn styles_list(app: &mut App, ui: &mut Ui, pane: &mut Pane, target: &[NodeId]) {
    let applied = |app: &App, prop: &str| target.first().and_then(|t| app.doc.get(*t)).and_then(|n| n.style_refs.get(prop).cloned());
    let mut shown_groups = 0;
    for (label, text) in [("Text styles", true), ("Color styles", false)] {
        let entries = crate::styles::style_entries(&app.doc, text);
        if entries.is_empty() {
            continue;
        }
        pane.y += if shown_groups == 0 { theme::STYLES_LABEL_GAP } else { theme::STYLES_GROUP_GAP };
        shown_groups += 1;
        let lr = pane.rect(theme::SECTION_CONTENT_X, 0.0, theme::SECTION_CONTENT_W, theme::SUBLABEL_HEIGHT);
        ui.painter().text(Pos2::new(lr.left(), lr.center().y), Align2::LEFT_CENTER, label, theme::font_body(), theme::TEXT_SECONDARY);
        pane.y += theme::SUBLABEL_HEIGHT + theme::STYLES_ROW_GAP;
        let current = applied(app, if text { "text" } else { "fill" });
        let mut last_folder: Option<String> = None;
        for e in &entries {
            let folder_key = format!("{label}/{}", e.folder);
            if !e.folder.is_empty() && last_folder.as_deref() != Some(e.folder.as_str()) {
                let r = pane.rect(8.0, 0.0, theme::RIGHT_PANEL_WIDTH - 17.0, theme::STYLE_FOLDER_ROW_H);
                widgets::record(ui, &format!("folder:{folder_key}"), r);
                let resp = ui.interact(r, ui.id().with(("sfolder", &folder_key)), Sense::click());
                if resp.hovered() {
                    ui.painter().rect_filled(r, theme::cr(theme::RADIUS_MEDIUM), theme::BG_HOVER);
                }
                let open = !app.props.folded.contains(&folder_key);
                icons::paint(
                    ui.painter(),
                    if open { Icon::ChevronDown } else { Icon::ChevronRight },
                    Pos2::new(r.left() + 14.0, r.center().y),
                    12.0,
                    theme::ICON_TERTIARY,
                );
                ui.painter().text(Pos2::new(r.left() + 28.0, r.center().y), Align2::LEFT_CENTER, &e.folder, theme::font_body(), theme::TEXT_SECONDARY);
                if resp.clicked() {
                    if open {
                        app.props.folded.insert(folder_key.clone());
                    } else {
                        app.props.folded.remove(&folder_key);
                    }
                }
                pane.y += theme::STYLE_FOLDER_ROW_H;
            }
            last_folder = Some(e.folder.clone());
            if !e.folder.is_empty() && app.props.folded.contains(&folder_key) {
                continue;
            }
            let indent = if e.folder.is_empty() { 0.0 } else { theme::STYLE_INDENT };
            let r = pane.rect(8.0, 0.0, theme::RIGHT_PANEL_WIDTH - 17.0, theme::STYLE_ROW_H);
            widgets::record(ui, &format!("style:{}", e.key), r);
            let resp = ui.interact(r, ui.id().with((if text { "tstyle" } else { "cstyle" }, &e.key)), Sense::click());
            let is_current = current.as_deref() == Some(e.key.as_str());
            if is_current {
                ui.painter().rect_filled(r, theme::cr(theme::RADIUS_MEDIUM), theme::BG_SELECTED);
            } else if resp.hovered() {
                ui.painter().rect_filled(r, theme::cr(theme::RADIUS_MEDIUM), theme::BG_HOVER);
            }
            if text {
                let Some(Style::Text(t)) = app.doc.styles.get(&e.key).cloned() else { continue };
                ui.painter().text(Pos2::new(r.left() + 10.0 + indent, r.center().y), Align2::LEFT_CENTER, "Ag", theme::font_body_strong(), theme::TEXT);
                let nm = widgets::elide(ui.painter(), &e.leaf, &theme::font_body(), 110.0 - indent);
                ui.painter().text(Pos2::new(r.left() + 36.0 + indent, r.center().y), Align2::LEFT_CENTER, nm, theme::font_body(), theme::TEXT);
                let lh = match t.line_height {
                    LineHeight::Pixels(p) => fmt_num(p),
                    LineHeight::Percent(p) => fmt_num(t.font_size * p / 100.0),
                    LineHeight::Auto => fmt_num(t.font_size),
                };
                ui.painter().text(
                    Pos2::new(r.right() - 10.0, r.center().y),
                    Align2::RIGHT_CENTER,
                    format!("{}/{}", fmt_num(t.font_size), lh),
                    theme::font_body(),
                    theme::TEXT_TERTIARY,
                );
            } else {
                let Some(Style::Fill(paints)) = app.doc.styles.get(&e.key).cloned() else { continue };
                let c = Pos2::new(r.left() + theme::STYLE_SWATCH_CX + indent, r.center().y);
                let cols = paints.first().map(paint_colors).filter(|c| !c.is_empty()).unwrap_or_else(|| vec![Color32::GRAY]);
                widgets::swatch_circle(ui.painter(), c, theme::STYLE_SWATCH / 2.0, &cols);
                let nm = widgets::elide(ui.painter(), &e.leaf, &theme::font_body(), 130.0 - indent);
                ui.painter().text(Pos2::new(r.left() + theme::STYLE_LABEL_X + indent, r.center().y), Align2::LEFT_CENTER, nm, theme::font_body(), theme::TEXT);
            }
            if resp.clicked() && !target.is_empty() {
                app.apply_style(target, &e.key);
            }
            pane.y += theme::STYLE_ROW_H;
        }
    }
    library_styles_list(app, ui, pane, target);
}

/// Styles of enabled libraries this file has not used yet, one group per
/// library. Clicking one copies it into the file and applies it to `target`.
fn library_styles_list(app: &mut App, ui: &mut Ui, pane: &mut Pane, target: &[NodeId]) {
    let all: Vec<crate::libraries::LibraryStyle> =
        app.library_styles().into_iter().filter(|s| s.local_key.is_none() && matches!(s.style, Style::Text(_) | Style::Fill(_))).collect();
    let mut libs: Vec<(String, String)> = all.iter().map(|s| (s.library_id.clone(), s.library_name.clone())).collect();
    libs.dedup();
    for (lib_id, lib_name) in libs {
        for (label, text) in [("Text styles", true), ("Color styles", false)] {
            let rows: Vec<&crate::libraries::LibraryStyle> = all.iter().filter(|s| s.library_id == lib_id && s.text == text).collect();
            if rows.is_empty() {
                continue;
            }
            pane.y += theme::STYLES_GROUP_GAP;
            let lr = pane.rect(theme::SECTION_CONTENT_X, 0.0, theme::SECTION_CONTENT_W, theme::SUBLABEL_HEIGHT);
            let head = widgets::elide(ui.painter(), &format!("{lib_name} \u{b7} {label}"), &theme::font_body(), lr.width());
            ui.painter().text(Pos2::new(lr.left(), lr.center().y), Align2::LEFT_CENTER, head, theme::font_body(), theme::TEXT_SECONDARY);
            pane.y += theme::SUBLABEL_HEIGHT + theme::STYLES_ROW_GAP;
            for s in rows {
                let r = pane.rect(8.0, 0.0, theme::RIGHT_PANEL_WIDTH - 17.0, theme::STYLE_ROW_H);
                widgets::record(ui, &format!("libstyle:{}", s.name), r);
                let resp = ui.interact(r, ui.id().with(("libstyle", &s.library_id, &s.key)), Sense::click());
                if resp.hovered() {
                    ui.painter().rect_filled(r, theme::cr(theme::RADIUS_MEDIUM), theme::BG_HOVER);
                }
                let (_, leaf) = crate::styles::split_name(&s.name);
                match &s.style {
                    Style::Fill(paints) => {
                        let c = Pos2::new(r.left() + theme::STYLE_SWATCH_CX, r.center().y);
                        let cols = paints.first().map(paint_colors).filter(|c| !c.is_empty()).unwrap_or_else(|| vec![Color32::GRAY]);
                        widgets::swatch_circle(ui.painter(), c, theme::STYLE_SWATCH / 2.0, &cols);
                        let nm = widgets::elide(ui.painter(), &leaf, &theme::font_body(), 130.0);
                        ui.painter().text(Pos2::new(r.left() + theme::STYLE_LABEL_X, r.center().y), Align2::LEFT_CENTER, nm, theme::font_body(), theme::TEXT);
                    }
                    _ => {
                        ui.painter().text(Pos2::new(r.left() + 10.0, r.center().y), Align2::LEFT_CENTER, "Ag", theme::font_body_strong(), theme::TEXT);
                        let nm = widgets::elide(ui.painter(), &leaf, &theme::font_body(), 110.0);
                        ui.painter().text(Pos2::new(r.left() + 36.0, r.center().y), Align2::LEFT_CENTER, nm, theme::font_body(), theme::TEXT);
                    }
                }
                if resp.clicked() && !target.is_empty() {
                    if let Some(key) = app.use_library_style(&s.library_id, &s.key) {
                        app.apply_style(target, &key);
                    }
                }
                pane.y += theme::STYLE_ROW_H;
            }
        }
    }
}

/// Unique colours of the selection subtree; editing one replaces it everywhere.
fn selection_colors_section(app: &mut App, ui: &mut Ui, pane: &mut Pane, sel: &[NodeId]) {
    let cols = crate::styles::selection_colors(&app.doc, sel);
    let nested = sel.iter().any(|s| app.doc.get(*s).map(|n| !n.children.is_empty()).unwrap_or(false));
    if cols.is_empty() || !(cols.len() >= 2 || sel.len() > 1 || nested) {
        return;
    }
    section_sep(ui, pane, false);
    title(ui, pane, "Selection colors", None, false);
    for (i, c) in cols.iter().enumerate() {
        let sw = pane.rect(theme::SWATCH_ROW_X, 4.0, theme::SWATCH_SIZE, theme::SWATCH_SIZE);
        let hex_r = pane.rect(theme::SWATCH_ROW_X + theme::SWATCH_SIZE + 8.0, 0.0, 96.0, theme::FIELD_HEIGHT);
        widgets::record(ui, &format!("selsw:{i}"), sw);
        widgets::record(ui, &format!("selhex:{i}"), hex_r);
        let swr = ui.interact(sw, ui.id().with(("selsw", i)), Sense::click());
        widgets::swatch(ui.painter(), sw, to_c32(*c, 1.0));
        if swr.clicked() {
            let rgb = u32::from_str_radix(&c.to_hex(), 16).unwrap_or(0);
            open_picker(app, PickTarget::SelColor(rgb), to_c32(*c, 1.0), sw, sel.to_vec());
        }
        if let Some(s) = field::text_input(ui, hex_r, ("selhex", i), &c.to_hex()) {
            if let Some(nc) = Color::from_hex(&s) {
                app.replace_selection_color(sel, *c, nc);
            }
        }
        pane.y += theme::FIELD_HEIGHT + 8.0;
    }
    pane.y += 4.0;
}

// ---------------------------------------------------------------- selection
fn selection_ui(app: &mut App, ui: &mut Ui, pane: &mut Pane, sel: &[NodeId]) {
    let Some(n0) = app.doc.get(sel[0]).map(|n| resolve_node(&app.doc, n).into_owned()) else { return };
    let multi = sel.len() > 1;
    let is_text = matches!(n0.kind, NodeKind::Text(_));
    let is_frame = n0.kind.frame_props().is_some();
    let is_line = matches!(n0.kind, NodeKind::Line);
    let parent_al = n0.parent.and_then(|p| app.doc.get(p)).and_then(|p| p.kind.frame_props()).map(|f| f.auto_layout.is_some()).unwrap_or(false);
    let parent_frame = n0.parent.and_then(|p| app.doc.get(p)).map(|p| p.kind.frame_props().is_some()).unwrap_or(false);

    // ---- header: name + quick toggles
    pane.y += theme::RIGHT_SELECTION_HEADER_Y - theme::RIGHT_CONTENT_TOP;
    let name_r = pane.rect(theme::RIGHT_NAME_FIELD_X - 1.0, 0.0, theme::RIGHT_NAME_FIELD_W, theme::FIELD_HEIGHT);
    if multi {
        ui.painter().text(
            Pos2::new(name_r.left() + 8.0, name_r.center().y),
            Align2::LEFT_CENTER,
            format!("{} layers", sel.len()),
            theme::font_large(),
            theme::TEXT,
        );
    } else {
        let id = ui.id().with("sel-name");
        if ui.rect_contains_pointer(name_r) || ui.memory(|m| m.has_focus(id)) {
            let col = if ui.memory(|m| m.has_focus(id)) { theme::BORDER_SELECTED } else { theme::BORDER };
            ui.painter().rect_stroke(name_r, theme::cr(theme::RADIUS_MEDIUM), Stroke::new(1.0, col), StrokeKind::Inside);
        }
        let comp = matches!(n0.kind, NodeKind::Component(_) | NodeKind::ComponentSet(_) | NodeKind::Instance { .. }) || n0.component.source.is_some();
        if let Some(s) = field::edit_text(ui, name_r, id, &n0.name, theme::font_large(), if comp { theme::TEXT_COMPONENT } else { theme::TEXT }, 8.0) {
            app.rename(sel[0], &s);
        }
    }
    let btns = [
        (Icon::Mask, "Use as mask", n0.is_mask),
        (if n0.locked { Icon::Lock } else { Icon::Unlock }, "Lock", n0.locked),
        (if n0.visible { Icon::Eye } else { Icon::EyeOff }, "Hide / show", !n0.visible),
    ];
    for (i, (icon, tip, on)) in btns.iter().enumerate() {
        let r = pane.rect(theme::RIGHT_NAME_ACTION_X[i] - 1.0, 0.0, theme::PANEL_ACTION_BUTTON, theme::PANEL_ACTION_BUTTON);
        if widgets::icon_button(ui, r, ("hdr", i), *icon, *on, tip).clicked() {
            match i {
                0 => app.edit_nodes(sel, |n| n.is_mask = !on),
                1 => app.edit_nodes(sel, |n| n.locked = !on),
                _ => app.edit_nodes(sel, |n| n.visible = !n.visible),
            }
        }
    }
    pane.y += theme::FIELD_HEIGHT + 8.0;
    section_sep(ui, pane, false);
    if !multi {
        super::components::sections(app, ui, pane, sel[0]);
    }
    // ---- Vertex: shown while the vector is in point editing mode
    if !multi && app.canvas.vedit.as_ref().is_some_and(|v| v.id == sel[0]) {
        super::vertex::section(app, ui, pane);
    }

    // ---- Position
    title(ui, pane, "Position", None, false);
    sublabel(ui, pane, "Alignment");
    let h = [Icon::AlignLeft, Icon::AlignHCenter, Icon::AlignRight];
    let v = [Icon::AlignTop, Icon::AlignVCenter, Icon::AlignBottom];
    if let Some(i) = segmented(ui, pane, theme::SECTION_CONTENT_X, "alignh", &h, None) {
        app.align([Align::Left, Align::HCenter, Align::Right][i]);
    }
    if let Some(i) = segmented(ui, pane, theme::SECTION_CONTENT_X + 3.0 * theme::SEGMENT_W + theme::SEGMENT_GROUP_GAP, "alignv", &v, None) {
        app.align([Align::Top, Align::VCenter, Align::Bottom][i]);
    }
    let dist_r = pane.rect(theme::DISTRIBUTE_BTN_X, 0.0, theme::PANEL_ACTION_BUTTON, theme::PANEL_ACTION_BUTTON);
    widgets::record(ui, "distribute", dist_r);
    if widgets::icon_button(ui, dist_r, "distribute", Icon::Distribute, false, "Distribute spacing").clicked() {
        app.distribute_selection();
    }
    row_end(pane);
    sublabel(ui, pane, "Position");
    let xs = common(app, sel, |n| n.transform.tx);
    let ys = common(app, sel, |n| n.transform.ty);
    if let Some(x) = field::num_field(ui, pane.col(0, 0.0), "x", Prefix::Text("X"), xs, "") {
        app.edit_nodes(sel, |n| n.transform.tx = x);
    }
    if let Some(y) = field::num_field(ui, pane.col(1, 0.0), "y", Prefix::Text("Y"), ys, "") {
        app.edit_nodes(sel, |n| n.transform.ty = y);
    }
    row_end(pane);
    sublabel(ui, pane, "Rotation");
    let rot = common(app, sel, |n| (-n.transform.rotation_rad().to_degrees() * 100.0).round() / 100.0);
    if let Some(deg) = field::num_field(ui, pane.col(0, 0.0), "rot", Prefix::Icon(Icon::Rotation), rot, "°") {
        if multi {
            // A group rotates as one body around its bounding-box centre.
            app.rotate_selection(-(deg - rot.unwrap_or(0.0)).to_radians());
        } else {
            let rad = -deg.to_radians();
            app.edit_nodes(sel, |n| n.transform = crate::canvas::handles::rotate_about_center(&n.transform, n.size, rad));
        }
    }
    for i in 0..3 {
        widgets::record(ui, &format!("xform:{i}"), pane.rect(theme::FIELD_COL_X[1] + i as f32 * theme::SEGMENT_W, 0.0, theme::SEGMENT_W, theme::FIELD_HEIGHT));
    }
    if let Some(i) = segmented(ui, pane, theme::FIELD_COL_X[1], "xform", &[Icon::Rotate90, Icon::FlipH, Icon::FlipV], None) {
        match i {
            0 => app.rotate_selection(-std::f32::consts::FRAC_PI_2),
            1 => app.flip_selection(true),
            _ => app.flip_selection(false),
        }
    }
    row_end(pane);
    pane.y += 4.0;

    let al_frame = is_frame && !multi && n0.kind.frame_props().map(|f| f.auto_layout.is_some()).unwrap_or(false);

    // ---- Auto layout (carries the dimensions of auto-layout frames)
    if is_frame && !multi {
        section_sep(ui, pane, false);
        let al = n0.kind.frame_props().and_then(|f| f.auto_layout);
        if title(ui, pane, "Auto layout", Some(if al.is_some() { Icon::Minus } else { Icon::Plus }), al.is_none()) {
            app.toggle_auto_layout();
        }
        if let Some(a) = al {
            auto_layout_ui(app, ui, pane, sel, a, &n0, parent_al);
        }
    }

    // ---- Layout: dimensions, resizing and constraints of everything else
    if !al_frame {
        section_sep(ui, pane, false);
        title(ui, pane, "Layout", None, false);
        dimensions_ui(app, ui, pane, sel, is_line);
        if parent_al || is_text {
            resizing_ui(app, ui, pane, sel, &n0);
        }
        if parent_frame && !parent_al && !multi {
            sublabel(ui, pane, "Constraints");
            let hn = ["Left", "Right", "Left & Right", "Center", "Scale"].map(String::from);
            let vn = ["Top", "Bottom", "Top & Bottom", "Center", "Scale"].map(String::from);
            let order = [Constraint::Min, Constraint::Max, Constraint::Stretch, Constraint::Center, Constraint::Scale];
            let hi = order.iter().position(|c| *c == n0.constraints.horizontal).unwrap_or(0);
            let vi = order.iter().position(|c| *c == n0.constraints.vertical).unwrap_or(0);
            if let Some(i) = dropdown(ui, pane.col(0, 0.0), "con-h", &hn[hi], &hn) {
                app.edit_nodes(sel, |n| n.constraints.horizontal = order[i]);
            }
            if let Some(i) = dropdown(ui, pane.col(1, 0.0), "con-v", &vn[vi], &vn) {
                app.edit_nodes(sel, |n| n.constraints.vertical = order[i]);
            }
            row_end(pane);
        }
        pane.y += 4.0;
    }

    // ---- Typography
    if let NodeKind::Text(t) = &n0.kind {
        section_sep(ui, pane, false);
        title(ui, pane, "Typography", None, false);
        super::typography::section(app, ui, pane, sel, t);
        pane.y += 4.0;
    }

    // ---- Appearance
    section_sep(ui, pane, false);
    title(ui, pane, "Appearance", None, false);
    let op = common(app, sel, |n| (n.opacity * 100.0).round());
    let op_r = pane.col(0, 0.0);
    if let Some(v) = var_bind::numeric(app, ui, op_r, sel, &["opacity"], |ui| field::num_field(ui, op_r, "opacity", Prefix::Icon(Icon::Opacity), op, "%")) {
        let o = (v / 100.0).clamp(0.0, 1.0);
        app.edit_nodes(sel, |n| n.opacity = o);
    }
    let has_radius = !matches!(n0.kind, NodeKind::Text(_) | NodeKind::Line | NodeKind::Group | NodeKind::Vector { .. });
    if has_radius {
        let rad = common(app, sel, |n| n.corner_radii[0]);
        let rad_r = pane.col(1, 0.0);
        let radius_keys = ["topLeftRadius", "topRightRadius", "bottomRightRadius", "bottomLeftRadius"];
        if let Some(v) = var_bind::numeric(app, ui, rad_r, sel, &radius_keys, |ui| field::num_field(ui, rad_r, "radius", Prefix::Icon(Icon::Radius), rad, "")) {
            let v = v.max(0.0);
            app.edit_nodes(sel, |n| n.corner_radii = [v; 4]);
        }
    }
    row_end(pane);
    if is_frame {
        let clip = n0.kind.frame_props().map(|f| f.clips_content).unwrap_or(false);
        if let Some(v) = checkbox(ui, pane, "clip", "Clip content", clip) {
            app.edit_nodes(sel, |n| {
                if let Some(f) = n.kind.frame_props_mut() {
                    f.clips_content = v;
                }
            });
        }
    }
    if is_frame {
        var_bind::mode_rows(app, ui, pane, sel);
    }
    pane.y += 4.0;

    // ---- Fill
    if matches!(n0.kind, NodeKind::Text(_)) {
        // Text colour lives in the text style, not in `Node::fills`.
        section_sep(ui, pane, false);
        title(ui, pane, "Fill", None, false);
        super::typography::fill_section(app, ui, pane, sel);
    } else if !is_line {
        section_sep(ui, pane, false);
        if paint_title(app, ui, pane, sel, "Fill", false, n0.fills.is_empty()) {
            let paint = Paint::solid(Color::from_hex(&format!("{:06X}", theme::DEFAULT_SHAPE_FILL)).unwrap_or(Color::WHITE));
            app.edit_nodes(sel, |n| n.fills.push(paint.clone()));
        }
        paint_rows(app, ui, pane, sel, &n0.fills, false);
        if !n0.fills.is_empty() {
            // 16px under the last row (8 of it came with the row).
            pane.y += 8.0;
        }
    }

    // ---- Stroke
    section_sep(ui, pane, false);
    super::stroke::section(app, ui, pane, sel, &n0);

    // ---- Effects
    section_sep(ui, pane, false);
    super::effects::section(app, ui, pane, sel, &n0.effects);

    // ---- Layout guide (frames)
    if is_frame && !multi {
        if let Some(fp) = n0.kind.frame_props() {
            super::layoutguide::section(app, ui, pane, sel, &fp.grids);
        }
    }

    // ---- Selection colors / Styles
    selection_colors_section(app, ui, pane, sel);
    if !app.doc.styles.is_empty() {
        section_sep(ui, pane, false);
        title(ui, pane, "Styles", None, false);
        styles_list(app, ui, pane, sel);
    }

    // ---- Export
    section_sep(ui, pane, false);
    pane.y += super::export::section(app, ui, pane.x0, pane.oy + pane.y, sel);
}

fn sizing_of(n: &Node, horizontal: bool) -> Sizing {
    if let Some(al) = n.kind.frame_props().and_then(|f| f.auto_layout) {
        let primary_h = al.direction == LayoutDirection::Horizontal;
        let s = if horizontal == primary_h { al.primary_sizing } else { al.counter_sizing };
        if s == Sizing::Hug {
            return Sizing::Hug;
        }
    }
    if let NodeKind::Text(t) = &n.kind {
        if t.auto_resize == TextAutoResize::WidthAndHeight || (t.auto_resize == TextAutoResize::Height && !horizontal) {
            return Sizing::Hug;
        }
    }
    if horizontal {
        n.layout_child.horizontal
    } else {
        n.layout_child.vertical
    }
}

fn set_sizing(app: &mut App, sel: &[NodeId], horizontal: bool, s: Sizing) {
    app.edit_nodes(sel, |n| {
        if s != Sizing::Hug {
            if horizontal {
                n.layout_child.horizontal = s;
            } else {
                n.layout_child.vertical = s;
            }
        } else if horizontal {
            n.layout_child.horizontal = Sizing::Fixed;
        } else {
            n.layout_child.vertical = Sizing::Fixed;
        }
        if let Some(al) = n.kind.frame_props_mut().and_then(|f| f.auto_layout.as_mut()) {
            let primary_h = al.direction == LayoutDirection::Horizontal;
            let target = if horizontal == primary_h { &mut al.primary_sizing } else { &mut al.counter_sizing };
            *target = if s == Sizing::Hug { Sizing::Hug } else { Sizing::Fixed };
        }
        if let NodeKind::Text(t) = &mut n.kind {
            if s == Sizing::Hug {
                t.auto_resize = if horizontal { TextAutoResize::WidthAndHeight } else { TextAutoResize::Height };
            } else if t.auto_resize != TextAutoResize::None {
                t.auto_resize = TextAutoResize::None;
            }
            // Hug takes effect now, not at the next text edit (#71).
            n.size = crate::layout::fit_text(t, n.size);
        }
    });
}

fn resize_selection(app: &mut App, sel: &[NodeId], w: Option<f32>, h: Option<f32>) {
    let mut tx = Tx::new();
    for id in sel {
        let Some(n) = app.doc.get(*id) else { continue };
        let size = v2(w.unwrap_or(n.size.x).max(1.0), h.unwrap_or(n.size.y).max(if matches!(n.kind, NodeKind::Line) { 0.0 } else { 1.0 }));
        let t = n.transform;
        apply_resize(&mut app.doc, &mut tx.ed, *id, size, t, true);
    }
    app.run_tx(tx);
}

fn dimensions_ui(app: &mut App, ui: &mut Ui, pane: &mut Pane, sel: &[NodeId], is_line: bool) {
    sublabel(ui, pane, "Dimensions");
    let ws = common(app, sel, |n| n.size.x);
    let hs = common(app, sel, |n| n.size.y);
    let (wr, hr) = (pane.col(0, 0.0), pane.col(1, 0.0));
    let w_new = var_bind::numeric(app, ui, wr, sel, &["width"], |ui| field::num_field(ui, wr, "w", Prefix::Text("W"), ws, ""));
    let h_new = if is_line { None } else { var_bind::numeric(app, ui, hr, sel, &["height"], |ui| field::num_field(ui, hr, "h", Prefix::Text("H"), hs, "")) };
    if w_new.is_some() || h_new.is_some() {
        resize_selection(app, sel, w_new, h_new);
    }
    row_end(pane);
}

fn resizing_ui(app: &mut App, ui: &mut Ui, pane: &mut Pane, sel: &[NodeId], n0: &Node) {
    sublabel(ui, pane, "Resizing");
    let opts = ["Fixed", "Hug", "Fill"].map(String::from);
    let order = [Sizing::Fixed, Sizing::Hug, Sizing::Fill];
    let (hs, vs) = (sizing_of(n0, true), sizing_of(n0, false));
    if let Some(i) = dropdown(ui, pane.col(0, 0.0), "rs-h", &format!("W: {}", opts[order.iter().position(|s| *s == hs).unwrap_or(0)]), &opts) {
        set_sizing(app, sel, true, order[i]);
    }
    if let Some(i) = dropdown(ui, pane.col(1, 0.0), "rs-v", &format!("H: {}", opts[order.iter().position(|s| *s == vs).unwrap_or(0)]), &opts) {
        set_sizing(app, sel, false, order[i]);
    }
    row_end(pane);
}

fn auto_layout_ui(app: &mut App, ui: &mut Ui, pane: &mut Pane, sel: &[NodeId], a: AutoLayout, n0: &Node, parent_al: bool) {
    sublabel(ui, pane, "Flow");
    let dir_sel = if a.wrap {
        2
    } else if a.direction == LayoutDirection::Horizontal {
        0
    } else {
        1
    };
    if let Some(i) = segmented(ui, pane, theme::SECTION_CONTENT_X, "al-dir", &[Icon::DirHorizontal, Icon::DirVertical, Icon::Wrap], Some(dir_sel)) {
        update_al(app, sel, |al| match i {
            0 => {
                al.direction = LayoutDirection::Horizontal;
                al.wrap = false;
            }
            1 => {
                al.direction = LayoutDirection::Vertical;
                al.wrap = false;
            }
            _ => {
                al.wrap = true;
                al.primary_sizing = Sizing::Fixed;
            }
        });
    }
    let gap_r = pane.col(1, 0.0);
    if let Some(v) = var_bind::numeric(app, ui, gap_r, sel, &["itemSpacing"], |ui| {
        field::num_field(ui, gap_r, "al-gap", Prefix::Icon(Icon::Spacing), Some(a.item_spacing), "")
    }) {
        update_al(app, sel, |al| al.item_spacing = v);
    }
    row_end(pane);
    dimensions_ui(app, ui, pane, sel, false);
    let _ = parent_al;
    resizing_ui(app, ui, pane, sel, n0);
    sublabel(ui, pane, "Padding");
    let ph = a.padding[1].max(a.padding[3]);
    let pv = a.padding[0].max(a.padding[2]);
    let ph_r = pane.col(0, 0.0);
    if let Some(v) = var_bind::numeric(app, ui, ph_r, sel, &["paddingLeft", "paddingRight"], |ui| {
        field::num_field(ui, ph_r, "al-ph", Prefix::Icon(Icon::PaddingH), Some(ph), "")
    }) {
        update_al(app, sel, |al| {
            al.padding[1] = v;
            al.padding[3] = v;
        });
    }
    let pv_r = pane.col(1, 0.0);
    if let Some(v) = var_bind::numeric(app, ui, pv_r, sel, &["paddingTop", "paddingBottom"], |ui| {
        field::num_field(ui, pv_r, "al-pv", Prefix::Icon(Icon::PaddingV), Some(pv), "")
    }) {
        update_al(app, sel, |al| {
            al.padding[0] = v;
            al.padding[2] = v;
        });
    }
    row_end(pane);
    sublabel(ui, pane, "Alignment");
    let pi = match a.primary_align {
        LayoutAlign::Start | LayoutAlign::Baseline => 0,
        LayoutAlign::Center => 1,
        LayoutAlign::End => 2,
        LayoutAlign::SpaceBetween => 3,
    };
    let ci = match a.counter_align {
        LayoutAlign::Center => 1,
        LayoutAlign::End => 2,
        _ => 0,
    };
    let al_icons = [Icon::AlignLeft, Icon::AlignHCenter, Icon::AlignRight];
    if let Some(i) = segmented(ui, pane, theme::SECTION_CONTENT_X, "al-main", &al_icons, Some(pi.min(2))) {
        update_al(app, sel, |al| al.primary_align = [LayoutAlign::Start, LayoutAlign::Center, LayoutAlign::End][i]);
    }
    let sb = pane.rect(theme::SECTION_CONTENT_X + 3.0 * theme::SEGMENT_W + 4.0, 0.0, 24.0, theme::FIELD_HEIGHT);
    let sbr = widgets::icon_button(ui, sb, "al-space", Icon::Spacing, pi == 3, "Space between");
    if sbr.clicked() {
        update_al(app, sel, |al| al.primary_align = if pi == 3 { LayoutAlign::Start } else { LayoutAlign::SpaceBetween });
    }
    let v_icons = [Icon::AlignTop, Icon::AlignVCenter, Icon::AlignBottom];
    if let Some(i) = segmented(ui, pane, theme::FIELD_COL_X[1] + 16.0, "al-cross", &v_icons, Some(ci)) {
        update_al(app, sel, |al| al.counter_align = [LayoutAlign::Start, LayoutAlign::Center, LayoutAlign::End][i]);
    }
    row_end(pane);
    pane.y += 4.0;
}

fn update_al(app: &mut App, sel: &[NodeId], f: impl Fn(&mut AutoLayout)) {
    app.edit_nodes(sel, |n| {
        if let Some(al) = n.kind.frame_props_mut().and_then(|f| f.auto_layout.as_mut()) {
            f(al);
        }
    });
}

// ---------------------------------------------------------------- paints
fn paint_label(p: &Paint) -> String {
    match &p.kind {
        PaintKind::Solid(c) => c.to_hex(),
        PaintKind::LinearGradient { .. } => "Linear".into(),
        PaintKind::RadialGradient { .. } => "Radial".into(),
        PaintKind::AngularGradient { .. } => "Angular".into(),
        PaintKind::DiamondGradient { .. } => "Diamond".into(),
        PaintKind::Image { .. } => "Image".into(),
    }
}

fn paint_preview(p: &Paint) -> Color32 {
    match &p.kind {
        PaintKind::Solid(c) => to_c32(*c, p.opacity),
        PaintKind::LinearGradient { stops, .. }
        | PaintKind::RadialGradient { stops, .. }
        | PaintKind::AngularGradient { stops, .. }
        | PaintKind::DiamondGradient { stops, .. } => stops.first().map(|s| to_c32(s.color, p.opacity)).unwrap_or(Color32::GRAY),
        PaintKind::Image { .. } => Color32::from_gray(150),
    }
}

fn edit_paint_list(app: &mut App, sel: &[NodeId], stroke: bool, f: impl Fn(&mut Vec<Paint>)) {
    app.edit_nodes(sel, |n| {
        // Hand-editing a paint detaches it from its shared style.
        n.style_refs.remove(if stroke { "stroke" } else { "fill" });
        if stroke {
            f(&mut n.stroke.paints)
        } else {
            f(&mut n.fills)
        }
    });
}

/// Width of the swatch + hex + opacity field group of a paint row (the eye and minus follow it).
const PAINT_GROUP_W: f32 = 159.0;
/// x of the divider between the hex and opacity cells, from the group's left edge.
const PAINT_DIVIDER_X: f32 = 104.0;

pub(crate) fn paint_rows(app: &mut App, ui: &mut Ui, pane: &mut Pane, sel: &[NodeId], paints: &[Paint], stroke: bool) {
    let linked: Option<String> = sel
        .first()
        .and_then(|s| app.doc.get(*s))
        .and_then(|n| n.style_refs.get(if stroke { "stroke" } else { "fill" }))
        .map(|k| app.doc.style_name(k).to_string());
    // Motion mode: the first solid paint carries the colour keyframe diamond (the animated paint
    // of FillColor / StrokeColor), so the rows are narrower to make room for it.
    let anim_prop = if stroke { desigma_doc::motion::AnimProperty::StrokeColor } else { desigma_doc::motion::AnimProperty::FillColor };
    let motion = app.motion.enabled && linked.is_none();
    let anim_row = if motion { paints.iter().position(|p| matches!(p.kind, PaintKind::Solid(_))) } else { None };
    let (group_w, divider_x) = if motion { (134.0, 72.0) } else { (PAINT_GROUP_W, PAINT_DIVIDER_X) };
    if !paints.is_empty() {
        // Figma sits the first row 4px under the title row.
        pane.y += 4.0;
    }
    for (i, p) in paints.iter().enumerate() {
        let key = (stroke, i);
        let animated = anim_row == Some(i);
        let tracked = animated && sel.iter().any(|id| app.doc.get(*id).is_some_and(|n| n.motion.track(anim_prop).is_some()));
        // the animated paint shows its colour at the playhead
        let shown_paint;
        let p = match crate::motion::inspector::shown(app, sel, anim_prop).first().and_then(|v| v.color()).filter(|_| animated) {
            Some(c) => {
                shown_paint = Paint { kind: PaintKind::Solid(c), ..p.clone() };
                &shown_paint
            }
            None => p,
        };
        let g = pane.rect(theme::SWATCH_ROW_X, 0.0, group_w, theme::FIELD_HEIGHT);
        let sw = Rect::from_min_size(Pos2::new(g.left() + 4.0, g.center().y - theme::SWATCH_SIZE / 2.0), Vec2::splat(theme::SWATCH_SIZE));
        let hex_r =
            Rect::from_min_max(Pos2::new(g.left() + theme::FIELD_PREFIX_X + theme::FIELD_PREFIX_ICON, g.top()), Pos2::new(g.left() + divider_x, g.bottom()));
        let op_r = Rect::from_min_max(Pos2::new(g.left() + divider_x + 1.0, g.top()), Pos2::new(g.right() - 16.0, g.bottom()));
        let eye_r = pane.rect(theme::RIGHT_NAME_ACTION_X[1] - 1.0 + 4.0, 0.0, theme::PANEL_ACTION_BUTTON, theme::PANEL_ACTION_BUTTON);
        let minus_r = pane.rect(theme::SECTION_TRAILING_X, 0.0, theme::PANEL_ACTION_BUTTON, theme::PANEL_ACTION_BUTTON);
        widgets::record(ui, &format!("sw:{stroke}:{i}"), sw);
        widgets::record(ui, &format!("hex:{stroke}:{i}"), hex_r);
        // Group chrome: one rounded field with a hairline between hex and opacity.
        let hex_id = ui.id().with(("txt", ("hex", key)));
        let op_id = ui.id().with(("num", ("op", key)));
        let focused = ui.memory(|m| m.has_focus(hex_id) || m.has_focus(op_id));
        {
            let pt = ui.painter();
            pt.rect_filled(g, theme::cr(theme::RADIUS_MEDIUM), theme::BG_SECONDARY);
            if focused {
                pt.rect_stroke(g, theme::cr(theme::RADIUS_MEDIUM), Stroke::new(1.0, theme::BORDER_SELECTED), StrokeKind::Inside);
            } else if ui.rect_contains_pointer(g) {
                pt.rect_stroke(g, theme::cr(theme::RADIUS_MEDIUM), Stroke::new(1.0, theme::BORDER), StrokeKind::Inside);
            }
        }
        let swr = ui.interact(sw.expand(2.0), ui.id().with(("sw", key)), Sense::click());
        widgets::picker::paint_swatch(ui.painter(), sw, p);
        let var_key = format!("{}/{i}", if stroke { "strokes" } else { "fills" });
        let bound = var_bind::pill_field(app, ui, Rect::from_min_max(hex_r.min, Pos2::new(g.right(), hex_r.bottom())), sel, &[var_key.as_str()]);
        if !bound {
            widgets::vline(ui.painter(), g.left() + divider_x, g.top(), g.bottom(), theme::BG);
        }
        if swr.clicked() && !bound && !tracked {
            let target = if stroke { PickTarget::Stroke(i) } else { PickTarget::Fill(i) };
            open_picker(app, target, paint_preview(p), sw, sel.to_vec());
        }
        let font = theme::font_body();
        match &p.kind {
            _ if bound => {}
            _ if linked.is_some() => {
                widgets::field_text(ui.painter(), hex_r, 0.0, 4.0, linked.as_deref().unwrap_or(""), font.clone(), theme::TEXT);
            }
            PaintKind::Solid(c) => {
                if let Some(s) = field::edit_text(ui, hex_r, hex_id, &c.to_hex(), font.clone(), theme::TEXT, 0.0) {
                    if let Some(nc) = Color::from_hex(&s) {
                        if animated {
                            crate::motion::inspector::edit_prop(app, sel, anim_prop, |cur| {
                                desigma_doc::motion::AnimValue::Color(Color { a: cur.color().map_or(c.a, |o| o.a), ..nc })
                            });
                        } else {
                            edit_paint_list(app, sel, stroke, |ps| {
                                if let Some(p) = ps.get_mut(i) {
                                    p.kind = PaintKind::Solid(Color { a: c.a, ..nc });
                                }
                            });
                        }
                    }
                }
            }
            _ => {
                widgets::field_text(ui.painter(), hex_r, 0.0, 4.0, &paint_label(p), font.clone(), theme::TEXT);
            }
        }
        if !bound && !motion && matches!(p.kind, PaintKind::Solid(_)) {
            var_bind::hover_button(app, ui, hex_r, sel, &[var_key.as_str()], VariableKind::Color);
        }
        if !bound {
            // Opacity: "100" then a separate "%" in text_secondary.
            let pct = fmt_num((p.opacity * 100.0).round());
            if let Some(s) = field::edit_text(ui, op_r, op_id, &pct, font.clone(), theme::TEXT, 8.0) {
                if let Some(v) = field::parse_number(&s) {
                    let o = (v / 100.0).clamp(0.0, 1.0);
                    edit_paint_list(app, sel, stroke, |ps| {
                        if let Some(p) = ps.get_mut(i) {
                            p.opacity = o;
                        }
                    });
                }
            }
            let pct_x = g.right() - 8.0 - widgets::text_width(ui.painter(), "%", &font);
            widgets::text_on_baseline(ui.painter(), pct_x, widgets::centered_baseline(g, &font), "%", font, theme::TEXT_SECONDARY);
        }
        if widgets::icon_button(ui, eye_r, ("peye", key), if p.visible { Icon::Eye } else { Icon::EyeOff }, false, "").clicked() {
            edit_paint_list(app, sel, stroke, |ps| {
                if let Some(p) = ps.get_mut(i) {
                    p.visible = !p.visible;
                }
            });
        }
        if widgets::icon_button(ui, minus_r, ("pminus", key), Icon::Minus, false, "").clicked() {
            edit_paint_list(app, sel, stroke, |ps| {
                if i < ps.len() {
                    ps.remove(i);
                }
            });
        }
        if animated {
            let d_r = pane.rect(theme::SWATCH_ROW_X + 138.0, 0.0, 24.0, 24.0);
            crate::motion::inspector::diamond_for(app, ui, sel, anim_prop, d_r);
        }
        pane.y += theme::FIELD_HEIGHT + 8.0;
    }
}

// ---------------------------------------------------------------- color picker
fn node_paint(app: &App, target: PickTarget, ids: &[NodeId]) -> Option<Paint> {
    let n = app.doc.get(*ids.first()?)?;
    match target {
        PickTarget::Fill(i) => n.fills.get(i).cloned(),
        PickTarget::Stroke(i) => n.stroke.paints.get(i).cloned(),
        _ => None,
    }
}

/// Colours for the picker's swatch grid: this page, the whole document and colour styles.
fn swatch_sources(app: &App) -> Vec<(String, Vec<Color32>)> {
    let conv = |cs: Vec<Color>| cs.into_iter().map(|c| to_c32(c, 1.0)).collect::<Vec<_>>();
    let page = conv(crate::styles::selection_colors(&app.doc, &[app.page]));
    let doc = conv(crate::styles::selection_colors(&app.doc, &app.doc.pages.clone()));
    let styles: Vec<Color32> = crate::styles::style_entries(&app.doc, false)
        .iter()
        .filter_map(|e| match app.doc.styles.get(&e.key) {
            Some(Style::Fill(ps)) => ps.first().map(widgets::picker::preview_color),
            _ => None,
        })
        .collect();
    let mut out = vec![("On this page".to_string(), page), ("Document colors".to_string(), doc)];
    if !styles.is_empty() {
        out.push(("Color styles".to_string(), styles));
    }
    out
}

pub(crate) fn open_picker(app: &mut App, target: PickTarget, color: Color32, anchor: Rect, ids: Vec<NodeId>) {
    let paint = node_paint(app, target, &ids).unwrap_or_else(|| {
        let [r, g, b, a] = color.to_srgba_unmultiplied();
        Paint { opacity: a as f32 / 255.0, ..Paint::solid(Color::rgba(r, g, b, 255)) }
    });
    let state = widgets::picker::State::new(&paint);
    let sources = swatch_sources(app);
    app.props.picker = Some(Picker { target, tx: Tx::new(), anchor, ids, paint, state, sources, eyedrop: false });
}

/// Solid colour (alpha included) the picker currently holds, for the solid-only targets.
fn picker_color(p: &Picker) -> Color {
    let [r, g, b, a] = widgets::picker::primary_rgba(&p.paint);
    Color::rgba(r, g, b, a)
}

fn apply_picker(doc: &mut Document, p: &mut Picker) {
    let c = picker_color(p);
    if let PickTarget::SelColor(old) = p.target {
        if let Some(old) = Color::from_hex(&format!("{old:06X}")) {
            crate::styles::replace_color(doc, &mut p.tx.ed, &p.ids, old, Color { a: 1.0, ..c });
        }
        return;
    }
    for id in &p.ids {
        let Some(n) = p.tx.ed.node(doc, *id) else { continue };
        match p.target {
            PickTarget::Fill(i) => {
                if let Some(pt) = n.fills.get_mut(i) {
                    let visible = pt.visible;
                    *pt = Paint { visible, ..p.paint.clone() };
                }
            }
            PickTarget::Stroke(i) => {
                if let Some(pt) = n.stroke.paints.get_mut(i) {
                    let visible = pt.visible;
                    *pt = Paint { visible, ..p.paint.clone() };
                }
            }
            PickTarget::SelColor(_) => {}
            PickTarget::GridColor(i) => {
                if let Some(fp) = n.kind.frame_props_mut() {
                    if let Some(g) = fp.grids.get_mut(i) {
                        *g = super::layoutguide::with_color(g, c);
                    }
                }
            }
            PickTarget::TextFill => {
                if let NodeKind::Text(t) = &mut n.kind {
                    let col = Color { a: 1.0, ..c };
                    let op = c.a;
                    crate::richtext::apply_all(t, |st| super::typography::set_solid(st, col, Some(op)));
                }
            }
            PickTarget::PageBg => n.kind = NodeKind::Page { background: c },
            PickTarget::Shadow(i) => {
                if let Some(e) = n.effects.get_mut(i) {
                    super::effects::set_shadow(e, |col, _, _, _| *col = c);
                }
            }
            PickTarget::NoiseColor(i, second) => {
                if let Some(e) = n.effects.get_mut(i) {
                    super::effects::set_noise_color(e, second, c);
                }
            }
        }
    }
}

/// Reads the page colour under `pos` (screen space) by rendering a 1x1 pixel of the page.
fn sample_canvas(app: &App, pos: Pos2) -> Option<[u8; 3]> {
    use desigma_render::tiny_skia::Pixmap;
    let p = app.view.to_page(app.canvas_rect.min, pos);
    let vp = desigma_render::Viewport { offset: DVec2::new(p.x - 0.5, p.y - 0.5), zoom: 1.0, dpr: 1.0 };
    let mut pm = Pixmap::new(1, 1)?;
    desigma_render::render_page(&app.doc, app.page, &vp, &mut pm);
    let px = pm.pixel(0, 0)?.demultiply();
    Some([px.red(), px.green(), px.blue()])
}

/// Creates a colour style from the picker's paint and links it to the edited layers.
fn create_style(app: &mut App, pk: &mut Picker) {
    let stroke = matches!(pk.target, PickTarget::Stroke(_));
    let (key, n) = app.doc.fresh_style_key("Color style");
    let name = format!("Color style {n}");
    // Creating the style and linking it to the layers is one undo step (the picker's transaction).
    let mut styles = app.doc.styles.clone();
    let mut style_meta = app.doc.style_meta.clone();
    styles.insert(key.clone(), Style::Fill(vec![Paint { visible: true, ..pk.paint.clone() }]));
    style_meta.insert(
        key.clone(),
        StyleMeta { name, key: None, description: String::new(), property: if stroke { StyleProperty::Stroke } else { StyleProperty::Fill } },
    );
    if let Err(e) = pk.tx.cmd(&mut app.doc, Command::SetStyles { styles, style_meta }) {
        app.show_toast(format!("Edit failed: {e}"), true);
        return;
    }
    for id in &pk.ids {
        if let Some(node) = pk.tx.ed.node(&mut app.doc, *id) {
            node.style_refs.insert(if stroke { "stroke" } else { "fill" }.into(), key.clone());
        }
    }
    app.show_toast(format!("Created \"{key}\""), false);
}

pub(crate) fn picker_popup(app: &mut App, ui: &mut Ui, region: Rect) {
    use widgets::picker;
    let Some(mut pk) = app.props.picker.take() else { return };
    let ctx = ui.ctx().clone();
    let paint_types = matches!(pk.target, PickTarget::Fill(_) | PickTarget::Stroke(_));
    let mut styles: Vec<picker::StyleItem> = if paint_types {
        crate::styles::style_entries(&app.doc, false)
            .into_iter()
            .filter_map(|e| match app.doc.styles.get(&e.key) {
                Some(Style::Fill(ps)) => {
                    ps.first().map(|p| picker::StyleItem { name: app.doc.style_name(&e.key).to_string(), key: e.key.clone(), paint: p.clone(), library: None })
                }
                _ => None,
            })
            .collect()
    } else {
        Vec::new()
    };
    // Colour styles of enabled libraries, grouped per library after the document's own.
    if paint_types && pk.state.tab == picker::Tab::Libraries {
        for ls in app.library_styles().into_iter().filter(|s| !s.text && s.local_key.is_none()) {
            if let Style::Fill(ps) = &ls.style {
                if let Some(p) = ps.first() {
                    styles.push(picker::StyleItem {
                        name: ls.name.clone(),
                        key: ls.key.clone(),
                        paint: p.clone(),
                        library: Some((ls.library_id.clone(), ls.library_name.clone())),
                    });
                }
            }
        }
    }
    let env = picker::Env { paint_types, sources: &pk.sources, styles: &styles, can_add_style: paint_types };
    let h = picker::panel_height(&env, picker::mode_of(&pk.paint.kind), pk.state.tab, pk.state.source);
    let screen = ctx.content_rect();
    let x = (region.left() - picker::W).max(screen.left() + 8.0);
    let y = (pk.anchor.top() - 8.0).min(screen.bottom() - h - 16.0).max(screen.top() + 8.0);
    let mut out = picker::Out::default();
    let mut panel = Rect::NOTHING;
    egui::Area::new(egui::Id::new("color-picker")).order(egui::Order::Foreground).fixed_pos(Pos2::new(x, y)).show(&ctx, |ui| {
        let (r, o) = picker::show(ui, Pos2::new(x, y), &mut pk.state, &mut pk.paint, &env);
        panel = r;
        out = o;
    });
    let mut changed = out.changed;
    if out.eyedrop {
        pk.eyedrop = true;
    }
    if out.want_image {
        if let Some(path) = rfd::FileDialog::new().add_filter("Images", &crate::canvas::interact::IMAGE_EXTENSIONS[..4]).pick_file() {
            match std::fs::read(&path).map_err(|_| "Cannot read image".to_string()).and_then(crate::canvas::interact::prepare_image) {
                Ok((bytes, _, _)) => {
                    let key = format!("img-{}", app.doc.images.len() + 1);
                    app.doc.images.insert(key.clone(), bytes);
                    pk.paint.kind = PaintKind::Image { image_ref: key, scale_mode: ImageScaleMode::Fill };
                    pk.paint.opacity = 1.0;
                    changed = true;
                }
                Err(e) => app.show_toast(e, true),
            }
        }
    }
    // Eyedropper: the next canvas click samples the colour under the pointer.
    if pk.eyedrop {
        ctx.set_cursor_icon(egui::CursorIcon::Crosshair);
        let (clicked, ptr, esc) = ctx.input(|i| (i.pointer.primary_pressed(), i.pointer.interact_pos(), i.key_pressed(egui::Key::Escape)));
        if esc {
            pk.eyedrop = false;
        } else if let (true, Some(p)) = (clicked, ptr) {
            if app.canvas_rect.contains(p) && !panel.contains(p) {
                if let Some([r, g, b]) = sample_canvas(app, p) {
                    let a = widgets::picker::active_rgba(&pk.paint, pk.state.sel)[3];
                    widgets::picker::set_active(&mut pk.paint, pk.state.sel, [r, g, b, a]);
                    changed = true;
                }
                pk.eyedrop = false;
            }
        }
    }
    gradient_handles(app, &ctx, &mut pk, &mut changed);
    if changed {
        if pk.target == PickTarget::TextFill && app.canvas.text.is_some() {
            // Colour of the characters being edited: goes through the text session.
            let c = picker_color(&pk);
            let col = Color { a: 1.0, ..c };
            crate::canvas::textsession::style_selection_as(app, |st| super::typography::set_solid(st, col, Some(c.a)), crate::textedit::EditKind::Style);
        } else {
            pk.tx.ed.revert(&mut app.doc);
            apply_picker(&mut app.doc, &mut pk);
            app.changed();
        }
    }
    if out.add_style {
        create_style(app, &mut pk);
        app.changed();
        out.close = true;
    }
    let mut remove = false;
    if out.remove {
        pk.tx.ed.revert(&mut app.doc);
        remove = true;
    }
    let (pressed, ptr, esc) = ctx.input(|i| (i.pointer.any_pressed(), i.pointer.interact_pos(), i.key_pressed(egui::Key::Escape)));
    let outside =
        !pk.eyedrop && !pk.state.menu_open && pressed && ptr.map(|p| !panel.contains(p) && !pk.anchor.contains(p) && !handle_hit(&pk, app, p)).unwrap_or(false);
    let esc = esc && !pk.eyedrop && !pk.state.menu_open;
    if let Some((lib, key)) = out.apply_library_style.take() {
        // Import the library style and apply it together with the picker's own edits: one undo step.
        let ids = pk.ids.clone();
        app.apply_library_style(pk.tx, &ids, &lib, &key);
        return;
    }
    if outside || esc || out.close || out.apply_style.is_some() || remove {
        let ids = pk.ids.clone();
        let target = pk.target;
        let tx = pk.tx;
        if remove {
            // Drop the paint in the same undo step.
            let mut tx = tx;
            for id in &ids {
                if let Some(n) = tx.ed.node(&mut app.doc, *id) {
                    match target {
                        PickTarget::Fill(i) if i < n.fills.len() => {
                            n.fills.remove(i);
                        }
                        PickTarget::Stroke(i) if i < n.stroke.paints.len() => {
                            n.stroke.paints.remove(i);
                        }
                        _ => {}
                    }
                }
            }
            app.run_tx(tx);
        } else {
            app.run_tx(tx);
        }
        if let Some(key) = out.apply_style {
            app.apply_style(&ids, &key);
        }
    } else {
        app.props.picker = Some(pk);
    }
}

/// Handles of the gradient on the canvas are in screen space; clicks on them must not close the picker.
fn handle_hit(pk: &Picker, app: &App, p: Pos2) -> bool {
    handle_points(pk, app).iter().any(|(_, c)| c.distance(p) < 12.0)
}

/// Screen positions of the three gradient handles of the edited node.
fn handle_points(pk: &Picker, app: &App) -> Vec<(usize, Pos2)> {
    let Some(h) = widgets::picker::handles_of(&pk.paint.kind) else { return Vec::new() };
    let Some(id) = pk.ids.first() else { return Vec::new() };
    let Some(n) = app.doc.get(*id) else { return Vec::new() };
    let w = app.doc.world_transform(*id);
    h.iter()
        .enumerate()
        .map(|(i, v)| {
            let page = w.apply(DVec2::new(v.x * n.size.x, v.y * n.size.y));
            (i, app.view.to_screen(app.canvas_rect.min, page))
        })
        .collect()
}

/// On-canvas gradient editing: the start/end/width handles of the edited paint.
fn gradient_handles(app: &mut App, ctx: &egui::Context, pk: &mut Picker, changed: &mut bool) {
    let pts = handle_points(pk, app);
    if pts.len() != 3 || !matches!(pk.target, PickTarget::Fill(_) | PickTarget::Stroke(_)) {
        return;
    }
    let Some(id) = pk.ids.first().copied() else { return };
    let (Some(n), Some(inv)) = (app.doc.get(id), app.doc.world_transform(id).invert()) else { return };
    let size = n.size;
    let origin = app.canvas_rect.min;
    let view = app.view;
    let canvas = app.canvas_rect;
    let mode = widgets::picker::mode_of(&pk.paint.kind);
    let mut moved: Option<(usize, DVec2)> = None;
    egui::Area::new(egui::Id::new("gradient-handles")).order(egui::Order::Foreground).fixed_pos(canvas.min).interactable(true).show(ctx, |ui| {
        let p = ui.painter().with_clip_rect(canvas);
        let stroke = |w: f32, c: Color32| Stroke::new(w, c);
        p.line_segment([pts[0].1, pts[1].1], stroke(3.0, Color32::from_black_alpha(0x66)));
        p.line_segment([pts[0].1, pts[1].1], stroke(1.5, Color32::WHITE));
        p.line_segment([pts[0].1, pts[2].1], stroke(1.0, Color32::from_white_alpha(0xB2)));
        let stops = widgets::picker::stops_of(&pk.paint.kind).cloned().unwrap_or_default();
        for (i, c) in &pts {
            let r = Rect::from_center_size(*c, Vec2::splat(16.0));
            let resp = ui.interact(r, ui.id().with(("gh", *i)), Sense::drag());
            if resp.dragged() {
                if let Some(pt) = resp.interact_pointer_pos() {
                    let local = inv.apply(view.to_page(origin, pt));
                    moved = Some((*i, DVec2::new(local.x / size.x.max(1e-3), local.y / size.y.max(1e-3))));
                }
            }
            let fill = if *i == 0 {
                stops.first().map(|s| to_c32(s.color, 1.0))
            } else if *i == 1 {
                stops.last().map(|s| to_c32(s.color, 1.0))
            } else {
                None
            };
            let rad = if *i == 2 { 4.0 } else { 6.5 };
            p.circle_filled(*c, rad + 1.5, Color32::from_black_alpha(0x4D));
            p.circle_filled(*c, rad, Color32::WHITE);
            if let Some(f) = fill {
                p.circle_filled(*c, rad - 2.0, f);
            }
        }
        // Stop markers along the axis of a linear gradient.
        if mode == widgets::picker::Mode::Linear {
            for (si, s) in stops.iter().enumerate() {
                let c = pts[0].1 + (pts[1].1 - pts[0].1) * s.pos.clamp(0.0, 1.0);
                let sel = pk.state.sel == si;
                p.rect_filled(Rect::from_center_size(c, Vec2::splat(10.0)), theme::cr(3), if sel { Color32::WHITE } else { Color32::from_white_alpha(0xCC) });
                p.rect_filled(Rect::from_center_size(c, Vec2::splat(6.0)), theme::cr(2), to_c32(s.color, 1.0));
            }
        }
    });
    if let Some((i, v)) = moved {
        let v = DVec2::new(v.x, v.y);
        match &mut pk.paint.kind {
            PaintKind::LinearGradient { handles, .. }
            | PaintKind::RadialGradient { handles, .. }
            | PaintKind::AngularGradient { handles, .. }
            | PaintKind::DiamondGradient { handles, .. } => {
                if i == 0 {
                    // Moving the start/centre drags the whole gradient.
                    let d = DVec2::new(v.x - handles[0].x, v.y - handles[0].y);
                    for h in handles.iter_mut() {
                        *h = DVec2::new(h.x + d.x, h.y + d.y);
                    }
                } else {
                    handles[i] = v;
                }
                *changed = true;
            }
            _ => {}
        }
    }
}

/// Opens an inspector popup on the current selection (`--panel`, for screenshots).
pub fn preset_popup(app: &mut App, spec: &str, screen: Vec2) {
    if spec == "picker-page" {
        // The page background picker, anchored on the Page row swatch.
        let bg = match app.doc.get(app.page).map(|n| &n.kind) {
            Some(NodeKind::Page { background }) => *background,
            _ => Color::WHITE,
        };
        let sw =
            Rect::from_min_size(Pos2::new(screen.x - theme::RIGHT_PANEL_WIDTH + 17.0 + theme::PAGE_SWATCH_CX - 6.0, 131.0), Vec2::splat(theme::PAGE_SWATCH));
        open_picker(app, PickTarget::PageBg, to_c32(bg, 1.0), sw, vec![app.page]);
        return;
    }
    let Some(id) = app.selection.first().copied() else { return };
    let sw = Rect::from_min_size(Pos2::new(screen.x - theme::RIGHT_PANEL_WIDTH + 29.0, 660.0), Vec2::splat(theme::SWATCH_SIZE));
    match spec {
        "picker" | "picker-gradient" => {
            if app.doc.get(id).is_some_and(|n| n.fills.is_empty()) {
                app.edit_nodes(&[id], |n| n.fills.push(Paint::solid(Color::rgb(0xD9, 0xD9, 0xD9))));
            }
            if spec == "picker-gradient" {
                app.edit_nodes(&[id], |n| {
                    if let Some(p) = n.fills.first_mut() {
                        *p = widgets::picker::convert(p, widgets::picker::Mode::Linear);
                    }
                });
            }
            let c = app.doc.get(id).and_then(|n| n.fills.first()).map(paint_preview).unwrap_or(Color32::GRAY);
            open_picker(app, PickTarget::Fill(0), c, sw, vec![id]);
        }
        "fill-d9" => {
            // Screenshot aid: the Figma reference fill (#D9D9D9, 100 %) for crop comparisons.
            app.edit_nodes(&[id], |n| n.fills = vec![Paint::solid(Color::rgb(0xD9, 0xD9, 0xD9))]);
        }
        "effects-menu" => {
            app.props.scroll = 1.0e6;
            app.props.fx_menu = true;
        }
        "stroke-settings" => {
            app.props.scroll = 1.0e6;
            if app.doc.get(id).is_some_and(|n| n.stroke.paints.is_empty()) {
                app.edit_nodes(&[id], |n| {
                    n.stroke.paints.push(Paint::solid(Color::BLACK));
                    n.stroke.weight = 1.0;
                });
            }
            let anchor = Rect::from_min_size(Pos2::new(screen.x - 56.0, 760.0), Vec2::splat(24.0));
            app.props.pop = Some(super::effects::PopState { kind: super::effects::PopKind::Stroke, anchor, ids: vec![id] });
        }
        "type-details" => {
            let anchor = Rect::from_min_size(Pos2::new(screen.x - 48.0, 330.0), Vec2::splat(24.0));
            app.props.pop = Some(super::effects::PopState { kind: super::effects::PopKind::TextDetails, anchor, ids: vec![id] });
        }
        "effect-noise" | "effect-texture" | "effect-glass" => {
            app.props.scroll = 1.0e6;
            let kind = match spec {
                "effect-noise" => 4,
                "effect-texture" => 5,
                _ => 6,
            };
            let e = super::effects::default_effect(kind, 1);
            app.edit_nodes(&[id], |n| {
                n.effects.push(e.clone());
                if kind == 6 {
                    // Glass shows what is behind it: make the fill a faint tint.
                    n.fills = vec![Paint { opacity: 0.12, ..Paint::solid(Color::WHITE) }];
                }
            });
            let i = app.doc.get(id).map(|n| n.effects.len().saturating_sub(1)).unwrap_or(0);
            let anchor = Rect::from_min_size(Pos2::new(screen.x - 80.0, 800.0), Vec2::splat(24.0));
            app.props.pop = Some(super::effects::PopState { kind: super::effects::PopKind::Effect(i), anchor, ids: vec![id] });
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_conversion_roundtrip() {
        let c = Color::rgba(0x12, 0x34, 0x56, 255);
        let back = from_c32(to_c32(c, 1.0));
        assert_eq!(back.to_hex(), c.to_hex());
        let half = to_c32(c, 0.5);
        assert!((half.a() as i32 - 128).abs() <= 1);
    }

    #[test]
    fn sizing_reflects_hug_frames_and_text() {
        let mut n = Node::frame(NodeId(1), desigma_doc::Rect::new(0.0, 0.0, 10.0, 10.0));
        n.kind = NodeKind::Frame(FrameProps { auto_layout: Some(AutoLayout { primary_sizing: Sizing::Hug, ..Default::default() }), ..Default::default() });
        assert_eq!(sizing_of(&n, true), Sizing::Hug);
        assert_eq!(sizing_of(&n, false), Sizing::Fixed);
    }

    /// Switching a text's height to Hug fits the box to its wrapped lines at once (#71).
    #[test]
    fn hug_height_fits_a_text_box_to_its_lines() {
        let mut d = Document::new("t");
        let page = d.add_page("P");
        let id = d.alloc_id();
        let mut t = Node::text(
            id,
            desigma_doc::Rect::new(0.0, 0.0, 120.0, 20.0),
            "one two three four five six seven eight nine ten eleven twelve",
            TextStyle::default(),
        );
        if let NodeKind::Text(p) = &mut t.kind {
            p.auto_resize = TextAutoResize::None;
        }
        let id = d.insert(page, t).unwrap();
        let mut app = App::new(d);
        set_sizing(&mut app, &[id], false, Sizing::Hug);
        let n = app.doc.get(id).unwrap();
        assert_eq!(n.size.x, 120.0, "width stays fixed");
        assert!(n.size.y > 40.0, "height grows to the wrapped lines: {}", n.size.y);
        let hugged = n.size.y;
        set_sizing(&mut app, &[id], false, Sizing::Fixed);
        assert_eq!(app.doc.get(id).unwrap().size.y, hugged, "back to fixed keeps the size");
    }
}
