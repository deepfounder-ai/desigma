//! Inspector sections for components and instances: instance header with the
//! swap picker, component properties, property definitions on main
//! components and binding a layer to a property.

use super::properties::{row_end, section_sep, sublabel, title, Pane};
use crate::app::App;
use crate::icons::Icon;
use crate::theme;
use crate::widgets;
use crate::widgets::field;
use crate::widgets::menu::{dropdown, popup_menu, MenuItem, MenuResult};
use desigma_doc::*;
use egui::{Align2, Pos2, Rect, Sense, Ui, Vec2};

/// Draws the component-related sections for a single selected layer, right
/// below the name header. Each section ends with a separator.
pub(super) fn sections(app: &mut App, ui: &mut Ui, pane: &mut Pane, id: NodeId) {
    let Some(n) = app.doc.get(id).cloned() else { return };
    variant_editor(app, ui, pane);
    match &n.kind {
        NodeKind::Instance { .. } if is_top_instance(&n) => {
            instance_header(app, ui, pane, id);
            instance_props(app, ui, pane, id);
            overrides_row(app, ui, pane, id);
        }
        NodeKind::Component(_) | NodeKind::ComponentSet(_) => component_props(app, ui, pane, id),
        _ => {
            if is_linked(&n) {
                overrides_row(app, ui, pane, id);
            } else {
                layer_bindings(app, ui, pane, id);
            }
        }
    }
}

fn comp_label(doc: &Document, comp: NodeId) -> String {
    let Some(c) = doc.get(comp) else { return "Missing component".into() };
    match c.parent.and_then(|p| doc.get(p)).filter(|p| matches!(p.kind, NodeKind::ComponentSet(_))) {
        Some(set) => format!("{} / {}", set.name, c.name),
        None => c.name.clone(),
    }
}

fn all_components(doc: &Document) -> Vec<NodeId> {
    let mut v: Vec<NodeId> = doc.nodes.values().filter(|n| matches!(n.kind, NodeKind::Component(_))).map(|n| n.id).collect();
    v.sort_by_key(|id| comp_label(doc, *id));
    v
}

/// 28x16 switch; returns the new state when clicked.
fn toggle(ui: &mut Ui, rect: Rect, key: &str, on: bool) -> Option<bool> {
    let r = ui.interact(rect, ui.id().with(("tog", key)), Sense::click());
    widgets::record(ui, &format!("toggle:{key}"), rect);
    let sw = Rect::from_min_size(Pos2::new(rect.left(), rect.center().y - 8.0), Vec2::new(28.0, 16.0));
    let p = ui.painter();
    p.rect_filled(
        sw,
        theme::cr(8),
        if on {
            theme::BG_BRAND
        } else if r.hovered() {
            theme::BG_TERTIARY
        } else {
            theme::BG_SECONDARY
        },
    );
    let kx = if on { sw.right() - 8.0 } else { sw.left() + 8.0 };
    p.circle_filled(Pos2::new(kx, sw.center().y), 6.0, egui::Color32::WHITE);
    r.clicked().then_some(!on)
}

