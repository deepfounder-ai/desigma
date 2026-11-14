use crate::{edit::Edit, layout};
use desigma_doc::*;
pub fn apply_resize(doc: &mut Document, ed: &mut Edit, id: NodeId, size: Vec2, transform: Affine, relayout: bool) {
    let Some((old, old_transform)) = doc.get(id).map(|n| (n.size, n.transform)) else { return };
    if let Some(n) = ed.node(doc, id) {
        n.size = size;
        n.transform = transform;
        scale_vector_geometry(&mut n.kind, old, size);
        if let NodeKind::Text(t) = &mut n.kind {
            if t.auto_resize == TextAutoResize::WidthAndHeight {
                t.auto_resize = TextAutoResize::None;
            }
        }
    }
    // a table scales its columns and rows with it
    if matches!(doc.get(id).map(|n| &n.kind), Some(NodeKind::Table(_))) {
        crate::table::scale_table(doc, ed, id, size);
        if relayout {
            layout::relayout_up(doc, ed, id);
        }
        return;
    }
    // a section's content stays where it is on the board when the section grows or shrinks
    if matches!(doc.get(id).map(|n| &n.kind), Some(NodeKind::BoardSection(_))) {
        keep_children_in_place(doc, ed, id, old_transform);
    }
    if doc.get(id).map(|n| n.kind.is_container()).unwrap_or(false) {
        layout::apply_constraints(doc, ed, id, old, size);
        layout::layout_node(doc, ed, id);
    }
    if relayout {
        layout::relayout_up(doc, ed, id);
    }
}

/// After the box of `id` moved from `old` (its transform before the edit) to its current transform,
/// re-expresses the children so they keep their place on the board.
fn keep_children_in_place(doc: &mut Document, ed: &mut Edit, id: NodeId, old: Affine) {
    let Some(n) = doc.get(id) else { return };
    let Some(inv) = n.transform.invert() else { return };
    let delta = inv * old;
    if delta.tx.abs() < 1e-4 && delta.ty.abs() < 1e-4 && (delta.a - 1.0).abs() < 1e-5 && (delta.d - 1.0).abs() < 1e-5 {
        return;
    }
    for c in n.children.clone() {
        if let Some(k) = ed.node(doc, c) {
            k.transform = delta * k.transform;
        }
    }
}

/// Scales vector paths and network geometry when a `NodeKind::Vector` changes size.
pub(crate) fn scale_vector_geometry(kind: &mut NodeKind, old: Vec2, new: Vec2) {
    let NodeKind::Vector { paths, network } = kind else { return };
    if old.x <= 1e-4 || old.y <= 1e-4 {
        return;
    }
    let sx = new.x / old.x;
    let sy = new.y / old.y;
    if (sx - 1.0).abs() < 1e-5 && (sy - 1.0).abs() < 1e-5 {
        return;
    }
    let scale_pt = |p: &mut Vec2| {
        p.x *= sx;
        p.y *= sy;
    };
    for path in paths.iter_mut() {
        for cmd in &mut path.cmds {
            match cmd {
                PathCmd::MoveTo(p) | PathCmd::LineTo(p) => scale_pt(p),
                PathCmd::QuadTo(c, p) => {
                    scale_pt(c);
                    scale_pt(p);
                }
                PathCmd::CubicTo(c1, c2, p) => {
                    scale_pt(c1);
                    scale_pt(c2);
                    scale_pt(p);
                }
                PathCmd::Close => {}
            }
        }
    }
    if let Some(net) = network.as_mut() {
        for v in &mut net.vertices {
            scale_pt(&mut v.pos);
        }
        for s in &mut net.segments {
            scale_pt(&mut s.ta);
            scale_pt(&mut s.tb);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::v2;

    #[test]
    fn resizing_svg_frame_and_group_scales_vector_paths() {
        let mut doc = Document::new("test");
        let page = doc.add_page("Page 1");
        let mut frame = Node::new(doc.alloc_id(), NodeKind::Frame(FrameProps { clips_content: true, ..Default::default() }));
        frame.size = v2(100.0, 100.0);
        let frame_id = doc.insert(page, frame).unwrap();

        let mut group = Node::new(doc.alloc_id(), NodeKind::Group);
        group.size = v2(100.0, 100.0);
        group.constraints = Constraints { horizontal: Constraint::Scale, vertical: Constraint::Scale };
        let group_id = doc.insert(frame_id, group).unwrap();

        let path = Path {
            cmds: vec![PathCmd::MoveTo(v2(10.0, 20.0)), PathCmd::LineTo(v2(50.0, 40.0)), PathCmd::Close],
            fill_rule: FillRule::NonZero,
        };
        let mut vec_node = Node::new(doc.alloc_id(), NodeKind::Vector { paths: vec![path], network: None });
        vec_node.transform = Affine::translate(10.0, 10.0);
        vec_node.size = v2(50.0, 40.0);
        vec_node.constraints = Constraints { horizontal: Constraint::Scale, vertical: Constraint::Scale };
        let vec_id = doc.insert(group_id, vec_node).unwrap();

        let mut ed = Edit::default();
        apply_resize(&mut doc, &mut ed, frame_id, v2(200.0, 300.0), Affine::IDENTITY, true);

        let g = doc.get(group_id).unwrap();
        assert_eq!(g.size, v2(100.0, 120.0));
        assert_eq!(g.transform.tx, 20.0);
        assert_eq!(g.transform.ty, 30.0);
        let v = doc.get(vec_id).unwrap();
        assert_eq!(v.transform.tx, 0.0);
        assert_eq!(v.transform.ty, 0.0);
        assert_eq!(v.size, v2(100.0, 120.0));
        let NodeKind::Vector { paths, .. } = &v.kind else { panic!("expected vector") };
        assert_eq!(paths[0].cmds[0], PathCmd::MoveTo(v2(20.0, 60.0)));
        assert_eq!(paths[0].cmds[1], PathCmd::LineTo(v2(100.0, 120.0)));
    }
}
