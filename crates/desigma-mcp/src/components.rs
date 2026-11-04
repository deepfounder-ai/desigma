//! Component tools: `desigma_create_component`, `desigma_create_instance` and
//! `desigma_override_instance`. Instances are built and edited through the same planner as
//! `desigma_apply_nodes`, so they are validated, laid out and one undo step each.
use crate::engine::Host;
use desigma_doc::{create_component as doc_create_component, swap_instance, Document, Node, NodeId, NodeKind};
use serde::Deserialize;
use serde_json::{json, Map, Value};

/// First layer below `root` (depth first) that is called `name`.
pub(crate) fn descendant_named(doc: &Document, root: NodeId, name: &str) -> Option<NodeId> {
    let mut stack: Vec<NodeId> = doc.get(root)?.children.iter().rev().copied().collect();
    while let Some(id) = stack.pop() {
        let n = doc.get(id)?;
        if n.name == name {
            return Some(id);
        }
        stack.extend(n.children.iter().rev());
    }
    None
}

fn layer_names(names: impl Iterator<Item = String>) -> String {
    let mut seen: Vec<String> = Vec::new();
    for n in names {
        if !seen.contains(&n) {
            seen.push(n);
        }
    }
    seen.truncate(30);
    seen.join(", ")
}

pub(crate) fn missing_layer(doc: &Document, root: NodeId, key: &str) -> String {
    let mut names = Vec::new();
    let mut stack: Vec<NodeId> = doc.get(root).map(|n| n.children.iter().rev().copied().collect()).unwrap_or_default();
    while let Some(id) = stack.pop() {
        if let Some(n) = doc.get(id) {
            names.push(n.name.clone());
            stack.extend(n.children.iter().rev());
        }
    }
    format!("no layer named \"{key}\" inside the instance (layers: {})", layer_names(names.into_iter()))
}

pub(crate) fn missing_layer_in(nodes: &[Node], key: &str) -> String {
    format!("no layer named \"{key}\" inside the instance (layers: {})", layer_names(nodes.iter().skip(1).map(|n| n.name.clone())))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateComponentArgs {
    node_id: u64,
}

pub fn create_component<H: Host>(host: &mut H, args: Value) -> Result<Value, String> {
    let a: CreateComponentArgs = serde_json::from_value(args).map_err(|e| format!("invalid arguments: {e}"))?;
    let id = NodeId(a.node_id);
    let n = host.doc().get(id).ok_or_else(|| format!("node {} not found", a.node_id))?;
    match n.kind {
        NodeKind::Component(_) => return Err("the node is already a main component".into()),
        NodeKind::ComponentSet(_) | NodeKind::Page { .. } => return Err("pages and component sets cannot be turned into a component".into()),
        _ => {}
    }
    if n.parent.is_none() {
        return Err("page roots cannot be turned into a component".into());
    }
    let cmd = doc_create_component(host.doc_mut(), id);
    host.exec(cmd)?;
    let doc = host.doc();
    let comp = if doc.get(id).is_some_and(|n| matches!(n.kind, NodeKind::Component(_))) { id } else { doc.get(id).and_then(|n| n.parent).unwrap_or(id) };
    let name = doc.get(comp).map(|n| n.name.clone()).unwrap_or_default();
    Ok(json!({"component_id":comp,"name":name,"wrapped":comp != id,"undo_steps":1}))
}

/// `{component_id, parent_id, x, y, name, index, overrides}` becomes one `instance_of` spec.
pub fn create_instance<H: Host>(host: &mut H, args: Value) -> Result<Value, String> {
    let Value::Object(mut a) = args else { return Err("invalid arguments: expected an object".into()) };
    let take = |a: &mut Map<String, Value>, k: &str| a.remove(k);
    let component = take(&mut a, "component_id").ok_or("component_id is required")?;
    let parent = take(&mut a, "parent_id").ok_or("parent_id is required")?;
    let mut spec = Map::new();
    spec.insert("instance_of".into(), component);
    for k in ["x", "y", "name", "index", "overrides", "opacity", "visible", "layout_child"] {
        if let Some(v) = take(&mut a, k) {
            spec.insert(k.into(), v);
        }
    }
    if let Some(k) = a.keys().next() {
        return Err(format!("invalid arguments: unknown field `{k}`"));
    }
    let mut r = crate::apply_nodes::apply_nodes(host, json!({"parent_id":parent,"nodes":[Value::Object(spec)]}))?;
    r["instance_id"] = r["nodes"][0]["node_id"].clone();
    Ok(r)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OverrideArgs {
    instance_id: u64,
    overrides: Map<String, Value>,
}

/// Changes layers inside an existing instance by name; each change is recorded as an override.
pub fn override_instance<H: Host>(host: &mut H, args: Value) -> Result<Value, String> {
    let a: OverrideArgs = serde_json::from_value(args).map_err(|e| format!("invalid arguments: {e}"))?;
    let root = NodeId(a.instance_id);
    if !host.doc().get(root).is_some_and(|n| matches!(n.kind, NodeKind::Instance { .. })) {
        return Err(format!("node {} is not an instance", a.instance_id));
    }
    // Swaps first: they rebuild the inside of a nested instance, so names are resolved after.
    let mut swapped = 0;
    for (key, v) in &a.overrides {
        let Some(to) = v.get("swap").and_then(Value::as_u64) else { continue };
        let target = descendant_named(host.doc(), root, key).ok_or_else(|| missing_layer(host.doc(), root, key))?;
        if !host.doc().get(target).is_some_and(|n| matches!(n.kind, NodeKind::Instance { .. })) {
            return Err(format!("overrides.{key}: swap only works on an instance"));
        }
        if !host.doc().get(NodeId(to)).is_some_and(|n| matches!(n.kind, NodeKind::Component(_))) {
            return Err(format!("overrides.{key}: node {to} is not a main component"));
        }
        let cmd = swap_instance(host.doc_mut(), target, NodeId(to));
        host.exec(cmd)?;
        swapped += 1;
    }
    let mut specs = Vec::new();
    for (key, v) in &a.overrides {
        let Some(obj) = v.as_object() else { return Err(format!("overrides.{key}: expected an object")) };
        let mut obj = obj.clone();
        obj.remove("swap");
        if obj.is_empty() {
            continue;
        }
        let target = descendant_named(host.doc(), root, key).ok_or_else(|| missing_layer(host.doc(), root, key))?;
        obj.insert("node_id".into(), json!(target));
        specs.push(Value::Object(obj));
    }
    if specs.is_empty() {
        return Ok(json!({"changed":swapped > 0,"swapped":swapped,"updated":0}));
    }
    let mut r = crate::apply_nodes::apply_nodes(host, json!({"nodes":specs}))?;
    r["swapped"] = json!(swapped);
    Ok(r)
}
