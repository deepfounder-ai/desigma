//! Typed, local document operations shared by the MCP transport and future bridges.
use desigma_doc::{Command, Document, History, Node, NodeId, NodeKind, Rect, TextStyle};
use serde::{de::DeserializeOwned, Deserialize};
use serde_json::{json, Value};

pub struct Engine {
    pub doc: Document,
    pub history: History,
}

impl Default for Engine {
    fn default() -> Self {
        let mut doc = Document::new("Untitled");
        doc.add_page("Page 1");
        Self { doc, history: History::default() }
    }
}

/// What a tool call needs from its document owner: the MCP file session or the editor.
pub trait Host {
    fn doc(&self) -> &Document;
    fn exec(&mut self, cmd: Command) -> Result<(), String>;
    fn undo(&mut self) -> Result<bool, String>;
    fn redo(&mut self) -> Result<bool, String>;
    fn can_undo(&self) -> bool;
    fn can_redo(&self) -> bool;
    /// Allocates a fresh node id on the document.
    fn alloc_id(&mut self) -> Result<NodeId, String>;
}

impl Host for Engine {
    fn doc(&self) -> &Document {
        &self.doc
    }
    fn exec(&mut self, cmd: Command) -> Result<(), String> {
        self.history.exec(&mut self.doc, cmd).map_err(|e| e.to_string())
    }
    fn undo(&mut self) -> Result<bool, String> {
        self.history.undo(&mut self.doc).map_err(|e| e.to_string())
    }
    fn redo(&mut self) -> Result<bool, String> {
        self.history.redo(&mut self.doc).map_err(|e| e.to_string())
    }
    fn can_undo(&self) -> bool {
        self.history.can_undo()
    }
    fn can_redo(&self) -> bool {
        self.history.can_redo()
    }
    fn alloc_id(&mut self) -> Result<NodeId, String> {
        self.doc.try_alloc_id().ok_or_else(|| "Document has exhausted node IDs".into())
    }
}

impl Engine {
    pub fn call(&mut self, name: &str, args: Value) -> Result<Value, String> {
        call(self, name, args)
    }
}

