//! Pointer/keyboard interaction on the canvas: tool state machine.

use super::builder;
use super::create::{self, PenPoint};
use super::handles::{self, Handle, HandleHit};
use super::hit;
use super::reorder;
use super::snap;
use super::textsession as ts;
pub use super::textsession::{begin_text_edit, finish_text};
use super::tools::{press_action, PressAction, PressTarget, Tool};
use super::vector;
use super::*;
use crate::edit::Tx;
use crate::layout;
use crate::ops::apply_resize;
use crate::theme;
use crate::util::*;
use desigma_doc::{Command, Document, NodeKind};
use egui::{Modifiers, Pos2, Response};

pub fn dv(p: Pos2) -> DV {
    DV::new(p.x, p.y)
}

struct Inp {
    pos: Option<Pos2>,
    pressed: bool,
    down: bool,
    released: bool,
    mid_pressed: bool,
    mid_down: bool,
    mods: Modifiers,
    space: bool,
}

fn gather(ui: &Ui, typing: bool) -> Inp {
    ui.input(|i| Inp {
        pos: i.pointer.latest_pos(),
        pressed: i.pointer.primary_pressed(),
        down: i.pointer.primary_down(),
        released: i.pointer.primary_released(),
        mid_pressed: i.pointer.button_pressed(egui::PointerButton::Middle),
        mid_down: i.pointer.middle_down(),
        mods: i.modifiers,
        space: !typing && i.key_down(egui::Key::Space),
    })
}

/// Corners (page space) of the box the selection handles wrap, plus whether
/// it is a single straight line (only end handles).
pub fn selection_box(app: &App) -> Option<([DV; 4], bool)> {
    match app.selection.as_slice() {
        [] => None,
        [one] => {
            let n = app.doc.get(*one)?;
            Some((world_corners(app.shown_doc(), *one), matches!(n.kind, NodeKind::Line)))
        }
        many => {
            let bb = union_all(many.iter().map(|i| world_aabb(app.shown_doc(), *i)))?;
            Some((rect_corners(&bb), false))
        }
    }
}

/// Containers that may be descended into on click (ancestors of the selection).
fn entered(app: &App) -> Vec<NodeId> {
    let mut v = Vec::new();
    for id in &app.selection {
        let mut cur = app.doc.get(*id).and_then(|n| n.parent);
        while let Some(p) = cur {
            v.push(p);
            cur = app.doc.get(p).and_then(|n| n.parent);
        }
    }
    v
}

pub fn hit_under(app: &App, page: DV, deep: bool) -> Option<NodeId> {
    let tol = 2.0 / app.view.zoom;
    hit::hit_test(&app.doc, app.page, page, tol, deep, &entered(app))
}

/// Rects of snap candidates (everything visible near the viewport except `exclude` subtrees).
pub fn snap_targets(app: &App, exclude: &[NodeId]) -> Vec<Rect> {
    let z = app.view.zoom;
    let cr = app.canvas_rect;
    let view = Rect::new(app.view.offset.x, app.view.offset.y, cr.width() / z, cr.height() / z);
    let view = Rect::new(view.x - view.w * 0.5, view.y - view.h * 0.5, view.w * 2.0, view.h * 2.0);
    let mut out = Vec::new();
    fn rec(doc: &Document, id: NodeId, exclude: &[NodeId], view: &Rect, out: &mut Vec<Rect>) {
        let Some(n) = doc.get(id) else { return };
        if matches!(n.kind, NodeKind::Table(_)) {
            return;
        }
        for c in &n.children {
            if exclude.contains(c) || out.len() > 4000 {
                continue;
            }
            let Some(k) = doc.get(*c) else { continue };
            if !k.visible {
                continue;
            }
            let bb = world_aabb(doc, *c);
            if bb.intersects(view) {
                out.push(bb);
                rec(doc, *c, exclude, view, out);
            }
        }
    }
    rec(&app.doc, app.page, exclude, &view, &mut out);
    out.extend(rulers::guide_rects(app));
    out
}

/// Rounds `v` to a whole pixel unless snapping to the pixel grid is off.
fn px(app: &App, v: f32) -> f32 {
    if app.ui.no_pixel_snap {
        v
    } else {
        v.round()
    }
}

fn snapped_on(guides: &[snap::Guide], axis: snap::GuideAxis) -> bool {
    guides.iter().any(|g| g.axis == axis)
}

fn snap_point(p: DV, targets: &[Rect], zoom: f32) -> (DV, snap::SnapResult) {
    let thr = theme::SNAP_THRESHOLD_PX / zoom;
    let r = snap::snap_edges(&Rect::new(p.x, p.y, 0.0, 0.0), &[p.x], &[p.y], targets, thr);
    (v2(p.x + r.dx, p.y + r.dy), r)
}

// ------------------------------------------------------------------ main entry
pub fn process(app: &mut App, ui: &mut Ui, rect: ERect, resp: &Response) {
    let ctx = ui.ctx().clone();
    let typing = ctx.egui_wants_keyboard_input() && app.canvas.text.is_none();
    let inp = gather(ui, typing);
    let hovered = resp.hovered();

    // Wheel pan / pinch & ctrl+wheel zoom.
    if hovered && (app.canvas.text.is_none() || matches!(app.canvas.inter, Interaction::None)) {
        let (scroll, zoom_delta, mods) = ui.input(|i| (i.smooth_scroll_delta, i.zoom_delta(), i.modifiers));
        if (zoom_delta - 1.0).abs() > 1e-4 {
            if let Some(p) = inp.pos {
                let z = app.view.zoom * zoom_delta;
                app.view.zoom_at(rect.min, p, z);
            }
        } else if scroll != egui::Vec2::ZERO {
            let mut s = scroll;
            if mods.shift && s.x == 0.0 {
                s = egui::vec2(s.y, 0.0);
            }
            app.view.offset = v2(app.view.offset.x - s.x / app.view.zoom, app.view.offset.y - s.y / app.view.zoom);
        }
    }

    if app.canvas.text.is_some() {
        ts::text_keys(app, &ctx);
        // drag-select with the primary button held
        if app.canvas.text.as_ref().is_some_and(|t| t.dragging) {
            match inp.pos {
                Some(p) if inp.down => ts::drag_to(app, app.view.to_page(rect.min, p)),
                _ => {
                    if let Some(t) = app.canvas.text.as_mut() {
                        t.dragging = false;
                    }
                }
            }
        }
    }

    if let Some(v) = &app.canvas.vedit {
        if app.selection != [v.id] || !matches!(app.tool, Tool::Move) {
            vector::exit(app);
        } else {
            vector::reload(app);
        }
    }

    app.canvas.pointer_page = inp.pos.filter(|p| rect.contains(*p)).map(|p| app.view.to_page(rect.min, p));
    if app.canvas.vedit.is_some() {
        vector::hover(app, rect, inp.pos);
    }
    if app.tool == Tool::ShapeBuilder {
        builder::hover(app, app.canvas.pointer_page);
    }

    if (inp.pressed || inp.mid_pressed) && hovered {
        on_press(app, rect, &inp);
    }
    if !matches!(app.canvas.inter, Interaction::None) {
        if inp.down || inp.mid_down {
            on_drag(app, ui, rect, &inp);
        }
        let ended = match app.canvas.inter {
            Interaction::Pan => !inp.down && !inp.mid_down,
            Interaction::PenDrag => !inp.down,
            _ => inp.released || !inp.down && !inp.pressed,
        };
        if ended {
            on_release(app, rect, &inp);
        }
    }
    if resp.triple_clicked() && hovered {
        if let (Some(id), Some(pos)) = (app.canvas.text.as_ref().map(|t| t.id), inp.pos) {
            let page = app.view.to_page(rect.min, pos);
            if ts::node_contains(app, id, page) {
                ts::select_paragraph(app, page);
            }
        }
    } else if resp.double_clicked() && hovered {
        on_double_click(app, rect, &inp);
    }
    update_hover_and_cursor(app, rect, &inp, hovered);
}

