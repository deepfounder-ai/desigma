//! Read context for agents: `desigma_get_tree` (compact scoped layer tree) and
//! `desigma_get_assets` (components, styles and variables in one list).
use crate::engine::{bounded_text, Host};
use desigma_doc::{Document, Node, NodeId, NodeKind, Paint, PaintKind, Style, VariableValue};
use serde::Deserialize;
use serde_json::{json, Map, Value};

const MAX_NODES: usize = 2000;
const MAX_ASSETS: usize = 500;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TreeArgs {
    node_id: Option<u64>,
    #[serde(default = "default_depth")]
    depth: usize,
    #[serde(default = "default_limit")]
    limit: usize,
}
fn default_depth() -> usize {
    3
}
fn default_limit() -> usize {
    MAX_NODES
}

fn r1(v: f32) -> f64 {
    (f64::from(v) * 10.0).round() / 10.0
}

fn paint_text(p: &Paint) -> String {
    let hex = |c: &desigma_doc::Color| format!("#{}{}", c.to_hex(), if c.a < 0.999 { format!("@{:.0}%", c.a * 100.0) } else { String::new() });
    let stops = |s: &[desigma_doc::GradientStop]| s.iter().map(|g| g.color.to_hex()).collect::<Vec<_>>().join(">");
    let body = match &p.kind {
        PaintKind::Solid(c) => hex(c),
        PaintKind::LinearGradient { stops: s, .. } => format!("linear({})", stops(s)),
        PaintKind::RadialGradient { stops: s, .. } => format!("radial({})", stops(s)),
        PaintKind::AngularGradient { stops: s, .. } => format!("angular({})", stops(s)),
        PaintKind::DiamondGradient { stops: s, .. } => format!("diamond({})", stops(s)),
        PaintKind::Image { .. } => "image".into(),
    };
    if p.visible {
        body
    } else {
        format!("({body})")
    }
}

fn paints_text(ps: &[Paint]) -> Option<String> {
    (!ps.is_empty()).then(|| ps.iter().map(paint_text).collect::<Vec<_>>().join(" + "))
}

fn tree_node(doc: &Document, n: &Node, depth: usize, left: &mut usize) -> Value {
    *left = left.saturating_sub(1);
    let mut o = Map::new();
    o.insert("id".into(), json!(n.id));
    o.insert("name".into(), json!(n.name));
    o.insert("kind".into(), json!(n.kind.default_name()));
    o.insert("x".into(), json!(r1(n.transform.tx)));
    o.insert("y".into(), json!(r1(n.transform.ty)));
    o.insert("w".into(), json!(r1(n.size.x)));
    o.insert("h".into(), json!(r1(n.size.y)));
    if !n.visible {
        o.insert("hidden".into(), json!(true));
    }
    let fill = match &n.kind {
        NodeKind::Text(t) => paints_text(&t.style.fills),
        _ => paints_text(&n.fills),
    };
    if let Some(f) = fill {
        o.insert("fill".into(), json!(f));
    }
    if let Some(s) = paints_text(&n.stroke.paints) {
        o.insert("stroke".into(), json!(format!("{s} {}px", r1(n.stroke.weight))));
    }
    if let NodeKind::Text(t) = &n.kind {
        o.insert("text".into(), json!(t.content.chars().take(60).collect::<String>()));
    }
    if let NodeKind::Instance { component: Some(c), .. } = &n.kind {
        o.insert("instance_of".into(), json!(c));
    }
    if let Some(al) = n.kind.frame_props().and_then(|f| f.auto_layout.as_ref()) {
        o.insert("layout".into(), json!(format!("{:?} gap {}", al.direction, r1(al.item_spacing))));
    }
    if !n.variables.bindings.is_empty() {
        let b: Vec<String> = n.variables.bindings.iter().map(|(k, v)| format!("{k}={}", doc.variables.get(v).map(|x| x.name.as_str()).unwrap_or(v))).collect();
        o.insert("bound".into(), json!(b));
    }
    if !n.children.is_empty() {
        if depth == 0 || *left == 0 {
            o.insert("child_count".into(), json!(n.children.len()));
        } else {
            let mut kids = Vec::new();
            for c in &n.children {
                if *left == 0 {
                    break;
                }
                if let Some(cn) = doc.get(*c) {
                    kids.push(tree_node(doc, cn, depth - 1, left));
                }
            }
            if kids.len() < n.children.len() {
                o.insert("child_count".into(), json!(n.children.len()));
            }
            o.insert("children".into(), Value::Array(kids));
        }
    }
    Value::Object(o)
}

