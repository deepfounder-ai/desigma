//! Right-click context menu for the selection (canvas and layers).

use super::menus::{self, Action};
use crate::app::App;
use crate::widgets::menu::{popup_tree, MenuItem, TreeResult};
use desigma_doc::find::MatchCriteria;
use desigma_doc::NodeKind;
use egui::{Context, Id};

/// A menu entry: leaves run `act`; entries with `kids` open a submenu.
struct Entry {
    item: MenuItem,
    act: Option<Action>,
    kids: Vec<(MenuItem, Action)>,
}

fn leaf(item: MenuItem, act: Action) -> Entry {
    Entry { item, act: Some(act), kids: vec![] }
}

/// The "Select" submenu: structural selection and the "same ..." family.
fn select_submenu(has: bool) -> Entry {
    let same = |label: &str, c: MatchCriteria| (MenuItem::new(label).disabled(!has), Action::SelectSame(c));
    let mut kids = vec![
        (MenuItem::new("Select children").shortcut("Enter").disabled(!has), Action::SelectChildren),
        (MenuItem::new("Select parent").shortcut("Shift+Enter").disabled(!has), Action::SelectParent),
        (MenuItem::new("Select inverse").shortcut("Ctrl+Shift+A"), Action::SelectInverse),
        (MenuItem::new("Select matching layers").shortcut("Ctrl+Alt+A").disabled(!has).sep(), Action::SelectMatching),
    ];
    kids.extend([
        same("Select all with same fill", MatchCriteria::Fill),
        same("Select all with same stroke", MatchCriteria::Stroke),
        same("Select all with same effect", MatchCriteria::Effect),
        same("Select all with same font", MatchCriteria::Font),
        same("Select all with same instance", MatchCriteria::Instance),
    ]);
    Entry { item: MenuItem::new("Select").disabled(!has).sep(), act: None, kids }
}

/// Entries given the current selection.
fn entries(app: &App) -> Vec<Entry> {
    let has = !app.selection.is_empty();
    let insts = app.selected_instances();
    let has_inst = !insts.is_empty();
    let any_container =
        app.selection.iter().any(|id| app.doc.get(*id).is_some_and(|n| matches!(n.kind, NodeKind::Group | NodeKind::Frame(_) | NodeKind::Section)));
    let can_create = app
        .selection
        .iter()
        .any(|id| app.doc.get(*id).is_some_and(|n| !matches!(n.kind, NodeKind::Component(_) | NodeKind::ComponentSet(_) | NodeKind::Page { .. })));
    let can_mark = app.selection.iter().any(|id| crate::dev::markable(app, *id));
    let has_status = app.selection.iter().any(|id| app.doc.get(*id).is_some_and(|n| n.dev.status.is_some()));
    let mut list = vec![
        leaf(MenuItem::new("Duplicate").shortcut("Ctrl+D").disabled(!has), Action::Duplicate),
        leaf(MenuItem::new("Group selection").shortcut("Ctrl+G").disabled(!has), Action::Group),
        leaf(MenuItem::new("Frame selection").shortcut("Ctrl+Alt+G").disabled(!has), Action::FrameSelection),
        leaf(MenuItem::new("Ungroup").shortcut("Ctrl+Shift+G").disabled(!any_container), Action::Ungroup),
        leaf(MenuItem::new("Add auto layout").shortcut("Shift+A").disabled(!has).sep(), Action::AutoLayout),
        leaf(MenuItem::new("Create component").shortcut("Ctrl+Alt+K").disabled(!can_create), Action::CreateComponent),
        leaf(MenuItem::new("Combine as variants").shortcut("Ctrl+Shift+K").disabled(!has), Action::CombineVariants),
        leaf(MenuItem::new("Detach instance").shortcut("Ctrl+Alt+B").disabled(!has_inst), Action::Detach),
        leaf(MenuItem::new("Go to main component").disabled(!has_inst), Action::GoToMain),
        leaf(MenuItem::new("Reset all changes").disabled(!has_inst).sep(), Action::ResetInstance),
        leaf(MenuItem::new("Bring to front").shortcut("Ctrl+Shift+]").disabled(!has), Action::BringToFront),
        leaf(MenuItem::new("Send to back").shortcut("Ctrl+Shift+[").disabled(!has).sep(), Action::SendToBack),
        select_submenu(has),
        leaf(MenuItem::new("Mark as ready for dev").disabled(!can_mark), Action::MarkReadyForDev),
    ];
    if has_status {
        list.push(leaf(MenuItem::new("Mark as completed"), Action::MarkCompleted));
        list.push(leaf(MenuItem::new("Remove dev status"), Action::ClearDevStatus));
    }
    list.push(leaf(MenuItem::new("Generate image…"), Action::AiGenerate));
    list.push(leaf(MenuItem::new("Edit image with AI…").disabled(app.ai_editable().is_none()).sep(), Action::AiEdit));
    list.push(leaf(MenuItem::new("Delete").shortcut("Del").disabled(!has), Action::Delete));
    list
}

/// The action behind an index path of the open menu.
fn resolve(entries: &[Entry], path: &[usize]) -> Option<Action> {
    let e = entries.get(*path.first()?)?;
    match path.get(1) {
        None => e.act,
        Some(k) => e.kids.get(*k).map(|k| k.1),
    }
}

pub fn show(app: &mut App, ctx: &Context) {
    let Some(at) = app.ui.ctx_menu else { return };
    let entries = entries(app);
    let list: Vec<MenuItem> = entries
        .iter()
        .map(|e| {
            let mut it = e.item.clone();
            it.children = e.kids.iter().map(|k| k.0.clone()).collect();
            it
        })
        .collect();
    match popup_tree(ctx, Id::new("selection-menu"), at, 230.0, &list, None) {
        TreeResult::Picked(path) => {
            app.ui.ctx_menu = None;
            if let Some(a) = resolve(&entries, &path) {
                menus::run(app, ctx, a);
            }
        }
        TreeResult::Closed => app.ui.ctx_menu = None,
        TreeResult::Open => {}
    }
}

#[cfg(test)]
pub fn labels(app: &App) -> Vec<String> {
    entries(app).into_iter().map(|e| e.item.label).collect()
}
