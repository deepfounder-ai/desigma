//! Document-level editing operations (selection commands, z-order, grouping,
//! alignment, clipboard, pages, file IO). Everything goes through `Tx`.

use crate::app::{App, View};
use crate::canvas::tools::Tool;
use crate::clipboard;
use crate::edit::{Edit, Tx};
use crate::io;
use crate::layout;
use crate::theme;
use crate::util::*;
use desigma_doc::*;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    HCenter,
    Right,
    Top,
    VCenter,
    Bottom,
}

/// Sets size + transform of a node, applying constraints/auto-layout to its
/// content and fixing ancestors. Shared by canvas resizing and the inspector.
pub use desigma_engine::apply_resize;

/// Rotates every `(node, world transform at start)` by `delta` radians about
/// the page-space point `center`, keeping each node's parent untouched.
pub fn rotate_nodes_about(doc: &mut Document, ed: &mut Edit, nodes: &[(NodeId, Affine)], center: Vec2, delta: f32) {
    let rot = Affine::translate(center.x, center.y) * Affine::rotate(delta) * Affine::translate(-center.x, -center.y);
    transform_nodes(doc, ed, nodes, rot);
}

/// Mirrors nodes across the vertical (`horizontal`) or horizontal line through `center`.
pub fn flip_nodes_about(doc: &mut Document, ed: &mut Edit, nodes: &[(NodeId, Affine)], center: Vec2, horizontal: bool) {
    let s = if horizontal { Affine::scale(-1.0, 1.0) } else { Affine::scale(1.0, -1.0) };
    let m = Affine::translate(center.x, center.y) * s * Affine::translate(-center.x, -center.y);
    transform_nodes(doc, ed, nodes, m);
}

/// Applies the page-space transform `rot` to every `(node, world transform at start)`.
fn transform_nodes(doc: &mut Document, ed: &mut Edit, nodes: &[(NodeId, Affine)], rot: Affine) {
    // Compute everything first: relayout of a shared group parent moves its transform.
    let new: Vec<(NodeId, Affine)> = nodes
        .iter()
        .filter_map(|(id, world0)| {
            let pinv = parent_world(doc, *id).invert()?;
            Some((*id, pinv * rot * *world0))
        })
        .collect();
    for (id, t) in &new {
        if let Some(n) = ed.node(doc, *id) {
            n.transform = *t;
        }
    }
    for (id, _) in &new {
        layout::relayout_up(doc, ed, *id);
    }
}

impl App {
    /// Mirrors the selection across the centre of its bounding box.
    pub fn flip_selection(&mut self, horizontal: bool) {
        let tops = self.top_selection();
        let Some(bb) = union_all(tops.iter().map(|i| world_aabb(&self.doc, *i))) else { return };
        let nodes: Vec<(NodeId, Affine)> = tops.iter().map(|i| (*i, self.doc.world_transform(*i))).collect();
        let mut tx = Tx::new();
        flip_nodes_about(&mut self.doc, &mut tx.ed, &nodes, rect_center(&bb), horizontal);
        self.run_tx(tx);
    }

    /// Spaces three or more selected layers evenly along their dominant axis.
    pub fn distribute_selection(&mut self) -> bool {
        let tops = self.top_selection();
        if tops.len() < 3 {
            self.show_toast("Select three or more layers to distribute", false);
            return false;
        }
        let mut items: Vec<(NodeId, Rect)> = tops.iter().map(|i| (*i, world_aabb(&self.doc, *i))).collect();
        let bb = union_all(items.iter().map(|i| i.1)).unwrap_or_default();
        let horizontal = bb.w >= bb.h;
        #[allow(clippy::type_complexity)] // pair of fn-pointer axis accessors
        let (start, len): (fn(&Rect) -> f32, fn(&Rect) -> f32) = if horizontal { (|r| r.x, |r| r.w) } else { (|r| r.y, |r| r.h) };
        items.sort_by(|a, b| start(&a.1).partial_cmp(&start(&b.1)).unwrap_or(std::cmp::Ordering::Equal));
        let total: f32 = items.iter().map(|i| len(&i.1)).sum();
        let first = start(&items[0].1);
        let last = start(&items[items.len() - 1].1) + len(&items[items.len() - 1].1);
        let gap = (last - first - total) / (items.len() as f32 - 1.0);
        let mut tx = Tx::new();
        let mut cursor = first;
        for (id, r) in &items {
            let d = cursor - start(r);
            let page_delta = if horizontal { v2(d, 0.0) } else { v2(0.0, d) };
            let dp = delta_to_parent(&self.doc, *id, page_delta);
            if let Some(n) = tx.ed.node(&mut self.doc, *id) {
                n.transform.tx += dp.x;
                n.transform.ty += dp.y;
            }
            cursor += len(r) + gap;
        }
        for (id, _) in &items {
            layout::relayout_up(&mut self.doc, &mut tx.ed, *id);
        }
        self.run_tx(tx);
        true
    }

    /// Rotates the selection by `delta` radians around the centre of its bounding box.
    pub fn rotate_selection(&mut self, delta: f32) {
        let tops = self.top_selection();
        let Some(bb) = union_all(tops.iter().map(|i| world_aabb(&self.doc, *i))) else { return };
        let nodes: Vec<(NodeId, Affine)> = tops.iter().map(|i| (*i, self.doc.world_transform(*i))).collect();
        let mut tx = Tx::new();
        rotate_nodes_about(&mut self.doc, &mut tx.ed, &nodes, rect_center(&bb), delta);
        self.run_tx(tx);
    }

    // ------------------------------------------------------------ plumbing
    pub fn run_tx(&mut self, tx: Tx) -> bool {
        if self.read_only() {
            // an old version, or a file shared view-only, is shown read-only
            tx.cancel(&mut self.doc);
            self.changed();
            let msg = if self.history.viewing.is_some() { "Viewing an old version is read-only" } else { "The host shared this file as view only" };
            self.show_toast(msg, true);
            return false;
        }
        if self.scrub.is_some() {
            // Scrubbing: edits stay live in the document; `scrub_end` records them as one step.
            self.changed();
            return true;
        }
        match tx.commit(&mut self.doc, &mut self.hist) {
            Ok(changed) => {
                self.changed();
                self.sanitize();
                changed
            }
            Err(e) => {
                self.show_toast(format!("Edit failed: {e}"), true);
                false
            }
        }
    }