// ------------------------------------------------------------------ press
fn handle_hit(app: &App, rect: ERect, pos: Pos2) -> Option<HandleHit> {
    if !matches!(app.tool, Tool::Move | Tool::Scale) || app.canvas.vedit.is_some() {
        return None;
    }
    // a connector has no box to resize: it is moved by its ends
    if !app.selection.is_empty() && app.selection.iter().all(|id| matches!(app.doc.get(*id).map(|n| &n.kind), Some(NodeKind::Connector(_)))) {
        return None;
    }
    let (corners, is_line) = selection_box(app)?;
    let s = corners.map(|c| dv(app.view.to_screen(rect.min, c)));
    let w = dist(s[0], s[1]);
    let h = dist(s[0], s[3]);
    let allow_mid = w.min(h) > theme::CANVAS_HANDLE_SIZE * 3.0;
    let hit = handles::hit_handle(&s, dv(pos), theme::CANVAS_HANDLE_HIT, theme::CANVAS_ROTATE_ZONE, allow_mid || is_line)?;
    match hit {
        HandleHit::Resize(h) if is_line && !matches!(h, Handle::L | Handle::R) => None,
        HandleHit::Rotate(_) if is_line => None,
        other => Some(other),
    }
}

fn on_press(app: &mut App, rect: ERect, inp: &Inp) {
    let Some(pos) = inp.pos else { return };
    if inp.pressed && !inp.space && rulers::press(app, rect, pos) {
        return;
    }
    let page = app.view.to_page(rect.min, pos);
    // Text editing: click inside the edited node moves the caret, elsewhere ends editing.
    if let Some(sess) = &app.canvas.text {
        let id = sess.id;
        if ts::node_contains(app, id, page) {
            ts::press_at(app, page, inp.mods.shift);
            return;
        }
        // clicking another cell of the table being typed in moves the typing there
        let in_table = desigma_doc::table_of_cell(&app.doc, id).map(|(t, ..)| t);
        let other_cell = in_table.and_then(|t| {
            let on_divider = crate::board::table::divider_at(app, rect.min, t, pos).is_some();
            (app.tool == Tool::Move && !inp.space && !on_divider).then(|| crate::board::table::cell_at(app, t, page)).flatten()
        });
        finish_text(app, true);
        if let Some((cell, ..)) = other_cell {
            begin_text_edit(app, cell);
            return;
        }
    }
    let mid = inp.mid_pressed && !inp.pressed;
    if app.canvas.vedit.is_some() && !mid && !inp.space && vector::press(app, rect, pos, inp.mods) {
        return;
    }
    // board: the + buttons and dividers of a table
    if app.is_board() && !mid && !inp.space && crate::board::table::press(app, rect.min, pos, page) {
        return;
    }
    // board: a magnet dot starts a connector
    if app.is_board() && !mid && !inp.space && matches!(app.tool, Tool::Move | Tool::Connector) {
        if let Some((node, magnet)) = crate::board::connect::dot_hit(app, rect, pos) {
            crate::board::connect::start(app, Some((node, magnet)), page);
            return;
        }
    }
    // board: while a vote is open a click on an item adds a vote (Alt takes one back)
    if app.is_board() && !mid && !inp.space && app.tool == Tool::Move && app.doc.board_session.voting() {
        if let Some(id) = hit_under(app, page, true) {
            if crate::board::session::votable(&app.doc, id) {
                crate::board::session::vote(app, id, !inp.mods.alt);
                return;
            }
        }
    }
    let deep = inp.mods.command;
    let target = if let Some(h) = handle_hit(app, rect, pos) {
        PressTarget::Handle(h)
    } else if let Some(id) = label_hit(app, rect, pos).or_else(|| hit_under(app, page, deep)) {
        if app.is_selected(id) {
            PressTarget::SelectedNode(id)
        } else {
            PressTarget::Node(id)
        }
    } else {
        PressTarget::Empty
    };
    let action = press_action(app.tool, target, inp.space, mid);
    match action {
        PressAction::Pan => app.canvas.inter = Interaction::Pan,
        PressAction::Resize(h) => start_resize(app, h, page),
        PressAction::Rotate(c) => start_rotate(app, c, page),
        PressAction::MoveNode { select_first } => {
            let id = match target {
                PressTarget::Node(i) | PressTarget::SelectedNode(i) => i,
                _ => return,
            };
            if select_first {
                if inp.mods.shift {
                    app.selection.push(id);
                } else {
                    app.select_only(id);
                }
            }
            app.canvas.inter =
                Interaction::Pending(Pending { node: id, start_screen: pos, start_page: page, shift: inp.mods.shift, was_selected: !select_first });
        }
        PressAction::Marquee => {
            let base = if inp.mods.shift { app.selection.clone() } else { vec![] };
            if !inp.mods.shift {
                app.clear_selection();
            }
            app.canvas.inter = Interaction::Marquee { start: page, cur: page, base };
        }
        PressAction::Draw(tool) => start_draw(app, tool, page),
        PressAction::Build => builder::press(app, page, inp.mods.alt),
        PressAction::PenPoint => pen_press(app, rect, pos, page, inp.mods),
        PressAction::Erase => crate::board::erase::press(app, page),
        PressAction::Connect => {
            let from = hit_under(app, page, true)
                .filter(|id| app.doc.get(*id).is_some_and(|n| crate::board::connectable(&n.kind)))
                .map(|id| (id, desigma_doc::Magnet::Auto));
            crate::board::connect::start(app, from, page);
        }
        PressAction::TextCreate => text_press(app, page),
        PressAction::Nothing => {
            if app.tool == Tool::Image {
                place_image(app, page);
            }
        }
    }
}

fn label_hit(app: &App, rect: ERect, pos: Pos2) -> Option<NodeId> {
    super::draw::label_rects(app, rect).into_iter().find(|(_, r)| r.contains(pos)).map(|(id, _)| id)
}

fn start_resize(app: &mut App, handle: Handle, page: DV) {
    let sel = app.top_selection();
    if sel.is_empty() {
        return;
    }
    let tx = Tx::new();
    let kind = if sel.len() == 1 {
        let id = sel[0];
        let Some(n) = app.doc.get(id) else { return };
        ResizeKind::Single { id, world0: app.doc.world_transform(id), t0: n.transform, size0: n.size }
    } else {
        let nodes: Vec<_> = sel.iter().filter_map(|id| app.doc.get(*id).map(|n| (*id, world_aabb(&app.doc, *id), n.transform, n.size))).collect();
        let Some(bbox0) = union_all(nodes.iter().map(|n| n.1)) else { return };
        ResizeKind::Multi { bbox0, nodes }
    };
    let targets = snap_targets(app, &sel);
    let _ = page;
    app.canvas.inter = Interaction::Resize(ResizeState { tx, handle, kind, targets });
}