// ---------------------------------------------------------------- instance
fn instance_header(app: &mut App, ui: &mut Ui, pane: &mut Pane, inst: NodeId) {
    let comp = app.main_of(inst);
    let label = comp.map(|c| comp_label(&app.doc, c)).unwrap_or_else(|| "Missing component".into());
    let row_y = 0.0;
    // Swap picker (the component name), "go to main" and the more menu.
    let dd = pane.rect(theme::SECTION_CONTENT_X, row_y, 156.0, theme::FIELD_HEIGHT);
    let comps = all_components(&app.doc);
    let names: Vec<String> = comps.iter().map(|c| comp_label(&app.doc, *c)).collect();
    widgets::record(ui, "instance-swap", dd);
    if let Some(i) = dropdown(ui, dd, "instance-swap", &label, &names) {
        app.swap_instance(inst, comps[i]);
    }
    ui.painter().rect_stroke(
        dd,
        theme::cr(theme::RADIUS_MEDIUM),
        egui::Stroke::new(1.0, theme::CANVAS_COMPONENT.gamma_multiply(0.55)),
        egui::StrokeKind::Inside,
    );
    let go = pane.rect(181.0, row_y, theme::PANEL_ACTION_BUTTON, theme::PANEL_ACTION_BUTTON);
    widgets::record(ui, "go-to-main", go);
    if widgets::icon_button(ui, go, "go-main", Icon::Component, false, "Go to main component").clicked() {
        app.go_to_main(inst);
    }
    let more = pane.rect(theme::SECTION_TRAILING_X, row_y, theme::PANEL_ACTION_BUTTON, theme::PANEL_ACTION_BUTTON);
    widgets::record(ui, "instance-more", more);
    if widgets::icon_button(ui, more, "inst-more", Icon::ChevronDown, app.ui.inst_menu.is_some(), "More").clicked() {
        app.ui.inst_menu = if app.ui.inst_menu.is_some() { None } else { Some(more) };
    }
    pane.y += theme::FIELD_HEIGHT + 8.0;
    if let Some(info) = library_instance_info(&app.doc, inst) {
        let row = pane.rect(theme::SECTION_CONTENT_X, 0.0, theme::SECTION_CONTENT_W, 20.0);
        crate::libraries::instance_library_row(ui, row, &info);
        pane.y += 28.0;
    }
    section_sep(ui, pane, false);

    if let Some(anchor) = app.ui.inst_menu {
        let changed = has_changes(app, inst);
        let items = [
            MenuItem::new("Go to main component").disabled(comp.is_none()),
            MenuItem::new("Reset all changes").disabled(!changed),
            MenuItem::new("Detach instance").shortcut("Ctrl+Alt+B"),
        ];
        match popup_menu(ui.ctx(), egui::Id::new("inst-menu"), anchor.left_bottom() + Vec2::new(-150.0, 2.0), 180.0, &items, Some(anchor)) {
            MenuResult::Picked(i) => {
                app.ui.inst_menu = None;
                match i {
                    0 => app.go_to_main(inst),
                    1 => app.reset_selection(),
                    _ => app.detach_selection(),
                }
            }
            MenuResult::Closed => app.ui.inst_menu = None,
            MenuResult::Open => {}
        }
    }
}

fn has_changes(app: &App, inst: NodeId) -> bool {
    let mut any = false;
    app.doc.walk(inst, |n, _| {
        if !n.component.overrides.is_empty() || (n.id == inst && !n.component.values.is_empty()) {
            any = true;
        }
    });
    any
}

fn instance_props(app: &mut App, ui: &mut Ui, pane: &mut Pane, inst: NodeId) {
    let Some(comp) = app.main_of(inst) else { return };
    let defs = prop_defs(&app.doc, comp);
    if defs.is_empty() {
        return;
    }
    title(ui, pane, "Properties", None, false);
    for def in defs {
        let value = prop_value(&app.doc, inst, &def);
        let lab = pane.rect(theme::SECTION_CONTENT_X, 0.0, 92.0, theme::FIELD_HEIGHT);
        let is_set = app.doc.get(inst).is_some_and(|n| n.component.values.contains_key(&def.name));
        let shown = widgets::elide(ui.painter(), &def.name, &theme::font_body(), lab.width() - 4.0);
        ui.painter().text(
            Pos2::new(lab.left(), lab.center().y),
            Align2::LEFT_CENTER,
            shown,
            theme::font_body(),
            if is_set { theme::TEXT } else { theme::TEXT_SECONDARY },
        );
        let ctl = pane.col(1, 0.0);
        let key = format!("prop:{}", def.name);
        widgets::record(ui, &key, ctl);
        match (&def.kind, &value) {
            (PropKind::Boolean, PropValue::Bool(b)) => {
                if let Some(v) = toggle(ui, ctl, &key, *b) {
                    app.set_instance_prop(inst, &def.name, Some(PropValue::Bool(v)));
                }
            }
            (PropKind::Text, PropValue::Text(s)) => {
                if let Some(v) = field::text_input(ui, ctl, &key, s) {
                    app.set_instance_prop(inst, &def.name, Some(PropValue::Text(v)));
                }
            }
            (PropKind::Variant { options }, v) => {
                let cur = match v {
                    PropValue::Variant(s) => s.clone(),
                    _ => String::new(),
                };
                if let Some(i) = dropdown(ui, ctl, &key, &cur, options) {
                    app.set_instance_prop(inst, &def.name, Some(PropValue::Variant(options[i].clone())));
                }
            }
            (PropKind::InstanceSwap { .. }, v) => {
                let comps = all_components(&app.doc);
                let names: Vec<String> = comps.iter().map(|c| comp_label(&app.doc, *c)).collect();
                let cur = match v {
                    PropValue::Instance(c) => comp_label(&app.doc, *c),
                    _ => String::new(),
                };
                if let Some(i) = dropdown(ui, ctl, &key, &cur, &names) {
                    app.set_instance_prop(inst, &def.name, Some(PropValue::Instance(comps[i])));
                }
            }
            _ => {}
        }
        row_end(pane);
    }
    pane.y += 4.0;
    section_sep(ui, pane, false);
}