    /// Applies `f` to each node and commits one undo step.
    pub fn edit_nodes(&mut self, ids: &[NodeId], mut f: impl FnMut(&mut Node)) {
        let mut tx = Tx::new();
        for id in ids {
            if let Some(n) = tx.ed.node(&mut self.doc, *id) {
                f(n);
            }
            layout::relayout_around(&mut self.doc, &mut tx.ed, *id);
        }
        self.run_tx(tx);
    }

    /// Starts/ends a scrub session following the flag set by the numeric fields.
    pub fn scrub_sync(&mut self, ctx: &egui::Context) {
        let id = egui::Id::new(crate::widgets::field::SCRUB_FLAG);
        let mut active = ctx.data(|d| d.get_temp::<bool>(id)).unwrap_or(false);
        if active && !ctx.input(|i| i.pointer.any_down()) {
            active = false;
            ctx.data_mut(|d| d.insert_temp(id, false));
        }
        match (active, self.scrub.is_some()) {
            (true, false) => self.scrub = Some(self.doc.nodes.clone()),
            (false, true) => self.scrub_end(),
            _ => {}
        }
    }

    /// Folds everything changed since the scrub began into one undo step.
    pub fn scrub_end(&mut self) {
        let Some(base) = self.scrub.take() else { return };
        let changed: Vec<NodeId> = base.iter().filter(|(id, old)| self.doc.nodes.get(id).is_some_and(|cur| cur != *old)).map(|(id, _)| *id).collect();
        let mut updates: Vec<Command> = changed.iter().filter_map(|id| self.doc.get(*id)).map(|n| Command::Update { node: Box::new(n.clone()) }).collect();
        crate::comp_ops::record_overrides(&self.doc, &base, &mut updates);
        for id in &changed {
            if let (Some(old), Some(cur)) = (base.get(id), self.doc.get_mut(*id)) {
                *cur = old.clone();
            }
        }
        if updates.is_empty() {
            return;
        }
        let cmd = if updates.len() == 1 { updates.into_iter().next().unwrap() } else { Command::Batch(updates) };
        let cmd = crate::comp_ops::with_sync(&mut self.doc, cmd);
        if let Err(e) = self.hist.exec(&mut self.doc, cmd) {
            self.show_toast(format!("Edit failed: {e}"), true);
        }
        self.changed();
        self.sanitize();
    }

    /// Returns whether the document changed (a history step or a pending version restore).
    pub fn undo(&mut self) -> bool {
        if self.try_undo_restore() {
            return true;
        }
        match self.hist.undo(&mut self.doc) {
            Ok(true) => {
                self.changed();
                self.sanitize();
                true
            }
            Ok(false) => false,
            // a person in the session deleted what this step touches: nothing to undo
            Err(desigma_doc::DocError::NotFound(_)) if self.collab.active() => false,
            Err(e) => {
                self.show_toast(format!("Undo failed: {e}"), true);
                false
            }
        }
    }

    /// Returns whether the document changed (a history step or a pending version restore).
    pub fn redo(&mut self) -> bool {
        if self.try_redo_restore() {
            return true;
        }
        match self.hist.redo(&mut self.doc) {
            Ok(true) => {
                self.changed();
                self.sanitize();
                true
            }
            Ok(false) => false,
            // a person in the session deleted what this step touches: nothing to redo
            Err(desigma_doc::DocError::NotFound(_)) if self.collab.active() => false,
            Err(e) => {
                self.show_toast(format!("Redo failed: {e}"), true);
                false
            }
        }
    }

    /// Selected nodes that have no selected ancestor, in z/document order.
    pub fn top_selection(&self) -> Vec<NodeId> {
        self.selection.iter().copied().filter(|id| !self.selection.iter().any(|o| o != id && self.doc.is_ancestor(*o, *id))).collect()
    }

    pub fn selection_bounds(&self) -> Option<Rect> {
        union_all(self.selection.iter().map(|id| world_aabb(&self.doc, *id)))
    }

    pub fn unique_name(&self, base: &str) -> String {
        let n = self.doc.nodes.values().filter(|n| n.name.starts_with(base)).count();
        format!("{base} {}", n + 1)
    }

    pub(crate) fn container_for_insert(&self) -> NodeId {
        if let [only] = self.selection.as_slice() {
            if let Some(n) = self.doc.get(*only) {
                if n.kind.is_container() && !matches!(n.kind, NodeKind::Instance { .. } | NodeKind::Table(_)) {
                    return *only;
                }
                if let Some(p) = n.parent {
                    return p;
                }
            }
        }
        self.page
    }

    // ------------------------------------------------------------ delete / duplicate
    pub fn delete_selection(&mut self) {
        let tops = self.top_selection();
        if tops.is_empty() {
            return;
        }
        let mut tx = Tx::new();
        let mut parents = Vec::new();
        for id in &tops {
            // Layers inside an instance cannot be removed, only hidden (like Figma).
            if self.doc.get(*id).is_some_and(|n| is_linked(n) && !is_top_instance(n)) {
                if let Some(n) = tx.ed.node(&mut self.doc, *id) {
                    n.visible = false;
                }
                continue;
            }
            if let Some(p) = self.doc.get(*id).and_then(|n| n.parent) {
                if !parents.contains(&p) {
                    parents.push(p);
                }
            }
            if let Err(e) = tx.cmd(&mut self.doc, Command::Remove { id: *id }) {
                self.show_toast(format!("Delete failed: {e}"), true);
            }
        }
        for p in parents {
            if self.doc.get(p).is_some() {
                layout::relayout_around(&mut self.doc, &mut tx.ed, p);
            }
        }
        self.run_tx(tx);
        self.selection.clear();
    }

