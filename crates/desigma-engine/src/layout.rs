//! Auto-layout, constraints and group fitting.
//!
//! `compute` is a pure function (easy to test); the `layout_*`/`relayout_*`
//! functions apply it to a [`Document`] through an [`Edit`] session so every
//! touched node is recorded for undo.

use crate::edit::Edit;
use crate::util::{aabb_of, rect_corners, v2};
use desigma_doc::{
    resolve_node, AutoLayout, Constraint, Document, LayoutAlign, LayoutDirection, NodeId, NodeKind, Rect, Sizing, TextAutoResize, TextProps, Vec2,
};

/// One participating child, as seen by the solver.
#[derive(Clone, Copy, Debug)]
pub struct ChildIn {
    pub size: Vec2,
    /// Stretches along the primary axis (Fill or grow > 0).
    pub main_fill: bool,
    /// Stretches along the counter axis.
    pub cross_fill: bool,
    pub grow: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LayoutOut {
    pub frame: Vec2,
    /// Child rectangles in frame-local coordinates, same order as the input.
    pub rects: Vec<Rect>,
}

struct Axes {
    horiz: bool,
}

impl Axes {
    fn main(&self, v: Vec2) -> f32 {
        if self.horiz {
            v.x
        } else {
            v.y
        }
    }
    fn cross(&self, v: Vec2) -> f32 {
        if self.horiz {
            v.y
        } else {
            v.x
        }
    }
    fn vec(&self, main: f32, cross: f32) -> Vec2 {
        if self.horiz {
            v2(main, cross)
        } else {
            v2(cross, main)
        }
    }
}

/// Solves an auto-layout frame. `frame` is the current frame size (used for
/// fixed axes). Padding order is top, right, bottom, left.
pub fn compute(al: &AutoLayout, frame: Vec2, kids: &[ChildIn]) -> LayoutOut {
    let ax = Axes { horiz: al.direction == LayoutDirection::Horizontal };
    let [pt, pr, pb, pl] = al.padding;
    let (pm_start, pm_end, pc_start, pc_end) = if ax.horiz { (pl, pr, pt, pb) } else { (pt, pb, pl, pr) };
    let pm = pm_start + pm_end;
    let pc = pc_start + pc_end;
    let hug_main = al.primary_sizing == Sizing::Hug && !al.wrap;
    let hug_cross = al.counter_sizing == Sizing::Hug;
    let mut frame_main = ax.main(frame);
    let mut frame_cross = ax.cross(frame);

    if kids.is_empty() {
        if hug_main {
            frame_main = pm;
        }
        if hug_cross {
            frame_cross = pc;
        }
        return LayoutOut { frame: ax.vec(frame_main, frame_cross), rects: vec![] };
    }

    // Split into lines.
    let mut lines: Vec<Vec<usize>> = Vec::new();
    if al.wrap {
        let avail = (frame_main - pm).max(0.0);
        let mut cur: Vec<usize> = Vec::new();
        let mut used = 0.0;
        for (i, k) in kids.iter().enumerate() {
            let m = ax.main(k.size);
            let add = if cur.is_empty() { m } else { al.item_spacing + m };
            if !cur.is_empty() && used + add > avail + 1e-3 {
                lines.push(std::mem::take(&mut cur));
                used = 0.0;
                cur.push(i);
                used += m;
            } else {
                cur.push(i);
                used += add;
            }
        }
        lines.push(cur);
    } else {
        lines.push((0..kids.len()).collect());
    }

    // Hug along the primary axis (single line only).
    if hug_main {
        let total: f32 = kids.iter().map(|k| ax.main(k.size)).sum::<f32>() + al.item_spacing * (kids.len() as f32 - 1.0);
        frame_main = total + pm;
    }
    let main_avail = (frame_main - pm).max(0.0);

    // Cross extent of each line.
    let mut line_cross: Vec<f32> =
        lines.iter().map(|l| l.iter().filter(|i| !kids[**i].cross_fill || al.wrap).map(|i| ax.cross(kids[*i].size)).fold(0.0, f32::max)).collect();
    if !al.wrap {
        if hug_cross {
            frame_cross = line_cross[0] + pc;
        } else {
            line_cross[0] = (frame_cross - pc).max(0.0);
        }
    } else {
        let total: f32 = line_cross.iter().sum::<f32>() + al.counter_spacing * (lines.len() as f32 - 1.0);
        if hug_cross {
            frame_cross = total + pc;
        }
    }

    let mut rects = vec![Rect::default(); kids.len()];
    let mut cross_cursor = pc_start;
    for (li, line) in lines.iter().enumerate() {
        let k = line.len() as f32;
        // Main sizes (distribute fill children).
        let fixed: f32 = line.iter().filter(|i| !kids[**i].main_fill || al.wrap).map(|i| ax.main(kids[*i].size)).sum();
        let weight: f32 = line.iter().filter(|i| kids[**i].main_fill && !al.wrap).map(|i| fill_weight(&kids[*i])).sum();
        let free_for_fill = (main_avail - al.item_spacing * (k - 1.0) - fixed).max(0.0);
        let mains: Vec<f32> = line
            .iter()
            .map(|i| {
                let kid = &kids[*i];
                if kid.main_fill && !al.wrap && !hug_main && weight > 0.0 {
                    free_for_fill * fill_weight(kid) / weight
                } else {
                    ax.main(kid.size)
                }
            })
            .collect();
        let sum_main: f32 = mains.iter().sum();
        let (start, gap) = match al.primary_align {
            LayoutAlign::SpaceBetween if k > 1.0 => (0.0, ((main_avail - sum_main) / (k - 1.0)).max(al.item_spacing.min(0.0))),
            LayoutAlign::Center => (((main_avail - sum_main - al.item_spacing * (k - 1.0)) / 2.0).max(0.0), al.item_spacing),
            LayoutAlign::End => ((main_avail - sum_main - al.item_spacing * (k - 1.0)).max(0.0), al.item_spacing),
            _ => (0.0, al.item_spacing),
        };
        let mut cursor = pm_start + start;
        for (j, i) in line.iter().enumerate() {
            let kid = &kids[*i];
            let m = mains[j];
            let c = if kid.cross_fill { line_cross[li] } else { ax.cross(kid.size) };
            let off = match al.counter_align {
                LayoutAlign::Center => (line_cross[li] - c) / 2.0,
                LayoutAlign::End => line_cross[li] - c,
                _ => 0.0,
            };
            let pos = ax.vec(cursor, cross_cursor + off);
            let size = ax.vec(m, c);
            rects[*i] = Rect::new(pos.x, pos.y, size.x, size.y);
            cursor += m + gap;
        }
        cross_cursor += line_cross[li] + al.counter_spacing;
    }
    LayoutOut { frame: ax.vec(frame_main, frame_cross), rects }
}

fn fill_weight(k: &ChildIn) -> f32 {
    if k.grow > 0.0 {
        k.grow
    } else {
        1.0
    }
}

/// Measured text extent when the text hugs its content (no wrapping).
pub fn estimate_text_size(t: &TextProps) -> Vec2 {
    crate::textmeasure::measure(t, None)
}

/// New node size after a text edit, following the node's auto-resize mode.
pub fn fit_text(t: &TextProps, size: Vec2) -> Vec2 {
    match t.auto_resize {
        TextAutoResize::WidthAndHeight => estimate_text_size(t),
        TextAutoResize::Height => v2(size.x, crate::textmeasure::measure(t, Some(size.x.max(1.0))).y),
        _ => size,
    }
}

fn child_in(doc: &Document, id: NodeId, horiz: bool) -> ChildIn {
    // sizes bound to variables count with the value of the mode in effect
    let n = &*resolve_node(doc, doc.get(id).unwrap());
    let lc = n.layout_child;
    let (main_s, cross_s) = if horiz { (lc.horizontal, lc.vertical) } else { (lc.vertical, lc.horizontal) };
    ChildIn { size: n.size, main_fill: main_s == Sizing::Fill || lc.grow > 0.0, cross_fill: cross_s == Sizing::Fill, grow: lc.grow }
}

/// Lays out `id` and every container below it (children first so hugging
/// sizes bubble up), writing through `ed`.
pub fn layout_node(doc: &mut Document, ed: &mut Edit, id: NodeId) {
    let Some(n) = doc.get(id) else { return };
    let children = n.children.clone();
    for c in &children {
        if doc.get(*c).map(|k| k.kind.is_container()).unwrap_or(false) {
            layout_node(doc, ed, *c);
        }
    }
    if matches!(doc.get(id).map(|n| &n.kind), Some(NodeKind::Group)) {
        fit_group(doc, ed, id);
        return;
    }
    if matches!(doc.get(id).map(|n| &n.kind), Some(NodeKind::Table(_))) {
        crate::table::sync_table(doc, ed, id);
        return;
    }
    // gap / padding bound to variables count with the value of the mode in effect
    let Some(al) = doc.get(id).map(|n| resolve_node(doc, n)).and_then(|n| n.kind.frame_props().and_then(|f| f.auto_layout)) else { return };
    let horiz = al.direction == LayoutDirection::Horizontal;
    let ids: Vec<NodeId> =
        children.iter().copied().filter(|c| doc.get(*c).map(|k| resolve_node(doc, k).visible && !k.layout_child.absolute).unwrap_or(false)).collect();
    let kids: Vec<ChildIn> = ids.iter().map(|c| child_in(doc, *c, horiz)).collect();
    let frame_size = doc.get(id).unwrap().size;
    let out = compute(&al, frame_size, &kids);
    if out.frame != frame_size {
        if let Some(n) = ed.node(doc, id) {
            n.size = out.frame;
        }
    }
    for (cid, r) in ids.iter().zip(out.rects.iter()) {
        let n = doc.get(*cid).unwrap();
        let new_size = v2(r.w, r.h);
        let size_changed = (n.size.x - new_size.x).abs() > 1e-3 || (n.size.y - new_size.y).abs() > 1e-3;
        let pos_changed = (n.transform.tx - r.x).abs() > 1e-3 || (n.transform.ty - r.y).abs() > 1e-3;
        if size_changed || pos_changed {
            if let Some(n) = ed.node(doc, *cid) {
                n.transform.tx = r.x;
                n.transform.ty = r.y;
                n.size = new_size;
            }
        }
        if size_changed && doc.get(*cid).map(|n| n.kind.is_container()).unwrap_or(false) {
            layout_node(doc, ed, *cid);
        }
    }
}

/// Re-fits a group's box to its children (group transform follows the content).
pub fn fit_group(doc: &mut Document, ed: &mut Edit, id: NodeId) {
    let Some(g) = doc.get(id) else { return };
    let mut pts = Vec::new();
    for c in &g.children {
        if let Some(k) = doc.get(*c) {
            let r = Rect::new(0.0, 0.0, k.size.x, k.size.y);
            pts.extend(rect_corners(&r).map(|p| k.transform.apply(p)));
        }
    }
    if pts.is_empty() {
        return;
    }
    let bb = aabb_of(&pts);
    let size = v2(bb.w, bb.h);
    if bb.x.abs() < 1e-3 && bb.y.abs() < 1e-3 && (g.size.x - size.x).abs() < 1e-3 && (g.size.y - size.y).abs() < 1e-3 {
        return;
    }
    let children = g.children.clone();
    let shift = desigma_doc::Affine::translate(bb.x, bb.y);
    if let Some(g) = ed.node(doc, id) {
        g.transform = g.transform * shift;
        g.size = size;
    }
    for c in children {
        if let Some(k) = ed.node(doc, c) {
            k.transform.tx -= bb.x;
            k.transform.ty -= bb.y;
        }
    }
}

/// After any edit on `start`, re-run layout/fit on its ancestors.
pub fn relayout_up(doc: &mut Document, ed: &mut Edit, start: NodeId) {
    // on a board, connectors follow the moved node live (every move, resize and rotate comes through here)
    crate::connect::reroute_touched(doc, ed);
    let mut cur = doc.get(start).and_then(|n| n.parent);
    while let Some(p) = cur {
        if matches!(doc.get(p).map(|n| &n.kind), Some(NodeKind::Table(_))) {
            crate::table::sync_table(doc, ed, p);
        }
        let is_group = matches!(doc.get(p).map(|n| &n.kind), Some(NodeKind::Group));
        let is_al = doc.get(p).and_then(|n| n.kind.frame_props()).map(|f| f.auto_layout.is_some()).unwrap_or(false);
        if is_group {
            fit_group(doc, ed, p);
        } else if is_al {
            layout_node(doc, ed, p);
        }
        cur = doc.get(p).and_then(|n| n.parent);
    }
}

/// Convenience: lay out `id`'s subtree then fix its ancestors.
pub fn relayout_around(doc: &mut Document, ed: &mut Edit, id: NodeId) {
    layout_node(doc, ed, id);
    relayout_up(doc, ed, id);
}

/// Applies children constraints when a (non auto-layout) frame is resized.
pub fn apply_constraints(doc: &mut Document, ed: &mut Edit, frame: NodeId, old: Vec2, new: Vec2) {
    let Some(f) = doc.get(frame) else { return };
    let is_group = matches!(f.kind, NodeKind::Group);
    if f.kind.frame_props().map(|p| p.auto_layout.is_some()).unwrap_or(false)
        || matches!(f.kind, NodeKind::BoardSection(_) | NodeKind::Table(_))
    {
        return;
    }
    let dw = new.x - old.x;
    let dh = new.y - old.y;
    if dw.abs() < 1e-4 && dh.abs() < 1e-4 {
        return;
    }
    for c in f.children.clone() {
        let Some(k) = doc.get(c) else { continue };
        let (cons_h, cons_v) = if is_group {
            (Constraint::Scale, Constraint::Scale)
        } else {
            (k.constraints.horizontal, k.constraints.vertical)
        };
        let rotated = k.transform.b.abs() > 1e-4 || k.transform.c.abs() > 1e-4;
        if rotated && (cons_h != Constraint::Scale || cons_v != Constraint::Scale) {
            continue;
        }
        let (mut x, mut w) = (k.transform.tx, k.size.x);
        let (mut y, mut h) = (k.transform.ty, k.size.y);
        let old_child = k.size;
        let rx = if old.x > 0.0 { new.x / old.x } else { 1.0 };
        let ry = if old.y > 0.0 { new.y / old.y } else { 1.0 };
        match cons_h {
            Constraint::Min => {}
            Constraint::Max => x += dw,
            Constraint::Center => x += dw / 2.0,
            Constraint::Stretch => w += dw,
            Constraint::Scale => {
                x *= rx;
                w *= rx;
            }
        }
        match cons_v {
            Constraint::Min => {}
            Constraint::Max => y += dh,
            Constraint::Center => y += dh / 2.0,
            Constraint::Stretch => h += dh,
            Constraint::Scale => {
                y *= ry;
                h *= ry;
            }
        }
        let w = w.max(1.0);
        let h = h.max(1.0);
        if let Some(k) = ed.node(doc, c) {
            k.transform.tx = x;
            k.transform.ty = y;
            k.size = v2(w, h);
            crate::resize::scale_vector_geometry(&mut k.kind, old_child, v2(w, h));
        }
        if (w - old_child.x).abs() > 1e-4 || (h - old_child.y).abs() > 1e-4 {
            apply_constraints(doc, ed, c, old_child, v2(w, h));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use desigma_doc::{FrameProps, Node};

    fn kid(w: f32, h: f32) -> ChildIn {
        ChildIn { size: v2(w, h), main_fill: false, cross_fill: false, grow: 0.0 }
    }
    fn al() -> AutoLayout {
        AutoLayout { item_spacing: 10.0, padding: [5.0, 5.0, 5.0, 5.0], ..Default::default() }
    }

    #[test]
    fn horizontal_start_fixed() {
        let out = compute(&al(), v2(300.0, 100.0), &[kid(50.0, 20.0), kid(30.0, 40.0)]);
        assert_eq!(out.rects[0], Rect::new(5.0, 5.0, 50.0, 20.0));
        assert_eq!(out.rects[1], Rect::new(65.0, 5.0, 30.0, 40.0));
        assert_eq!(out.frame, v2(300.0, 100.0));
    }

    #[test]
    fn hug_both_axes() {
        let mut a = al();
        a.primary_sizing = Sizing::Hug;
        a.counter_sizing = Sizing::Hug;
        let out = compute(&a, v2(0.0, 0.0), &[kid(50.0, 20.0), kid(30.0, 40.0)]);
        assert_eq!(out.frame, v2(50.0 + 30.0 + 10.0 + 10.0, 40.0 + 10.0));
    }

    #[test]
    fn vertical_center_alignment() {
        let mut a = al();
        a.direction = LayoutDirection::Vertical;
        a.counter_align = LayoutAlign::Center;
        a.primary_align = LayoutAlign::Center;
        let out = compute(&a, v2(100.0, 100.0), &[kid(40.0, 20.0)]);
        assert_eq!(out.rects[0].x, 30.0); // (100-10 - 40)/2 + 5
        assert_eq!(out.rects[0].y, 40.0); // (90 - 20)/2 + 5
    }

    #[test]
    fn fill_children_share_space() {
        let mut k1 = kid(10.0, 10.0);
        k1.main_fill = true;
        let mut k2 = kid(10.0, 10.0);
        k2.main_fill = true;
        let k3 = kid(40.0, 10.0);
        let out = compute(&al(), v2(200.0, 50.0), &[k1, k2, k3]);
        // avail = 190 - 20 (spacing) - 40 = 130 -> 65 each
        assert_eq!(out.rects[0].w, 65.0);
        assert_eq!(out.rects[1].w, 65.0);
        assert_eq!(out.rects[2].x, 5.0 + 65.0 + 10.0 + 65.0 + 10.0);
    }

    #[test]
    fn cross_fill_stretches() {
        let mut k = kid(10.0, 10.0);
        k.cross_fill = true;
        let out = compute(&al(), v2(100.0, 80.0), &[k]);
        assert_eq!(out.rects[0].h, 70.0);
    }

    #[test]
    fn space_between() {
        let mut a = al();
        a.primary_align = LayoutAlign::SpaceBetween;
        a.padding = [0.0; 4];
        let out = compute(&a, v2(100.0, 20.0), &[kid(10.0, 10.0), kid(10.0, 10.0), kid(10.0, 10.0)]);
        assert_eq!(out.rects[0].x, 0.0);
        assert_eq!(out.rects[1].x, 45.0);
        assert_eq!(out.rects[2].x, 90.0);
    }

    #[test]
    fn wrap_breaks_lines() {
        let mut a = al();
        a.wrap = true;
        a.counter_spacing = 4.0;
        a.padding = [0.0; 4];
        let out = compute(&a, v2(75.0, 100.0), &[kid(30.0, 10.0), kid(30.0, 20.0), kid(30.0, 10.0)]);
        assert_eq!(out.rects[1].x, 40.0);
        assert_eq!(out.rects[2].x, 0.0);
        assert_eq!(out.rects[2].y, 24.0); // line 1 cross = 20, + counter spacing 4
    }

    #[test]
    fn empty_hug_collapses_to_padding() {
        let mut a = al();
        a.primary_sizing = Sizing::Hug;
        a.counter_sizing = Sizing::Hug;
        assert_eq!(compute(&a, v2(99.0, 99.0), &[]).frame, v2(10.0, 10.0));
    }

    fn doc_with_frame() -> (Document, NodeId, NodeId, NodeId) {
        let mut d = Document::new("t");
        let page = d.add_page("p");
        let fid = d.alloc_id();
        let mut f = Node::frame(fid, Rect::new(0.0, 0.0, 10.0, 10.0));
        f.kind = NodeKind::Frame(FrameProps {
            auto_layout: Some(AutoLayout { primary_sizing: Sizing::Hug, counter_sizing: Sizing::Hug, item_spacing: 8.0, ..Default::default() }),
            ..Default::default()
        });
        let f = d.insert(page, f).unwrap();
        let a = d.alloc_id();
        let a = d.insert(f, Node::rectangle(a, Rect::new(100.0, 100.0, 20.0, 20.0))).unwrap();
        let b = d.alloc_id();
        let b = d.insert(f, Node::rectangle(b, Rect::new(0.0, 0.0, 30.0, 10.0))).unwrap();
        (d, f, a, b)
    }

    #[test]
    fn applies_to_document_and_hugs() {
        let (mut d, f, a, b) = doc_with_frame();
        let mut ed = Edit::new();
        layout_node(&mut d, &mut ed, f);
        assert_eq!(d.get(f).unwrap().size, v2(58.0, 20.0));
        assert_eq!(d.get(a).unwrap().transform.tx, 0.0);
        assert_eq!(d.get(b).unwrap().transform.tx, 28.0);
        // resizing a child re-hugs the parent through relayout_up
        ed.node(&mut d, a).unwrap().size = v2(40.0, 20.0);
        relayout_up(&mut d, &mut ed, a);
        assert_eq!(d.get(f).unwrap().size.x, 78.0);
    }

    #[test]
    fn gap_bound_to_a_variable_follows_the_mode() {
        use desigma_doc::{VariableKind, VariableStore, VariableValue};
        let (mut d, f, _, b) = doc_with_frame();
        let mut s = VariableStore::default();
        let c = s.add_collection("Space");
        let light = s.collections[&c].default_mode.clone();
        let dark = s.add_mode(&c, "Wide").unwrap();
        let gap = s.add_variable(&c, "Gap", VariableKind::Number).unwrap();
        s.set_value(&gap, &light, VariableValue::Number(8.0));
        s.set_value(&gap, &dark, VariableValue::Number(20.0));
        d.variables = s.variables;
        d.variable_collections = s.collections;
        d.get_mut(f).unwrap().variables.bindings.insert("itemSpacing".into(), gap);
        let mut ed = Edit::new();
        layout_node(&mut d, &mut ed, f);
        assert_eq!(d.get(b).unwrap().transform.tx, 28.0);
        d.get_mut(f).unwrap().variables.modes.insert(c, dark);
        layout_node(&mut d, &mut ed, f);
        assert_eq!(d.get(b).unwrap().transform.tx, 40.0);
    }

    #[test]
    fn group_fit_follows_children() {
        let mut d = Document::new("t");
        let page = d.add_page("p");
        let gid = d.alloc_id();
        let mut g = Node::new(gid, NodeKind::Group);
        g.size = v2(1.0, 1.0);
        let g = d.insert(page, g).unwrap();
        let rid = d.alloc_id();
        let r = d.insert(g, Node::rectangle(rid, Rect::new(10.0, 20.0, 30.0, 40.0))).unwrap();
        let mut ed = Edit::new();
        fit_group(&mut d, &mut ed, g);
        assert_eq!(d.get(g).unwrap().size, v2(30.0, 40.0));
        assert_eq!(d.get(g).unwrap().transform.tx, 10.0);
        assert_eq!(d.get(r).unwrap().transform.tx, 0.0);
    }

    #[test]
    fn constraints_on_resize() {
        let mut d = Document::new("t");
        let page = d.add_page("p");
        let fid = d.alloc_id();
        let f = d.insert(page, Node::frame(fid, Rect::new(0.0, 0.0, 100.0, 100.0))).unwrap();
        let rid = d.alloc_id();
        let r = d.insert(f, Node::rectangle(rid, Rect::new(80.0, 10.0, 10.0, 10.0))).unwrap();
        {
            let n = d.get_mut(r).unwrap();
            n.constraints.horizontal = Constraint::Max;
            n.constraints.vertical = Constraint::Stretch;
        }
        let mut ed = Edit::new();
        apply_constraints(&mut d, &mut ed, f, v2(100.0, 100.0), v2(150.0, 120.0));
        let n = d.get(r).unwrap();
        assert_eq!(n.transform.tx, 130.0);
        assert_eq!(n.size.y, 30.0);
    }

    #[test]
    fn text_estimate_scales_with_content() {
        let t = |s: &str| TextProps {
            content: s.into(),
            style: Default::default(),
            runs: vec![],
            auto_resize: Default::default(),
            max_lines: None,
            ..Default::default()
        };
        assert!(estimate_text_size(&t("hello world")).x > estimate_text_size(&t("hi")).x);
        assert!(estimate_text_size(&t("a\nb")).y > estimate_text_size(&t("a")).y);
    }
}