/// "Changed in this instance" row with a reset button, for instances and the
/// layers inside them.
fn overrides_row(app: &mut App, ui: &mut Ui, pane: &mut Pane, id: NodeId) {
    let Some(n) = app.doc.get(id) else { return };
    let fields: Vec<OverrideField> = n.component.overrides.iter().copied().collect();
    if fields.is_empty() {
        return;
    }
    let names: Vec<&str> = fields.iter().map(|f| override_name(*f)).collect();
    let r = pane.rect(theme::SECTION_CONTENT_X, 0.0, 128.0, theme::FIELD_HEIGHT);
    let shown = widgets::elide(ui.painter(), &format!("Changed: {}", names.join(", ")), &theme::font_body(), r.width());
    ui.painter().text(Pos2::new(r.left(), r.center().y), Align2::LEFT_CENTER, shown, theme::font_body(), theme::TEXT_SECONDARY);
    let btn = pane.rect(theme::FIELD_COL_X[1] + 24.0, 0.0, 64.0, theme::FIELD_HEIGHT);
    widgets::record(ui, "reset-overrides", btn);
    let resp = ui.interact(btn, ui.id().with("reset-overrides"), Sense::click());
    ui.painter().rect_filled(btn, theme::cr(theme::RADIUS_MEDIUM), if resp.hovered() { theme::BG_TERTIARY } else { theme::BG_SECONDARY });
    ui.painter().text(btn.center(), Align2::CENTER_CENTER, "Reset", theme::font_body(), theme::TEXT);
    if resp.clicked() {
        app.reset_node_overrides(id, &fields);
    }
    row_end(pane);
    pane.y += 4.0;
    section_sep(ui, pane, false);
}

fn override_name(f: OverrideField) -> &'static str {
    match f {
        OverrideField::Name => "name",
        OverrideField::Visible => "visibility",
        OverrideField::Locked => "lock",
        OverrideField::Opacity => "opacity",
        OverrideField::Blend => "blend",
        OverrideField::Size => "size",
        OverrideField::Transform => "position",
        OverrideField::Fills => "fill",
        OverrideField::Stroke => "stroke",
        OverrideField::Effects => "effects",
        OverrideField::CornerRadii => "radius",
        OverrideField::Characters => "text",
        OverrideField::TextStyle => "text style",
        OverrideField::Layout => "layout",
        OverrideField::MainComponent => "component",
    }
}

// ---------------------------------------------------------------- main component
fn kind_label(k: &PropKind) -> &'static str {
    match k {
        PropKind::Boolean => "Boolean",
        PropKind::Text => "Text",
        PropKind::Variant { .. } => "Variant",
        PropKind::InstanceSwap { .. } => "Instance",
    }
}

/// All component sets in the document, ordered by name, for the move-to-set picker.
fn all_sets(doc: &Document) -> Vec<NodeId> {
    let mut sets: Vec<NodeId> = doc.nodes.values().filter(|node| matches!(node.kind, NodeKind::ComponentSet(_))).map(|node| node.id).collect();
    sets.sort_by_key(|id| doc.get(*id).map(|node| node.name.clone()).unwrap_or_default());
    sets
}

fn action_button(ui: &mut Ui, pane: &mut Pane, key: &str, label: &str) -> bool {
    let rect = pane.rect(theme::SECTION_CONTENT_X, 0.0, 104.0, theme::FIELD_HEIGHT);
    let response = ui.interact(rect, ui.id().with(key), Sense::click());
    ui.painter().rect_filled(rect, theme::cr(theme::RADIUS_MEDIUM), if response.hovered() { theme::BG_TERTIARY } else { theme::BG_SECONDARY });
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, label, theme::font_body(), theme::TEXT);
    pane.y += theme::FIELD_HEIGHT + 4.0;
    response.clicked()
}