pub fn call<H: Host>(host: &mut H, name: &str, args: Value) -> Result<Value, String> {
    match name {
        "desigma_get_document" => {
            let _: Empty = parse(args)?;
            Ok(json!({"name":host.doc().name,"kind":host.doc().kind,
                "pages":host.doc().pages.iter().filter_map(|id|host.doc().get(*id)).map(summary).collect::<Vec<_>>(),
                "node_count":host.doc().nodes.len(),"variable_count":host.doc().variables.len(),
                "can_undo":host.can_undo(),"can_redo":host.can_redo()}))
        }
        "desigma_list_nodes" => {
            let a: ListArgs = parse(args)?;
            if a.limit == 0 || a.limit > 500 {
                return Err("limit must be between 1 and 500".into());
            }
            if let Some(id) = a.parent_id {
                node(host, id)?;
            }
            if let Some(q) = &a.query {
                bounded_text("query", q, 1024)?;
            }
            let query = a.query.unwrap_or_default().to_lowercase();
            let matching: Vec<_> = host
                .doc()
                .nodes
                .values()
                .filter(|n| a.parent_id.is_none_or(|id| n.parent == Some(NodeId(id))) && n.name.to_lowercase().contains(&query))
                .collect();
            let nodes: Vec<_> = matching.iter().skip(a.offset).take(a.limit).map(|n| summary(n)).collect();
            let next = a.offset.saturating_add(nodes.len());
            Ok(json!({"nodes":nodes,"total":matching.len(),"offset":a.offset,
                "next_offset":(next < matching.len()).then_some(next)}))
        }
        "desigma_get_node" => {
            let a: NodeArgs = parse(args)?;
            Ok(json!({"node":node(host, a.node_id)?}))
        }
        "desigma_get_variables" => {
            let a: VariablesArgs = parse(args)?;
            let effective = if let Some(id) = a.node_id {
                let n = node(host, id)?;
                Some(
                    n.variables
                        .bindings
                        .iter()
                        .map(|(property, variable)| {
                            json!({
                                "property":property,"variable_id":variable,
                                "mode_id":host.doc().variables.get(variable).and_then(|v|host.doc().variable_mode(n.id,&v.collection)),
                                "value":host.doc().resolve_variable(n.id,variable)
                            })
                        })
                        .collect::<Vec<_>>(),
                )
            } else {
                None
            };
            Ok(json!({"variables":host.doc().variables,"collections":host.doc().variable_collections,"effective_bindings":effective}))
        }
        "desigma_generate_code" => {
            let a: CodeArgs = parse(args)?;
            let id = node(host, a.node_id)?.id;
            let doc = desigma_doc::apply_bound_variables(host.doc());
            let code = match a.language.as_str() {
                "css" => desigma_export::css_for(&doc, id),
                "tailwind" => desigma_export::tailwind_for(&doc, id),
                "react" => desigma_export::react_for(&doc, id),
                "swiftui" => desigma_export::swiftui_for(&doc, id),
                "compose" => desigma_export::compose_for(&doc, id),
                "svg" => desigma_export::export_svg(&doc, id, &Default::default()),
                _ => return Err("language must be css, tailwind, react, swiftui, compose, or svg".into()),
            };
            Ok(json!({"node_id":id,"language":a.language,"code":code}))
        }
        "desigma_create_node" => create(host, parse(args)?),
        "desigma_update_node" => update(host, parse(args)?),
        "desigma_delete_page" => {
            let a: NodeArgs = parse(args)?;
            let n = node(host, a.node_id)?;
            if !matches!(n.kind, NodeKind::Page { .. }) || n.parent.is_some() {
                return Err("desigma_delete_page only accepts a page node id".into());
            }
            if host.doc().pages.len() <= 1 {
                return Err("cannot delete the last page".into());
            }
            let id = n.id;
            host.exec(Command::Remove { id })?;
            Ok(json!({"page_id":id,"deleted":true}))
        }
        "desigma_delete_node" => {
            let a: NodeArgs = parse(args)?;
            let n = node(host, a.node_id)?;
            if n.parent.is_none() {
                return Err("page roots cannot be deleted by this tool".into());
            }
            let id = n.id;
            host.exec(Command::Remove { id })?;
            Ok(json!({"node_id":id,"deleted":true}))
        }
        "desigma_undo" | "desigma_redo" => {
            let _: Empty = parse(args)?;
            let changed = if name == "desigma_undo" { host.undo() } else { host.redo() }?;
            Ok(json!({"changed":changed,"can_undo":host.can_undo(),"can_redo":host.can_redo()}))
        }
        n if crate::motion::EDIT_TOOLS.contains(&n) => crate::motion::call(host, n, args),
        _ => Err(format!("unknown tool: {name}")),
    }
}

/// Reject broken graphs before recursive exporters or inherited-mode resolution use them.
pub fn validate_document(doc: &Document) -> Result<(), String> {
    use std::collections::BTreeSet;
    if doc.nodes.len() > 100_000 {
        return Err("Document exceeds the 100000 layer limit".into());
    }
    for (id, node) in &doc.nodes {
        if *id != node.id || id.0 == 0 {
            return Err("Document has inconsistent node IDs".into());
        }
        let mut seen = BTreeSet::new();
        let mut current = Some(*id);
        while let Some(parent) = current {
            if !seen.insert(parent) || seen.len() > 256 {
                return Err("Document contains a cycle or exceeds 256 nesting levels".into());
            }
            let ancestor = doc.get(parent).ok_or("Document references a missing parent")?;
            current = ancestor.parent;
        }
        let mut children = BTreeSet::new();
        for child in &node.children {
            if !children.insert(child) || doc.get(*child).is_none_or(|n| n.parent != Some(*id)) {
                return Err("Document has duplicate or inconsistent child links".into());
            }
        }
        if let Some(parent) = node.parent {
            if !doc.get(parent).is_some_and(|n| n.children.contains(id)) {
                return Err("Document has inconsistent parent links".into());
            }
        }
    }
    if doc.pages.iter().any(|id| doc.get(*id).is_none_or(|n| n.parent.is_some() || !matches!(n.kind, NodeKind::Page { .. }))) {
        return Err("Document has invalid page roots".into());
    }
    Ok(())
}

fn node<H: Host>(host: &H, id: u64) -> Result<&Node, String> {
    if id == 0 {
        return Err("node IDs must be positive integers".into());
    }
    host.doc().get(NodeId(id)).ok_or_else(|| format!("node {id} not found"))
}

