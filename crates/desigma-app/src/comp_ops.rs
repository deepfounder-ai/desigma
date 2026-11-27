//! Component and instance editing glue: override recording, main-component
//! sync at commit time, and the selection commands (create, detach, swap,
//! variants, properties). Pure logic lives in `desigma_doc::components`.

use crate::app::{App, VariantEdit, VariantEditKind};
use crate::canvas::create::container_at;
use crate::edit::Tx;
use crate::layout;
use desigma_doc::*;

pub use desigma_engine::components::{record_overrides, with_sync};

fn is_noop(c: &Command) -> bool {
    matches!(c, Command::Batch(v) if v.is_empty())
}

/// One row of the Assets panel.
pub struct AssetEntry {
    pub page: String,
    /// Frame or section the component sits in (empty when directly on the page).
    pub container: String,
    pub name: String,
    pub id: NodeId,
    /// Number of variants when the entry stands for a component set.
    pub variants: usize,
}

impl App {
    fn push_cmd(&mut self, tx: &mut Tx, cmd: Command) -> bool {
        if is_noop(&cmd) {
            return false;
        }
        match tx.cmd(&mut self.doc, cmd) {
            Ok(()) => true,
            Err(e) => {
                self.show_toast(format!("Edit failed: {e}"), true);
                false
            }
        }
    }

    /// Re-fits self-sizing texts below `root` (after a property changed their characters).
    fn refit_texts(&mut self, tx: &mut Tx, root: NodeId) {
        let mut ids = Vec::new();
        self.doc.walk(root, |n, _| {
            if matches!(&n.kind, NodeKind::Text(t) if t.auto_resize != TextAutoResize::None) {
                ids.push(n.id);
            }
        });
        for id in ids {
            let Some(n) = self.doc.get(id) else { continue };
            let NodeKind::Text(t) = &n.kind else { continue };
            let size = layout::fit_text(t, n.size);
            if size != n.size {
                if let Some(n) = tx.ed.node(&mut self.doc, id) {
                    n.size = size;
                }
                layout::relayout_around(&mut self.doc, &mut tx.ed, id);
            }
        }
    }

    /// Selected instance roots (a selected layer inside an instance counts as its instance).
    pub fn selected_instances(&self) -> Vec<NodeId> {
        let mut out = Vec::new();
        for id in self.top_selection() {
            let inst = match self.doc.get(id).map(|n| &n.kind) {
                Some(NodeKind::Instance { .. }) if self.doc.get(id).is_some_and(is_top_instance) => Some(id),
                _ => instance_root_of(&self.doc, id),
            };
            if let Some(i) = inst {
                if !out.contains(&i) {
                    out.push(i);
                }
            }
        }
        out
    }

    /// Component an instance was made from.
    pub fn main_of(&self, instance: NodeId) -> Option<NodeId> {
        self.doc.get(instance).and_then(instance_component).filter(|c| self.doc.get(*c).is_some())
    }

    /// Ctrl+Alt+K: turns the selection into main components.
    pub fn create_component(&mut self) {
        let tops = self.top_selection();
        let mut tx = Tx::new();
        let mut made = Vec::new();
        for id in tops {
            let cmd = create_component(&mut self.doc, id);
            if self.push_cmd(&mut tx, cmd) {
                made.push(id);
            }
        }
        if made.is_empty() {
            return;
        }
        self.run_tx(tx);
        self.selection = made.iter().filter_map(|id| self.component_for(*id)).collect();
        self.show_toast(if made.len() == 1 { "Created component" } else { "Created components" }, false);
    }

    /// The component a just-converted node became (itself, or its wrapper).
    fn component_for(&self, id: NodeId) -> Option<NodeId> {
        let n = self.doc.get(id)?;
        if matches!(n.kind, NodeKind::Component(_)) {
            return Some(id);
        }
        n.parent.filter(|p| matches!(self.doc.get(*p).map(|n| &n.kind), Some(NodeKind::Component(_))))
    }

    /// Ctrl+Alt+B: detaches the selected instances.
    pub fn detach_selection(&mut self) {
        let insts = self.selected_instances();
        let mut tx = Tx::new();
        let mut done = Vec::new();
        for i in insts {
            let cmd = detach_instance(&mut self.doc, i);
            if self.push_cmd(&mut tx, cmd) {
                done.push(i);
            }
        }
        if done.is_empty() {
            return;
        }
        self.run_tx(tx);
        self.selection = done;
    }

    /// Values that identify a variant by its component name.
    fn variant_values(&self, variant: NodeId) -> Vec<(String, String)> {
        self.doc.get(variant).map(|node| parse_variant_name(&node.name)).unwrap_or_default()
    }