/// Variant lifecycle controls for a component set, a variant, or a standalone component.
fn variant_section(app: &mut App, ui: &mut Ui, pane: &mut Pane, id: NodeId) {
    let Some(node) = app.doc.get(id).cloned() else { return };
    match &node.kind {
        NodeKind::ComponentSet(_) => {
            let variants = components_of(&app.doc, id);
            let add = title(ui, pane, "Variants", Some(Icon::Plus), variants.is_empty());
            if add {
                if let Some(first) = variants.first().copied() {
                    app.open_add_variant_copy(id, first);
                }
            }
            for variant in variants {
                let Some(child) = app.doc.get(variant) else { continue };
                let row = pane.rect(theme::SECTION_CONTENT_X, 0.0, theme::SECTION_CONTENT_W - 28.0, theme::FIELD_HEIGHT);
                let resp = ui.interact(row, ui.id().with(("variant-row", variant.0)), Sense::click());
                if resp.clicked() {
                    app.selection = vec![variant];
                }
                let selected = app.selection == [variant];
                ui.painter().text(
                    Pos2::new(row.left(), row.center().y),
                    Align2::LEFT_CENTER,
                    &child.name,
                    theme::font_body(),
                    if selected { theme::TEXT } else { theme::TEXT_SECONDARY },
                );
                let minus = pane.rect(theme::SECTION_TRAILING_X, 0.0, theme::PANEL_ACTION_BUTTON, theme::PANEL_ACTION_BUTTON);
                if widgets::icon_button(ui, minus, ("variant-remove", variant.0), Icon::Minus, false, "Remove variant").clicked() {
                    if let Err(error) = app.remove_variant(variant) {
                        app.show_toast(error, true);
                    }
                }
                row_end(pane);
                pane.y += 4.0;
            }
            section_sep(ui, pane, false);
        }
        NodeKind::Component(_) => {
            if variant_set(&app.doc, id).is_some() {
                title(ui, pane, "Variant", None, false);
                if action_button(ui, pane, "variant-rename", "Rename variant") {
                    app.open_rename_variant(id);
                }
                if action_button(ui, pane, "variant-remove", "Remove variant") {
                    if let Err(error) = app.remove_variant(id) {
                        app.show_toast(error, true);
                    }
                }
            } else if action_button(ui, pane, "variant-move", "Add to variant set…") {
                app.open_add_variant_move(id);
            }
            section_sep(ui, pane, false);
        }
        _ => {}
    }
}

