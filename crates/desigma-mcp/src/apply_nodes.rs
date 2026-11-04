//! `desigma_apply_nodes`: create or update a whole node tree from JSON in one transaction.
//!
//! The call is planned against a read-only view of the document first: every node spec is
//! parsed, validated and turned into a finished [`Node`], and every problem is collected with
//! the path of the node it belongs to. Only when the whole tree is clean are ids allocated and
//! one `Command::Batch` executed, so a failed call leaves document and history untouched and a
//! successful one is exactly one undo step. The result is checked with `validate_document`
//! and rolled back if it is broken.
use crate::engine::{bounded_text, colour, validate_document, Host};
use desigma_doc::{
    create_instance, record_override, swap_instance, Affine, ArcData, AutoLayout, BlendMode, Color, Command, Constraint, Document, Effect, FrameProps,
    GlassEffect, GradientStop, ImageScaleMode, LayoutAlign, LayoutDirection, LineHeight, Node, NodeId, NodeKind, OverrideField, Paint, PaintKind, Rect, Sizing,
    StrokeAlign, StrokeCap, StrokeJoin, TextAlignH, TextAlignV, TextAutoResize, TextCase, TextDecoration, TextStyle, Vec2,
};
use serde::{de::DeserializeOwned, Deserialize, Deserializer};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

const MAX_SPECS: usize = 5000;
const MAX_DEPTH: usize = 64;
const MAX_IMAGE_BYTES: usize = 16 * 1024 * 1024;
/// Plan-time ids for new nodes; swapped for real ids once the whole tree is valid.
const TEMP_BASE: u64 = 1 << 62;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Args {
    parent_id: Option<u64>,
    nodes: Vec<Value>,
    #[serde(default)]
    validate_only: bool,
}