    /// Default values for a new variant in `set`: the first option of each
    /// variant property.
    pub(crate) fn variant_defaults(&self, set: NodeId) -> Vec<(String, String)> {
        let Some(node) = self.doc.get(set) else { return Vec::new() };
        node.component
            .props
            .iter()
            .filter_map(|def| match &def.kind {
                PropKind::Variant { options } => options.first().map(|option| (def.name.clone(), option.clone())),
                _ => None,
            })
            .collect()
    }

    pub fn open_add_variant_copy(&mut self, set: NodeId, source: NodeId) {
        let mut values = self.variant_values(source);
        if let Some((_, value)) = values.first_mut() {
            value.push_str(" Copy");
        }
        self.ui.variant_edit = Some(VariantEdit { kind: VariantEditKind::AddCopy { set, source }, values, error: None });
    }

    pub fn open_add_variant_move(&mut self, source: NodeId) {
        self.ui.variant_edit = Some(VariantEdit { kind: VariantEditKind::AddMove { source, set: None }, values: Vec::new(), error: None });
    }

    pub fn open_rename_variant(&mut self, variant: NodeId) {
        self.ui.variant_edit = Some(VariantEdit { kind: VariantEditKind::Rename { variant }, values: self.variant_values(variant), error: None });
    }

    pub fn add_variant(&mut self, set: NodeId, source: NodeId, mode: AddVariantMode, values: &[(String, String)]) -> Result<NodeId, String> {
        let cmd = desigma_doc::add_variant(&mut self.doc, set, source, mode, values).map_err(|e| e.to_string())?;
        let mut tx = Tx::new();
        if !self.push_cmd(&mut tx, cmd) {
            tx.cancel(&mut self.doc);
            return Err("variant edit failed".into());
        }
        let id = desigma_doc::find_variant(&self.doc, set, values).ok_or("variant was not created")?;
        self.run_tx(tx);
        self.selection = vec![id];
        Ok(id)
    }

    pub fn remove_variant(&mut self, variant: NodeId) -> Result<NodeId, String> {
        let set = variant_set(&self.doc, variant).ok_or("variant has no component set")?;
        let cmd = desigma_doc::remove_variant(&mut self.doc, variant).map_err(|e| e.to_string())?;
        let mut tx = Tx::new();
        if !self.push_cmd(&mut tx, cmd) {
            tx.cancel(&mut self.doc);
            return Err("variant removal failed".into());
        }
        self.run_tx(tx);
        self.selection = vec![set];
        Ok(set)
    }

    pub fn rename_variant(&mut self, variant: NodeId, values: &[(String, String)]) -> Result<(), String> {
        let cmd = desigma_doc::rename_variant(&mut self.doc, variant, values).map_err(|e| e.to_string())?;
        let mut tx = Tx::new();
        if !self.push_cmd(&mut tx, cmd) {
            tx.cancel(&mut self.doc);
            return Err("variant rename failed".into());
        }
        self.run_tx(tx);
        self.selection = vec![variant];
        Ok(())
    }

    fn selected_variant(&self) -> Option<NodeId> {
        self.selection.first().copied().filter(|id| variant_set(&self.doc, *id).is_some())
    }

    pub fn add_variant_from_selection(&mut self) {
        let Some(id) = self.selection.first().copied() else { return };
        match self.doc.get(id).map(|node| node.kind.clone()) {
            Some(NodeKind::ComponentSet(_)) => {
                if let Some(source) = components_of(&self.doc, id).first().copied() {
                    self.open_add_variant_copy(id, source);
                }
            }
            Some(NodeKind::Component(_)) => {
                if let Some(parent) = variant_set(&self.doc, id) {
                    self.open_add_variant_copy(parent, id);
                } else {
                    self.open_add_variant_move(id);
                }
            }
            _ => self.show_toast("Select a component or component set", true),
        }
    }

    pub fn remove_variant_from_selection(&mut self) {
        if let Some(variant) = self.selected_variant() {
            if let Err(error) = self.remove_variant(variant) {
                self.show_toast(error, true);
            }
        }
    }

    pub fn rename_variant_from_selection(&mut self) {
        if let Some(variant) = self.selected_variant() {
            self.open_rename_variant(variant);
        }
    }