fn start_rotate(app: &mut App, _corner: usize, page: DV) {
    let tops = app.top_selection();
    if tops.len() > 1 {
        let Some(bb) = union_all(tops.iter().map(|i| world_aabb(&app.doc, *i))) else { return };
        let center = rect_center(&bb);
        let group = tops.iter().map(|i| (*i, app.doc.world_transform(*i))).collect();
        app.canvas.inter = Interaction::Rotate(RotateState {
            tx: Tx::new(),
            id: tops[0],
            center,
            start_angle: handles::angle_of(center, page),
            t0: Affine::IDENTITY,
            size: DV::ZERO,
            rot0: 0.0,
            group,
        });
        return;
    }
    let Some(id) = app.selection.first().copied() else { return };
    let Some(n) = app.doc.get(id) else { return };
    let w = app.doc.world_transform(id);
    let center = w.apply(v2(n.size.x / 2.0, n.size.y / 2.0));
    app.canvas.inter = Interaction::Rotate(RotateState {
        tx: Tx::new(),
        id,
        center,
        start_angle: handles::angle_of(center, page),
        t0: n.transform,
        size: n.size,
        rot0: n.transform.rotation_rad(),
        group: Vec::new(),
    });
}

fn start_draw(app: &mut App, tool: Tool, page: DV) {
    app.clear_selection();
    let targets = snap_targets(app, &[]);
    let (start, sr) = snap_point(page, &targets, app.view.zoom);
    let start = v2(
        if snapped_on(&sr.guides, snap::GuideAxis::X) { start.x } else { px(app, start.x) },
        if snapped_on(&sr.guides, snap::GuideAxis::Y) { start.y } else { px(app, start.y) },
    );
    let parent = create::container_at(&app.doc, app.page, start, &[]);
    // a table is its grid node plus one node per cell
    let nodes = if tool == Tool::Table {
        let mut ns = desigma_doc::table_nodes(&mut app.doc, DV::ZERO, desigma_doc::TABLE_DEFAULT_COLS, desigma_doc::TABLE_DEFAULT_ROWS);
        ns[0].name = app.unique_name("Table");
        ns
    } else {
        vec![create::new_shape(app, tool)]
    };
    let id = nodes[0].id;
    let mut tx = Tx::new();
    let index = app.doc.get(parent).map(|p| p.children.len()).unwrap_or(0);
    if tx.cmd(&mut app.doc, Command::Insert { parent, index, nodes }).is_err() {
        return;
    }
    app.selection = vec![id];
    app.canvas.inter = Interaction::Draw(DrawState { tx, tool, id, parent, start, targets, dragged: false });
    app.changed();
}

// ------------------------------------------------------------------ drag
fn on_drag(app: &mut App, ui: &Ui, rect: ERect, inp: &Inp) {
    let Some(pos) = inp.pos else { return };
    let cur = app.view.to_page(rect.min, pos);
    let zoom = app.view.zoom;
    let mut inter = std::mem::take(&mut app.canvas.inter);
    match &mut inter {
        Interaction::Pan => {
            let d = ui.input(|i| i.pointer.delta());
            app.view.offset = v2(app.view.offset.x - d.x / zoom, app.view.offset.y - d.y / zoom);
        }
        Interaction::Pending(p) => {
            if (pos - p.start_screen).length() > theme::DRAG_THRESHOLD_PX {
                let (node, start, alt) = (p.node, p.start_page, inp.mods.alt);
                if let Some(mut m) = start_move(app, node, start, alt) {
                    update_move(app, &mut m, cur, inp);
                    inter = Interaction::Move(m);
                } else {
                    inter = Interaction::None;
                }
            }
        }
        Interaction::Move(m) => update_move(app, m, cur, inp),
        Interaction::Guide(g) => rulers::drag(app, g, rect, pos),
        Interaction::Connect(c) => crate::board::connect::drag(app, c, cur),
        Interaction::Table(d) => crate::board::table::drag(app, d, cur),
        Interaction::Marquee { start, cur: c, base } => {
            *c = cur;
            let r = rect_from_points(*start, cur);
            let mut sel = base.clone();
            for id in hit::marquee_select(&app.doc, app.page, &r) {
                if !sel.contains(&id) {
                    sel.push(id);
                }
            }
            app.selection = sel;
        }
        Interaction::Resize(r) => update_resize(app, r, cur, inp),
        Interaction::Rotate(r) => {
            r.tx.ed.revert(&mut app.doc);
            if r.group.is_empty() {
                let mut ang = r.rot0 + (handles::angle_of(r.center, cur) - r.start_angle);
                if inp.mods.shift {
                    ang = handles::snap_angle(ang, 15.0);
                }
                let t = handles::rotate_about_center(&r.t0, r.size, ang);
                if let Some(n) = r.tx.ed.node(&mut app.doc, r.id) {
                    n.transform = t;
                }
                layout::relayout_up(&mut app.doc, &mut r.tx.ed, r.id);
            } else {
                let mut delta = handles::angle_of(r.center, cur) - r.start_angle;
                if inp.mods.shift {
                    delta = handles::snap_angle(delta, 15.0);
                }
                crate::ops::rotate_nodes_about(&mut app.doc, &mut r.tx.ed, &r.group, r.center, delta);
            }
            app.changed();
        }
        Interaction::Draw(d) => {
            let (mut c, snapped) = if inp.mods.command { (cur, snap::SnapResult::default()) } else { snap_point(cur, &d.targets, zoom) };
            if inp.mods.shift {
                c = cur;
            }
            if !inp.mods.command {
                if !snapped_on(&snapped.guides, snap::GuideAxis::X) {
                    c.x = px(app, c.x);
                }
                if !snapped_on(&snapped.guides, snap::GuideAxis::Y) {
                    c.y = px(app, c.y);
                }
            }
            app.canvas.guides = snapped.guides;
            app.canvas.measures.clear();
            if dist(d.start, c) * zoom > theme::DRAG_THRESHOLD_PX {
                d.dragged = true;
            }
            let g = create::draw_geom(d.tool, d.start, c, inp.mods.shift || crate::board::create::keeps_square(d.tool, app.board.wide), inp.mods.alt);
            create::apply_geom(&mut app.doc, &mut d.tx, d.id, d.parent, &g);
            app.changed();
        }
        Interaction::Pencil(pts) => {
            let brush = app.tool.is_brush();
            if brush && (inp.mods.shift || app.tool == Tool::Washi) && !pts.is_empty() {
                // Shift (and washi tape always) draws a straight line; Shift also snaps it to 45 degrees
                let first = pts[0];
                let end = if inp.mods.shift { create::snap_to_45(first, cur) } else { cur };
                pts.truncate(1);
                pts.push(end);
            } else if pts.last().map(|l| dist(*l, cur) * zoom > 1.5).unwrap_or(true) {
                pts.push(cur);
            }
        }
        Interaction::Erase(e) => crate::board::erase::drag(app, e, cur),
        Interaction::VectorDrag => vector::drag(app, cur, inp.mods),
        Interaction::Builder => builder::drag(app, cur),
        Interaction::PenDrag => {
            if let Some(pen) = &mut app.canvas.pen {
                if let Some(last) = pen.pts.last_mut() {
                    if dist(last.pos, cur) * zoom > 3.0 {
                        // Shift keeps the handle on a 45 degree angle
                        let cur = if inp.mods.shift { create::snap_to_45(last.pos, cur) } else { cur };
                        last.hout = Some(cur);
                        last.hin = Some(sub(scale(last.pos, 2.0), cur));
                    }
                }
            }
        }
        Interaction::None => {}
    }
    if matches!(app.canvas.inter, Interaction::None) {
        app.canvas.inter = inter;
    }
}