/// A compact layer tree below `node_id` (or every page root), `depth` levels deep.
pub fn get_tree<H: Host>(host: &H, args: Value) -> Result<Value, String> {
    let a: TreeArgs = serde_json::from_value(args).map_err(|e| format!("invalid arguments: {e}"))?;
    if a.depth > 16 || a.limit == 0 || a.limit > MAX_NODES {
        return Err(format!("depth must be 0..16 and limit 1..{MAX_NODES}"));
    }
    let doc = host.doc();
    let mut left = a.limit;
    let roots: Vec<NodeId> = match a.node_id {
        Some(id) => vec![NodeId(id)],
        None => doc.pages.clone(),
    };
    let mut tree = Vec::new();
    for id in roots {
        let n = doc.get(id).ok_or_else(|| format!("node {} not found", id.0))?;
        tree.push(tree_node(doc, n, a.depth, &mut left));
    }
    Ok(json!({"tree":tree,"truncated":left == 0}))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AssetArgs {
    query: Option<String>,
}

fn value_text(doc: &Document, v: &VariableValue) -> Value {
    match v {
        VariableValue::Color(c) => json!(format!("#{}", c.to_hex())),
        VariableValue::Number(n) => json!(n),
        VariableValue::String(s) => json!(s),
        VariableValue::Boolean(b) => json!(b),
        VariableValue::Alias(id) => json!(format!("alias:{}", doc.variables.get(id).map(|x| x.name.as_str()).unwrap_or(id))),
    }
}

/// Components, styles and variables of the document in one list.
pub fn get_assets<H: Host>(host: &H, args: Value) -> Result<Value, String> {
    let a: AssetArgs = serde_json::from_value(args).map_err(|e| format!("invalid arguments: {e}"))?;
    let q = match &a.query {
        Some(q) => {
            bounded_text("query", q, 1024)?;
            q.to_lowercase()
        }
        None => String::new(),
    };
    let doc = host.doc();
    let hit = |name: &str| name.to_lowercase().contains(&q);
    let components: Vec<Value> = doc
        .nodes
        .values()
        .filter(|n| matches!(n.kind, NodeKind::Component(_)) && hit(&n.name))
        .map(|n| {
            let set = n.parent.and_then(|p| doc.get(p)).filter(|p| matches!(p.kind, NodeKind::ComponentSet(_)));
            let layers: Vec<&str> = n.children.iter().filter_map(|c| doc.get(*c)).map(|c| c.name.as_str()).take(12).collect();
            json!({"id":n.id,"name":n.name,"set":set.map(|s| s.name.clone()),"w":r1(n.size.x),"h":r1(n.size.y),
                "layers":layers,"instances":desigma_doc::instances_of(doc, n.id).len()})
        })
        .collect();
    let styles: Vec<Value> = doc
        .styles
        .iter()
        .map(|(id, s)| {
            let (kind, detail) = match s {
                Style::Fill(p) => ("fill", paints_text(p)),
                Style::Text(t) => ("text", Some(format!("{} {} {}px", t.font_family, t.font_weight, t.font_size))),
                Style::Effect(e) => ("effect", Some(format!("{} effects", e.len()))),
                Style::Grid(g) => ("grid", Some(format!("{} grids", g.len()))),
            };
            json!({"id":id,"name":doc.style_meta.get(id).map(|m| m.name.clone()).unwrap_or_default(),"kind":kind,"detail":detail})
        })
        .filter(|v| hit(v["name"].as_str().unwrap_or("")))
        .collect();
    let collections: Vec<Value> = doc
        .variable_collections
        .values()
        .map(|c| json!({"id":c.id,"name":c.name,"default_mode":c.default_mode,"modes":c.modes.iter().map(|m| json!({"id":m.id,"name":m.name})).collect::<Vec<_>>()}))
        .collect();
    let variables: Vec<Value> = doc
        .variables
        .values()
        .filter(|v| hit(&v.name))
        .map(|v| {
            let mode_name = |m: &str| {
                doc.variable_collections
                    .get(&v.collection)
                    .and_then(|c| c.modes.iter().find(|x| x.id == m))
                    .map(|x| x.name.clone())
                    .unwrap_or_else(|| m.to_string())
            };
            let values: Map<String, Value> = v.values.iter().map(|(m, x)| (mode_name(m), value_text(doc, x))).collect();
            json!({"id":v.id,"name":v.name,"type":format!("{:?}", v.kind).to_lowercase(),"collection":v.collection,"values":values})
        })
        .collect();
    let cut = |mut v: Vec<Value>| {
        let more = v.len() > MAX_ASSETS;
        v.truncate(MAX_ASSETS);
        (v, more)
    };
    let (components, c_more) = cut(components);
    let (styles, s_more) = cut(styles);
    let (variables, v_more) = cut(variables);
    Ok(json!({"components":components,"styles":styles,"variable_collections":collections,"variables":variables,"truncated":c_more || s_more || v_more}))
}