    /// Applies the inline variant editor. On failure the editor stays open with
    /// the error so the values are not lost.
    pub fn submit_variant_edit(&mut self) {
        let Some(edit) = self.ui.variant_edit.take() else { return };
        let values = edit.values.clone();
        let result = match edit.kind {
            VariantEditKind::AddCopy { set, source } => self.add_variant(set, source, AddVariantMode::Copy, &values).map(|_| ()),
            VariantEditKind::AddMove { source, set: Some(set) } => self.add_variant(set, source, AddVariantMode::Move, &values).map(|_| ()),
            VariantEditKind::AddMove { set: None, .. } => Err("choose a component set first".into()),
            VariantEditKind::Rename { variant } => self.rename_variant(variant, &values),
        };
        if let Err(error) = result {
            let mut edit = edit;
            edit.error = Some(error);
            self.ui.variant_edit = Some(edit);
        }
    }

    /// Context menu "Combine as variants".
    pub fn combine_selection_as_variants(&mut self) {
        let tops = self.top_selection();
        if tops.is_empty() {
            return;
        }
        let mut tx = Tx::new();
        let mut comps = Vec::new();
        for id in tops {
            let is_comp = matches!(self.doc.get(id).map(|n| &n.kind), Some(NodeKind::Component(_)));
            if !is_comp {
                let cmd = create_component(&mut self.doc, id);
                self.push_cmd(&mut tx, cmd);
            }
            if let Some(c) = self.component_for(id) {
                comps.push(c);
            }
        }
        if comps.is_empty() {
            tx.cancel(&mut self.doc);
            return;
        }
        let cmd = combine_as_variants(&mut self.doc, &comps);
        if !self.push_cmd(&mut tx, cmd) {
            tx.cancel(&mut self.doc);
            return;
        }
        let set = comps.first().and_then(|c| self.doc.get(*c)).and_then(|n| n.parent);
        self.run_tx(tx);
        if let Some(set) = set {
            self.selection = vec![set];
        }
        self.show_toast("Combined as variants", false);
    }

    /// Selects the main component of an instance and brings it into view.
    pub fn go_to_main(&mut self, id: NodeId) {
        let inst = if matches!(self.doc.get(id).map(|n| &n.kind), Some(NodeKind::Instance { .. })) { Some(id) } else { instance_root_of(&self.doc, id) };
        match inst.and_then(|i| self.main_of(i)) {
            Some(comp) => self.go_to_main_component(comp),
            None => self.show_toast("Main component not found", true),
        }
    }

    /// Selects a main component, switching page if needed, and zooms to it.
    pub fn go_to_main_component(&mut self, comp: NodeId) {
        // Copies of library components live on the hidden library page.
        if is_library_node(&self.doc, comp) {
            self.go_to_library_main(comp);
            return;
        }
        if let Some(page) = self.doc.page_of(comp) {
            if page != self.page {
                self.switch_page(page);
            }
        }
        self.selection = vec![comp];
        self.fit_selection();
    }

    /// Adds an instance of `comp` centred on page point `at`; returns it.
    pub fn insert_instance(&mut self, comp: NodeId, at: Vec2) -> Option<NodeId> {
        let size = self.doc.get(comp)?.size;
        let target = Vec2::new(at.x - size.x / 2.0, at.y - size.y / 2.0);
        let mut parent = container_at(&self.doc, self.page, at, &[]);
        let mut tx = Tx::new();
        for attempt in 0..2 {
            let inv = self.doc.world_transform(parent).invert()?;
            let local = inv.apply(target);
            let cmd = create_instance(&mut self.doc, comp, parent, local);
            if is_noop(&cmd) {
                // Inside the component itself (or its descendants): fall back to the page.
                if attempt == 0 {
                    parent = self.page;
                    continue;
                }
                return None;
            }
            let id = match &cmd {
                Command::Insert { nodes, .. } => nodes.first().map(|n| n.id),
                _ => None,
            };
            if !self.push_cmd(&mut tx, cmd) {
                return None;
            }
            layout::relayout_around(&mut self.doc, &mut tx.ed, parent);
            self.run_tx(tx);
            if let Some(id) = id {
                self.selection = vec![id];
            }
            return id;
        }
        None
    }

    /// Sets (or with `None` resets) a component property on an instance.
    pub fn set_instance_prop(&mut self, inst: NodeId, name: &str, value: Option<PropValue>) {
        let mut tx = Tx::new();
        let cmd = set_instance_prop(&mut self.doc, inst, name, value);
        if self.push_cmd(&mut tx, cmd) {
            self.refit_texts(&mut tx, inst);
            self.run_tx(tx);
        }
    }

    /// Swaps an instance to another component.
    pub fn swap_instance(&mut self, inst: NodeId, comp: NodeId) {
        let mut tx = Tx::new();
        let cmd = swap_instance(&mut self.doc, inst, comp);
        if self.push_cmd(&mut tx, cmd) {
            self.run_tx(tx);
        }
    }