    pub fn duplicate_selection(&mut self) {
        let tops = self.top_selection();
        if tops.is_empty() {
            return;
        }
        let mut tx = Tx::new();
        let mut new_sel = Vec::new();
        // one clip for the whole selection, so connectors between duplicated nodes join the copies
        let copies = clipboard::capture(&self.doc, &tops).map(|c| clipboard::instantiate(&mut self.doc, &c)).unwrap_or_default();
        for (id, mut nodes) in tops.into_iter().zip(copies) {
            let Some(parent) = self.doc.get(id).and_then(|n| n.parent) else { continue };
            let index = self.doc.get(parent).and_then(|p| p.children.iter().position(|c| *c == id)).map(|i| i + 1).unwrap_or(usize::MAX);
            let in_al = self.doc.get(parent).and_then(|p| p.kind.frame_props()).map(|f| f.auto_layout.is_some()).unwrap_or(false);
            if let Some(root) = nodes.first_mut() {
                if !in_al {
                    root.transform.tx += theme::PASTE_OFFSET;
                    root.transform.ty += theme::PASTE_OFFSET;
                }
                root.name = self.doc.get(id).map(|n| n.name.clone()).unwrap_or_default();
                new_sel.push(root.id);
            }
            if tx.cmd(&mut self.doc, Command::Insert { parent, index, nodes }).is_ok() {
                layout::relayout_around(&mut self.doc, &mut tx.ed, parent);
            }
        }
        self.run_tx(tx);
        self.selection = new_sel;
    }

    // ------------------------------------------------------------ clipboard
    pub fn copy_selection(&mut self) -> Option<String> {
        let tops = self.top_selection();
        let clip = clipboard::capture(&self.doc, &tops)?;
        let json = clipboard::to_json(&clip);
        self.clip = Some(clip);
        self.paste_count = 0;
        json
    }

    pub fn cut_selection(&mut self) -> Option<String> {
        let s = self.copy_selection();
        self.delete_selection();
        s
    }

    pub fn paste(&mut self, system_text: Option<&str>) {
        // SVG markup on the system clipboard becomes layers.
        if system_text.is_some_and(|t| clipboard::paste_svg(self, t)) {
            return;
        }
        let clip = system_text.and_then(clipboard::from_json).or_else(|| self.clip.clone());
        let Some(clip) = clip else { return };
        let parent = self.container_for_insert();
        let lists = clipboard::instantiate(&mut self.doc, &clip);
        let mut tx = Tx::new();
        let mut new_sel = Vec::new();
        let off = theme::PASTE_OFFSET * self.paste_count as f32;
        for mut nodes in lists {
            if let Some(root) = nodes.first_mut() {
                root.transform.tx += off;
                root.transform.ty += off;
                new_sel.push(root.id);
            }
            let index = self.doc.get(parent).map(|p| p.children.len()).unwrap_or(0);
            let _ = tx.cmd(&mut self.doc, Command::Insert { parent, index, nodes });
        }
        layout::relayout_around(&mut self.doc, &mut tx.ed, parent);
        self.paste_count += 1;
        self.run_tx(tx);
        self.selection = new_sel;
    }

    // ------------------------------------------------------------ grouping
    /// Wraps `ids` (all siblings, in desired child order) into a new group or
    /// frame; returns the container id.
    fn wrap_in_container(&mut self, tx: &mut Tx, ids: &[NodeId], frame: bool) -> Option<NodeId> {
        let kind = if frame { NodeKind::Frame(FrameProps::default()) } else { NodeKind::Group };
        self.wrap_in_kind(tx, ids, kind)
    }

    /// Wraps `ids` into a new container of `kind` (group, frame or boolean group).
    pub(crate) fn wrap_in_kind(&mut self, tx: &mut Tx, ids: &[NodeId], kind: NodeKind) -> Option<NodeId> {
        let frame = matches!(kind, NodeKind::Frame(_));
        let first = *ids.first()?;
        let parent = self.doc.get(first)?.parent?;
        let siblings = &self.doc.get(parent)?.children;
        let max_index = ids.iter().filter_map(|i| siblings.iter().position(|c| c == i)).max()?;
        let pinv = self.doc.world_transform(parent).invert()?;
        let mut pts = Vec::new();
        let mut rel: Vec<(NodeId, Affine)> = Vec::new();
        for id in ids {
            let w = self.doc.world_transform(*id);
            let size = self.doc.get(*id)?.size;
            pts.extend(rect_corners(&Rect::new(0.0, 0.0, size.x, size.y)).map(|p| pinv.apply(w.apply(p))));
            rel.push((*id, pinv * w));
        }
        let bb = aabb_of(&pts);
        let gid = self.doc.alloc_id();
        let name = kind.default_name();
        let mut g = if frame { Node::frame(gid, bb) } else { Node::new(gid, kind) };
        if frame {
            g.fills.clear();
        }
        g.name = self.unique_name(name);
        g.transform = Affine::translate(bb.x, bb.y);
        g.size = v2(bb.w, bb.h);
        tx.cmd(&mut self.doc, Command::Insert { parent, index: max_index + 1, nodes: vec![g] }).ok()?;
        for (i, (id, in_parent)) in rel.into_iter().enumerate() {
            tx.cmd(&mut self.doc, Command::Reparent { id, parent: gid, index: i }).ok()?;
            if let Some(n) = tx.ed.node(&mut self.doc, id) {
                n.transform = Affine::translate(-bb.x, -bb.y) * in_parent;
            }
        }
        layout::relayout_up(&mut self.doc, &mut tx.ed, gid);
        Some(gid)
    }

    pub(crate) fn same_parent_tops(&self) -> Vec<NodeId> {
        let mut tops = self.top_selection();
        let Some(first) = tops.first().copied() else { return tops };
        let parent = self.doc.get(first).and_then(|n| n.parent);
        tops.retain(|t| self.doc.get(*t).and_then(|n| n.parent) == parent);
        if let Some(p) = parent.and_then(|p| self.doc.get(p)) {
            tops.sort_by_key(|t| p.children.iter().position(|c| c == t).unwrap_or(0));
        }
        tops
    }

    pub fn group_selection(&mut self, frame: bool) {
        let ids = self.same_parent_tops();
        if ids.is_empty() {
            return;
        }
        let mut tx = Tx::new();
        let g = self.wrap_in_container(&mut tx, &ids, frame);
        self.run_tx(tx);
        if let Some(g) = g {
            self.selection = vec![g];
        }
    }

