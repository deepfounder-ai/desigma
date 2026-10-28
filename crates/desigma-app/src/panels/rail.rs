//! Far-left icon rail.

use crate::app::{App, MenuKind, RailItem};
use crate::icons::{self, Icon};
use crate::theme;
use crate::widgets;
use egui::{Align2, Pos2, Rect, Sense, Ui, Vec2};

/// Rail entries in order; the flag marks the item that starts a new group (separator above it).
pub const ITEMS: [(RailItem, Icon, &str, bool); 6] = [
    (RailItem::File, Icon::File, "File", false),
    (RailItem::Assets, Icon::Assets, "Assets", false),
    (RailItem::Tools, Icon::Tools, "Tools", false),
    (RailItem::Variables, Icon::Variables, "Variables", true),
    (RailItem::Plugins, Icon::Component, "Plugins", false),
    (RailItem::Agents, Icon::Sparkle, "Agents", false),
];

/// Top y (relative to the rail top) of each item's 56px cell.
pub fn cell_tops() -> Vec<f32> {
    let mut y = theme::RAIL_FIRST_Y;
    ITEMS
        .iter()
        .map(|(_, _, _, gap)| {
            if *gap {
                y += theme::RAIL_GROUP_GAP;
            }
            let top = y;
            y += theme::RAIL_ITEM_SIZE;
            top
        })
        .collect()
}

pub fn show(app: &mut App, ui: &mut Ui, rect: Rect) {
    let p = ui.painter().clone();
    p.rect_filled(rect, 0.0, theme::BG);
    widgets::vline(&p, rect.right() - theme::BORDER_WIDTH, rect.top(), rect.bottom(), theme::BORDER);
    let inner_w = theme::RAIL_WIDTH - theme::BORDER_WIDTH;

    // Logo / main menu button.
    let mb = Rect::from_min_size(
        Pos2::new(rect.left() + theme::RAIL_MENU_BUTTON_X, rect.top() + theme::RAIL_MENU_BUTTON_Y),
        Vec2::splat(theme::RAIL_MENU_BUTTON_SIZE),
    );
    let r = ui.interact(mb, ui.id().with("rail-menu"), Sense::click());
    widgets::button_bg(&p, mb, &r, app.ui.menu == Some(MenuKind::Main), theme::cr(theme::RADIUS_MEDIUM));
    icons::paint(&p, Icon::Logo, mb.center(), 20.0, theme::ICON);
    if r.clicked() {
        toggle(app, MenuKind::Main, mb);
    }
    separator(&p, rect, rect.top() + theme::RAIL_SEPARATOR_Y, inner_w);

    for ((item, icon, label, gap_before), top) in ITEMS.iter().zip(cell_tops()) {
        let y = rect.top() + top;
        if *gap_before {
            separator(&p, rect, y - theme::RAIL_GROUP_GAP / 2.0, inner_w);
        }
        let ir = Rect::from_min_size(Pos2::new(rect.left(), y), Vec2::new(inner_w, theme::RAIL_ITEM_SIZE));
        let resp = ui.interact(ir, ui.id().with(("rail", *label)), Sense::click());
        let selected = if app.plugins.open {
            *item == RailItem::Plugins
        } else if app.vars.open {
            *item == RailItem::Variables
        } else {
            app.ui.rail == Some(*item)
                || (*item == RailItem::File && (app.ui.menu == Some(MenuKind::FileMenu) || (app.ui.rail.is_none() && !app.ui.left_collapsed)))
        };
        // Active item: 32x32 rounded tile behind the icon.
        let ic = Pos2::new(ir.center().x, ir.top() + theme::RAIL_ICON_CENTER_Y);
        if selected {
            p.rect_filled(Rect::from_center_size(ic, Vec2::splat(theme::RAIL_TILE)), theme::cr(theme::RADIUS_MEDIUM), theme::BG_SELECTED);
        } else if resp.hovered() {
            p.rect_filled(Rect::from_center_size(ic, Vec2::splat(theme::RAIL_TILE)), theme::cr(theme::RADIUS_MEDIUM), theme::BG_HOVER);
        }
        icons::paint(&p, *icon, ic, theme::RAIL_ICON_SIZE, theme::ICON);
        let font = if selected { theme::font_small_strong() } else { theme::font_small() };
        p.text(Pos2::new(ir.center().x, ir.top() + theme::RAIL_LABEL_CENTER_Y), Align2::CENTER_CENTER, *label, font, theme::TEXT);
        if resp.clicked() {
            if *item == RailItem::Plugins {
                app.open_plugins();
                continue;
            }
            if *item != RailItem::Variables && app.vars.open {
                // leaving the variables view returns to the editor
                app.close_variables();
            }
            if *item == RailItem::File {
                toggle(app, MenuKind::FileMenu, ir);
            } else {
                if *item == RailItem::Variables {
                    app.toggle_variables();
                    continue;
                }
                app.ui.rail = if app.ui.rail == Some(*item) { None } else { Some(*item) };
                if app.ui.rail == Some(RailItem::Agents) {
                    app.open_agents(ui.ctx());
                } else if !matches!(item, RailItem::Assets | RailItem::Agents) {
                    app.show_toast(format!("{label} panel is not available in this build"), false);
                }
            }
        }
    }

    // Bottom: help (keyboard shortcuts, About, the GitHub repo).
    let br = Rect::from_min_size(
        Pos2::new(rect.left() + theme::RAIL_MENU_BUTTON_X, rect.bottom() - 8.0 - theme::RAIL_MENU_BUTTON_SIZE),
        Vec2::splat(theme::RAIL_MENU_BUTTON_SIZE),
    );
    if widgets::icon_button(ui, br, "help", Icon::Help, false, "Help").clicked() {
        crate::panels::menus::run(app, ui.ctx(), crate::panels::menus::Action::Shortcuts);
    }
}

/// 1px #444 line inset from both edges of the rail.
fn separator(p: &egui::Painter, rect: Rect, y: f32, inner_w: f32) {
    widgets::hline(p, rect.left() + theme::RAIL_SEPARATOR_INSET, rect.left() + inner_w - theme::RAIL_SEPARATOR_INSET, y, theme::BORDER);
}

fn toggle(app: &mut App, kind: MenuKind, anchor: Rect) {
    if app.ui.menu == Some(kind) {
        app.ui.menu = None;
    } else {
        app.ui.menu = Some(kind);
        app.ui.menu_anchor = Some(anchor);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cells_follow_the_measured_rail_layout() {
        // File, Assets, Tools every 56px, then a 16px gap before Variables, Plugins and Agents.
        assert_eq!(cell_tops(), vec![56.0, 112.0, 168.0, 240.0, 296.0, 352.0]);
        // separators sit at y=48 (under the logo) and 232 (middle of the group gap)
        assert_eq!(theme::RAIL_SEPARATOR_Y, 48.0);
        assert_eq!(cell_tops()[3] - theme::RAIL_GROUP_GAP / 2.0, 232.0);
    }
}