    /// "Reset all changes" on the selected instances.
    pub fn reset_selection(&mut self) {
        let insts = self.selected_instances();
        let mut tx = Tx::new();
        let mut any = false;
        for i in insts {
            let cmd = reset_instance(&mut self.doc, i);
            any |= self.push_cmd(&mut tx, cmd);
        }
        if any {
            let roots = self.selected_instances();
            for r in roots {
                self.refit_texts(&mut tx, r);
            }
            self.run_tx(tx);
        }
    }

    /// Resets specific overrides of one layer inside an instance.
    pub fn reset_node_overrides(&mut self, node: NodeId, fields: &[OverrideField]) {
        let mut tx = Tx::new();
        let cmd = reset_overrides(&mut self.doc, node, fields);
        if self.push_cmd(&mut tx, cmd) {
            self.run_tx(tx);
        }
    }

    /// Owner of the property definitions shown for `comp`: its set, else itself.
    pub fn prop_owner(&self, comp: NodeId) -> NodeId {
        self.doc.get(comp).and_then(|n| n.parent).filter(|p| matches!(self.doc.get(*p).map(|n| &n.kind), Some(NodeKind::ComponentSet(_)))).unwrap_or(comp)
    }

    fn comp_edit(&mut self, f: impl FnOnce(&mut Document) -> Command) {
        let mut tx = Tx::new();
        let cmd = f(&mut self.doc);
        if self.push_cmd(&mut tx, cmd) {
            self.run_tx(tx);
        }
    }

    /// Adds a property definition with a unique name.
    pub fn add_prop_def(&mut self, owner: NodeId, kind: PropKind) {
        let base = match &kind {
            PropKind::Boolean => "Boolean",
            PropKind::Text => "Text",
            PropKind::InstanceSwap { .. } => "Instance",
            PropKind::Variant { .. } => "Variant",
        };
        let taken: Vec<String> = self.doc.get(owner).map(|n| n.component.props.iter().map(|p| p.name.clone()).collect()).unwrap_or_default();
        let mut name = base.to_string();
        let mut i = 1;
        while taken.contains(&name) {
            i += 1;
            name = format!("{base} {i}");
        }
        let default = match &kind {
            PropKind::Boolean => PropValue::Bool(true),
            PropKind::Text => PropValue::Text("Text".into()),
            PropKind::InstanceSwap { .. } => PropValue::Instance(
                self.doc.nodes.values().find(|n| matches!(n.kind, NodeKind::Component(_)) && n.id != owner).map(|n| n.id).unwrap_or(NodeId(0)),
            ),
            PropKind::Variant { options } => PropValue::Variant(options.first().cloned().unwrap_or_default()),
        };
        self.comp_edit(|d| add_prop(d, owner, ComponentPropDef { name, kind, default }));
    }

    pub fn rename_prop_def(&mut self, owner: NodeId, old: &str, new: &str) {
        let (old, new) = (old.to_string(), new.trim().to_string());
        self.comp_edit(|d| rename_prop(d, owner, &old, &new));
    }

    pub fn delete_prop_def(&mut self, owner: NodeId, name: &str) {
        let name = name.to_string();
        self.comp_edit(|d| delete_prop(d, owner, &name));
    }

    pub fn set_prop_default(&mut self, owner: NodeId, name: &str, value: PropValue) {
        let Some(mut n) = self.doc.get(owner).cloned() else { return };
        if let Some(d) = n.component.props.iter_mut().find(|d| d.name == name) {
            d.default = value;
        }
        self.comp_edit(|_| Command::Update { node: Box::new(n) });
    }

    /// Binds a layer's visibility / characters / instance to a property (or unbinds with `None`).
    pub fn bind_prop(&mut self, node: NodeId, field: OverrideField, prop: Option<&str>) {
        let prop = prop.map(String::from);
        self.comp_edit(|d| bind_prop(d, node, field, prop.as_deref()));
    }