    pub fn ungroup_selection(&mut self) {
        let tops = self.top_selection();
        let mut tx = Tx::new();
        let mut new_sel = Vec::new();
        for id in tops {
            let Some(g) = self.doc.get(id).cloned() else { continue };
            let is_group = matches!(g.kind, NodeKind::Group) || (matches!(g.kind, NodeKind::Frame(_)) && g.parent.is_some());
            let Some(parent) = g.parent else { continue };
            if !is_group || g.children.is_empty() {
                continue;
            }
            let mut index = self.doc.get(parent).and_then(|p| p.children.iter().position(|c| *c == id)).unwrap_or(0);
            for c in g.children.clone() {
                let t = self.doc.get(c).map(|n| n.transform).unwrap_or_default();
                if tx.cmd(&mut self.doc, Command::Reparent { id: c, parent, index }).is_ok() {
                    if let Some(n) = tx.ed.node(&mut self.doc, c) {
                        n.transform = g.transform * t;
                    }
                    new_sel.push(c);
                    index += 1;
                }
            }
            let _ = tx.cmd(&mut self.doc, Command::Remove { id });
            layout::relayout_around(&mut self.doc, &mut tx.ed, parent);
        }
        self.run_tx(tx);
        if !new_sel.is_empty() {
            self.selection = new_sel;
        }
    }

    /// Shift+A: wraps the selection in an auto-layout frame, or toggles
    /// auto-layout on a selected frame.
    pub fn toggle_auto_layout(&mut self) {
        let tops = self.same_parent_tops();
        if tops.is_empty() {
            return;
        }
        let mut tx = Tx::new();
        let single_frame = tops.len() == 1 && self.doc.get(tops[0]).map(|n| n.kind.frame_props().is_some() && n.parent.is_some()).unwrap_or(false);
        let target = if single_frame {
            tops[0]
        } else {
            // Order children by position along the dominant axis.
            let rects: Vec<(NodeId, Rect)> = tops.iter().map(|t| (*t, world_aabb(&self.doc, *t))).collect();
            let bb = union_all(rects.iter().map(|r| r.1)).unwrap_or_default();
            let horizontal = {
                let xs = rects.iter().map(|r| r.1.x + r.1.w / 2.0);
                let ys = rects.iter().map(|r| r.1.y + r.1.h / 2.0);
                let spread = |it: &mut dyn Iterator<Item = f32>| {
                    let v: Vec<f32> = it.collect();
                    v.iter().cloned().fold(f32::MIN, f32::max) - v.iter().cloned().fold(f32::MAX, f32::min)
                };
                spread(&mut xs.clone()) >= spread(&mut ys.clone()) || bb.w >= bb.h
            };
            let mut ordered = rects.clone();
            ordered.sort_by(|a, b| {
                let (ka, kb) = if horizontal { (a.1.x, b.1.x) } else { (a.1.y, b.1.y) };
                ka.partial_cmp(&kb).unwrap_or(std::cmp::Ordering::Equal)
            });
            let ids: Vec<NodeId> = ordered.iter().map(|r| r.0).collect();
            let Some(g) = self.wrap_in_container(&mut tx, &ids, true) else { return };
            if let Some(n) = tx.ed.node(&mut self.doc, g) {
                if let Some(fp) = n.kind.frame_props_mut() {
                    fp.auto_layout = Some(AutoLayout {
                        direction: if horizontal { LayoutDirection::Horizontal } else { LayoutDirection::Vertical },
                        item_spacing: theme::AUTO_LAYOUT_SPACING,
                        padding: [theme::AUTO_LAYOUT_PADDING; 4],
                        primary_sizing: Sizing::Hug,
                        counter_sizing: Sizing::Hug,
                        ..Default::default()
                    });
                }
            }
            layout::relayout_around(&mut self.doc, &mut tx.ed, g);
            self.run_tx(tx);
            self.selection = vec![g];
            return;
        };
        if let Some(n) = tx.ed.node(&mut self.doc, target) {
            if let Some(fp) = n.kind.frame_props_mut() {
                fp.auto_layout = match fp.auto_layout {
                    Some(_) => None,
                    None => Some(AutoLayout {
                        item_spacing: theme::AUTO_LAYOUT_SPACING,
                        padding: [theme::AUTO_LAYOUT_PADDING; 4],
                        primary_sizing: Sizing::Hug,
                        counter_sizing: Sizing::Hug,
                        ..Default::default()
                    }),
                };
            }
        }
        layout::relayout_around(&mut self.doc, &mut tx.ed, target);
        self.run_tx(tx);
    }

    // ------------------------------------------------------------ transforms
    pub fn nudge(&mut self, dx: f32, dy: f32) {
        let tops = self.top_selection();
        let mut tx = Tx::new();
        for id in &tops {
            let d = delta_to_parent(&self.doc, *id, v2(dx, dy));
            if let Some(n) = tx.ed.node(&mut self.doc, *id) {
                n.transform.tx += d.x;
                n.transform.ty += d.y;
            }
            layout::relayout_up(&mut self.doc, &mut tx.ed, *id);
        }
        self.run_tx(tx);
    }

    pub fn align(&mut self, a: Align) {
        let tops = self.top_selection();
        if tops.is_empty() {
            return;
        }
        let target = if tops.len() == 1 {
            match self.doc.get(tops[0]).and_then(|n| n.parent) {
                Some(p) if !matches!(self.doc.get(p).map(|n| &n.kind), Some(NodeKind::Page { .. })) => world_aabb(&self.doc, p),
                _ => world_aabb(&self.doc, tops[0]),
            }
        } else {
            union_all(tops.iter().map(|t| world_aabb(&self.doc, *t))).unwrap_or_default()
        };
        let mut tx = Tx::new();
        for id in &tops {
            let r = world_aabb(&self.doc, *id);
            let d = match a {
                Align::Left => v2(target.x - r.x, 0.0),
                Align::HCenter => v2(target.x + target.w / 2.0 - (r.x + r.w / 2.0), 0.0),
                Align::Right => v2(target.x + target.w - (r.x + r.w), 0.0),
                Align::Top => v2(0.0, target.y - r.y),
                Align::VCenter => v2(0.0, target.y + target.h / 2.0 - (r.y + r.h / 2.0)),
                Align::Bottom => v2(0.0, target.y + target.h - (r.y + r.h)),
            };
            let d = delta_to_parent(&self.doc, *id, d);
            if let Some(n) = tx.ed.node(&mut self.doc, *id) {
                n.transform.tx += d.x;
                n.transform.ty += d.y;
            }
            layout::relayout_up(&mut self.doc, &mut tx.ed, *id);
        }
        self.run_tx(tx);
    }

