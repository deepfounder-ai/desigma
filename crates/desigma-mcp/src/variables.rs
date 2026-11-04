//! Variable tools: `desigma_set_variables` (create or update collections, modes and values by
//! name) and `desigma_bind_variable` (bind a variable to a node field). Binding by name inside
//! `desigma_apply_nodes` specs (`"bind"`) shares [`resolve_binding`].
use crate::engine::{colour, Host};
use desigma_doc::{bindable_keys, property_kind, scope_allows, set_bound_property, Document, Node, Variable, VariableKind, VariableStore, VariableValue};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetArgs {
    collections: Vec<CollectionSpec>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CollectionSpec {
    name: String,
    modes: Option<Vec<String>>,
    #[serde(default)]
    variables: Vec<VariableSpec>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VariableSpec {
    name: String,
    #[serde(rename = "type")]
    kind: String,
    /// One value for every mode.
    value: Option<Value>,
    /// Mode name -> value.
    values: Option<BTreeMap<String, Value>>,
    scopes: Option<Vec<String>>,
    description: Option<String>,
}

fn kind_of(s: &str) -> Result<VariableKind, String> {
    match s {
        "color" => Ok(VariableKind::Color),
        "number" => Ok(VariableKind::Number),
        "string" => Ok(VariableKind::String),
        "boolean" => Ok(VariableKind::Boolean),
        _ => Err(format!("type must be color, number, string or boolean (got \"{s}\")")),
    }
}

fn value_of(kind: VariableKind, v: &Value) -> Result<VariableValue, String> {
    match kind {
        VariableKind::Color => Ok(VariableValue::Color(colour(v.as_str().ok_or("a colour is a string like \"#3366ff\"")?, "value")?)),
        VariableKind::Number => match v.as_f64() {
            Some(n) if n.is_finite() && n.abs() <= 1e6 => Ok(VariableValue::Number(n as f32)),
            _ => Err("a number value must be finite and within +-1,000,000".into()),
        },
        VariableKind::String => match v.as_str() {
            Some(s) if s.len() <= 10_000 => Ok(VariableValue::String(s.to_string())),
            _ => Err("a string value must be text of at most 10000 bytes".into()),
        },
        VariableKind::Boolean => v.as_bool().map(VariableValue::Boolean).ok_or_else(|| "a boolean value must be true or false".into()),
    }
}

/// Upserts collections, modes and variables by name in one undo step.
pub fn set_variables<H: Host>(host: &mut H, args: Value) -> Result<Value, String> {
    let a: SetArgs = serde_json::from_value(args).map_err(|e| format!("invalid arguments: {e}"))?;
    if a.collections.is_empty() || a.collections.len() > 50 || a.collections.iter().map(|c| c.variables.len()).sum::<usize>() > 2000 {
        return Err("pass 1-50 collections and at most 2000 variables".into());
    }
    let mut store = VariableStore::from_doc(host.doc());
    let (mut errors, mut out) = (Vec::new(), Vec::new());
    for (ci, c) in a.collections.iter().enumerate() {
        let at = format!("collections[{ci}] \"{}\"", c.name);
        if c.name.trim().is_empty() || c.name.len() > 256 {
            errors.push(format!("{at}: name must be 1..256 bytes"));
            continue;
        }
        let coll = match store.collections.values().find(|x| x.name == c.name) {
            Some(x) => x.id.clone(),
            None => {
                let id = store.add_collection(&c.name);
                if let Some(first) = c.modes.as_ref().and_then(|m| m.first()) {
                    let mode = store.collections[&id].default_mode.clone();
                    store.rename_mode(&id, &mode, first);
                }
                id
            }
        };
        for m in c.modes.iter().flatten() {
            if !store.collections[&coll].modes.iter().any(|x| x.name == *m) {
                store.add_mode(&coll, m);
            }
        }
        let mode_id = |store: &VariableStore, name: &str| store.collections[&coll].modes.iter().find(|m| m.name == name).map(|m| m.id.clone());
        let mut var_ids = Map::new();
        for (vi, v) in c.variables.iter().enumerate() {
            let vat = format!("{at}.variables[{vi}] \"{}\"", v.name);
            let kind = match kind_of(&v.kind) {
                Ok(k) => k,
                Err(e) => {
                    errors.push(format!("{vat}: {e}"));
                    continue;
                }
            };
            let existing = store.variables.values().find(|x| x.collection == coll && x.name == v.name).map(|x| (x.id.clone(), x.kind));
            let id = match existing {
                Some((_, k)) if k != kind => {
                    errors.push(format!("{vat}: already exists with a different type; delete it in the editor first"));
                    continue;
                }
                Some((id, _)) => id,
                None => match store.add_variable(&coll, &v.name, kind) {
                    Some(id) => id,
                    None => {
                        errors.push(format!("{vat}: cannot create the variable"));
                        continue;
                    }
                },
            };
            let mut writes: Vec<(String, VariableValue)> = Vec::new();
            if let Some(val) = &v.value {
                match value_of(kind, val) {
                    Ok(x) => writes.extend(store.collections[&coll].modes.iter().map(|m| (m.id.clone(), x.clone()))),
                    Err(e) => errors.push(format!("{vat}: value: {e}")),
                }
            }
            for (mode, val) in v.values.iter().flatten() {
                match (mode_id(&store, mode), value_of(kind, val)) {
                    (Some(m), Ok(x)) => writes.push((m, x)),
                    (None, _) => errors.push(format!("{vat}: unknown mode \"{mode}\"; list it in modes")),
                    (_, Err(e)) => errors.push(format!("{vat}: values.{mode}: {e}")),
                }
            }
            for (m, x) in writes {
                store.set_value(&id, &m, x);
            }
            if let Some(var) = store.variables.get_mut(&id) {
                if let Some(s) = &v.scopes {
                    var.scopes = s.clone();
                }
                if let Some(d) = &v.description {
                    var.description = d.clone();
                }
            }
            var_ids.insert(v.name.clone(), json!(id));
        }
        let modes: Map<String, Value> = store.collections[&coll].modes.iter().map(|m| (m.name.clone(), json!(m.id))).collect();
        out.push(json!({"collection_id":coll,"name":c.name,"modes":modes,"variables":var_ids}));
    }
    if !errors.is_empty() {
        return Err(format!("desigma_set_variables failed; nothing was changed\n- {}", errors.join("\n- ")));
    }
    if store.matches_doc(host.doc()) {
        return Ok(json!({"changed":false,"collections":out}));
    }
    host.exec(store.command())?;
    Ok(json!({"changed":true,"collections":out,"undo_steps":1}))
}

/// `{node_id, bindings:{field: variable name | null}}` as an `apply_nodes` update.
pub fn bind_variable<H: Host>(host: &mut H, args: Value) -> Result<Value, String> {
    let Value::Object(mut a) = args else { return Err("invalid arguments: expected an object".into()) };
    let node = a.remove("node_id").ok_or("node_id is required")?;
    let bindings = a.remove("bindings").ok_or("bindings is required")?;
    if let Some(k) = a.keys().next() {
        return Err(format!("invalid arguments: unknown field `{k}`"));
    }
    crate::apply_nodes::apply_nodes(host, json!({"nodes":[{"node_id":node,"bind":bindings}]}))
}

/// Property keys a friendly field name stands for.
pub(crate) fn keys_for(field: &str) -> Result<Vec<String>, String> {
    let one = |k: &str| Ok(vec![k.to_string()]);
    let many = |k: &[&str]| Ok(k.iter().map(|s| s.to_string()).collect());
    match field {
        "fill" => one("fills/0"),
        "stroke" => one("strokes/0"),
        "stroke_width" => one("strokeWeight"),
        "gap" => one("itemSpacing"),
        "counter_gap" => one("counterAxisSpacing"),
        "padding" => many(&["paddingTop", "paddingRight", "paddingBottom", "paddingLeft"]),
        "padding_top" => one("paddingTop"),
        "padding_right" => one("paddingRight"),
        "padding_bottom" => one("paddingBottom"),
        "padding_left" => one("paddingLeft"),
        "radius" => many(&["topLeftRadius", "topRightRadius", "bottomRightRadius", "bottomLeftRadius"]),
        "width" => one("width"),
        "height" => one("height"),
        "size" => many(&["width", "height"]),
        "opacity" => one("opacity"),
        "visible" => one("visible"),
        "text" => one("characters"),
        "font_size" => one("fontSize"),
        "font_weight" => one("fontWeight"),
        "line_height" => one("lineHeight"),
        "letter_spacing" => one("letterSpacing"),
        raw if property_kind(raw).is_some() => one(raw),
        _ => Err(format!(
            "unknown bind field \"{field}\" (fill, stroke, stroke_width, gap, counter_gap, padding, radius, width, height, size, opacity, visible, text, font_size, font_weight, line_height, letter_spacing)"
        )),
    }
}

fn find_variable<'a>(doc: &'a Document, name: &str) -> Result<&'a Variable, String> {
    if let Some(v) = doc.variables.get(name) {
        return Ok(v);
    }
    let hits: Vec<&Variable> = doc.variables.values().filter(|v| v.name == name).collect();
    match hits.as_slice() {
        [v] => Ok(v),
        [] => {
            let mut names: Vec<&str> = doc.variables.values().map(|v| v.name.as_str()).collect();
            names.truncate(20);
            Err(format!("no variable named \"{name}\" (create it with desigma_set_variables; existing: {})", names.join(", ")))
        }
        _ => Err(format!("{} variables are named \"{name}\"; pass the variable id instead", hits.len())),
    }
}