    /// Components for the Assets panel, in page order.
    pub fn local_components(&self) -> Vec<AssetEntry> {
        let mut out = Vec::new();
        for pid in &self.doc.pages {
            let Some(page) = self.doc.get(*pid) else { continue };
            let mut found = Vec::new();
            self.doc.walk(*pid, |n, _| {
                if matches!(n.kind, NodeKind::Component(_)) {
                    found.push(n.id);
                }
            });
            for c in found {
                let Some(n) = self.doc.get(c) else { continue };
                let set = n.parent.and_then(|p| self.doc.get(p)).filter(|p| matches!(p.kind, NodeKind::ComponentSet(_)));
                // A set is listed once, through its first variant.
                if let Some(set) = set {
                    if set.children.iter().copied().find(|k| matches!(self.doc.get(*k).map(|x| &x.kind), Some(NodeKind::Component(_)))) != Some(c) {
                        continue;
                    }
                }
                let container = set
                    .map(|s| s.parent)
                    .unwrap_or(n.parent)
                    .and_then(|p| self.doc.get(p))
                    .filter(|p| !matches!(p.kind, NodeKind::Page { .. }))
                    .map(|p| p.name.clone())
                    .unwrap_or_default();
                let label = set.map(|s| s.name.clone()).unwrap_or_else(|| n.name.clone());
                out.push(AssetEntry { page: page.name.clone(), container, name: label, id: c, variants: set.map(|s| s.children.len()).unwrap_or(0) });
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::VariantEditKind;

    fn frame(doc: &mut Document, page: NodeId, name: &str, x: f32) -> NodeId {
        let id = doc.alloc_id();
        let mut node = Node::frame(id, Rect::new(x, 0.0, 100.0, 40.0));
        node.name = name.into();
        doc.insert(page, node).unwrap();
        id
    }

    fn apply(doc: &mut Document, cmd: Command) {
        cmd.apply(doc).unwrap();
    }

    fn fixture() -> (App, NodeId, NodeId, NodeId) {
        let mut doc = Document::new("t");
        let page = doc.add_page("P");
        let small = frame(&mut doc, page, "Size=Small", 0.0);
        let large = frame(&mut doc, page, "Size=Large", 150.0);
        let small_cmd = create_component(&mut doc, small);
        apply(&mut doc, small_cmd);
        let large_cmd = create_component(&mut doc, large);
        apply(&mut doc, large_cmd);
        let combine_cmd = combine_as_variants(&mut doc, &[small, large]);
        apply(&mut doc, combine_cmd);
        let set = doc.get(small).unwrap().parent.unwrap();
        (App::new(doc), page, set, small)
    }

    #[test]
    fn variant_editor_adds_renames_and_undoes_as_one_step() {
        let (mut app, _page, set, small) = fixture();
        app.open_add_variant_copy(set, small);
        let edit = app.ui.variant_edit.as_mut().expect("editor");
        edit.values = vec![("Size".into(), "Medium".into())];
        app.submit_variant_edit();
        assert_eq!(components_of(&app.doc, set).len(), 3);
        let medium = app.doc.nodes.values().find(|node| node.name == "Size=Medium").map(|node| node.id).expect("medium variant");
        assert_eq!(app.doc.get(medium).unwrap().parent, Some(set));
        assert_eq!(app.selection, vec![medium], "adding selects the new variant");

        app.open_rename_variant(medium);
        app.ui.variant_edit.as_mut().unwrap().values = vec![("Size".into(), "Wide".into())];
        app.submit_variant_edit();
        assert_eq!(app.doc.get(medium).unwrap().name, "Size=Wide");
        assert_eq!(app.selection, vec![medium], "renaming keeps the variant selected");
        app.undo();
        assert_eq!(app.doc.get(medium).unwrap().name, "Size=Medium", "rename is one undo step");
    }

    #[test]
    fn variant_remove_refuses_instances_and_raw_rename_is_guarded() {
        let (mut app, page, set, small) = fixture();
        let large = components_of(&app.doc, set).into_iter().find(|id| *id != small).unwrap();
        let instance = create_instance(&mut app.doc, large, page, Vec2::new(0.0, 200.0));
        apply(&mut app.doc, instance);
        let error = app.remove_variant(large).unwrap_err();
        assert!(error.contains("instance"), "{error}");
        assert_eq!(components_of(&app.doc, set).len(), 2);

        app.rename(small, "Size=Other");
        assert_eq!(app.doc.get(small).unwrap().name, "Size=Small", "raw rename is refused for variants");
    }

    #[test]
    fn variant_editor_moves_a_standalone_component_into_a_set() {
        let (mut app, page, set, _small) = fixture();
        let standalone = frame(&mut app.doc, page, "Icon", 300.0);
        let cmd = create_component(&mut app.doc, standalone);
        apply(&mut app.doc, cmd);
        app.open_add_variant_move(standalone);
        let edit = app.ui.variant_edit.as_mut().unwrap();
        edit.kind = VariantEditKind::AddMove { source: standalone, set: Some(set) };
        edit.values = vec![("Size".into(), "Icon".into())];
        app.submit_variant_edit();
        assert_eq!(app.doc.get(standalone).unwrap().parent, Some(set));
        assert_eq!(app.doc.get(standalone).unwrap().name, "Size=Icon");
        assert_eq!(app.selection, vec![standalone], "moving selects the new variant");
    }
}