    /// Moves the selection one step (or to the extreme) in z-order.
    pub fn reorder(&mut self, forward: bool, extreme: bool) {
        let tops = self.same_parent_tops();
        let Some(parent) = tops.first().and_then(|t| self.doc.get(*t)).and_then(|n| n.parent) else { return };
        let len = self.doc.get(parent).map(|p| p.children.len()).unwrap_or(0);
        let mut tx = Tx::new();
        let order: Vec<NodeId> = if forward { tops.iter().rev().copied().collect() } else { tops.clone() };
        for (k, id) in order.iter().enumerate() {
            let cur = self.doc.get(parent).and_then(|p| p.children.iter().position(|c| c == id)).unwrap_or(0);
            let target = match (forward, extreme) {
                (true, true) => len.saturating_sub(1),
                (false, true) => k,
                (true, false) => (cur + 1).min(len.saturating_sub(1)),
                (false, false) => cur.saturating_sub(1),
            };
            if target != cur {
                let _ = tx.cmd(&mut self.doc, Command::Reparent { id: *id, parent, index: target });
            }
        }
        layout::relayout_around(&mut self.doc, &mut tx.ed, parent);
        self.run_tx(tx);
    }

    /// Reparents `ids` into `new_parent` at `index` (index counted after the
    /// dragged nodes are removed), preserving their page position.
    pub fn move_to_parent(&mut self, ids: &[NodeId], new_parent: NodeId, index: usize) {
        let Some(npw) = self.doc.world_transform(new_parent).invert() else { return };
        let mut tx = Tx::new();
        let mut old_parents = Vec::new();
        for (i, id) in ids.iter().enumerate() {
            if *id == new_parent || self.doc.is_ancestor(*id, new_parent) {
                continue;
            }
            let world = self.doc.world_transform(*id);
            if let Some(p) = self.doc.get(*id).and_then(|n| n.parent) {
                old_parents.push(p);
            }
            if let Err(e) = tx.cmd(&mut self.doc, Command::Reparent { id: *id, parent: new_parent, index: index + i }) {
                self.show_toast(format!("Cannot move layer: {e}"), true);
                continue;
            }
            if let Some(n) = tx.ed.node(&mut self.doc, *id) {
                n.transform = npw * world;
            }
        }
        for p in old_parents {
            if self.doc.get(p).is_some() {
                layout::relayout_around(&mut self.doc, &mut tx.ed, p);
            }
        }
        layout::relayout_around(&mut self.doc, &mut tx.ed, new_parent);
        self.run_tx(tx);
    }

    // ------------------------------------------------------------ naming / flags
    pub fn rename(&mut self, id: NodeId, name: &str) {
        let name = name.trim().to_string();
        if name.is_empty() {
            return;
        }
        self.edit_nodes(&[id], |n| n.name = name.clone());
    }

    pub fn toggle_visible(&mut self, id: NodeId) {
        self.edit_nodes(&[id], |n| n.visible = !n.visible);
    }

    pub fn toggle_lock(&mut self, id: NodeId) {
        self.edit_nodes(&[id], |n| n.locked = !n.locked);
    }

    // ------------------------------------------------------------ pages
    /// Pre-order copy of a subtree (root first), as `InsertPage` expects.
    fn subtree_nodes(&self, id: NodeId) -> Vec<Node> {
        let mut out = Vec::new();
        self.doc.walk(id, |n, _| out.push(n.clone()));
        out
    }

    fn unique_page_name(&self, base: &str) -> String {
        let names: Vec<&str> = self.doc.pages.iter().filter_map(|p| self.doc.get(*p)).map(|n| n.name.as_str()).collect();
        if !names.contains(&base) {
            return base.to_string();
        }
        (2..).map(|i| format!("{base} {i}")).find(|c| !names.contains(&c.as_str())).unwrap_or_else(|| base.to_string())
    }

    /// Appends a new empty page (undoable) and switches to it.
    pub fn add_page(&mut self) -> Option<NodeId> {
        crate::decks::ui::flush_notes(self);
        let id = self.doc.alloc_id();
        let mut page = Node::new(id, NodeKind::Page { background: Color::from_hex(&format!("{:06X}", theme::DEFAULT_PAGE_BG)).unwrap_or(Color::WHITE) });
        page.name = format!("Page {}", self.doc.pages.len() + 1);
        let index = self.doc.pages.len();
        let mut tx = Tx::new();
        tx.cmd(&mut self.doc, Command::InsertPage { index, nodes: vec![page] }).ok()?;
        self.run_tx(tx);
        self.switch_page(id);
        Some(id)
    }

    /// Duplicates a page with all its layers right after it; returns the copy.
    pub fn duplicate_page(&mut self, id: NodeId) -> Option<NodeId> {
        let index = self.doc.pages.iter().position(|p| *p == id)? + 1;
        crate::decks::ui::flush_notes(self);
        let mut nodes = clipboard::clone_subtree(&mut self.doc, id);
        let root = nodes.first_mut()?;
        root.name = self.unique_page_name(&format!("{} copy", self.doc.get(id)?.name));
        let new_id = root.id;
        let mut tx = Tx::new();
        tx.cmd(&mut self.doc, Command::InsertPage { index, nodes }).ok()?;
        self.run_tx(tx);
        self.switch_page(new_id);
        Some(new_id)
    }

    /// Deletes a page; the last remaining page can never be deleted.
    pub fn delete_page(&mut self, id: NodeId) -> bool {
        if self.doc.pages.len() <= 1 || !self.doc.pages.contains(&id) {
            return false;
        }
        crate::decks::ui::flush_notes(self);
        let pos = self.doc.pages.iter().position(|p| *p == id).unwrap_or(0);
        let mut tx = Tx::new();
        if tx.cmd(&mut self.doc, Command::RemovePage { id }).is_err() {
            return false;
        }
        self.run_tx(tx);
        if self.page == id || !self.doc.nodes.contains_key(&self.page) {
            let next = self.doc.pages[pos.min(self.doc.pages.len() - 1)];
            self.switch_page(next);
        }
        true
    }