fn start_move(app: &mut App, clicked: NodeId, start: DV, alt: bool) -> Option<MoveState> {
    let mut ids = app.top_selection();
    if ids.is_empty() {
        ids.push(clicked);
    }
    let mut tx = Tx::new();
    if alt {
        let mut dup = Vec::new();
        // one clip for the whole selection, so connectors between duplicated nodes join the copies
        let clip = crate::clipboard::capture(&app.doc, &ids);
        let copies = clip.map(|c| crate::clipboard::instantiate(&mut app.doc, &c)).unwrap_or_default();
        for (id, nodes) in ids.iter().zip(copies) {
            let Some(parent) = app.doc.get(*id).and_then(|n| n.parent) else { continue };
            let index = app.doc.get(parent).and_then(|p| p.children.iter().position(|c| c == id)).map(|i| i + 1).unwrap_or(usize::MAX);
            let root = nodes.first().map(|n| n.id);
            if tx.cmd(&mut app.doc, Command::Insert { parent, index, nodes }).is_ok() {
                dup.extend(root);
            }
        }
        if !dup.is_empty() {
            ids = dup;
            app.selection = ids.clone();
        }
    }
    let orig_bbox = union_all(ids.iter().map(|i| world_aabb(&app.doc, *i)))?;
    let targets = snap_targets(app, &ids);
    Some(MoveState { tx, ids, start, orig_bbox, targets, moved: false, reorder: None })
}

fn parent_has_auto_layout(doc: &Document, id: NodeId) -> bool {
    let Some(n) = doc.get(id) else { return false };
    if n.layout_child.absolute {
        return false;
    }
    n.parent.and_then(|p| doc.get(p)).and_then(|p| p.kind.frame_props()).map(|f| f.auto_layout.is_some()).unwrap_or(false)
}

fn update_move(app: &mut App, m: &mut MoveState, cur: DV, inp: &Inp) {
    m.tx.ed.revert(&mut app.doc);
    let mut d = sub(cur, m.start);
    let mut lock_x = false;
    let mut lock_y = false;
    if inp.mods.shift {
        if d.x.abs() > d.y.abs() {
            d.y = 0.0;
            lock_y = true;
        } else {
            d.x = 0.0;
            lock_x = true;
        }
    }
    app.canvas.guides.clear();
    app.canvas.measures.clear();
    if !inp.mods.command {
        let moving = Rect::new(m.orig_bbox.x + d.x, m.orig_bbox.y + d.y, m.orig_bbox.w, m.orig_bbox.h);
        let s = snap::snap_rect(&moving, &m.targets, theme::SNAP_THRESHOLD_PX / app.view.zoom);
        if !lock_x {
            d.x += s.dx;
        }
        if !lock_y {
            d.y += s.dy;
        }
        // whole pixels where no layer / guide edge was snapped
        if !lock_x && !snapped_on(&s.guides, snap::GuideAxis::X) {
            d.x = px(app, m.orig_bbox.x + d.x) - m.orig_bbox.x;
        }
        if !lock_y && !snapped_on(&s.guides, snap::GuideAxis::Y) {
            d.y = px(app, m.orig_bbox.y + d.y) - m.orig_bbox.y;
        }
        app.canvas.guides = s.guides;
        app.canvas.measures = s.measures;
    }
    // Auto-layout children follow the pointer too (their slot is decided on release).
    for id in &m.ids {
        let dp = delta_to_parent(&app.doc, *id, d);
        if let Some(n) = m.tx.ed.node(&mut app.doc, *id) {
            n.transform.tx += dp.x;
            n.transform.ty += dp.y;
        }
    }
    for id in &m.ids {
        if !parent_has_auto_layout(&app.doc, *id) {
            layout::relayout_up(&mut app.doc, &mut m.tx.ed, *id);
        }
    }
    m.reorder = match m.ids.as_slice() {
        [only] if parent_has_auto_layout(&app.doc, *only) => {
            let parent = app.doc.get(*only).and_then(|n| n.parent);
            parent.filter(|p| ts::node_contains(app, *p, cur)).and_then(|p| reorder::insertion_at(&app.doc, p, *only, cur))
        }
        _ => None,
    };
    m.moved = true;
    app.changed();
}

fn update_resize(app: &mut App, r: &mut ResizeState, cur: DV, inp: &Inp) {
    r.tx.ed.revert(&mut app.doc);
    let mut mouse = cur;
    app.canvas.guides.clear();
    app.canvas.measures.clear();
    let axis_aligned = match &r.kind {
        ResizeKind::Single { world0, .. } => world0.b.abs() < 1e-4 && world0.c.abs() < 1e-4,
        ResizeKind::Multi { .. } => true,
    };
    if axis_aligned && !inp.mods.command && !inp.mods.shift {
        let xe: Vec<f32> = if r.handle.affects_x() { vec![mouse.x] } else { vec![] };
        let ye: Vec<f32> = if r.handle.affects_y() { vec![mouse.y] } else { vec![] };
        let s = snap::snap_edges(&Rect::new(mouse.x, mouse.y, 0.0, 0.0), &xe, &ye, &r.targets, theme::SNAP_THRESHOLD_PX / app.view.zoom);
        mouse = v2(mouse.x + s.dx, mouse.y + s.dy);
        if r.handle.affects_x() && !snapped_on(&s.guides, snap::GuideAxis::X) {
            mouse.x = px(app, mouse.x);
        }
        if r.handle.affects_y() && !snapped_on(&s.guides, snap::GuideAxis::Y) {
            mouse.y = px(app, mouse.y);
        }
        app.canvas.guides = s.guides;
    }
    match &r.kind {
        ResizeKind::Single { id, world0, t0, size0 } => {
            let Some(inv) = world0.invert() else { return };
            let ml = inv.apply(mouse);
            let is_text = matches!(app.doc.get(*id).map(|n| &n.kind), Some(NodeKind::Text(_)));
            let is_line = matches!(app.doc.get(*id).map(|n| &n.kind), Some(NodeKind::Line));
            let (mut ns, d) = handles::resize_local(*size0, r.handle, ml, inp.mods.shift && !is_line || is_text && r.handle.is_corner(), inp.mods.alt);
            if is_line {
                ns.y = 0.0;
            }
            let t = *t0 * Affine::translate(d.x, if is_line { 0.0 } else { d.y });
            apply_resize(&mut app.doc, &mut r.tx.ed, *id, ns, t, true);
        }
        ResizeKind::Multi { bbox0, nodes } => {
            let ml = sub(mouse, v2(bbox0.x, bbox0.y));
            let (ns, d) = handles::resize_local(v2(bbox0.w, bbox0.h), r.handle, ml, inp.mods.shift, inp.mods.alt);
            let nb = Rect::new(bbox0.x + d.x, bbox0.y + d.y, ns.x, ns.y);
            let (sx, sy) = (ns.x / bbox0.w.max(1e-3), ns.y / bbox0.h.max(1e-3));
            let mut ids = Vec::new();
            for (id, aabb0, t0, size0) in nodes {
                let (nx, ny) = (nb.x + (aabb0.x - bbox0.x) * sx, nb.y + (aabb0.y - bbox0.y) * sy);
                let pinv = parent_world(&app.doc, *id).invert().unwrap_or(Affine::IDENTITY);
                let rotated = t0.b.abs() > 1e-4 || t0.c.abs() > 1e-4;
                if rotated {
                    let c0 = rect_center(aabb0);
                    let c1 = v2(nb.x + (c0.x - bbox0.x) * sx, nb.y + (c0.y - bbox0.y) * sy);
                    let dp = apply_linear(&pinv, sub(c1, c0));
                    apply_resize(&mut app.doc, &mut r.tx.ed, *id, *size0, Affine { tx: t0.tx + dp.x, ty: t0.ty + dp.y, ..*t0 }, false);
                } else {
                    let o = pinv.apply(v2(nx, ny));
                    let t = Affine { tx: o.x, ty: o.y, ..*t0 };
                    apply_resize(&mut app.doc, &mut r.tx.ed, *id, v2((size0.x * sx).max(1.0), (size0.y * sy).max(1.0)), t, false);
                }
                ids.push(*id);
            }
            for id in ids {
                layout::relayout_up(&mut app.doc, &mut r.tx.ed, id);
            }
        }
    }
    app.changed();
}