fn default_value(doc: &Document, var: &Variable) -> Option<VariableValue> {
    let mut cur = var;
    for _ in 0..8 {
        let mode = doc.variable_collections.get(&cur.collection)?.default_mode.clone();
        match cur.values.get(&mode)? {
            VariableValue::Alias(id) => cur = doc.variables.get(id)?,
            v => return Some(v.clone()),
        }
    }
    None
}

/// Binds (`Some(name)`) or unbinds (`None`) the variable for `field` on `node`, writing the
/// variable's default-mode value into the node so layout sees it.
pub(crate) fn resolve_binding(doc: &Document, node: &mut Node, field: &str, variable: Option<&str>) -> Result<(), String> {
    let keys = keys_for(field)?;
    let Some(name) = variable else {
        for k in &keys {
            node.variables.bindings.remove(k);
        }
        return Ok(());
    };
    let var = find_variable(doc, name)?;
    let value = default_value(doc, var).ok_or_else(|| format!("variable \"{name}\" has no value in its default mode"))?;
    for key in keys {
        if property_kind(&key) != Some(var.kind) {
            return Err(format!(
                "bind {field}: variable \"{}\" is a {:?} variable, \"{key}\" needs a {:?}",
                var.name,
                var.kind,
                property_kind(&key).unwrap_or(var.kind)
            ));
        }
        if !scope_allows(&var.scopes, &key, &node.kind) {
            return Err(format!("bind {field}: the scopes of variable \"{}\" do not allow {key}", var.name));
        }
        if !bindable_keys(&node.kind).contains(&key.as_str()) {
            return Err(format!("bind {field}: this layer cannot bind {key} (set auto_layout first for gap and padding; radius is not for text)"));
        }
        if key.starts_with("fills/") && node.fills.is_empty() && !matches!(&node.kind, desigma_doc::NodeKind::Text(t) if !t.style.fills.is_empty()) {
            return Err(format!("bind {field}: the layer has no fill to bind; set a fill first"));
        }
        if key.starts_with("strokes/") && node.stroke.paints.is_empty() {
            return Err(format!("bind {field}: the layer has no stroke to bind; set a stroke first"));
        }
        set_bound_property(node, &key, &value);
        node.variables.bindings.insert(key, var.id.clone());
    }
    Ok(())
}