    /// Moves a page to `index` (position after removal) in the page list.
    pub fn move_page(&mut self, id: NodeId, index: usize) {
        let Some(from) = self.doc.pages.iter().position(|p| *p == id) else { return };
        let index = index.min(self.doc.pages.len().saturating_sub(1));
        if from == index {
            return;
        }
        crate::decks::ui::flush_notes(self);
        let nodes = self.subtree_nodes(id);
        let mut tx = Tx::new();
        if tx.cmd(&mut self.doc, Command::RemovePage { id }).is_ok() {
            let _ = tx.cmd(&mut self.doc, Command::InsertPage { index, nodes });
        }
        self.run_tx(tx);
    }

    pub fn switch_page(&mut self, id: NodeId) {
        if self.doc.pages.contains(&id) {
            crate::decks::ui::flush_notes(self);
            self.page = id;
            self.selection.clear();
            self.canvas.reset_interaction();
            self.changed();
            self.fit_all();
        }
    }

    // ------------------------------------------------------------ view
    pub fn page_bounds(&self) -> Option<Rect> {
        let kids = &self.doc.get(self.page)?.children;
        union_all(kids.iter().filter(|c| self.doc.get(**c).map(|n| n.visible).unwrap_or(false)).map(|c| world_aabb(&self.doc, *c)))
    }

    pub fn fit_all(&mut self) {
        let b = self.page_bounds().unwrap_or(Rect::new(0.0, 0.0, 800.0, 600.0));
        self.view = View::fit(b, self.canvas_rect.size());
    }

    pub fn fit_selection(&mut self) {
        match self.selection_bounds() {
            Some(b) => self.view = View::fit(b, self.canvas_rect.size()),
            None => self.fit_all(),
        }
    }

    /// Zooms to `zoom` (a factor) centred on the selection, else on the page content.
    pub fn zoom_on_content(&mut self, zoom: f32) {
        let b = self.selection_bounds().or_else(|| self.page_bounds()).unwrap_or(Rect::new(0.0, 0.0, 800.0, 600.0));
        let z = zoom.clamp(theme::ZOOM_MIN, theme::ZOOM_MAX);
        let size = self.canvas_rect.size();
        self.view = View { zoom: z, offset: Vec2::new(b.x + b.w / 2.0 - size.x / 2.0 / z, b.y + b.h / 2.0 - size.y / 2.0 / z) };
    }

    pub fn zoom_to(&mut self, zoom: f32) {
        let c = self.canvas_rect.center();
        self.view.zoom_at(self.canvas_rect.min, c, zoom);
    }

    pub fn zoom_by(&mut self, factor: f32) {
        self.zoom_to(self.view.zoom * factor);
    }

    // ------------------------------------------------------------ files
    pub fn replace_document(&mut self, doc: Document, path: Option<PathBuf>) {
        crate::decks::ui::flush_notes(self);
        self.end_collab();
        self.stop_plugin();
        // Pending edits of the library file being left are saved first (from the live state, not an old version).
        self.exit_version_view();
        self.flush_autosave();
        self.lib_file = None;
        self.renaming_file = false;
        let mut doc = doc;
        io::ensure_page(&mut doc);
        self.page = doc.pages[0];
        self.doc = doc;
        self.decks = Default::default();
        self.hist = History::default();
        self.history.reset();
        self.comments.close_popovers();
        self.selection.clear();
        self.hover = None;
        self.file_path = path;
        self.file_hash = Some(crate::home::session::doc_hash(&self.doc));
        if let Some(b) = &self.bridge.bridge {
            b.update_document(&self.doc.name, self.file_path.as_deref());
        }
        // Watch this file for edits made by other programs (generators, sync clients).
        match self.file_path.clone() {
            Some(p) => self.watch_external(&p),
            None => self.stop_watching(),
        }
        self.canvas.reset_interaction();
        // a design tool means nothing on a board and the other way round
        self.tool = Tool::Move;
        self.board.dots = None;
        self.board.target = None;
        self.board.tool_menu = None;
        self.board.bar_menu = None;
        self.layers.reset();
        self.fonts.load_for(&self.doc);
        self.changed();
        self.fit_all();
    }

    pub fn open_dialog(&mut self) {
        if let Some(p) = io::pick_open() {
            self.request_open(crate::instance::Open::File(p));
        }
    }