/// `null` clears a field, absent leaves it alone.
fn nullable<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Value>, D::Error> {
    Ok(Some(Value::deserialize(d)?))
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct Spec {
    node_id: Option<u64>,
    parent_id: Option<u64>,
    index: Option<usize>,
    kind: Option<String>,
    name: Option<String>,
    x: Option<f32>,
    y: Option<f32>,
    width: Option<Value>,
    height: Option<Value>,
    rotation: Option<f32>,
    opacity: Option<f32>,
    visible: Option<bool>,
    locked: Option<bool>,
    blend: Option<String>,
    fill: Option<Value>,
    #[serde(default, deserialize_with = "nullable")]
    stroke: Option<Value>,
    effects: Option<Vec<Value>>,
    corner_radius: Option<Value>,
    corner_smoothing: Option<f32>,
    clips_content: Option<bool>,
    #[serde(default, deserialize_with = "nullable")]
    auto_layout: Option<Value>,
    layout_child: Option<LayoutChildSpec>,
    constraints: Option<ConstraintsSpec>,
    is_mask: Option<bool>,
    text: Option<String>,
    text_style: Option<TextStyleSpec>,
    text_resize: Option<String>,
    sides: Option<u32>,
    inner_ratio: Option<f32>,
    children: Option<Vec<Value>>,
    instance_of: Option<u64>,
    overrides: Option<BTreeMap<String, Value>>,
    swap: Option<u64>,
    bind: Option<BTreeMap<String, Option<String>>>,
    replace_children: Option<bool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PaintSpec {
    #[serde(rename = "type")]
    kind: String,
    color: Option<String>,
    stops: Option<Vec<StopSpec>>,
    angle: Option<f32>,
    handles: Option<[[f32; 2]; 3]>,
    image_ref: Option<String>,
    image_base64: Option<String>,
    scale_mode: Option<String>,
    opacity: Option<f32>,
    blend: Option<String>,
    visible: Option<bool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StopSpec {
    pos: f32,
    color: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StrokeSpec {
    color: Option<Value>,
    width: Option<f32>,
    align: Option<String>,
    cap: Option<String>,
    join: Option<String>,
    dashes: Option<Vec<f32>>,
    side_weights: Option<[f32; 4]>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EffectSpec {
    #[serde(rename = "type")]
    kind: String,
    color: Option<String>,
    x: Option<f32>,
    y: Option<f32>,
    blur: Option<f32>,
    spread: Option<f32>,
    show_behind: Option<bool>,
    visible: Option<bool>,
    light_angle: Option<f32>,
    light_intensity: Option<f32>,
    refraction: Option<f32>,
    depth: Option<f32>,
    dispersion: Option<f32>,
    frost: Option<f32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AutoLayoutSpec {
    direction: Option<String>,
    gap: Option<f32>,
    counter_gap: Option<f32>,
    wrap: Option<bool>,
    padding: Option<Value>,
    primary_align: Option<String>,
    counter_align: Option<String>,
    reverse_z: Option<bool>,
    stroke_included: Option<bool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LayoutChildSpec {
    absolute: Option<bool>,
    grow: Option<f32>,
    min_width: Option<f32>,
    max_width: Option<f32>,
    min_height: Option<f32>,
    max_height: Option<f32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConstraintsSpec {
    horizontal: Option<String>,
    vertical: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TextStyleSpec {
    font_family: Option<String>,
    font_weight: Option<u16>,
    italic: Option<bool>,
    font_size: Option<f32>,
    line_height: Option<Value>,
    letter_spacing: Option<f32>,
    paragraph_spacing: Option<f32>,
    align_h: Option<String>,
    align_v: Option<String>,
    case: Option<String>,
    decoration: Option<String>,
    color: Option<Value>,
    features: Option<BTreeMap<String, u32>>,
}

enum Dim {
    Px(f32),
    Hug,
    Fill,
}

struct NodeError {
    path: String,
    name: String,
    message: String,
}

struct Planner<'a> {
    doc: &'a Document,
    errors: Vec<NodeError>,
    updates: BTreeMap<NodeId, Node>,
    inserts: Vec<(NodeId, usize, Vec<Node>)>,
    images: BTreeMap<String, Vec<u8>>,
    next_temp: u64,
    specs: usize,
    report: Vec<Value>,
    /// Lazily cloned copy of the document that instances are built in.
    scratch: Option<Document>,
    /// Children dropped by `replace_children`.
    removals: Vec<NodeId>,
    /// Final child order per container updated with `replace_children`.
    reorders: Vec<(NodeId, Vec<NodeId>)>,
    /// Existing nodes that top-level specs point at (insert parent or update target), with the
    /// spec path, so a node removed by `replace_children` anywhere in the call is refused.
    top_refs: Vec<(String, NodeId)>,
}

/// Entry point for the `desigma_apply_nodes` tool.
pub fn apply_nodes<H: Host>(host: &mut H, args: Value) -> Result<Value, String> {
    let a: Args = serde_json::from_value(args).map_err(|e| format!("invalid arguments: {e}"))?;
    if a.nodes.is_empty() {
        return Err("nodes must contain at least one node spec".into());
    }
    let mut p = Planner {
        doc: host.doc(),
        errors: Vec::new(),
        updates: BTreeMap::new(),
        inserts: Vec::new(),
        images: BTreeMap::new(),
        next_temp: 0,
        specs: 0,
        report: Vec::new(),
        scratch: None,
        removals: Vec::new(),
        reorders: Vec::new(),
        top_refs: Vec::new(),
    };
    for (i, v) in a.nodes.iter().enumerate() {
        p.top_level(format!("nodes[{i}]"), v, a.parent_id);
    }
    p.check_removed_refs();
    if !p.errors.is_empty() {
        return Err(format_errors(&p.errors));
    }
    p.layout();
    let Planner { updates, inserts, images, report, next_temp, removals, reorders, .. } = p;
    let (created, updated) = (inserts.iter().map(|i| i.2.len()).sum::<usize>(), updates.len());
    if a.validate_only {
        return Ok(json!({"changed":false,"valid":true,"would_create":created,"would_update":updated}));
    }
    if created == 0 && removals.is_empty() && updates.iter().all(|(id, n)| host.doc().get(*id) == Some(n)) {
        return Ok(json!({"changed":false,"created":0,"updated":0,"nodes":[]}));
    }
    // Only now, with a clean plan, spend real ids.
    let mut real = Vec::with_capacity(next_temp as usize);
    for _ in 0..next_temp {
        real.push(host.alloc_id()?);
    }
    let map = |id: NodeId| if id.0 >= TEMP_BASE { real.get((id.0 - TEMP_BASE) as usize).copied().unwrap_or(id) } else { id };
    let mut cmds = Vec::new();
    for (id, mut node) in updates {
        // Children are changed by Insert, Remove and Reparent below, never by Update.
        if let Some(orig) = host.doc().get(id) {
            node.children = orig.children.clone();
        }
        cmds.push(Command::Update { node: Box::new(node) });
    }
    let removed = removals.len();
    for id in removals {
        cmds.push(Command::Remove { id });
    }
    for (parent, index, mut nodes) in inserts {
        for n in &mut nodes {
            n.id = map(n.id);
            n.parent = n.parent.map(map);
            n.children = n.children.iter().copied().map(map).collect();
        }
        cmds.push(Command::Insert { parent, index, nodes });
    }
    for (parent, order) in reorders {
        for (index, id) in order.into_iter().enumerate() {
            cmds.push(Command::Reparent { id: map(id), parent, index });
        }
    }
    host.exec(Command::Batch(cmds))?;
    if let Err(e) = validate_document(host.doc()) {
        // The plan was clean but the result is not: take it back out.
        host.undo()?;
        // Not a step anyone chose: redo must not bring it back.
        host.discard_redo();
        return Err(format!("the result failed document validation and was rolled back: {e}"));
    }
    // Bitmap bytes sit outside undo; add them only once the layers using them are in.
    for (k, v) in images {
        host.doc_mut().images.insert(k, v);
    }
    let report: Vec<Value> = report
        .into_iter()
        .map(|mut r| {
            if let Some(t) = r["node_id"].as_u64().filter(|t| *t >= TEMP_BASE) {
                r["node_id"] = json!(map(NodeId(t)));
            }
            r
        })
        .collect();
    Ok(json!({"changed":true,"created":created,"updated":updated,"removed":removed,"nodes":report,"undo_steps":1}))
}

fn format_errors(errors: &[NodeError]) -> String {
    let mut out = format!("desigma_apply_nodes failed; nothing was changed ({} problem{})", errors.len(), if errors.len() == 1 { "" } else { "s" });
    for e in errors.iter().take(50) {
        out.push_str(&format!("\n- {}{}: {}", e.path, if e.name.is_empty() { String::new() } else { format!(" \"{}\"", e.name) }, e.message));
    }
    if errors.len() > 50 {
        out.push_str(&format!("\n- ... and {} more", errors.len() - 50));
    }
    out
}

impl<'a> Planner<'a> {
    fn err(&mut self, path: &str, name: &str, message: impl Into<String>) {
        self.errors.push(NodeError { path: path.into(), name: name.into(), message: message.into() });
    }

    /// A spec that targets a node (or a parent) that `replace_children` deletes in this call
    /// would fail at apply time; say so per node instead.
    fn check_removed_refs(&mut self) {
        if self.removals.is_empty() {
            return;
        }
        for (path, id) in std::mem::take(&mut self.top_refs) {
            let mut cur = Some(id);
            while let Some(c) = cur {
                if self.removals.contains(&c) {
                    self.err(&path, "", format!("node {} is removed by replace_children in this call, so it cannot be used here", id.0));
                    break;
                }
                cur = self.doc.get(c).and_then(|n| n.parent);
            }
        }
    }

    fn top_level(&mut self, path: String, v: &Value, default_parent: Option<u64>) {
        let Some(spec) = self.parse_spec(&path, v) else { return };
        if let Some(id) = spec.node_id.or(spec.parent_id).or(default_parent) {
            self.top_refs.push((path.clone(), NodeId(id)));
        }
        match (spec.node_id, spec.parent_id.or(default_parent)) {
            (Some(_), _) if spec.parent_id.is_some() => self.err(&path, "", "node_id and parent_id cannot be combined; use desigma_move_node to reparent"),
            (Some(id), _) => self.update(&path, spec, NodeId(id), 0),
            (None, Some(parent)) => {
                let parent = NodeId(parent);
                if !self.doc.get(parent).is_some_and(|n| n.kind.is_container()) {
                    return self.err(&path, spec.name.as_deref().unwrap_or(""), format!("parent {} is missing or not a container", parent.0));
                }
                let index = spec.index.unwrap_or(usize::MAX);
                if let Some(nodes) = self.create(&path, &spec, 0) {
                    self.inserts.push((parent, index, nodes));
                }
            }
            (None, None) => self.err(&path, spec.name.as_deref().unwrap_or(""), "needs node_id (update) or a parent (parent_id on the node or on the call)"),
        }
    }

    fn parse_spec(&mut self, path: &str, v: &Value) -> Option<Spec> {
        self.specs += 1;
        if self.specs > MAX_SPECS {
            if self.specs == MAX_SPECS + 1 {
                self.err(path, "", format!("more than {MAX_SPECS} node specs in one call; split the tree"));
            }
            return None;
        }
        match serde_json::from_value::<Spec>(v.clone()) {
            Ok(s) => Some(s),
            Err(e) => {
                let name = v.get("name").and_then(Value::as_str).unwrap_or("");
                self.err(path, name, e.to_string());
                None
            }
        }
    }

    fn temp_id(&mut self) -> NodeId {
        self.next_temp += 1;
        NodeId(TEMP_BASE + self.next_temp - 1)
    }

    /// Builds a new subtree, root first. Returns `None` when anything in it was invalid.
    fn create(&mut self, path: &str, spec: &Spec, depth: usize) -> Option<Vec<Node>> {
        let name = spec.name.clone().unwrap_or_default();
        if let Some(component) = spec.instance_of {
            return self.create_instance(path, spec, NodeId(component));
        }
        if spec.overrides.is_some() || spec.swap.is_some() {
            self.err(path, &name, "overrides and swap only work together with instance_of");
            return None;
        }
        let Some(kind) = spec.kind.as_deref() else {
            self.err(path, &name, "kind is required for a new node (frame, group, rectangle, ellipse, text, line, polygon, star)");
            return None;
        };
        let id = self.temp_id();
        let mut node = match new_node(id, kind, spec) {
            Ok(n) => n,
            Err(e) => {
                self.err(path, &name, e);
                return None;
            }
        };
        if !self.props(path, &mut node, spec, true) {
            // Still walk the children so one call reports every problem.
            self.walk_bad_children(path, spec, depth);
            return None;
        }
        let report_at = self.report.len();
        self.report.push(json!({"path":path,"node_id":id,"name":node.name,"created":true}));
        let mut out = vec![node];
        let mut ok = true;
        if let Some(children) = &spec.children {
            if !out[0].kind.is_container() {
                self.err(path, &name, "children are only valid on frame, group and other container kinds");
                return None;
            }
            if depth >= MAX_DEPTH {
                self.err(path, &name, format!("nesting is deeper than {MAX_DEPTH} levels"));
                return None;
            }
            for (i, cv) in children.iter().enumerate() {
                let cpath = format!("{path}.children[{i}]");
                let Some(cs) = self.parse_spec(&cpath, cv) else {
                    ok = false;
                    continue;
                };
                if cs.node_id.is_some() || cs.parent_id.is_some() || cs.index.is_some() {
                    self.err(&cpath, cs.name.as_deref().unwrap_or(""), "node_id, parent_id and index are not allowed inside the children of a new node");
                    ok = false;
                    continue;
                }
                match self.create(&cpath, &cs, depth + 1) {
                    Some(mut sub) => {
                        sub[0].parent = Some(id);
                        out[0].children.push(sub[0].id);
                        out.extend(sub);
                    }
                    None => ok = false,
                }
            }
        }
        if !ok {
            self.report.truncate(report_at);
            return None;
        }
        Some(out)
    }

    /// Builds a new instance of `component` (root first, temp ids) and applies the overrides.
    fn create_instance(&mut self, path: &str, spec: &Spec, component: NodeId) -> Option<Vec<Node>> {
        let name = spec.name.clone().unwrap_or_default();
        if spec.kind.is_some() || spec.children.is_some() || spec.node_id.is_some() {
            self.err(path, &name, "instance_of cannot be combined with kind, children or node_id; use overrides for the inside");
            return None;
        }
        if !matches!(self.doc.get(component).map(|n| &n.kind), Some(NodeKind::Component(_))) {
            self.err(path, &name, format!("instance_of: node {} is not a main component", component.0));
            return None;
        }
        let mut scratch = self.scratch.take().unwrap_or_else(|| self.doc.clone());
        let nodes = self.build_instance(path, spec, component, &mut scratch);
        self.scratch = Some(scratch);
        nodes
    }

    fn build_instance(&mut self, path: &str, spec: &Spec, component: NodeId, scratch: &mut Document) -> Option<Vec<Node>> {
        let name = spec.name.clone().unwrap_or_default();
        let Some(container) = scratch.pages.first().copied() else {
            self.err(path, &name, "document has no page");
            return None;
        };
        let at = Vec2 { x: spec.x.unwrap_or(0.0), y: spec.y.unwrap_or(0.0) };
        let cmd = create_instance(scratch, component, container, at);
        if let Err(e) = cmd.apply(scratch) {
            self.err(path, &name, format!("cannot create the instance: {e}"));
            return None;
        }
        let root = scratch.get(container).and_then(|c| c.children.last().copied())?;
        let overrides: Vec<(String, Spec)> = {
            let mut out = Vec::new();
            for (key, v) in spec.overrides.iter().flatten() {
                let opath = format!("{path}.overrides.{key}");
                match serde_json::from_value::<Spec>(v.clone()) {
                    Ok(o)
                        if o.node_id.is_some()
                            || o.parent_id.is_some()
                            || o.children.is_some()
                            || o.instance_of.is_some()
                            || o.overrides.is_some()
                            || o.kind.is_some() =>
                    {
                        self.err(&opath, &name, "an override can only set properties (text, fill, visible, opacity, swap, ...)");
                    }
                    Ok(o) => out.push((key.clone(), o)),
                    Err(e) => self.err(&opath, &name, e.to_string()),
                }
            }
            out
        };
        // Swaps first: they rebuild the inside of the nested instance.
        for (key, o) in &overrides {
            let Some(to) = o.swap else { continue };
            let opath = format!("{path}.overrides.{key}");
            let Some(target) = crate::components::descendant_named(scratch, root, key) else {
                self.err(&opath, &name, crate::components::missing_layer(scratch, root, key));
                continue;
            };
            if !matches!(scratch.get(target).map(|n| &n.kind), Some(NodeKind::Instance { .. })) {
                self.err(&opath, &name, "swap only works on a nested instance");
            } else if !matches!(scratch.get(NodeId(to)).map(|n| &n.kind), Some(NodeKind::Component(_))) {
                self.err(&opath, &name, format!("swap: node {to} is not a main component"));
            } else if let Err(e) = swap_instance(scratch, target, NodeId(to)).apply(scratch) {
                self.err(&opath, &name, format!("swap failed: {e}"));
            }
        }
        let mut ids = Vec::new();
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            ids.push(id);
            if let Some(n) = scratch.get(id) {
                stack.extend(n.children.iter().rev());
            }
        }
        let map: BTreeMap<NodeId, NodeId> = ids.iter().map(|i| (*i, self.temp_id())).collect();
        let mut nodes: Vec<Node> = ids
            .iter()
            .filter_map(|i| scratch.get(*i))
            .map(|n| {
                let mut n = n.clone();
                n.id = map[&n.id];
                n.parent = n.parent.and_then(|p| map.get(&p).copied());
                n.children = n.children.iter().filter_map(|c| map.get(c).copied()).collect();
                n
            })
            .collect();
        nodes[0].parent = None;
        let before = nodes.len();
        let mut root_node = nodes[0].clone();
        let mut ok = self.props(path, &mut root_node, spec, false);
        nodes[0] = root_node;
        for (key, o) in &overrides {
            let opath = format!("{path}.overrides.{key}");
            let found = nodes.iter().skip(1).position(|n| n.name == *key).map(|i| i + 1);
            let Some(i) = found else {
                if o.swap.is_none() {
                    self.err(&opath, &name, crate::components::missing_layer_in(&nodes, key));
                    ok = false;
                }
                continue;
            };
            if o.swap.is_some() && o.text.is_none() && o.fill.is_none() {
                continue;
            }
            let orig = nodes[i].clone();
            let mut node = orig.clone();
            ok &= self.props(&opath, &mut node, o, false);
            let fields = record_override(&orig, &node);
            node.component.overrides.extend(fields.into_iter().filter(|f| *f != OverrideField::Transform));
            nodes[i] = node;
        }
        debug_assert!(nodes.len() == before);
        if !ok {
            return None;
        }
        self.report.push(json!({"path":path,"node_id":nodes[0].id,"name":nodes[0].name,"created":true,"instance_of":component}));
        Some(nodes)
    }

    fn walk_bad_children(&mut self, path: &str, spec: &Spec, depth: usize) {
        for (i, cv) in spec.children.iter().flatten().enumerate() {
            let cpath = format!("{path}.children[{i}]");
            if let Some(cs) = self.parse_spec(&cpath, cv) {
                if depth < MAX_DEPTH {
                    let _ = self.create(&cpath, &cs, depth + 1);
                }
            }
        }
    }

    fn update(&mut self, path: &str, spec: Spec, id: NodeId, depth: usize) {
        let Some(orig) = self.doc.get(id) else {
            return self.err(path, spec.name.as_deref().unwrap_or(""), format!("node {} not found", id.0));
        };
        let name = orig.name.clone();
        if spec.instance_of.is_some() || spec.overrides.is_some() || spec.swap.is_some() {
            return self.err(path, &name, "instance_of, overrides and swap create new instances; to change an existing one use desigma_override_instance");
        }
        if orig.parent.is_none() {
            return self.err(path, &name, "page roots cannot be updated by this tool");
        }
        if self.updates.contains_key(&id) {
            return self.err(path, &name, format!("node {} appears twice in one call", id.0));
        }
        if let Some(k) = &spec.kind {
            if !kind_matches(&orig.kind, k) {
                return self.err(path, &name, format!("kind cannot be changed (node is {}, spec says {k})", orig.kind.default_name()));
            }
        }
        let mut node = orig.clone();
        let ok = self.props(path, &mut node, &spec, false);
        let before = self.errors.len();
        let has_children = spec.children.is_some();
        if has_children && depth >= MAX_DEPTH {
            return self.err(path, &name, format!("nesting is deeper than {MAX_DEPTH} levels"));
        }
        if has_children && !node.kind.is_container() {
            self.err(path, &name, "children are only valid on frame, group and other container kinds");
        }
        let existing: BTreeSet<NodeId> = node.children.iter().copied().collect();
        let replace = spec.replace_children == Some(true);
        if replace && !has_children {
            self.err(path, &name, "replace_children needs a children list (use [] to remove all)");
        }
        let mut order: Vec<NodeId> = Vec::new();
        for (i, cv) in spec.children.iter().flatten().enumerate() {
            let cpath = format!("{path}.children[{i}]");
            let Some(cs) = self.parse_spec(&cpath, cv) else { continue };
            if cs.parent_id.is_some() || cs.index.is_some() {
                self.err(&cpath, cs.name.as_deref().unwrap_or(""), "parent_id and index are not allowed inside children");
                continue;
            }
            match cs.node_id {
                Some(cid) if !existing.contains(&NodeId(cid)) => {
                    self.err(&cpath, cs.name.as_deref().unwrap_or(""), format!("node {cid} is not a child of {}; use desigma_move_node first", id.0))
                }
                Some(cid) => {
                    order.push(NodeId(cid));
                    self.update(&cpath, cs, NodeId(cid), depth + 1)
                }
                None => {
                    if let Some(nodes) = self.create(&cpath, &cs, depth + 1) {
                        order.push(nodes[0].id);
                        self.inserts.push((id, usize::MAX, nodes));
                    }
                }
            }
        }
        if replace && self.errors.len() == before {
            let keep: BTreeSet<NodeId> = order.iter().copied().collect();
            if keep.len() != order.len() {
                self.err(path, &name, "replace_children lists a child twice");
            } else {
                self.removals.extend(node.children.iter().copied().filter(|c| !keep.contains(c)));
                node.children = order.clone();
                self.reorders.push((id, order));
            }
        }
        if ok && orig.component.source.is_some() {
            // An edit inside an instance is an override, or the next sync would undo it.
            let fields = record_override(orig, &node);
            node.component.overrides.extend(fields.into_iter().filter(|f| *f != OverrideField::Transform));
        }
        if ok && self.errors.len() == before && self.doc.get(id) != Some(&node) {
            self.report.push(json!({"path":path,"node_id":id,"name":node.name,"created":false}));
            self.updates.insert(id, node);
        }
    }

    /// Applies every property of `spec` to `node`. Returns false when any was invalid.
    fn props(&mut self, path: &str, node: &mut Node, s: &Spec, is_new: bool) -> bool {
        let mut errs: Vec<String> = Vec::new();
        let doc_images = &self.doc.images;
        let images = &mut self.images;
        props(node, s, is_new, doc_images, images, &mut errs);
        for (field, variable) in s.bind.iter().flatten() {
            if let Err(e) = crate::variables::resolve_binding(self.doc, node, field, variable.as_deref()) {
                errs.push(e);
            }
        }
        let name = node.name.clone();
        let ok = errs.is_empty();
        for e in errs {
            self.err(path, &name, e);
        }
        ok
    }

    /// Runs the auto-layout solver over every container the plan touched, in a scratch
    /// document, and folds the resulting positions and sizes back into the plan.
    fn layout(&mut self) {
        let doc = self.doc;
        let is_layout = |n: &Node| n.kind.frame_props().is_some_and(|f| f.auto_layout.is_some()) || matches!(n.kind, NodeKind::Group);
        // Existing containers whose children changed or that changed themselves.
        let mut existing_roots: BTreeSet<NodeId> = BTreeSet::new();
        for (parent, _, _) in &self.inserts {
            if doc.get(*parent).is_some_and(is_layout) {
                existing_roots.insert(*parent);
            }
        }
        for (id, n) in &self.updates {
            if is_layout(n) {
                existing_roots.insert(*id);
            }
            if let Some(p) = n.parent.filter(|p| doc.get(*p).is_some_and(is_layout)) {
                existing_roots.insert(p);
            }
        }
        // A container that grows or shrinks moves its siblings and resizes hugging ancestors,
        // so every auto-layout ancestor above a touched container is solved too.
        for seed in existing_roots.clone() {
            let mut cur = doc.get(seed).and_then(|n| n.parent);
            while let Some(p) = cur {
                let Some(pn) = doc.get(p).filter(|n| is_layout(n)) else { break };
                existing_roots.insert(p);
                cur = pn.parent;
            }
        }
        // Drop roots that sit inside another root: laying out the outer one covers them.
        let roots: Vec<NodeId> = existing_roots.iter().copied().filter(|r| !is_inside_any(doc, *r, &existing_roots)).collect();
        for root in roots {
            let mut nodes = Vec::new();
            self.collect(root, &mut nodes);
            self.layout_region(root, nodes);
        }
        // New subtrees under a parent that does not lay them out are their own region.
        for i in 0..self.inserts.len() {
            let (parent, _, nodes) = &self.inserts[i];
            if doc.get(*parent).is_some_and(is_layout) || is_inside_any(doc, *parent, &existing_roots) {
                continue;
            }
            let root = nodes[0].id;
            let nodes = nodes.clone();
            self.layout_region(root, nodes);
        }
    }

    /// Existing subtree of `id` with the plan applied: updated nodes substituted, inserts added.
    fn collect(&self, id: NodeId, out: &mut Vec<Node>) {
        let Some(orig) = self.doc.get(id) else { return };
        let mut node = self.updates.get(&id).unwrap_or(orig).clone();
        for (parent, _, nodes) in &self.inserts {
            if *parent == id && !node.children.contains(&nodes[0].id) {
                node.children.push(nodes[0].id);
            }
        }
        let children = node.children.clone();
        out.push(node);
        for c in children {
            if self.doc.get(c).is_some() {
                self.collect(c, out);
            } else if let Some((_, _, nodes)) = self.inserts.iter().find(|i| i.2.first().is_some_and(|n| n.id == c)) {
                out.extend(nodes.iter().cloned());
            }
        }
    }

    fn layout_region(&mut self, root: NodeId, nodes: Vec<Node>) {
        use desigma_engine::{edit::Edit, layout};
        let mut scratch = Document::new("scratch");
        scratch.variables = self.doc.variables.clone();
        scratch.variable_collections = self.doc.variable_collections.clone();
        let mut ids = Vec::with_capacity(nodes.len());
        for n in nodes {
            ids.push(n.id);
            scratch.nodes.insert(n.id, n);
        }
        let mut ed = Edit::new();
        layout::layout_node(&mut scratch, &mut ed, root);
        // Text that wraps to a width the solver just decided needs its height refitted.
        let mut refit = false;
        for id in &ids {
            if let Some(n) = scratch.nodes.get_mut(id) {
                if let NodeKind::Text(t) = &n.kind {
                    if t.auto_resize == TextAutoResize::Height {
                        let fitted = layout::fit_text(t, n.size);
                        if (fitted.y - n.size.y).abs() > 1e-3 {
                            n.size = fitted;
                            refit = true;
                        }
                    }
                }
            }
        }
        if refit {
            layout::layout_node(&mut scratch, &mut Edit::new(), root);
        }
        for id in ids {
            let Some(done) = scratch.nodes.get(&id) else { continue };
            let (tx, ty, size) = (done.transform.tx, done.transform.ty, done.size);
            let apply = |n: &mut Node| {
                n.transform.tx = tx;
                n.transform.ty = ty;
                n.size = size;
            };
            if let Some(n) = self.inserts.iter_mut().flat_map(|i| i.2.iter_mut()).find(|n| n.id == id) {
                apply(n);
            } else if let Some(n) = self.updates.get_mut(&id) {
                apply(n);
            } else if let Some(orig) = self.doc.get(id) {
                if orig.transform.tx != tx || orig.transform.ty != ty || orig.size != size {
                    let mut n = orig.clone();
                    apply(&mut n);
                    self.updates.insert(id, n);
                }
            }
        }
    }
}

fn is_inside_any(doc: &Document, id: NodeId, roots: &BTreeSet<NodeId>) -> bool {
    let mut cur = doc.get(id).and_then(|n| n.parent);
    while let Some(p) = cur {
        if roots.contains(&p) {
            return true;
        }
        cur = doc.get(p).and_then(|n| n.parent);
    }
    false
}

fn kind_matches(kind: &NodeKind, want: &str) -> bool {
    matches!(
        (kind, want),
        (NodeKind::Frame(_) | NodeKind::Component(_) | NodeKind::Instance { .. }, "frame")
            | (NodeKind::Group, "group")
            | (NodeKind::Rectangle, "rectangle")
            | (NodeKind::Ellipse(_), "ellipse")
            | (NodeKind::Text(_), "text")
            | (NodeKind::Line, "line")
            | (NodeKind::Polygon { .. }, "polygon")
            | (NodeKind::Star { .. }, "star")
    )
}

fn new_node(id: NodeId, kind: &str, s: &Spec) -> Result<Node, String> {
    let grey = || vec![Paint::solid(Color::rgb(0xD9, 0xD9, 0xD9))];
    let mut n = match kind {
        "frame" => {
            let mut n = Node::new(id, NodeKind::Frame(FrameProps::default()));
            n.fills = vec![Paint::solid(Color::WHITE)];
            n
        }
        "group" => Node::new(id, NodeKind::Group),
        "rectangle" => {
            let mut n = Node::new(id, NodeKind::Rectangle);
            n.fills = grey();
            n
        }
        "ellipse" => {
            let mut n = Node::new(id, NodeKind::Ellipse(ArcData::default()));
            n.fills = grey();
            n
        }
        "text" => Node::text(id, Rect::new(0.0, 0.0, 0.0, 0.0), "", TextStyle::default()),
        "line" => {
            let mut n = Node::new(id, NodeKind::Line);
            n.stroke.paints = vec![Paint::solid(Color::BLACK)];
            n
        }
        "polygon" => {
            let mut n = Node::new(id, NodeKind::Polygon { count: s.sides.unwrap_or(3).clamp(3, 64) });
            n.fills = grey();
            n
        }
        "star" => {
            let mut n =
                Node::new(id, NodeKind::Star { count: s.sides.unwrap_or(5).clamp(3, 64), inner_ratio: s.inner_ratio.unwrap_or(0.38).clamp(0.01, 0.99) });
            n.fills = grey();
            n
        }
        other => return Err(format!("unknown kind \"{other}\" (use frame, group, rectangle, ellipse, text, line, polygon, star)")),
    };
    if kind != "text" {
        n.size = Vec2::new(100.0, if kind == "line" { 0.0 } else { 100.0 });
    }
    if kind != "polygon" && kind != "star" && (s.sides.is_some() || s.inner_ratio.is_some()) {
        return Err("sides and inner_ratio are only valid on polygon and star".into());
    }
    Ok(n)
}

// ---------- properties ----------

fn check(errs: &mut Vec<String>, field: &str, r: Result<(), String>) {
    if let Err(e) = r {
        errs.push(format!("{field}: {e}"));
    }
}

fn finite(v: f32, lo: f32, hi: f32) -> Result<f32, String> {
    if v.is_finite() && (lo..=hi).contains(&v) {
        Ok(v)
    } else {
        Err(format!("{v} is outside {lo}..={hi}"))
    }
}

/// Parses a snake_case, kebab-case or spaced name into a unit-variant enum.
fn variant<T: DeserializeOwned>(s: &str, options: &str) -> Result<T, String> {
    let pascal: String = s
        .split(['_', '-', ' '])
        .filter(|p| !p.is_empty())
        .map(|p| {
            let mut c = p.chars();
            c.next().map(|f| f.to_uppercase().collect::<String>() + &c.as_str().to_lowercase()).unwrap_or_default()
        })
        .collect();
    serde_json::from_value(Value::String(pascal)).map_err(|_| format!("\"{s}\" is not valid; use one of: {options}"))
}

fn blend(s: &str) -> Result<BlendMode, String> {
    variant(s, "normal, pass_through, darken, multiply, color_burn, linear_burn, lighten, screen, color_dodge, linear_dodge, overlay, soft_light, hard_light, difference, exclusion, hue, saturation, color, luminosity")
}

fn align(s: &str) -> Result<LayoutAlign, String> {
    variant(s, "start, center, end, space_between, baseline")
}

fn constraint(s: &str) -> Result<Constraint, String> {
    match s.to_ascii_lowercase().as_str() {
        "left" | "top" | "min" => Ok(Constraint::Min),
        "right" | "bottom" | "max" => Ok(Constraint::Max),
        "center" => Ok(Constraint::Center),
        "stretch" | "left_right" | "top_bottom" => Ok(Constraint::Stretch),
        "scale" => Ok(Constraint::Scale),
        _ => Err(format!("\"{s}\" is not valid; use min (left/top), max (right/bottom), center, stretch, scale")),
    }
}

fn dim(v: &Value) -> Result<Dim, String> {
    match v {
        Value::Number(n) => Ok(Dim::Px(finite(n.as_f64().unwrap_or(f64::NAN) as f32, 0.0, 1_000_000.0)?)),
        Value::String(s) if s == "hug" => Ok(Dim::Hug),
        Value::String(s) if s == "fill" => Ok(Dim::Fill),
        _ => Err("use a number of pixels, \"hug\" or \"fill\"".into()),
    }
}

fn props(node: &mut Node, s: &Spec, is_new: bool, doc_images: &BTreeMap<String, Vec<u8>>, images: &mut BTreeMap<String, Vec<u8>>, errs: &mut Vec<String>) {
    if let Some(name) = &s.name {
        check(errs, "name", bounded_text("name", name, 1024));
        node.name = name.clone();
    }
    if let Some(v) = s.opacity {
        check(errs, "opacity", finite(v, 0.0, 1.0).map(|v| node.opacity = v));
    }
    if let Some(v) = s.visible {
        node.visible = v;
    }
    if let Some(v) = s.locked {
        node.locked = v;
    }
    if let Some(v) = s.is_mask {
        node.is_mask = v;
    }
    if let Some(b) = &s.blend {
        check(errs, "blend", blend(b).map(|b| node.blend = b));
    }
    // geometry
    for (field, v) in [("x", s.x), ("y", s.y)] {
        if let Some(v) = v {
            check(errs, field, finite(v, -1_000_000.0, 1_000_000.0).map(|v| if field == "x" { node.transform.tx = v } else { node.transform.ty = v }));
        }
    }
    if let Some(deg) = s.rotation {
        check(
            errs,
            "rotation",
            finite(deg, -36_000.0, 36_000.0).map(|deg| {
                let r = Affine::rotate(deg.to_radians());
                node.transform = Affine { tx: node.transform.tx, ty: node.transform.ty, ..r };
            }),
        );
    }
    if let Some(r) = &s.corner_radius {
        let radii = match r {
            Value::Number(n) => finite(n.as_f64().unwrap_or(f64::NAN) as f32, 0.0, 1_000_000.0).map(|v| [v; 4]),
            Value::Array(a) if a.len() == 4 => {
                let mut out = [0.0; 4];
                a.iter().zip(out.iter_mut()).try_for_each(|(v, o)| finite(v.as_f64().unwrap_or(f64::NAN) as f32, 0.0, 1_000_000.0).map(|v| *o = v)).map(|_| out)
            }
            _ => Err("use one number or [top_left, top_right, bottom_right, bottom_left]".into()),
        };
        check(errs, "corner_radius", radii.map(|r| node.corner_radii = r));
    }
    if let Some(v) = s.corner_smoothing {
        check(errs, "corner_smoothing", finite(v, 0.0, 1.0).map(|v| node.corner_smoothing = v));
    }
    // paint
    if let Some(f) = &s.fill {
        match paints(f, doc_images, images) {
            Ok(p) => match &mut node.kind {
                NodeKind::Text(t) => t.style.fills = p,
                _ => node.fills = p,
            },
            Err(e) => errs.push(format!("fill: {e}")),
        }
    }
    if let Some(st) = &s.stroke {
        check(errs, "stroke", stroke(node, st, doc_images, images));
    }
    if let Some(list) = &s.effects {
        let mut out = Vec::new();
        for (i, e) in list.iter().enumerate() {
            check(errs, &format!("effects[{i}]"), effect(e).map(|e| out.push(e)));
        }
        node.effects = out;
    }
    // frame behaviour
    if s.clips_content.is_some() || s.auto_layout.is_some() {
        match node.kind.frame_props_mut() {
            None => errs.push("clips_content and auto_layout need a frame".into()),
            Some(fp) => {
                if let Some(c) = s.clips_content {
                    fp.clips_content = c;
                }
                match &s.auto_layout {
                    Some(Value::Null) => fp.auto_layout = None,
                    Some(v) => {
                        let mut al = fp.auto_layout.unwrap_or_default();
                        if let Err(e) = auto_layout(&mut al, v) {
                            errs.push(format!("auto_layout: {e}"));
                        }
                        fp.auto_layout = Some(al);
                    }
                    None => {}
                }
            }
        }
    }
    if let Some(lc) = &s.layout_child {
        let l = &mut node.layout_child;
        if let Some(v) = lc.absolute {
            l.absolute = v;
        }
        if let Some(v) = lc.grow {
            check(errs, "layout_child.grow", finite(v, 0.0, 1000.0).map(|v| l.grow = v));
        }
        for (field, v, slot) in [
            ("min_width", lc.min_width, &mut l.min_w),
            ("max_width", lc.max_width, &mut l.max_w),
            ("min_height", lc.min_height, &mut l.min_h),
            ("max_height", lc.max_height, &mut l.max_h),
        ] {
            if let Some(v) = v {
                check(errs, &format!("layout_child.{field}"), finite(v, 0.0, 1_000_000.0).map(|v| *slot = Some(v)));
            }
        }
    }
    if let Some(c) = &s.constraints {
        if let Some(h) = &c.horizontal {
            check(errs, "constraints.horizontal", constraint(h).map(|v| node.constraints.horizontal = v));
        }
        if let Some(v) = &c.vertical {
            check(errs, "constraints.vertical", constraint(v).map(|v| node.constraints.vertical = v));
        }
    }
    // size, after auto layout so hug knows its axis
    dims(node, s, errs);
    text(node, s, is_new, doc_images, images, errs);
}

fn dims(node: &mut Node, s: &Spec, errs: &mut Vec<String>) {
    for (field, v, horizontal) in [("width", &s.width, true), ("height", &s.height, false)] {
        let Some(v) = v else { continue };
        let d = match dim(v) {
            Ok(d) => d,
            Err(e) => {
                errs.push(format!("{field}: {e}"));
                continue;
            }
        };
        let own = node.kind.frame_props().and_then(|f| f.auto_layout).map(|al| {
            // Is this axis the layout's primary one?
            (al.direction == LayoutDirection::Horizontal) == horizontal
        });
        let set_own = |node: &mut Node, to: Sizing| {
            if let (Some(primary), Some(fp)) = (own, node.kind.frame_props_mut()) {
                if let Some(al) = &mut fp.auto_layout {
                    if primary {
                        al.primary_sizing = to;
                    } else {
                        al.counter_sizing = to;
                    }
                }
            }
        };
        let child = |node: &mut Node, to: Sizing| {
            if horizontal {
                node.layout_child.horizontal = to;
            } else {
                node.layout_child.vertical = to;
            }
        };
        match d {
            Dim::Px(px) => {
                if horizontal {
                    node.size.x = px;
                } else {
                    node.size.y = px;
                }
                child(node, Sizing::Fixed);
                set_own(node, Sizing::Fixed);
            }
            Dim::Fill => child(node, Sizing::Fill),
            Dim::Hug => {
                if matches!(node.kind, NodeKind::Text(_)) {
                    // text hugs by auto-resize, set in `text`
                } else if own.is_some() {
                    set_own(node, Sizing::Hug);
                } else {
                    errs.push(format!("{field}: \"hug\" needs an auto_layout frame or a text node"));
                }
            }
        }
    }
}

fn text(node: &mut Node, s: &Spec, is_new: bool, doc_images: &BTreeMap<String, Vec<u8>>, images: &mut BTreeMap<String, Vec<u8>>, errs: &mut Vec<String>) {
    let NodeKind::Text(t) = &mut node.kind else {
        if s.text.is_some() || s.text_style.is_some() || s.text_resize.is_some() {
            errs.push("text, text_style and text_resize are only valid on text nodes".into());
        }
        return;
    };
    if let Some(c) = &s.text {
        check(errs, "text", bounded_text("text", c, 100_000));
        if t.content != *c {
            t.content = c.clone();
            t.runs.clear();
            t.lists.clear();
        }
    }
    if let Some(ts) = &s.text_style {
        check(errs, "text_style", text_style(&mut t.style, ts, doc_images, images));
    }
    let width_px = matches!(&s.width, Some(Value::Number(_)));
    let width_fill = matches!(&s.width, Some(Value::String(w)) if w == "fill");
    let width_hug = matches!(&s.width, Some(Value::String(w)) if w == "hug");
    if let Some(r) = &s.text_resize {
        let parsed = match r.as_str() {
            "auto_width" => Ok(TextAutoResize::WidthAndHeight),
            "auto_height" => Ok(TextAutoResize::Height),
            "fixed" => Ok(TextAutoResize::None),
            "truncate" => Ok(TextAutoResize::Truncate),
            _ => Err(format!("\"{r}\" is not valid; use auto_width, auto_height, fixed, truncate")),
        };
        check(errs, "text_resize", parsed.map(|m| t.auto_resize = m));
    } else if width_px || width_fill {
        t.auto_resize = TextAutoResize::Height;
    } else if width_hug || (is_new && s.width.is_none()) {
        t.auto_resize = TextAutoResize::WidthAndHeight;
    }
    // Keep the box in step with the content, as the editor does after a text edit.
    let touched = is_new || s.text.is_some() || s.text_style.is_some() || s.text_resize.is_some() || s.width.is_some();
    if touched {
        node.size = desigma_engine::layout::fit_text(t, if is_new && !width_px { Vec2::new(0.0, 0.0) } else { node.size });
    }
    if let Some(h) = &s.height {
        if let (Ok(Dim::Px(px)), TextAutoResize::None | TextAutoResize::Truncate) = (dim(h), t.auto_resize) {
            node.size.y = px;
        }
    }
}

fn text_style(st: &mut TextStyle, s: &TextStyleSpec, doc_images: &BTreeMap<String, Vec<u8>>, images: &mut BTreeMap<String, Vec<u8>>) -> Result<(), String> {
    if let Some(f) = &s.font_family {
        bounded_text("font_family", f, 256)?;
        st.font_family = f.clone();
    }
    if let Some(w) = s.font_weight {
        if !(100..=900).contains(&w) {
            return Err("font_weight must be 100..=900".into());
        }
        st.font_weight = w;
    }
    if let Some(i) = s.italic {
        st.italic = i;
    }
    if let Some(v) = s.font_size {
        st.font_size = finite(v, 1.0, 1000.0).map_err(|e| format!("font_size {e}"))?;
    }
    if let Some(v) = &s.line_height {
        st.line_height = match v {
            Value::String(a) if a == "auto" => LineHeight::Auto,
            Value::String(p) if p.ends_with('%') => {
                LineHeight::Percent(p.trim_end_matches('%').trim().parse::<f32>().map_err(|_| "line_height percent is not a number".to_string())?)
            }
            Value::Number(n) => LineHeight::Pixels(finite(n.as_f64().unwrap_or(f64::NAN) as f32, 0.0, 1000.0).map_err(|e| format!("line_height {e}"))?),
            _ => return Err("line_height: use pixels, \"auto\" or a percentage like \"120%\"".into()),
        };
    }
    if let Some(v) = s.letter_spacing {
        st.letter_spacing = finite(v, -100.0, 100.0).map_err(|e| format!("letter_spacing {e}"))?;
    }
    if let Some(v) = s.paragraph_spacing {
        st.paragraph_spacing = finite(v, 0.0, 1000.0).map_err(|e| format!("paragraph_spacing {e}"))?;
    }
    if let Some(a) = &s.align_h {
        st.align_h = variant::<TextAlignH>(a, "left, center, right, justified").map_err(|e| format!("align_h {e}"))?;
    }
    if let Some(a) = &s.align_v {
        st.align_v = variant::<TextAlignV>(a, "top, center, bottom").map_err(|e| format!("align_v {e}"))?;
    }
    if let Some(c) = &s.case {
        st.case = variant::<TextCase>(c, "original, upper, lower, title, small_caps").map_err(|e| format!("case {e}"))?;
    }
    if let Some(d) = &s.decoration {
        st.decoration = variant::<TextDecoration>(d, "none, underline, strikethrough").map_err(|e| format!("decoration {e}"))?;
    }
    if let Some(c) = &s.color {
        st.fills = paints(c, doc_images, images).map_err(|e| format!("color: {e}"))?;
    }
    if let Some(f) = &s.features {
        st.features = f.iter().map(|(k, v)| (k.clone(), *v)).collect();
    }
    Ok(())
}

fn auto_layout(al: &mut AutoLayout, v: &Value) -> Result<(), String> {
    let s: AutoLayoutSpec = serde_json::from_value(v.clone()).map_err(|e| e.to_string())?;
    if let Some(d) = &s.direction {
        al.direction = variant::<LayoutDirection>(d, "horizontal, vertical").map_err(|e| format!("direction {e}"))?;
    }
    if let Some(g) = s.gap {
        al.item_spacing = finite(g, -10_000.0, 1_000_000.0).map_err(|e| format!("gap {e}"))?;
    }
    if let Some(g) = s.counter_gap {
        al.counter_spacing = finite(g, -10_000.0, 1_000_000.0).map_err(|e| format!("counter_gap {e}"))?;
    }
    if let Some(w) = s.wrap {
        al.wrap = w;
    }
    if let Some(p) = &s.padding {
        al.padding = padding(p)?;
    }
    if let Some(a) = &s.primary_align {
        al.primary_align = align(a).map_err(|e| format!("primary_align {e}"))?;
    }
    if let Some(a) = &s.counter_align {
        al.counter_align = align(a).map_err(|e| format!("counter_align {e}"))?;
    }
    if let Some(v) = s.reverse_z {
        al.reverse_z = v;
    }
    if let Some(v) = s.stroke_included {
        al.stroke_included = v;
    }
    Ok(())
}

/// CSS-like padding: one number, [v, h], [top, right, bottom, left] or {top,right,bottom,left}.
fn padding(v: &Value) -> Result<[f32; 4], String> {
    let n = |v: &Value| -> Result<f32, String> { finite(v.as_f64().unwrap_or(f64::NAN) as f32, 0.0, 1_000_000.0).map_err(|e| format!("padding {e}")) };
    match v {
        Value::Number(_) => Ok([n(v)?; 4]),
        Value::Array(a) if a.len() == 2 => Ok([n(&a[0])?, n(&a[1])?, n(&a[0])?, n(&a[1])?]),
        Value::Array(a) if a.len() == 4 => Ok([n(&a[0])?, n(&a[1])?, n(&a[2])?, n(&a[3])?]),
        Value::Object(o) => {
            let get = |k: &str| o.get(k).map_or(Ok(0.0), n);
            if let Some(bad) = o.keys().find(|k| !["top", "right", "bottom", "left"].contains(&k.as_str())) {
                return Err(format!("padding has unknown side \"{bad}\""));
            }
            Ok([get("top")?, get("right")?, get("bottom")?, get("left")?])
        }
        _ => Err("padding: use a number, [vertical, horizontal], [top, right, bottom, left] or {top,right,bottom,left}".into()),
    }
}

fn paints(v: &Value, doc_images: &BTreeMap<String, Vec<u8>>, images: &mut BTreeMap<String, Vec<u8>>) -> Result<Vec<Paint>, String> {
    match v {
        Value::Null => Ok(Vec::new()),
        Value::String(s) if s.trim().eq_ignore_ascii_case("none") => Ok(Vec::new()),
        Value::Array(a) => a.iter().enumerate().map(|(i, p)| paint(p, doc_images, images).map_err(|e| format!("[{i}] {e}"))).collect(),
        other => Ok(vec![paint(other, doc_images, images)?]),
    }
}

fn paint(v: &Value, doc_images: &BTreeMap<String, Vec<u8>>, images: &mut BTreeMap<String, Vec<u8>>) -> Result<Paint, String> {
    if let Value::String(s) = v {
        return Ok(Paint::solid(colour(s, "fill")?));
    }
    let p: PaintSpec = serde_json::from_value(v.clone()).map_err(|e| e.to_string())?;
    let t = p.kind.to_ascii_lowercase().replace("_gradient", "");
    let stop_list = |p: &PaintSpec| -> Result<Vec<GradientStop>, String> {
        let stops = p.stops.as_ref().ok_or("gradients need stops: [{pos, color}, ...]")?;
        if stops.len() < 2 || stops.len() > 64 {
            return Err("gradients need 2 to 64 stops".into());
        }
        stops
            .iter()
            .map(|s| Ok(GradientStop { pos: finite(s.pos, 0.0, 1.0).map_err(|e| format!("stop pos {e}"))?, color: colour(&s.color, "stop")? }))
            .collect()
    };
    let kind = match t.as_str() {
        "solid" => PaintKind::Solid(colour(p.color.as_deref().ok_or("solid paints need color")?, "color")?),
        "linear" | "radial" | "angular" | "diamond" => {
            let stops = stop_list(&p)?;
            let angle = finite(p.angle.unwrap_or(if t == "linear" { 180.0 } else { 0.0 }), -36_000.0, 36_000.0)?;
            let handles = match p.handles {
                Some(h) => h.map(|[x, y]| Vec2::new(x, y)),
                None => gradient_handles(&t, angle),
            };
            match t.as_str() {
                "linear" => PaintKind::LinearGradient { handles, stops },
                "radial" => PaintKind::RadialGradient { handles, stops },
                "angular" => PaintKind::AngularGradient { handles, stops },
                _ => PaintKind::DiamondGradient { handles, stops },
            }
        }
        "image" => {
            let key = match (&p.image_ref, &p.image_base64) {
                (Some(r), None) if doc_images.contains_key(r) || images.contains_key(r) => r.clone(),
                (Some(r), None) => return Err(format!("image_ref \"{r}\" is not in the document; pass image_base64 instead")),
                (None, Some(b64)) => store_image(b64, images)?,
                _ => return Err("image paints need exactly one of image_ref or image_base64".into()),
            };
            let scale_mode = match p.scale_mode.as_deref() {
                None => ImageScaleMode::Fill,
                Some(m) => variant::<ImageScaleMode>(m, "fill, fit, crop, tile")?,
            };
            PaintKind::Image { image_ref: key, scale_mode }
        }
        other => return Err(format!("unknown paint type \"{other}\" (use solid, linear, radial, angular, diamond, image)")),
    };
    if !matches!(t.as_str(), "solid") && p.color.is_some() {
        return Err("color is only valid on solid paints".into());
    }
    Ok(Paint {
        kind,
        opacity: finite(p.opacity.unwrap_or(1.0), 0.0, 1.0).map_err(|e| format!("opacity {e}"))?,
        visible: p.visible.unwrap_or(true),
        blend: p.blend.as_deref().map_or(Ok(BlendMode::Normal), blend)?,
    })
}

/// Gradient handles in the unit square: origin, x axis end, y axis end.
/// Linear: `angle` is CSS-like (0 = up, 90 = right, 180 = down) and the line spans the box.
/// Radial, angular and diamond: centred, half the box wide, turned by `angle`.
fn gradient_handles(kind: &str, angle: f32) -> [Vec2; 3] {
    let (s, c) = angle.to_radians().sin_cos();
    let dir = Vec2::new(s, -c);
    let perp = Vec2::new(-dir.y, dir.x);
    let centre = Vec2::new(0.5, 0.5);
    if kind == "linear" {
        let half = (s.abs() + c.abs()) / 2.0;
        let start = Vec2::new(centre.x - dir.x * half, centre.y - dir.y * half);
        let end = Vec2::new(centre.x + dir.x * half, centre.y + dir.y * half);
        [start, end, Vec2::new(start.x + perp.x * half * 2.0, start.y + perp.y * half * 2.0)]
    } else {
        let (ax, ay) = (Vec2::new(c, s), Vec2::new(-s, c));
        [centre, Vec2::new(centre.x + ax.x * 0.5, centre.y + ax.y * 0.5), Vec2::new(centre.x + ay.x * 0.5, centre.y + ay.y * 0.5)]
    }
}

fn store_image(b64: &str, images: &mut BTreeMap<String, Vec<u8>>) -> Result<String, String> {
    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::STANDARD.decode(b64.trim()).map_err(|e| format!("image_base64 is not valid base64: {e}"))?;
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err("image is larger than 16 MiB".into());
    }
    let known = bytes.starts_with(&[0x89, b'P', b'N', b'G'])
        || bytes.starts_with(&[0xFF, 0xD8, 0xFF])
        || bytes.starts_with(b"GIF8")
        || bytes.get(8..12) == Some(b"WEBP");
    if !known {
        return Err("image_base64 must be a PNG, JPEG, GIF or WebP file".into());
    }
    let hash = bytes.iter().fold(0xcbf29ce484222325u64, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x100000001b3));
    let key = format!("img-{hash:016x}");
    images.insert(key.clone(), bytes);
    Ok(key)
}

fn stroke(node: &mut Node, v: &Value, doc_images: &BTreeMap<String, Vec<u8>>, images: &mut BTreeMap<String, Vec<u8>>) -> Result<(), String> {
    match v {
        Value::Null => node.stroke.paints.clear(),
        Value::String(_) => node.stroke.paints = paints(v, doc_images, images)?,
        _ => {
            let s: StrokeSpec = serde_json::from_value(v.clone()).map_err(|e| e.to_string())?;
            if let Some(c) = &s.color {
                node.stroke.paints = paints(c, doc_images, images).map_err(|e| format!("color: {e}"))?;
            }
            if let Some(w) = s.width {
                node.stroke.weight = finite(w, 0.0, 1000.0).map_err(|e| format!("width {e}"))?;
            }
            if let Some(a) = &s.align {
                node.stroke.align = variant::<StrokeAlign>(a, "inside, center, outside").map_err(|e| format!("align {e}"))?;
            }
            if let Some(c) = &s.cap {
                node.stroke.cap = variant::<StrokeCap>(c, "none, round, square").map_err(|e| format!("cap {e}"))?;
            }
            if let Some(j) = &s.join {
                node.stroke.join = variant::<StrokeJoin>(j, "miter, bevel, round").map_err(|e| format!("join {e}"))?;
            }
            if let Some(d) = &s.dashes {
                if d.len() > 16 || d.iter().any(|v| !v.is_finite() || *v < 0.0) {
                    return Err("dashes must be up to 16 non-negative numbers".into());
                }
                node.stroke.dashes = d.clone();
            }
            if let Some(w) = s.side_weights {
                if w.iter().any(|v| !v.is_finite() || !(0.0..=1000.0).contains(v)) {
                    return Err("side_weights must be four numbers within 0..=1000".into());
                }
                node.stroke.side_weights = Some(w);
            }
        }
    }
    Ok(())
}

fn effect(v: &Value) -> Result<Effect, String> {
    let e: EffectSpec = serde_json::from_value(v.clone()).map_err(|e| e.to_string())?;
    let kind = e.kind.to_ascii_lowercase();
    let allowed: &[&str] = match kind.as_str() {
        "drop_shadow" => &["color", "x", "y", "blur", "spread", "show_behind", "visible"],
        "inner_shadow" => &["color", "x", "y", "blur", "spread", "visible"],
        "layer_blur" | "background_blur" => &["blur", "visible"],
        "glass" => &["light_angle", "light_intensity", "refraction", "depth", "dispersion", "frost", "visible"],
        other => return Err(format!("unknown effect type \"{other}\" (use drop_shadow, inner_shadow, layer_blur, background_blur, glass)")),
    };
    let given = [
        ("color", e.color.is_some()),
        ("x", e.x.is_some()),
        ("y", e.y.is_some()),
        ("blur", e.blur.is_some()),
        ("spread", e.spread.is_some()),
        ("show_behind", e.show_behind.is_some()),
        ("light_angle", e.light_angle.is_some()),
        ("light_intensity", e.light_intensity.is_some()),
        ("refraction", e.refraction.is_some()),
        ("depth", e.depth.is_some()),
        ("dispersion", e.dispersion.is_some()),
        ("frost", e.frost.is_some()),
    ];
    if let Some((bad, _)) = given.iter().find(|(f, set)| *set && !allowed.contains(f)) {
        return Err(format!("{bad} is not valid on {kind}; it takes: {}", allowed.join(", ")));
    }
    let visible = e.visible.unwrap_or(true);
    let range = |name: &str, v: Option<f32>, default: f32, lo: f32, hi: f32| finite(v.unwrap_or(default), lo, hi).map_err(|m| format!("{name} {m}"));
    Ok(match kind.as_str() {
        "drop_shadow" | "inner_shadow" => {
            let color = match &e.color {
                Some(c) => colour(c, "color")?,
                None => Color::rgba(0, 0, 0, 64),
            };
            let offset = Vec2::new(range("x", e.x, 0.0, -10_000.0, 10_000.0)?, range("y", e.y, 4.0, -10_000.0, 10_000.0)?);
            let radius = range("blur", e.blur, 8.0, 0.0, 1000.0)?;
            let spread = range("spread", e.spread, 0.0, -1000.0, 1000.0)?;
            if kind == "drop_shadow" {
                Effect::DropShadow { color, offset, radius, spread, visible, show_behind: e.show_behind.unwrap_or(false) }
            } else {
                Effect::InnerShadow { color, offset, radius, spread, visible }
            }
        }
        "layer_blur" => Effect::LayerBlur { radius: range("blur", e.blur, 8.0, 0.0, 1000.0)?, visible },
        "background_blur" => Effect::BackgroundBlur { radius: range("blur", e.blur, 16.0, 0.0, 1000.0)?, visible },
        _ => Effect::Glass(GlassEffect {
            light_angle: range("light_angle", e.light_angle, 315.0, -36_000.0, 36_000.0)?,
            light_intensity: range("light_intensity", e.light_intensity, 0.5, 0.0, 1.0)?,
            refraction: range("refraction", e.refraction, 0.5, 0.0, 1.0)?,
            depth: range("depth", e.depth, 10.0, 0.0, 1000.0)?,
            dispersion: range("dispersion", e.dispersion, 0.25, 0.0, 1.0)?,
            frost: range("frost", e.frost, 4.0, 0.0, 1000.0)?,
            visible,
        }),
    })
}

/// JSON schema for the tool descriptor.
pub fn schema() -> Value {
    let node = json!({
        "type":"object",
        "description":"One node spec. Give node_id to update an existing node (only the fields you pass change), or kind plus a parent to create. children builds the subtree below it: inside a new node every child is created; inside an updated node a child with node_id is updated and one without is created and appended.",
        "properties":{
            "node_id":{"type":"integer","minimum":1,"description":"Existing node to update."},
            "parent_id":{"type":"integer","minimum":1,"description":"Container for a new top-level node (overrides the call's parent_id)."},
            "index":{"type":"integer","minimum":0,"description":"Child position for a new top-level node; appended when omitted."},
            "kind":{"type":"string","enum":["frame","group","rectangle","ellipse","text","line","polygon","star"]},
            "name":{"type":"string","maxLength":1024},
            "x":{"type":"number"},"y":{"type":"number"},
            "width":{"description":"Pixels, \"hug\" (auto-layout frame or text) or \"fill\" (stretch inside an auto-layout parent).","oneOf":[{"type":"number","minimum":0},{"type":"string","enum":["hug","fill"]}]},
            "height":{"description":"Pixels, \"hug\" or \"fill\".","oneOf":[{"type":"number","minimum":0},{"type":"string","enum":["hug","fill"]}]},
            "rotation":{"type":"number","description":"Degrees, clockwise, around the top-left corner at x/y."},
            "opacity":{"type":"number","minimum":0,"maximum":1},
            "visible":{"type":"boolean"},"locked":{"type":"boolean"},"is_mask":{"type":"boolean"},
            "blend":{"type":"string","description":"normal, pass_through, darken, multiply, color_burn, linear_burn, lighten, screen, color_dodge, linear_dodge, overlay, soft_light, hard_light, difference, exclusion, hue, saturation, color, luminosity."},
            "fill":{"description":"\"#RRGGBB[AA]\", \"none\", a paint object, or an array of them (bottom first). Paint objects: {type:\"solid\",color} | {type:\"linear\"|\"radial\"|\"angular\"|\"diamond\",stops:[{pos:0..1,color}],angle?:degrees (linear: 0 up, 90 right, 180 down),handles?:[[x,y]x3 in 0..1 box space]} | {type:\"image\",image_ref|image_base64,scale_mode?:fill|fit|crop|tile}; all take opacity, blend, visible. On text this is the text colour."},
            "stroke":{"description":"\"#RRGGBB[AA]\", null to remove, or {color (as fill), width, align:inside|center|outside, cap:none|round|square, join:miter|bevel|round, dashes:[n], side_weights:[t,r,b,l]}."},
            "effects":{"type":"array","description":"Replaces all effects. Items: {type:\"drop_shadow\",color,x,y,blur,spread,show_behind} | {type:\"inner_shadow\",color,x,y,blur,spread} | {type:\"layer_blur\",blur} | {type:\"background_blur\",blur} | {type:\"glass\",light_angle,light_intensity,refraction,depth,dispersion,frost}; all take visible.","items":{"type":"object"}},
            "corner_radius":{"description":"One number or [top_left, top_right, bottom_right, bottom_left].","oneOf":[{"type":"number","minimum":0},{"type":"array","items":{"type":"number","minimum":0},"minItems":4,"maxItems":4}]},
            "corner_smoothing":{"type":"number","minimum":0,"maximum":1},
            "clips_content":{"type":"boolean","description":"Frames: clip children to the frame."},
            "auto_layout":{"description":"Frames: {direction:horizontal|vertical, gap, counter_gap, wrap, padding (number | [vertical,horizontal] | [t,r,b,l] | {top,right,bottom,left}), primary_align/counter_align: start|center|end|space_between|baseline, reverse_z, stroke_included}. null removes auto layout. Set the frame's own sizing with width/height \"hug\" or a number.","type":["object","null"]},
            "layout_child":{"type":"object","description":"Behaviour inside an auto-layout parent: {absolute, grow, min_width, max_width, min_height, max_height}. Stretching uses width/height \"fill\"."},
            "constraints":{"type":"object","description":"{horizontal, vertical}: min|max|center|stretch|scale (left/top and right/bottom are aliases)."},
            "text":{"type":"string","maxLength":100000,"description":"Text nodes only."},
            "text_style":{"type":"object","description":"Text nodes: {font_family, font_weight 100-900, italic, font_size, line_height (px | \"auto\" | \"120%\"), letter_spacing, paragraph_spacing, align_h:left|center|right|justified, align_v:top|center|bottom, case:original|upper|lower|title|small_caps, decoration:none|underline|strikethrough, color (as fill), features:{tnum:1}}."},
            "text_resize":{"type":"string","enum":["auto_width","auto_height","fixed","truncate"],"description":"Default: auto_width, or auto_height when a width is given."},
            "sides":{"type":"integer","minimum":3,"maximum":64,"description":"polygon and star."},
            "replace_children":{"type":"boolean","description":"Update only: children is the complete new child list. Entries with node_id keep (and reorder) that child, entries without are created, children not listed are deleted. Needs children (use [] to remove all)."},
            "instance_of":{"type":"integer","minimum":1,"description":"Create an instance of this main component instead of a plain node (no kind/children). x, y, name, opacity and visible apply to the instance."},
            "bind":{"type":"object","description":"Bind variables by name: {fill, stroke, stroke_width, gap, counter_gap, padding, radius, width, height, size, opacity, visible, text, font_size, font_weight, line_height, letter_spacing} -> variable name (or id), null to unbind. The variable's default-mode value is written into the node; create variables with desigma_set_variables. Gap and padding need auto_layout, fill and stroke need a fill or stroke in the same spec.","additionalProperties":{"type":["string","null"]}},
            "overrides":{"type":"object","description":"With instance_of: layer name inside the component -> {text, fill, stroke, visible, opacity, swap (component id), ...}.","additionalProperties":{"type":"object"}},
            "inner_ratio":{"type":"number","description":"star."},
            "children":{"type":"array","items":{"type":"object"},"description":"Child node specs, same shape, back to front."}
        },
        "additionalProperties":false
    });
    json!({"parent_id":{"type":"integer","minimum":1,"description":"Container for new top-level nodes that name no parent_id themselves (a page or frame ID)."},
        "nodes":{"type":"array","minItems":1,"maxItems":1000,"items":node},
        "validate_only":{"type":"boolean","default":false,"description":"Check the whole tree and report problems without changing anything."}})
}