/// Renders one layer to PNG bytes. Shared by the MCP session (which writes the
/// file) and, eventually, the editor bridge (which can hand the bytes to the UI
/// directly).
pub fn export_image<H: Host>(host: &H, node_id: u64, scale: f32) -> Result<(Vec<u8>, u32, u32), String> {
    let id = node(host, node_id)?.id;
    if !(0.25..=4.0).contains(&scale) {
        return Err("scale must be between 0.25 and 4".into());
    }
    // Resolve variable bindings first: a bound width/height, or effect
    // overflow (shadow/blur/outside stroke), can make the actual render far
    // larger than the node's own stored size.
    let doc = desigma_doc::apply_bound_variables(host.doc());
    let (w, h) = desigma_export::render_size(&doc, id, scale).ok_or("layer cannot be rendered")?;
    if u64::from(w) * u64::from(h) > 32_000_000 {
        return Err("export exceeds 32 megapixels; lower the scale or export a smaller layer".into());
    }
    let pm = desigma_export::render_export(&doc, id, scale).ok_or("layer cannot be rendered")?;
    let png = pm.encode_png().map_err(|e| format!("cannot encode PNG: {e}"))?;
    Ok((png, pm.width(), pm.height()))
}

/// Writes a saved design to `path`. Without `overwrite` the file must not exist (and an
/// existing symlink is never followed). With `overwrite` the bytes go to a temporary file in
/// the same directory first and replace the target with one rename, so a reader never sees a
/// half-written file; a symlink at `path` is refused rather than written through.
pub fn write_design(path: &std::path::Path, bytes: &[u8], overwrite: bool) -> Result<(), String> {
    use std::io::Write;
    if !overwrite {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|e| format!("Cannot create file (choose an unused .desigma filename, or pass overwrite=true): {e}"))?;
        if let Err(e) = file.write_all(bytes).and_then(|_| file.sync_all()) {
            drop(file);
            let _ = std::fs::remove_file(path);
            return Err(format!("Cannot save design: {e}"));
        }
        return Ok(());
    }
    if std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink() || !m.is_file()) {
        return Err("Cannot overwrite: the path exists and is not a regular file".into());
    }
    let dir = path.parent().ok_or("Save path needs a parent directory")?;
    let name = path.file_name().ok_or("Save path needs a filename")?.to_string_lossy();
    let tmp = dir.join(format!(".{name}.{}.tmp", std::process::id()));
    let res = (|| {
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if let Err(e) = res {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("Cannot save design: {e}"));
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportArgs {
    pub node_id: u64,
    #[serde(default = "default_scale")]
    pub scale: f32,
    pub path: String,
}
fn default_scale() -> f32 {
    1.0
}

fn create<H: Host>(host: &mut H, a: CreateArgs) -> Result<Value, String> {
    let parent = node(host, a.parent_id)?;
    if !parent.kind.is_container() {
        return Err("parent_id must identify a container".into());
    }
    let index = parent.children.len();
    geometry(a.x, a.y, a.width, a.height)?;
    if let Some(name) = &a.name {
        bounded_text("name", name, 1024)?;
    }
    if let Some(text) = &a.text {
        bounded_text("text", text, 100_000)?;
    }
    if a.kind != "text" && a.text.is_some() {
        return Err("text can only be set on text nodes".into());
    }
    let rect = Rect::new(a.x, a.y, a.width, a.height);
    // Allocate only after every argument has passed validation.
    let constructor: fn(NodeId, Rect) -> Node = match a.kind.as_str() {
        "frame" => Node::frame,
        "rectangle" => Node::rectangle,
        "ellipse" => Node::ellipse,
        "text" => |id, r| Node::text(id, r, "", TextStyle::default()),
        _ => return Err("kind must be frame, rectangle, ellipse, or text".into()),
    };
    // Imported native files may carry a stale allocation counter. Derive a
    // fresh ID from the actual nodes instead of risking an overwrite.
    let id = host.alloc_id()?;
    let mut node = constructor(id, rect);
    if let Some(name) = a.name {
        node.name = name;
    }
    if let (Some(text), NodeKind::Text(props)) = (a.text, &mut node.kind) {
        props.content = text;
    }
    host.exec(Command::Insert { parent: NodeId(a.parent_id), index, nodes: vec![node] })?;
    Ok(json!({"node":host.doc().get(id),"changed":true}))
}

fn update<H: Host>(host: &mut H, a: UpdateArgs) -> Result<Value, String> {
    let mut node = node(host, a.node_id)?.clone();
    if node.parent.is_none() {
        return Err("page roots cannot be updated by this tool".into());
    }
    if let Some(name) = a.name {
        bounded_text("name", &name, 1024)?;
        node.name = name;
    }
    if let Some(x) = a.x {
        node.transform.tx = x;
    }
    if let Some(y) = a.y {
        node.transform.ty = y;
    }
    if let Some(width) = a.width {
        node.size.x = width;
    }
    if let Some(height) = a.height {
        node.size.y = height;
    }
    geometry(node.transform.tx, node.transform.ty, node.size.x, node.size.y)?;
    if let Some(visible) = a.visible {
        node.visible = visible;
    }
    if let Some(opacity) = a.opacity {
        if !opacity.is_finite() || !(0.0..=1.0).contains(&opacity) {
            return Err("opacity must be between 0 and 1".into());
        }
        node.opacity = opacity;
    }
    if let Some(text) = a.text {
        bounded_text("text", &text, 100_000)?;
        let NodeKind::Text(props) = &mut node.kind else {
            return Err("text can only be set on text nodes".into());
        };
        if props.content != text {
            props.content = text;
            props.runs.clear();
            props.lists.clear();
        }
    }
    let changed = host.doc().get(node.id) != Some(&node);
    if changed {
        host.exec(Command::Update { node: Box::new(node) })?;
    }
    Ok(json!({"node":host.doc().get(NodeId(a.node_id)),"changed":changed}))
}

pub fn summary(n: &Node) -> Value {
    json!({"node_id":n.id,"name":n.name,"kind":n.kind.default_name(),"parent_id":n.parent,
        "child_count":n.children.len(),"visible":n.visible,"x":n.transform.tx,"y":n.transform.ty,
        "width":n.size.x,"height":n.size.y})
}

fn parse<T: DeserializeOwned>(args: Value) -> Result<T, String> {
    serde_json::from_value(args).map_err(|e| format!("invalid arguments: {e}"))
}
fn bounded_text(field: &str, value: &str, max: usize) -> Result<(), String> {
    if value.len() > max {
        Err(format!("{field} exceeds {max} UTF-8 bytes"))
    } else {
        Ok(())
    }
}
fn geometry(x: f32, y: f32, width: f32, height: f32) -> Result<(), String> {
    if [x, y, width, height].iter().any(|v| !v.is_finite() || v.abs() > 1_000_000.0) {
        return Err("geometry must be finite and within +/-1000000".into());
    }
    if width < 0.0 || height < 0.0 {
        return Err("width and height must be non-negative".into());
    }
    Ok(())
}
fn default_limit() -> usize {
    100
}
fn default_size() -> f32 {
    100.0
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Empty {}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NodeArgs {
    node_id: u64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListArgs {
    parent_id: Option<u64>,
    query: Option<String>,
    #[serde(default)]
    offset: usize,
    #[serde(default = "default_limit")]
    limit: usize,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VariablesArgs {
    node_id: Option<u64>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CodeArgs {
    node_id: u64,
    language: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateArgs {
    parent_id: u64,
    kind: String,
    name: Option<String>,
    text: Option<String>,
    #[serde(default)]
    x: f32,
    #[serde(default)]
    y: f32,
    #[serde(default = "default_size")]
    width: f32,
    #[serde(default = "default_size")]
    height: f32,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateArgs {
    node_id: u64,
    name: Option<String>,
    text: Option<String>,
    x: Option<f32>,
    y: Option<f32>,
    width: Option<f32>,
    height: Option<f32>,
    visible: Option<bool>,
    opacity: Option<f32>,
}

/// JSON tool descriptors kept beside the argument validators.
pub fn tools() -> Vec<Value> {
    let id = json!({"type":"integer","minimum":1,"description":"Desigma numeric node ID; obtain from list_nodes/get_document."});
    let coordinate = json!({"type":"number","minimum":-1000000,"maximum":1000000});
    let size = json!({"type":"number","minimum":0,"maximum":1000000});
    let text = json!({"type":"string","maxLength":100000,"description":"Plain text; limited to 100000 UTF-8 bytes."});
    let name = json!({"type":"string","maxLength":1024,"description":"Layer name; limited to 1024 UTF-8 bytes."});
    let mut out = Vec::new();
    let mut add = |tool: &str, description: &str, properties: Value, required: Vec<&str>, read_only: bool, idempotent: bool| {
        out.push(json!({"name":format!("desigma_{tool}"),"description":description,
            "inputSchema":{"type":"object","properties":properties,"required":required,"additionalProperties":false},
            "annotations":{"readOnlyHint":read_only,"destructiveHint":!read_only,"idempotentHint":idempotent,"openWorldHint":false}}));
    };
    add("get_document", "Summarize the current document, pages, counts, and undo state. session is \"file\" or \"editor\"; the editor adds its file path and has_undo (its undo stack is non-empty, which stays true after the user saves).", json!({}), vec![], true, true);
    add(
        "list_nodes",
        "List node summaries in numeric ID order. Query matches names case-insensitively; parent_id filters direct children. Use next_offset to paginate.",
        json!({"parent_id":id,"query":{"type":"string","maxLength":1024},"offset":{"type":"integer","minimum":0,"default":0},"limit":{"type":"integer","minimum":1,"maximum":500,"default":100}}),
        vec![],
        true,
        true,
    );
    add("get_node", "Read the full native node including geometry, paints, text, bindings, and child IDs.", json!({"node_id":id}), vec!["node_id"], true, true);
    add(
        "get_variables",
        "Read variable definitions and collections. With node_id, also resolve that node's direct bindings using inherited collection modes.",
        json!({"node_id":id}),
        vec![],
        true,
        true,
    );
    add(
        "generate_code",
        "Generate developer handoff code or SVG for a layer using resolved variable values. Returns source text; writes no files.",
        json!({"node_id":id,"language":{"type":"string","enum":["css","tailwind","react","swiftui","compose","svg"]}}),
        vec!["node_id", "language"],
        true,
        true,
    );
    add("create_node","Create one layer in a container; defaults to x/y=0, width/height=100. Supports undo. Text is only valid for text layers. Changes are in memory until saved.",
        json!({"parent_id":id,"kind":{"type":"string","enum":["frame","rectangle","ellipse","text"]},"name":name,"text":text,"x":coordinate,"y":coordinate,"width":size,"height":size}),vec!["parent_id","kind"],false,false);
    add("update_node","Update specified fields of one non-page layer. Coordinates are parent-relative. Replacing text clears text runs and list ranges. Supports undo; changes remain in memory until saved.",
        json!({"node_id":id,"name":name,"text":text,"x":coordinate,"y":coordinate,"width":size,"height":size,"visible":{"type":"boolean"},"opacity":{"type":"number","minimum":0,"maximum":1}}),vec!["node_id"],false,true);
    add(
        "delete_node",
        "Remove a non-page layer and its entire subtree. Supports undo; changes remain in memory until saved.",
        json!({"node_id":id}),
        vec!["node_id"],
        false,
        false,
    );
    add("undo", "Undo the latest successful edit; returns changed=false if the stack is empty. In the editor the stack is shared with the user, so this may undo their latest edit.", json!({}), vec![], false, false);
    add(
        "redo",
        "Redo the most recently undone edit (in the editor, shared with the user); returns changed=false if the stack is empty.",
        json!({}),
        vec![],
        false,
        false,
    );
    add(
        "export_image",
        "Render one layer to a PNG file inside the workspace (transparent background, effects included). Overwrites an existing .png.",
        json!({"node_id":id,"scale":{"type":"number","minimum":0.25,"maximum":4,"default":1},"path":{"type":"string","description":"Relative .png path inside the workspace; parent directory must exist."}}),
        vec!["node_id", "path"],
        false,
        true,
    );
    out.extend(crate::motion::tools());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variable_inspection_and_codegen_resolve_inherited_cross_collection_aliases() {
        use desigma_doc::{Variable, VariableCollection, VariableKind, VariableMode, VariableValue};
        let mut e = Engine::default();
        let id = create(&mut e, "rectangle", "Box");
        for collection in ["alias", "size"] {
            e.doc.variable_collections.insert(
                collection.into(),
                VariableCollection {
                    id: collection.into(),
                    name: collection.into(),
                    modes: vec![VariableMode { id: "light".into(), name: "Light".into() }, VariableMode { id: "dark".into(), name: "Dark".into() }],
                    default_mode: "light".into(),
                    variables: vec![],
                    key: None,
                },
            );
        }
        for (id, collection, light, dark) in [
            ("width", "size", VariableValue::Number(100.0), VariableValue::Number(240.0)),
            ("bound", "alias", VariableValue::Alias("width".into()), VariableValue::Alias("width".into())),
        ] {
            e.doc.variables.insert(
                id.into(),
                Variable {
                    id: id.into(),
                    name: id.into(),
                    collection: collection.into(),
                    kind: VariableKind::Number,
                    values: [("light".into(), light), ("dark".into(), dark)].into_iter().collect(),
                    description: String::new(),
                    scopes: vec![],
                    hidden_from_publishing: false,
                    key: None,
                },
            );
        }
        e.doc.get_mut(NodeId(1)).unwrap().variables.modes.insert("size".into(), "dark".into());
        e.doc.get_mut(NodeId(id)).unwrap().variables.bindings.insert("width".into(), "bound".into());
        let before = e.doc.to_json().unwrap();
        let values = e.call("desigma_get_variables", json!({"node_id":id})).unwrap();
        assert_eq!(values["effective_bindings"][0]["value"], json!({"Number":240.0}));
        let svg = e.call("desigma_generate_code", json!({"node_id":id,"language":"svg"})).unwrap();
        assert!(svg["code"].as_str().unwrap().contains("width=\"240\""));
        assert_eq!(e.doc.to_json().unwrap(), before);
    }
    fn create(e: &mut Engine, kind: &str, name: &str) -> u64 {
        e.call("desigma_create_node", json!({"parent_id":1,"kind":kind,"name":name})).unwrap()["node"]["id"].as_u64().unwrap()
    }
    #[test]
    fn crud_round_trips_through_undo_and_redo() {
        let mut e = Engine::default();
        let id = create(&mut e, "text", "Title");
        let before = e.doc.to_json().unwrap();
        e.call("desigma_update_node", json!({"node_id":id,"text":"Hello","x":17,"visible":false,"opacity":0.5})).unwrap();
        assert_eq!(e.doc.get(NodeId(id)).unwrap().transform.tx, 17.0);
        e.call("desigma_undo", json!({})).unwrap();
        assert_eq!(e.doc.to_json().unwrap(), before);
        e.call("desigma_redo", json!({})).unwrap();
        assert_eq!(e.doc.get(NodeId(id)).unwrap().kind.any_text().unwrap().content, "Hello");
        e.call("desigma_delete_node", json!({"node_id":id})).unwrap();
        assert!(e.doc.get(NodeId(id)).is_none());
        e.call("desigma_undo", json!({})).unwrap();
        assert!(e.doc.get(NodeId(id)).is_some());
    }
    #[test]
    fn invalid_mutations_leave_document_and_history_unchanged() {
        let mut e = Engine::default();
        let id = create(&mut e, "rectangle", "Box");
        let before = e.doc.to_json().unwrap();
        for (name, args) in [
            ("desigma_create_node", json!({"parent_id":1,"kind":"bad"})),
            ("desigma_create_node", json!({"parent_id":id,"kind":"frame"})),
            ("desigma_create_node", json!({"parent_id":1,"kind":"text","width":-1})),
            ("desigma_update_node", json!({"node_id":id,"name":"changed","text":"invalid"})),
            ("desigma_update_node", json!({"node_id":id,"x":1e100})),
            ("desigma_update_node", json!({"node_id":id,"opacity":2})),
            ("desigma_update_node", json!({"node_id":id,"unknown":true})),
            ("desigma_delete_node", json!({"node_id":1})),
            ("desigma_delete_node", json!({"node_id":0})),
        ] {
            assert!(e.call(name, args).is_err(), "{name}");
            assert_eq!(e.doc.to_json().unwrap(), before);
        }
        e.call("desigma_undo", json!({})).unwrap();
        assert!(!e.history.can_undo());
        assert!(e.doc.get(NodeId(id)).is_none());
    }
    #[test]
    fn name_filter_pagination_and_code_generation() {
        let mut e = Engine::default();
        let id = create(&mut e, "frame", "Card");
        create(&mut e, "rectangle", "card background");
        create(&mut e, "ellipse", "Icon");
        let result = e.call("desigma_list_nodes", json!({"parent_id":1,"query":"CARD","limit":1})).unwrap();
        assert_eq!(result["total"], 2);
        assert_eq!(result["nodes"][0]["node_id"], id);
        assert_eq!(result["next_offset"], 1);
        for language in ["css", "tailwind", "react", "swiftui", "compose", "svg"] {
            let output = e.call("desigma_generate_code", json!({"node_id":id,"language":language})).unwrap();
            assert!(!output["code"].as_str().unwrap().is_empty());
        }
        assert!(e.call("desigma_list_nodes", json!({"limit":501})).is_err());
        assert!(e.call("desigma_get_node", json!({"node_id":"2"})).is_err());
    }

    /// The node's own stored size (100x100) is tiny, but a variable bound to
    /// `width` resolves to 400000: the megapixel guard must measure the
    /// resolved, post-`apply_bound_variables` size that `render_export` will
    /// actually allocate, not the raw document's.
    #[test]
    fn export_image_rejects_pixel_counts_from_resolved_variable_bindings() {
        use desigma_doc::{Variable, VariableCollection, VariableKind, VariableMode, VariableValue};
        let mut e = Engine::default();
        let id = create(&mut e, "rectangle", "Big");
        e.doc.variable_collections.insert(
            "size".into(),
            VariableCollection {
                id: "size".into(),
                name: "size".into(),
                modes: vec![VariableMode { id: "m".into(), name: "M".into() }],
                default_mode: "m".into(),
                variables: vec![],
                key: None,
            },
        );
        e.doc.variables.insert(
            "big".into(),
            Variable {
                id: "big".into(),
                name: "big".into(),
                collection: "size".into(),
                kind: VariableKind::Number,
                values: [("m".into(), VariableValue::Number(400_000.0))].into_iter().collect(),
                description: String::new(),
                scopes: vec![],
                hidden_from_publishing: false,
                key: None,
            },
        );
        e.doc.get_mut(NodeId(id)).unwrap().variables.bindings.insert("width".into(), "big".into());
        let err = export_image(&e, id, 1.0).unwrap_err();
        assert!(err.contains("32 megapixel"), "{err}");
    }

    /// A host that counts commands proves `call` does not depend on `Engine`'s own history.
    #[test]
    fn free_call_runs_on_any_host() {
        struct Counting {
            e: Engine,
            execs: usize,
        }
        impl Host for Counting {
            fn doc(&self) -> &Document {
                &self.e.doc
            }
            fn exec(&mut self, cmd: Command) -> Result<(), String> {
                self.execs += 1;
                self.e.exec(cmd)
            }
            fn undo(&mut self) -> Result<bool, String> {
                self.e.undo()
            }
            fn redo(&mut self) -> Result<bool, String> {
                self.e.redo()
            }
            fn can_undo(&self) -> bool {
                self.e.can_undo()
            }
            fn can_redo(&self) -> bool {
                self.e.can_redo()
            }
            fn alloc_id(&mut self) -> Result<NodeId, String> {
                self.e.alloc_id()
            }
        }
        let mut h = Counting { e: Engine::default(), execs: 0 };
        let id = call(&mut h, "desigma_create_node", json!({"parent_id":1,"kind":"frame","name":"A"})).unwrap()["node"]["id"].as_u64().unwrap();
        call(&mut h, "desigma_update_node", json!({"node_id":id,"x":10.0})).unwrap();
        assert_eq!(h.execs, 2);
        assert!(call(&mut h, "desigma_undo", json!({})).unwrap()["changed"].as_bool().unwrap());
    }

    #[test]
    fn export_image_is_annotated_as_a_writing_idempotent_tool() {
        let t = tools().into_iter().find(|t| t["name"] == "desigma_export_image").unwrap();
        assert_eq!(t["annotations"]["readOnlyHint"], false);
        assert_eq!(t["annotations"]["idempotentHint"], true);
    }
}