// ------------------------------------------------------------------ release
fn on_release(app: &mut App, rect: ERect, inp: &Inp) {
    let cur = inp.pos.map(|p| app.view.to_page(rect.min, p)).unwrap_or(DV::ZERO);
    let inter = std::mem::take(&mut app.canvas.inter);
    app.canvas.guides.clear();
    app.canvas.measures.clear();
    match inter {
        Interaction::Pending(p) => {
            if p.was_selected {
                if p.shift {
                    app.selection.retain(|s| *s != p.node);
                } else {
                    app.select_only(p.node);
                    // a click on a cell of the table that was already selected makes it the active cell
                    if matches!(app.doc.get(p.node).map(|n| &n.kind), Some(NodeKind::Table(_))) {
                        crate::board::table::pick_cell(app, p.node, cur);
                    }
                }
            }
        }
        Interaction::Guide(g) => rulers::release(app, g),
        Interaction::Connect(c) => crate::board::connect::release(app, c),
        Interaction::Table(d) => crate::board::table::release(app, d),
        Interaction::Move(mut m) => {
            let parents: Vec<Option<NodeId>> = m.ids.iter().map(|i| app.doc.get(*i).and_then(|n| n.parent)).collect();
            if m.moved && !m.ids.is_empty() {
                reparent_after_move(app, &mut m, cur);
            }
            let same_parents = m.ids.iter().zip(&parents).all(|(i, p)| app.doc.get(*i).map(|n| n.parent) == Some(*p));
            if app.motion.enabled && m.moved && same_parents {
                // Motion mode: the drag keys Position at the playhead.
                crate::motion::inspector::commit_move(app, m);
            } else {
                app.run_tx(m.tx);
            }
        }
        Interaction::Resize(r) => {
            app.run_tx(r.tx);
        }
        Interaction::Rotate(r) => {
            app.run_tx(r.tx);
        }
        Interaction::Draw(mut d) => {
            if !d.dragged && crate::board::create::click_places(d.tool, app.is_board()) {
                // a click places a board item at its default size, centred on the pointer
                let r = crate::board::create::click_rect(d.tool, d.start, app.board.wide);
                create::apply_geom(&mut app.doc, &mut d.tx, d.id, d.parent, &create::DrawGeom { rect: r, line: None });
            } else if !d.dragged {
                let (w, h) = (theme::DEFAULT_SHAPE_SIZE, theme::DEFAULT_SHAPE_SIZE);
                let g = if matches!(d.tool, Tool::Line | Tool::Arrow) {
                    create::draw_geom(d.tool, d.start, add(d.start, v2(w, 0.0)), false, false)
                } else {
                    create::draw_geom(d.tool, d.start, add(d.start, v2(w, h)), false, false)
                };
                create::apply_geom(&mut app.doc, &mut d.tx, d.id, d.parent, &g);
            }
            let id = d.id;
            // a section drawn around notes takes them in
            if matches!(app.doc.get(id).map(|n| &n.kind), Some(NodeKind::BoardSection(_))) {
                crate::board::section::adopt_inside(app, &mut d.tx, id);
            }
            app.run_tx(d.tx);
            app.selection = vec![id];
            app.tool = if app.keep_tool { d.tool } else { Tool::Move };
            // a new table takes text in its first cell right away
            if d.tool == Tool::Table {
                if let Some(c) = desigma_doc::table_cell_node(&app.doc, id, 0, 0) {
                    crate::board::table::edit_cell(app, c);
                }
            }
            // a new sticky or shape takes text right away
            if crate::board::create::types_into(d.tool) {
                begin_text_edit(app, id);
            }
        }
        Interaction::Pencil(pts) => {
            // the stroke is smoothed into a few bezier anchors; ending on its start closes the shape
            let straight = app.tool.is_brush() && pts.len() == 2;
            let (dec, closed) = if straight { (pts.iter().map(|p| PenPoint::new(*p)).collect(), false) } else { create::smooth_stroke(&pts, app.view.zoom) };
            let brush = app.tool.is_brush().then_some(app.tool);
            let mut tx = Tx::new();
            if let Some(id) = create::commit_pen(app, &mut tx, &dec, closed && brush.is_none()) {
                if let Some(tool) = brush {
                    // marker and highlighter strokes stay open, thick and round
                    let (ink, size, pattern) = (app.board.ink, app.board.size, app.board.pattern);
                    if let Some(n) = tx.ed.node(&mut app.doc, id) {
                        crate::board::create::brush_style(n, tool, ink, size, pattern);
                    }
                }
                app.run_tx(tx);
                if brush.is_none() {
                    app.selection = vec![id];
                    app.tool = Tool::Move;
                }
            }
        }
        Interaction::Erase(e) => crate::board::erase::release(app, e),
        Interaction::VectorDrag => vector::release(app),
        Interaction::Builder => builder::release(app),
        Interaction::PenDrag | Interaction::Pan | Interaction::Marquee { .. } | Interaction::None => {}
    }
    app.changed();
}

fn reparent_after_move(app: &mut App, m: &mut MoveState, cur: DV) {
    let target = create::container_at(&app.doc, app.page, cur, &m.ids);
    let mut touched_parents = Vec::new();
    if let Some(r) = m.reorder.take() {
        // Dropped inside its auto-layout frame: move to the indicated slot.
        if let Some(&id) = m.ids.first() {
            if m.tx.cmd(&mut app.doc, Command::Reparent { id, parent: r.parent, index: r.index }).is_ok() {
                layout::relayout_around(&mut app.doc, &mut m.tx.ed, r.parent);
            }
        }
        return;
    }
    for id in m.ids.clone() {
        let Some(n) = app.doc.get(id) else { continue };
        let old_parent = n.parent;
        // Children of an auto-layout frame that stay inside it only reorder; leaving it takes them out of the flow.
        if old_parent == Some(target) || app.doc.is_ancestor(id, target) {
            continue;
        }
        let Some(inv) = app.doc.world_transform(target).invert() else { continue };
        let world = app.doc.world_transform(id);
        let index = app.doc.get(target).map(|p| p.children.len()).unwrap_or(0);
        if m.tx.cmd(&mut app.doc, Command::Reparent { id, parent: target, index }).is_ok() {
            if let Some(n) = m.tx.ed.node(&mut app.doc, id) {
                n.transform = inv * world;
            }
            touched_parents.extend(old_parent);
        }
    }
    if !touched_parents.is_empty() {
        for p in touched_parents {
            if app.doc.get(p).is_some() {
                layout::relayout_around(&mut app.doc, &mut m.tx.ed, p);
            }
        }
        layout::relayout_around(&mut app.doc, &mut m.tx.ed, target);
    }
}