/// Inline editor with one field per variant property. It stays open on error.
fn variant_editor(app: &mut App, ui: &mut Ui, pane: &mut Pane) {
    let Some(mut edit) = app.ui.variant_edit.clone() else { return };
    let heading = match edit.kind {
        crate::app::VariantEditKind::AddCopy { .. } => "Add variant",
        crate::app::VariantEditKind::AddMove { .. } => "Move component into set",
        crate::app::VariantEditKind::Rename { .. } => "Rename variant",
    };
    title(ui, pane, heading, None, false);
    if let crate::app::VariantEditKind::AddMove { set: None, .. } = edit.kind {
        let sets = all_sets(&app.doc);
        let names: Vec<String> = sets.iter().map(|id| app.doc.get(*id).map(|node| node.name.clone()).unwrap_or_default()).collect();
        let current = if names.is_empty() { "No component sets" } else { "Choose a component set" };
        if let Some(index) = dropdown(ui, pane.full(0.0), "variant-set", current, &names) {
            if let Some(set) = sets.get(index).copied() {
                edit.kind = crate::app::VariantEditKind::AddMove {
                    source: match edit.kind {
                        crate::app::VariantEditKind::AddMove { source, .. } => source,
                        _ => unreachable!(),
                    },
                    set: Some(set),
                };
                edit.values = app.variant_defaults(set);
            }
        }
        pane.y += 4.0;
    }
    for (name, value) in &mut edit.values {
        let label = pane.rect(theme::SECTION_CONTENT_X, 0.0, 100.0, theme::FIELD_HEIGHT);
        ui.painter().text(Pos2::new(label.left(), label.center().y), Align2::LEFT_CENTER, name.as_str(), theme::font_body(), theme::TEXT_SECONDARY);
        let field_rect = pane.rect(theme::FIELD_COL_X[1], 0.0, theme::FIELD_WIDTH_HALF, theme::FIELD_HEIGHT);
        if let Some(next) = field::text_input(ui, field_rect, ("variant-value", &*name), value) {
            *value = next;
        }
        pane.y += theme::FIELD_HEIGHT + 4.0;
    }
    if let Some(error) = &edit.error {
        let row = pane.rect(theme::SECTION_CONTENT_X, 0.0, theme::SECTION_CONTENT_W, theme::FIELD_HEIGHT);
        ui.painter().text(Pos2::new(row.left(), row.center().y), Align2::LEFT_CENTER, error, theme::font_body(), theme::DANGER);
        pane.y += theme::FIELD_HEIGHT + 4.0;
    }
    let submit = match edit.kind {
        crate::app::VariantEditKind::AddCopy { .. } | crate::app::VariantEditKind::AddMove { .. } => "Add",
        crate::app::VariantEditKind::Rename { .. } => "Rename",
    };
    if action_button(ui, pane, "variant-submit", submit) {
        app.ui.variant_edit = Some(edit);
        app.submit_variant_edit();
        return;
    }
    if action_button(ui, pane, "variant-cancel", "Cancel") {
        app.ui.variant_edit = None;
        return;
    }
    app.ui.variant_edit = Some(edit);
    section_sep(ui, pane, false);
}

fn component_props(app: &mut App, ui: &mut Ui, pane: &mut Pane, id: NodeId) {
    variant_section(app, ui, pane, id);
    let owner = if matches!(app.doc.get(id).map(|n| &n.kind), Some(NodeKind::ComponentSet(_))) { id } else { app.prop_owner(id) };
    let defs = app.doc.get(owner).map(|n| n.component.props.clone()).unwrap_or_default();
    let plus = pane.rect(theme::SECTION_TRAILING_X, 8.0, theme::PANEL_ACTION_BUTTON, theme::PANEL_ACTION_BUTTON);
    widgets::record(ui, "prop-add", plus);
    if title(ui, pane, "Properties", Some(Icon::Plus), defs.is_empty()) {
        app.ui.prop_menu = if app.ui.prop_menu.is_some() { None } else { Some(plus) };
    }
    for def in &defs {
        let name_r = pane.rect(theme::SECTION_CONTENT_X, 0.0, theme::SECTION_CONTENT_W - 28.0, theme::FIELD_HEIGHT);
        widgets::record(ui, &format!("propdef:{}", def.name), name_r);
        if matches!(def.kind, PropKind::Variant { .. }) {
            ui.painter().text(Pos2::new(name_r.left(), name_r.center().y), Align2::LEFT_CENTER, &def.name, theme::font_body(), theme::TEXT_SECONDARY);
        } else if let Some(new) = field::text_input(ui, name_r, ("propdef", &def.name), &def.name) {
            app.rename_prop_def(owner, &def.name, &new);
        }
        if !matches!(def.kind, PropKind::Variant { .. }) {
            let minus = pane.rect(theme::SECTION_TRAILING_X, 0.0, theme::PANEL_ACTION_BUTTON, theme::PANEL_ACTION_BUTTON);
            widgets::record(ui, &format!("propdel:{}", def.name), minus);
            if widgets::icon_button(ui, minus, ("propdel", &def.name), Icon::Minus, false, "Delete property").clicked() {
                app.delete_prop_def(owner, &def.name);
            }
        }
        row_end(pane);
        let lab = pane.col(0, 0.0);
        ui.painter().text(Pos2::new(lab.left(), lab.center().y), Align2::LEFT_CENTER, kind_label(&def.kind), theme::font_body(), theme::TEXT_SECONDARY);
        let ctl = pane.col(1, 0.0);
        let key = format!("default:{}", def.name);
        match (&def.kind, &def.default) {
            (PropKind::Boolean, PropValue::Bool(b)) => {
                if let Some(v) = toggle(ui, ctl, &key, *b) {
                    app.set_prop_default(owner, &def.name, PropValue::Bool(v));
                }
            }
            (PropKind::Text, PropValue::Text(s)) => {
                if let Some(v) = field::text_input(ui, ctl, &key, s) {
                    app.set_prop_default(owner, &def.name, PropValue::Text(v));
                }
            }
            (PropKind::Variant { .. }, PropValue::Variant(cur)) => {
                // Variant properties are derived from the variant names; their
                // defaults follow add/rename/remove, not a separate editor.
                ui.painter().text(Pos2::new(ctl.left(), ctl.center().y), Align2::LEFT_CENTER, cur, theme::font_body(), theme::TEXT_SECONDARY);
            }
            (PropKind::InstanceSwap { .. }, PropValue::Instance(c)) => {
                let comps: Vec<NodeId> = all_components(&app.doc).into_iter().filter(|x| *x != owner).collect();
                let names: Vec<String> = comps.iter().map(|x| comp_label(&app.doc, *x)).collect();
                if let Some(i) = dropdown(ui, ctl, &key, &comp_label(&app.doc, *c), &names) {
                    app.set_prop_default(owner, &def.name, PropValue::Instance(comps[i]));
                }
            }
            _ => {}
        }
        row_end(pane);
        pane.y += 4.0;
    }
    if let Some(anchor) = app.ui.prop_menu {
        let items = [MenuItem::new("Boolean"), MenuItem::new("Text"), MenuItem::new("Instance swap")];
        match popup_menu(ui.ctx(), egui::Id::new("prop-add-menu"), anchor.left_bottom() + Vec2::new(-150.0, 2.0), 170.0, &items, Some(anchor)) {
            MenuResult::Picked(i) => {
                app.ui.prop_menu = None;
                let kind = match i {
                    0 => PropKind::Boolean,
                    1 => PropKind::Text,
                    _ => PropKind::InstanceSwap { preferred: vec![] },
                };
                app.add_prop_def(owner, kind);
            }
            MenuResult::Closed => app.ui.prop_menu = None,
            MenuResult::Open => {}
        }
    }
    section_sep(ui, pane, false);
}