    pub fn open_path(&mut self, p: PathBuf) {
        // A variables-only JSON opened alongside a design merges into it.
        if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("json")) {
            if let Ok(text) = std::fs::read_to_string(&p) {
                // Figma REST comments (`/v1/files/:key/comments`) are merged the same way.
                if crate::comments::import::is_comments_payload(&text) {
                    self.import_comments_file(&p);
                    return;
                }
                if crate::variables::is_variables_payload(&text) {
                    self.import_variables_file(&p);
                    return;
                }
            }
        }
        let again = p.clone();
        if !self.leave_ok(move |app| app.open_path(again)) {
            return;
        }
        match io::load_path_noted(&p) {
            Ok((doc, notes)) => {
                let name = doc.name.clone();
                // An SVG becomes a new unsaved document, never overwritten by Save.
                let svg = p.extension().is_some_and(|e| e.eq_ignore_ascii_case("svg"));
                self.replace_document(doc, (!svg).then_some(p));
                match notes.split_first() {
                    None => self.show_toast(format!("Opened {name}"), false),
                    // What an import left out, so a missing script or icon is not a surprise.
                    Some((first, rest)) => {
                        let more = if rest.is_empty() { String::new() } else { format!(" (and {} more)", rest.len()) };
                        self.show_toast(format!("Opened {name}. Not carried over: {first}{more}"), false);
                    }
                }
            }
            Err(e) => self.show_toast(e, true),
        }
    }

    pub fn save(&mut self, as_new: bool) {
        crate::decks::ui::flush_notes(self);
        // Library files save back to the library; "Save as…" writes an external copy.
        if !as_new && self.lib_file.is_some() {
            self.save_library_file_with_toast();
            return;
        }
        let target = match (&self.file_path, as_new) {
            (Some(p), false) if p.extension().map(|e| e == io::NATIVE_EXT).unwrap_or(false) => Some(p.clone()),
            _ => io::pick_save(&self.doc.name),
        };
        let Some(path) = target else { return };
        match io::save_path(&self.doc, &path) {
            Ok(p) => {
                self.show_toast(format!("Saved {}", p.display()), false);
                if self.lib_file.is_none() {
                    self.file_path = Some(p.clone());
                    self.file_hash = Some(crate::home::session::doc_hash(&self.doc));
                    self.watch_external(&p);
                }
            }
            Err(e) => self.show_toast(e, true),
        }
    }

    pub fn new_document(&mut self) {
        if !self.leave_ok(|app| app.new_document()) {
            return;
        }
        let mut d = Document::new("Untitled");
        d.add_page("Page 1");
        self.replace_document(d, None);
    }

    /// A new, empty board (whiteboard) document.
    pub fn new_board_document(&mut self) {
        if !self.leave_ok(|app| app.new_board_document()) {
            return;
        }
        self.replace_document(crate::board::create::new_document("Untitled"), None);
    }

    pub fn export_selection(&mut self) {
        let Some(id) = self.selection.first().copied() else {
            self.show_toast("Select a layer to export", true);
            return;
        };
        let name = self.doc.get(id).map(|n| n.name.clone()).unwrap_or_default();
        let Some(path) = io::pick_export(&name) else { return };
        let png = desigma_export::export_png(&self.export_doc(), id, 1.0);
        if png.is_empty() {
            self.show_toast("Nothing to export", true);
            return;
        }
        match std::fs::write(&path, png) {
            Ok(()) => self.show_toast(format!("Exported {}", path.display()), false),
            Err(e) => self.show_toast(format!("Export failed: {e}"), true),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app_with_rects() -> (App, Vec<NodeId>) {
        let mut d = Document::new("t");
        let page = d.add_page("p");
        let mut ids = vec![];
        for i in 0..3 {
            let id = d.alloc_id();
            let r = d.insert(page, Node::rectangle(id, Rect::new(i as f32 * 100.0, 20.0 * i as f32, 50.0, 50.0))).unwrap();
            ids.push(r);
        }
        (App::new(d), ids)
    }

    #[test]
    fn group_and_undo() {
        let (mut app, ids) = app_with_rects();
        app.selection = vec![ids[0], ids[1]];
        app.group_selection(false);
        let g = app.selection[0];
        let gn = app.doc.get(g).unwrap();
        assert!(matches!(gn.kind, NodeKind::Group));
        assert_eq!(gn.children.len(), 2);
        assert_eq!(gn.size, v2(150.0, 70.0));
        // world position preserved
        assert_eq!(world_aabb(&app.doc, ids[1]).x, 100.0);
        app.undo();
        assert_eq!(app.doc.get(ids[0]).unwrap().parent, Some(app.page));
        assert_eq!(app.doc.get(app.page).unwrap().children.len(), 3);
    }

    #[test]
    fn delete_duplicate_paste() {
        let (mut app, ids) = app_with_rects();
        app.selection = vec![ids[0]];
        app.duplicate_selection();
        assert_eq!(app.doc.get(app.page).unwrap().children.len(), 4);
        assert_ne!(app.selection[0], ids[0]);
        app.delete_selection();
        assert_eq!(app.doc.get(app.page).unwrap().children.len(), 3);
        app.selection = vec![ids[2]];
        let json = app.copy_selection().unwrap();
        app.paste(Some(&json));
        assert_eq!(app.doc.get(app.page).unwrap().children.len(), 4);
        app.undo();
        assert_eq!(app.doc.get(app.page).unwrap().children.len(), 3);
    }

    #[test]
    fn align_left_multi() {
        let (mut app, ids) = app_with_rects();
        app.selection = ids.clone();
        app.align(Align::Left);
        for id in ids {
            assert_eq!(world_aabb(&app.doc, id).x, 0.0);
        }
    }

    #[test]
    fn nudge_and_reorder() {
        let (mut app, ids) = app_with_rects();
        app.selection = vec![ids[0]];
        app.nudge(10.0, 0.0);
        assert_eq!(app.doc.get(ids[0]).unwrap().transform.tx, 10.0);
        app.reorder(true, true);
        assert_eq!(app.doc.get(app.page).unwrap().children.last(), Some(&ids[0]));
        app.reorder(false, true);
        assert_eq!(app.doc.get(app.page).unwrap().children.first(), Some(&ids[0]));
    }

    #[test]
    fn auto_layout_wrap_orders_by_x() {
        let (mut app, ids) = app_with_rects();
        app.selection = vec![ids[2], ids[0]];
        app.toggle_auto_layout();
        let g = app.selection[0];
        let gn = app.doc.get(g).unwrap();
        assert!(gn.kind.frame_props().unwrap().auto_layout.is_some());
        assert_eq!(gn.children, vec![ids[0], ids[2]]);
        // 10 padding + 50 + 10 spacing + 50 + 10 padding
        assert_eq!(gn.size.x, 130.0);
    }

    #[test]
    fn move_to_parent_keeps_world_position() {
        let (mut app, ids) = app_with_rects();
        let fid = app.doc.alloc_id();
        let frame = app.doc.insert(app.page, Node::frame(fid, Rect::new(500.0, 500.0, 300.0, 300.0))).unwrap();
        let before = world_aabb(&app.doc, ids[1]);
        app.move_to_parent(&[ids[1]], frame, 0);
        assert_eq!(app.doc.get(ids[1]).unwrap().parent, Some(frame));
        let after = world_aabb(&app.doc, ids[1]);
        assert_eq!((before.x, before.y), (after.x, after.y));
        app.undo();
        assert_eq!(app.doc.get(ids[1]).unwrap().parent, Some(app.page));
    }

    #[test]
    fn ungroup_restores_children() {
        let (mut app, ids) = app_with_rects();
        app.selection = vec![ids[0], ids[1]];
        app.group_selection(false);
        app.ungroup_selection();
        assert_eq!(app.doc.get(ids[0]).unwrap().parent, Some(app.page));
        assert_eq!(world_aabb(&app.doc, ids[1]).x, 100.0);
    }

    #[test]
    fn pages_add_rename_duplicate_delete_are_undoable() {
        let (mut app, ids) = app_with_rects();
        let p1 = app.page;
        let p2 = app.add_page().unwrap();
        assert_eq!(app.doc.pages, vec![p1, p2]);
        assert_eq!(app.page, p2);
        app.rename(p2, "Second");
        assert_eq!(app.doc.get(p2).unwrap().name, "Second");
        app.undo();
        assert_eq!(app.doc.get(p2).unwrap().name, "Page 2");
        app.undo();
        assert_eq!(app.doc.pages, vec![p1]);
        app.redo();
        assert_eq!(app.doc.pages, vec![p1, p2]);

        let copy = app.duplicate_page(p1).unwrap();
        assert_eq!(app.doc.pages, vec![p1, copy, p2]);
        assert_eq!(app.doc.get(copy).unwrap().children.len(), ids.len());
        assert!(app.doc.get(copy).unwrap().name.ends_with("copy"));
        assert!(app.doc.get(copy).unwrap().children.iter().all(|c| !ids.contains(c)), "fresh ids");

        assert!(app.delete_page(copy));
        assert_eq!(app.doc.pages, vec![p1, p2]);
        assert!(app.doc.nodes.contains_key(&app.page));
        app.undo();
        assert_eq!(app.doc.pages, vec![p1, copy, p2]);
        assert_eq!(app.doc.get(copy).unwrap().children.len(), ids.len());
    }

    #[test]
    fn last_page_cannot_be_deleted_and_pages_reorder() {
        let (mut app, _ids) = app_with_rects();
        let p1 = app.page;
        assert!(!app.delete_page(p1));
        assert_eq!(app.doc.pages.len(), 1);
        let p2 = app.add_page().unwrap();
        let p3 = app.add_page().unwrap();
        app.move_page(p3, 0);
        assert_eq!(app.doc.pages, vec![p3, p1, p2]);
        app.undo();
        assert_eq!(app.doc.pages, vec![p1, p2, p3]);
        assert!(app.delete_page(p1));
        assert!(!app.delete_page(p2) || app.doc.pages.len() == 1);
        assert!(!app.doc.pages.is_empty());
        assert!(app.doc.nodes.contains_key(&app.page));
    }

    #[test]
    fn multi_rotate_pivots_about_bbox_center() {
        let (mut app, ids) = app_with_rects();
        app.selection = vec![ids[0], ids[2]];
        let bb = app.selection_bounds().unwrap();
        let c0 = rect_center(&bb);
        app.rotate_selection(std::f32::consts::FRAC_PI_2);
        for id in [ids[0], ids[2]] {
            let n = app.doc.get(id).unwrap();
            assert!((n.transform.rotation_rad() - std::f32::consts::FRAC_PI_2).abs() < 1e-4);
        }
        // the union box (now rotated) is centred on the same point and swapped in shape
        let bb1 = app.selection_bounds().unwrap();
        let c1 = rect_center(&bb1);
        assert!((c0.x - c1.x).abs() < 1e-2 && (c0.y - c1.y).abs() < 1e-2, "{c0:?} vs {c1:?}");
        assert!(dist(c0, c1) < 1e-2);
        assert!((bb1.w - bb.h).abs() < 1e-2 && (bb1.h - bb.w).abs() < 1e-2);
        // untouched node did not move
        assert_eq!(app.doc.get(ids[1]).unwrap().transform.tx, 100.0);
        app.undo();
        assert_eq!(app.doc.get(ids[0]).unwrap().transform.rotation_rad(), 0.0);
        assert!(!app.hist.can_undo());
    }

    #[test]
    fn flip_mirrors_about_selection_center() {
        let (mut app, ids) = app_with_rects();
        app.selection = vec![ids[0], ids[2]];
        let bb = app.selection_bounds().unwrap();
        let before: Vec<Rect> = ids.iter().map(|i| world_aabb(&app.doc, *i)).collect();
        app.flip_selection(true);
        let after: Vec<Rect> = ids.iter().map(|i| world_aabb(&app.doc, *i)).collect();
        // x mirrored inside the union box, y untouched, size kept
        assert!((after[0].x - (bb.x + bb.w - (before[0].x - bb.x) - before[0].w)).abs() < 1e-3);
        assert!((after[2].x - (bb.x + bb.w - (before[2].x - bb.x) - before[2].w)).abs() < 1e-3);
        assert_eq!((after[0].y, after[0].w), (before[0].y, before[0].w));
        assert_eq!(after[1], before[1], "unselected node untouched");
        app.flip_selection(true);
        let again = world_aabb(&app.doc, ids[0]);
        assert!((again.x - before[0].x).abs() < 1e-3, "flipping twice restores");
        app.undo();
        app.undo();
        assert_eq!(world_aabb(&app.doc, ids[0]), before[0]);
        app.flip_selection(false);
        let v = world_aabb(&app.doc, ids[0]);
        assert!((v.y - (bb.y + bb.h - (before[0].y - bb.y) - before[0].h)).abs() < 1e-3);
    }

    #[test]
    fn distribute_evens_out_gaps() {
        let mut d = Document::new("t");
        let page = d.add_page("p");
        let mut ids = vec![];
        for x in [0.0, 60.0, 300.0] {
            let id = d.alloc_id();
            ids.push(d.insert(page, Node::rectangle(id, Rect::new(x, 0.0, 50.0, 50.0))).unwrap());
        }
        let mut app = App::new(d);
        app.selection = ids.clone();
        assert!(app.distribute_selection());
        let xs: Vec<f32> = ids.iter().map(|i| world_aabb(&app.doc, *i).x).collect();
        assert_eq!(xs, vec![0.0, 150.0, 300.0]);
        app.undo();
        assert_eq!(world_aabb(&app.doc, ids[1]).x, 60.0);
        app.selection = ids[..2].to_vec();
        assert!(!app.distribute_selection(), "needs three layers");
    }
}