// ------------------------------------------------------------------ double click
fn on_double_click(app: &mut App, rect: ERect, inp: &Inp) {
    let Some(pos) = inp.pos else { return };
    let page = app.view.to_page(rect.min, pos);
    if app.tool == Tool::Pen {
        finish_pen(app);
        app.tool = Tool::Move;
        return;
    }
    if let Some(id) = app.canvas.text.as_ref().map(|t| t.id) {
        if ts::node_contains(app, id, page) {
            ts::select_word(app, page);
            return;
        }
    }
    if app.tool != Tool::Move {
        return;
    }
    if let Some(id) = hit_under(app, page, true) {
        let is_text = matches!(app.doc.get(id).map(|n| &n.kind), Some(NodeKind::Text(_)));
        let board_kind = app.doc.get(id).map(|n| &n.kind);
        if is_text {
            begin_text_edit(app, id);
        } else if matches!(board_kind, Some(NodeKind::Sticky(_) | NodeKind::ShapeWithText(_) | NodeKind::CodeBlock(_))) {
            // stickies and shapes take their text on double-click
            begin_text_edit(app, id);
        } else if matches!(board_kind, Some(NodeKind::Table(_))) {
            // a table takes the text of the cell under the pointer
            app.select_only(id);
            crate::board::table::edit_at(app, id, page);
        } else if matches!(board_kind, Some(NodeKind::BoardSection(_))) {
            app.select_only(id);
            crate::board::section::begin_rename(app, id);
        } else if matches!(board_kind, Some(NodeKind::Connector(_))) {
            crate::board::create::edit_label(app, id);
        } else if vector::is_vector(app, id) && (app.is_selected(id) || matches!(app.doc.get(id).map(|n| &n.kind), Some(NodeKind::Vector { .. }))) {
            // vectors open right away; basic shapes first get selected (stepping into their group)
            vector::enter(app, id);
        } else if app.is_selected(id) {
            // already the deepest hit: nothing further to enter
        } else {
            app.select_only(id);
        }
    }
}

// ------------------------------------------------------------------ pen
fn pen_press(app: &mut App, rect: ERect, pos: Pos2, page: DV, mods: Modifiers) {
    let zoom = app.view.zoom;
    if app.canvas.pen.is_none() && !app.tool.is_brush() {
        app.clear_selection();
        app.canvas.pen = Some(PenState { pts: vec![] });
    }
    let close = app
        .canvas
        .pen
        .as_ref()
        .and_then(|p| p.pts.first().filter(|_| p.pts.len() >= 3))
        .map(|f| {
            let s = app.view.to_screen(rect.min, f.pos);
            (s - pos).length() < 8.0
        })
        .unwrap_or(false);
    if app.tool == Tool::Pencil || app.tool.is_brush() {
        app.canvas.pen = None;
        app.canvas.inter = Interaction::Pencil(vec![page]);
        return;
    }
    if close {
        let pts = app.canvas.pen.take().map(|p| p.pts).unwrap_or_default();
        let mut tx = Tx::new();
        if let Some(id) = create::commit_pen(app, &mut tx, &pts, true) {
            app.run_tx(tx);
            app.selection = vec![id];
        }
        app.tool = Tool::Move;
        return;
    }
    let _ = zoom;
    if let Some(p) = &mut app.canvas.pen {
        // Shift constrains the new segment to 45 degree steps
        let at = match p.pts.last() {
            Some(last) if mods.shift => create::snap_to_45(last.pos, page),
            _ => page,
        };
        p.pts.push(PenPoint::new(at));
    }
    app.canvas.inter = Interaction::PenDrag;
}

pub fn finish_pen(app: &mut App) {
    let Some(pen) = app.canvas.pen.take() else { return };
    if matches!(app.canvas.inter, Interaction::PenDrag) {
        app.canvas.inter = Interaction::None;
    }
    if pen.pts.len() >= 2 {
        let mut tx = Tx::new();
        if let Some(id) = create::commit_pen(app, &mut tx, &pen.pts, false) {
            app.run_tx(tx);
            app.selection = vec![id];
        }
    }
}

// ------------------------------------------------------------------ images
/// Extensions offered by the image picker.
pub const IMAGE_EXTENSIONS: [&str; 5] = ["png", "jpg", "jpeg", "webp", "svg"];

/// Validates image bytes (PNG, JPEG or WebP) and returns the bytes to embed
/// plus the pixel size. WebP is re-encoded as PNG so every renderer build can
/// decode it.
pub fn prepare_image(bytes: Vec<u8>) -> Result<(Vec<u8>, u32, u32), String> {
    let fmt = image::guess_format(&bytes).map_err(|_| "Unsupported image (use PNG, JPEG or WebP)".to_string())?;
    if !matches!(fmt, image::ImageFormat::Png | image::ImageFormat::Jpeg | image::ImageFormat::WebP) {
        return Err("Unsupported image (use PNG, JPEG or WebP)".into());
    }
    let img = image::load_from_memory_with_format(&bytes, fmt).map_err(|e| format!("Cannot decode image: {e}"))?;
    let (w, h) = (img.width(), img.height());
    if fmt == image::ImageFormat::WebP {
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).map_err(|e| format!("Cannot convert image: {e}"))?;
        return Ok((out.into_inner(), w, h));
    }
    Ok((bytes, w, h))
}

fn place_image(app: &mut App, page: DV) {
    let exts: Vec<&str> = IMAGE_EXTENSIONS.iter().chain(crate::video::VIDEO_EXTENSIONS.iter()).copied().collect();
    let Some(path) = rfd::FileDialog::new().add_filter("Images and video", &exts).pick_file() else { return };
    let Ok(bytes) = std::fs::read(&path) else {
        app.show_toast("Cannot read file", true);
        return;
    };
    let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("Image").to_string();
    if path.extension().and_then(|e| e.to_str()).is_some_and(crate::video::is_video_extension) {
        place_video_bytes(app, page, &name, bytes);
        return;
    }
    if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("svg")) {
        // Vector layers, centered on the click.
        let parent = create::container_at(&app.doc, app.page, page, &[]);
        match String::from_utf8(bytes) {
            Ok(text) => {
                if let Err(e) = crate::clipboard::place_svg(app, &text, &name, page, parent) {
                    app.show_toast(e, true);
                } else {
                    app.tool = Tool::Move;
                }
            }
            Err(_) => app.show_toast("The SVG is not valid UTF-8", true),
        }
        return;
    }
    place_image_bytes(app, page, &name, bytes);
}

/// Embeds a video file and inserts a rectangle showing its poster frame at `page`. The poster is
/// a normal image fill; a [`desigma_doc::VideoFill`] on the layer's prototype settings makes it a
/// video in Present (see `crate::video`).
pub fn place_video_bytes(app: &mut App, page: DV, name: &str, bytes: Vec<u8>) -> Option<NodeId> {
    if desigma_doc::VideoContainer::sniff(&bytes).is_none() {
        app.show_toast("Not a supported video file (MP4, MOV or WebM)", true);
        return None;
    }
    let Some((poster, iw, ih)) = crate::video::poster_png(&bytes) else {
        app.show_toast("Cannot make a poster for this video", true);
        return None;
    };
    let mut n = app.doc.images.len() + 1;
    while app.doc.images.contains_key(&format!("img-{n}")) || app.doc.images.contains_key(&format!("video-{n}")) {
        n += 1;
    }
    let (poster_key, video_key) = (format!("img-{n}"), format!("video-{n}"));
    app.doc.images.insert(poster_key.clone(), poster);
    app.doc.images.insert(video_key.clone(), bytes);
    let parent = create::container_at(&app.doc, app.page, page, &[]);
    let inv = app.doc.world_transform(parent).invert()?;
    let o = inv.apply(page);
    let id = app.doc.alloc_id();
    let (w, h) = (iw as f32, ih as f32);
    let scale = (480.0f32 / w.max(h)).min(1.0);
    let mut node = desigma_doc::Node::rectangle(id, Rect::new(o.x, o.y, w * scale, h * scale));
    node.name = name.to_string();
    node.fills = vec![desigma_doc::Paint {
        kind: desigma_doc::PaintKind::Image { image_ref: poster_key, scale_mode: desigma_doc::ImageScaleMode::Fill },
        opacity: 1.0,
        visible: true,
        blend: desigma_doc::BlendMode::Normal,
    }];
    node.prototype.videos = vec![desigma_doc::VideoFill { paint: 0, source: video_key, autoplay: true, looping: false, muted: true }];
    let mut tx = Tx::new();
    let index = app.doc.get(parent).map(|p| p.children.len()).unwrap_or(0);
    tx.cmd(&mut app.doc, Command::Insert { parent, index, nodes: vec![node] }).ok()?;
    app.run_tx(tx);
    app.selection = vec![id];
    app.tool = Tool::Move;
    Some(id)
}