/// Link a layer's visibility / text / instance to a component property.
fn layer_bindings(app: &mut App, ui: &mut Ui, pane: &mut Pane, id: NodeId) {
    let Some(comp) = enclosing_component(&app.doc, id) else { return };
    if comp == id {
        return;
    }
    let Some(n) = app.doc.get(id).cloned() else { return };
    let defs = prop_defs(&app.doc, comp);
    #[allow(clippy::type_complexity)] // local row table of (label, field, predicate)
    let mut rows: Vec<(&'static str, OverrideField, fn(&PropKind) -> bool)> = vec![("Visibility", OverrideField::Visible, |k| matches!(k, PropKind::Boolean))];
    if matches!(n.kind, NodeKind::Text(_)) {
        rows.push(("Text", OverrideField::Characters, |k| matches!(k, PropKind::Text)));
    }
    if matches!(n.kind, NodeKind::Instance { .. }) {
        rows.push(("Instance", OverrideField::MainComponent, |k| matches!(k, PropKind::InstanceSwap { .. })));
    }
    let rows: Vec<_> = rows.into_iter().filter(|(_, f, accepts)| n.component.bindings.contains_key(f) || defs.iter().any(|d| accepts(&d.kind))).collect();
    if rows.is_empty() {
        return;
    }
    title(ui, pane, "Component properties", None, false);
    for (label, field, accepts) in rows {
        sublabel(ui, pane, label);
        let cur = n.component.bindings.get(&field).cloned();
        let mut opts = vec!["None".to_string()];
        opts.extend(defs.iter().filter(|d| accepts(&d.kind)).map(|d| d.name.clone()));
        let shown = cur.clone().unwrap_or_else(|| "None".into());
        let r = pane.full(0.0);
        widgets::record(ui, &format!("bind:{label}"), r);
        if let Some(i) = dropdown(ui, r, ("bind", label), &shown, &opts) {
            let prop = (i > 0).then(|| opts[i].clone());
            app.bind_prop(id, field, prop.as_deref());
        }
        row_end(pane);
    }
    pane.y += 4.0;
    section_sep(ui, pane, false);
}