/// Embeds `bytes` and inserts a rectangle filled with the image at `page`.
pub fn place_image_bytes(app: &mut App, page: DV, name: &str, bytes: Vec<u8>) -> Option<NodeId> {
    let (bytes, iw, ih) = match prepare_image(bytes) {
        Ok(v) => v,
        Err(e) => {
            app.show_toast(e, true);
            return None;
        }
    };
    let key = format!("img-{}", app.doc.images.len() + 1);
    app.doc.images.insert(key.clone(), bytes);
    let parent = create::container_at(&app.doc, app.page, page, &[]);
    let inv = app.doc.world_transform(parent).invert()?;
    let o = inv.apply(page);
    let id = app.doc.alloc_id();
    let (w, h) = (iw as f32, ih as f32);
    let scale = (400.0f32 / w.max(h)).min(1.0);
    let mut n = desigma_doc::Node::rectangle(id, Rect::new(o.x, o.y, w * scale, h * scale));
    n.name = name.to_string();
    n.fills = vec![desigma_doc::Paint {
        kind: desigma_doc::PaintKind::Image { image_ref: key, scale_mode: desigma_doc::ImageScaleMode::Fill },
        opacity: 1.0,
        visible: true,
        blend: desigma_doc::BlendMode::Normal,
    }];
    let mut tx = Tx::new();
    let index = app.doc.get(parent).map(|p| p.children.len()).unwrap_or(0);
    tx.cmd(&mut app.doc, Command::Insert { parent, index, nodes: vec![n] }).ok()?;
    app.run_tx(tx);
    app.selection = vec![id];
    app.tool = Tool::Move;
    Some(id)
}

// ------------------------------------------------------------------ text
fn text_press(app: &mut App, page: DV) {
    if let Some(id) = hit_under(app, page, true) {
        if matches!(app.doc.get(id).map(|n| &n.kind), Some(NodeKind::Text(_))) {
            begin_text_edit(app, id);
            ts::press_at(app, page, false);
            return;
        }
    }
    let parent = create::container_at(&app.doc, app.page, page, &[]);
    let Some((node, _)) = create::new_text(app, parent, page) else { return };
    let id = node.id;
    let mut tx = Tx::new();
    let index = app.doc.get(parent).map(|p| p.children.len()).unwrap_or(0);
    if tx.cmd(&mut app.doc, Command::Insert { parent, index, nodes: vec![node] }).is_err() {
        return;
    }
    app.selection = vec![id];
    app.canvas.text = Some(TextSession::new(id, 0, 0, tx, true));
    app.changed();
}

// ------------------------------------------------------------------ hover / cursor
fn update_hover_and_cursor(app: &mut App, rect: ERect, inp: &Inp, hovered: bool) {
    let mut cursor = egui::CursorIcon::Default;
    let idle = matches!(app.canvas.inter, Interaction::None);
    app.canvas.cursor = None;
    // board: magnet dots follow the pointer
    let board_page = (hovered && app.canvas.text.is_none()).then(|| inp.pos.map(|p| app.view.to_page(rect.min, p))).flatten();
    crate::board::connect::update_hover(app, board_page);
    crate::board::table::update_hover(app, rect.min, board_page);
    let over_dot = idle && hovered && inp.pos.is_some_and(|p| crate::board::connect::dot_hit(app, rect, p).is_some());
    if idle && hovered && app.canvas.text.is_none() {
        let page = inp.pos.map(|p| app.view.to_page(rect.min, p));
        match app.tool {
            Tool::Move | Tool::Scale => {
                app.hover = None;
                if inp.space {
                    cursor = egui::CursorIcon::Grab;
                } else if let Some(pos) = inp.pos {
                    match handle_hit(app, rect, pos) {
                        Some(HandleHit::Resize(h)) => cursor = h.cursor(),
                        Some(HandleHit::Rotate(_)) => cursor = egui::CursorIcon::Alias,
                        None => {
                            app.hover = label_hit(app, rect, pos).or_else(|| page.and_then(|p| hit_under(app, p, inp.mods.command)));
                        }
                    }
                }
            }
            Tool::Hand => cursor = egui::CursorIcon::Grab,
            Tool::Text => cursor = egui::CursorIcon::Text,
            _ => cursor = egui::CursorIcon::Crosshair,
        }
        if let Some(c) = inp.pos.and_then(|p| rulers::cursor_at(app, rect, p)) {
            cursor = c;
            app.hover = None;
        }
        if over_dot {
            cursor = egui::CursorIcon::Crosshair;
        }
        if let Some(c) = inp.pos.and_then(|p| crate::board::table::cursor_at(app, rect.min, p)) {
            cursor = c;
        }
        app.canvas.cursor = Some(cursor);
    } else if !idle {
        cursor = match &app.canvas.inter {
            Interaction::Pan => egui::CursorIcon::Grabbing,
            Interaction::Resize(r) => r.handle.cursor(),
            Interaction::Rotate(_) => egui::CursorIcon::Alias,
            Interaction::Draw(_) | Interaction::Pencil(_) | Interaction::Erase(_) | Interaction::PenDrag | Interaction::Connect(_) => {
                egui::CursorIcon::Crosshair
            }
            Interaction::Table(d) => {
                if d.axis == crate::board::table::Axis::Col {
                    egui::CursorIcon::ResizeHorizontal
                } else {
                    egui::CursorIcon::ResizeVertical
                }
            }
            Interaction::Guide(g) => {
                if g.axis == snap::GuideAxis::X {
                    egui::CursorIcon::ResizeHorizontal
                } else {
                    egui::CursorIcon::ResizeVertical
                }
            }
            _ => egui::CursorIcon::Default,
        };
        app.canvas.cursor = Some(cursor);
        app.hover = None;
    } else if app.canvas.text.is_some() {
        app.canvas.cursor = Some(egui::CursorIcon::Text);
    } else if !hovered {
        app.hover = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snap_point_aligns_to_edges() {
        let t = [Rect::new(100.0, 100.0, 50.0, 50.0)];
        let (p, r) = snap_point(v2(102.0, 300.0), &t, 1.0);
        assert_eq!(p.x, 100.0);
        assert_eq!(p.y, 300.0);
        assert_eq!(r.guides.len(), 1);
    }

    fn app_with_two() -> (App, NodeId, NodeId) {
        let mut d = Document::new("t");
        let page = d.add_page("p");
        let a = d.alloc_id();
        let a = d.insert(page, desigma_doc::Node::rectangle(a, Rect::new(0.0, 0.0, 100.0, 100.0))).unwrap();
        let b = d.alloc_id();
        let b = d.insert(page, desigma_doc::Node::rectangle(b, Rect::new(300.0, 0.0, 100.0, 100.0))).unwrap();
        (App::new(d), a, b)
    }

    #[test]
    fn move_with_snap_and_commit() {
        let (mut app, a, b) = app_with_two();
        app.selection = vec![a];
        let mut m = start_move(&mut app, a, v2(50.0, 50.0), false).unwrap();
        let inp = Inp { pos: None, pressed: false, down: true, released: false, mid_pressed: false, mid_down: false, mods: Modifiers::NONE, space: false };
        // move a so its right edge is 2 units from b's left edge -> snaps flush
        update_move(&mut app, &mut m, v2(50.0 + 198.0, 50.0), &inp);
        assert_eq!(app.doc.get(a).unwrap().transform.tx, 200.0);
        assert!(!app.canvas.guides.is_empty());
        assert!(app.run_tx(m.tx));
        assert_eq!(app.doc.get(a).unwrap().transform.tx, 200.0);
        app.undo();
        assert_eq!(app.doc.get(a).unwrap().transform.tx, 0.0);
        assert_eq!(app.doc.get(b).unwrap().transform.tx, 300.0);
    }

    #[test]
    fn shift_locks_axis_and_alt_duplicates() {
        let (mut app, a, _b) = app_with_two();
        app.selection = vec![a];
        let mut m = start_move(&mut app, a, v2(0.0, 0.0), true).unwrap();
        assert_ne!(m.ids[0], a, "alt-drag moves a duplicate");
        let inp = Inp {
            pos: None,
            pressed: false,
            down: true,
            released: false,
            mid_pressed: false,
            mid_down: false,
            mods: Modifiers { shift: true, command: true, ..Modifiers::NONE },
            space: false,
        };
        update_move(&mut app, &mut m, v2(80.0, 10.0), &inp);
        let dup = app.doc.get(m.ids[0]).unwrap();
        assert_eq!(dup.transform.tx, 80.0);
        assert_eq!(dup.transform.ty, 0.0);
        assert_eq!(app.doc.get(a).unwrap().transform.tx, 0.0);
        app.run_tx(m.tx);
        assert_eq!(app.doc.get(app.page).unwrap().children.len(), 3);
    }

    #[test]
    fn resize_single_rotated_keeps_anchor() {
        let (mut app, a, _b) = app_with_two();
        app.doc.get_mut(a).unwrap().transform = Affine::translate(100.0, 100.0) * Affine::rotate(std::f32::consts::FRAC_PI_2);
        let world0 = app.doc.world_transform(a);
        let anchor = world0.apply(v2(0.0, 0.0)); // TL stays when dragging BR
        let mut r = ResizeState {
            tx: Tx::new(),
            handle: Handle::BR,
            kind: ResizeKind::Single { id: a, world0, t0: app.doc.get(a).unwrap().transform, size0: v2(100.0, 100.0) },
            targets: vec![],
        };
        let inp = Inp {
            pos: None,
            pressed: false,
            down: true,
            released: false,
            mid_pressed: false,
            mid_down: false,
            mods: Modifiers { command: true, ..Modifiers::NONE },
            space: false,
        };
        // local (150, 120) maps to page via world0
        let target = world0.apply(v2(150.0, 120.0));
        update_resize(&mut app, &mut r, target, &inp);
        let n = app.doc.get(a).unwrap();
        assert!((n.size.x - 150.0).abs() < 1e-3 && (n.size.y - 120.0).abs() < 1e-3);
        let anchor_after = app.doc.world_transform(a).apply(v2(0.0, 0.0));
        assert!(dist(anchor, anchor_after) < 1e-3);
    }

    #[test]
    fn marquee_selects_enclosed() {
        let (mut app, a, b) = app_with_two();
        app.canvas.inter = Interaction::Marquee { start: v2(-10.0, -10.0), cur: v2(-10.0, -10.0), base: vec![] };
        let r = rect_from_points(v2(-10.0, -10.0), v2(150.0, 150.0));
        let ids = hit::marquee_select(&app.doc, app.page, &r);
        assert_eq!(ids, vec![a]);
        assert!(!ids.contains(&b));
    }

    fn encode(fmt: image::ImageFormat) -> Vec<u8> {
        let img = image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(8, 4, |x, y| image::Rgb([x as u8 * 30, y as u8 * 60, 90])));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, fmt).unwrap();
        out.into_inner()
    }

    #[test]
    fn prepare_image_accepts_png_jpeg_webp() {
        for fmt in [image::ImageFormat::Png, image::ImageFormat::Jpeg, image::ImageFormat::WebP] {
            let (bytes, w, h) = prepare_image(encode(fmt)).unwrap_or_else(|e| panic!("{fmt:?}: {e}"));
            assert_eq!((w, h), (8, 4), "{fmt:?}");
            if fmt == image::ImageFormat::WebP {
                assert_eq!(image::guess_format(&bytes).unwrap(), image::ImageFormat::Png, "webp is stored as png");
            }
        }
        assert!(prepare_image(b"not an image".to_vec()).is_err());
        assert!(prepare_image(b"GIF89a-not-supported".to_vec()).is_err());
    }

    #[test]
    fn place_jpeg_creates_image_fill_and_undoes() {
        let (mut app, _a, _b) = app_with_two();
        let n0 = app.doc.nodes.len();
        let id = place_image_bytes(&mut app, v2(500.0, 500.0), "photo", encode(image::ImageFormat::Jpeg)).unwrap();
        assert_eq!(app.doc.nodes.len(), n0 + 1);
        assert_eq!(app.doc.images.len(), 1);
        let n = app.doc.get(id).unwrap();
        assert_eq!(n.name, "photo");
        assert!(matches!(n.fills[0].kind, desigma_doc::PaintKind::Image { .. }));
        assert!((n.size.x / n.size.y - 2.0).abs() < 1e-3);
        app.undo();
        assert_eq!(app.doc.nodes.len(), n0);
    }

    #[test]
    fn place_video_makes_a_poster_layer_with_a_video_sidecar() {
        let (mut app, _a, _b) = app_with_two();
        let jpeg = |c: u8| {
            let mut out = Vec::new();
            let img = image::RgbImage::from_pixel(32, 16, image::Rgb([c, 0, 0]));
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 95).encode_image(&img).unwrap();
            out
        };
        let mp4 = crate::video::build_mjpeg_mp4(&[jpeg(250), jpeg(10)], 32, 16, 10);
        let n0 = app.doc.nodes.len();
        let id = place_video_bytes(&mut app, v2(500.0, 500.0), "clip", mp4.clone()).unwrap();
        let n = app.doc.get(id).unwrap();
        assert_eq!(n.name, "clip");
        assert!((n.size.x / n.size.y - 2.0).abs() < 1e-3);
        // the paint is a plain image (the poster = first frame), the sidecar points at the video bytes
        let desigma_doc::PaintKind::Image { image_ref, .. } = &n.fills[0].kind else { panic!() };
        let poster = image::load_from_memory(&app.doc.images[image_ref]).unwrap().to_rgba8();
        assert!(poster.get_pixel(4, 4)[0] > 200, "first frame is the poster");
        let v = &n.prototype.videos[0];
        assert_eq!((v.paint, v.autoplay, v.looping, v.muted), (0, true, false, true));
        assert_eq!(app.doc.images[&v.source], mp4);
        app.undo();
        assert_eq!(app.doc.nodes.len(), n0);
        // not a video: refused with a message, nothing inserted
        assert!(place_video_bytes(&mut app, v2(0.0, 0.0), "x", b"definitely not a video".to_vec()).is_none());
        assert_eq!(app.doc.nodes.len(), n0);
        // an unsupported codec still places, with the placeholder poster
        let mut h264 = mp4;
        let at = h264.windows(4).position(|w| w == b"jpeg").unwrap();
        h264[at..at + 4].copy_from_slice(b"avc1");
        let id = place_video_bytes(&mut app, v2(0.0, 0.0), "h264", h264).unwrap();
        assert_eq!(app.doc.get(id).unwrap().prototype.videos.len(), 1);
    }
}
