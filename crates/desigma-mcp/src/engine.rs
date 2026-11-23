//! Typed, local document operations shared by the MCP transport and future bridges.
use desigma_doc::{
    AutoLayout, Color, Command, Document, Effect, History, LayoutAlign, LayoutDirection, LineHeight, Node, NodeId, NodeKind, Paint, Rect, Sizing, Style,
    StyleMeta, StyleProperty, TextAlignH, TextAlignV, TextAutoResize, TextStyle, VariableKind, VariableStore, VariableValue, Vec2,
};
use desigma_svg::ImportOptions;
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
    /// Direct document access for two things undo does not record: the
    /// content-addressed bitmap bytes an imported SVG carries, and the node id
    /// reservation the `desigma_doc` component helpers do before they return
    /// their command. Callers must always follow it with an undoable insert.
    fn doc_mut(&mut self) -> &mut Document;
    /// Runs a command the component helpers built (a sync, a detach). Its updates are the
    /// main component's state arriving at the instances, not edits by the caller, so they must
    /// not be recorded as overrides the way [`Host::exec`] records an edit.
    fn exec_derived(&mut self, cmd: Command) -> Result<(), String> {
        self.exec(cmd)
    }
}

impl Host for Engine {
    fn doc(&self) -> &Document {
        &self.doc
    }
    fn exec(&mut self, cmd: Command) -> Result<(), String> {
        // Same preparation the editor does before committing: mirrored-node
        // edits become overrides, main-component edits re-sync their instances.
        let cmd = desigma_engine::components::prepare(&mut self.doc, cmd);
        self.history.exec(&mut self.doc, cmd).map_err(|e| e.to_string())
    }
    fn exec_derived(&mut self, cmd: Command) -> Result<(), String> {
        let cmd = desigma_engine::components::with_sync(&mut self.doc, cmd);
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
    fn doc_mut(&mut self) -> &mut Document {
        &mut self.doc
    }
}

/// A host that applies each call to a scratch document and records its command, so a
/// sequence of node tools can be validated before one `Command::Batch` reaches the real
/// document and the undo stack.
struct BatchHost<'a> {
    doc: &'a mut Document,
    commands: Vec<Command>,
}

impl Host for BatchHost<'_> {
    fn doc(&self) -> &Document {
        self.doc
    }
    fn exec(&mut self, cmd: Command) -> Result<(), String> {
        let original = cmd.clone();
        cmd.apply(self.doc).map_err(|e| e.to_string())?;
        self.commands.push(original);
        Ok(())
    }
    fn undo(&mut self) -> Result<bool, String> {
        Err("undo cannot run inside a batch".into())
    }
    fn redo(&mut self) -> Result<bool, String> {
        Err("redo cannot run inside a batch".into())
    }
    fn can_undo(&self) -> bool {
        false
    }
    fn can_redo(&self) -> bool {
        false
    }
    fn alloc_id(&mut self) -> Result<NodeId, String> {
        self.doc.try_alloc_id().ok_or_else(|| "Document has exhausted node IDs".into())
    }
    fn doc_mut(&mut self) -> &mut Document {
        self.doc
    }
}

impl Engine {
    pub fn call(&mut self, name: &str, args: Value) -> Result<Value, String> {
        call(self, name, args)
    }
}

/// Effect variants exposed by `desigma_set_effects`.
pub const EFFECT_TYPES: &[&str] = &["drop_shadow", "inner_shadow", "layer_blur", "background_blur"];

/// Node-edit tools that `desigma_batch` may compose into one undo step.
pub const BATCHABLE: &[&str] = &[
    "desigma_create_node",
    "desigma_duplicate_node",
    "desigma_update_node",
    "desigma_move_node",
    "desigma_set_appearance",
    "desigma_set_effects",
    "desigma_set_text_style",
    "desigma_set_auto_layout",
    "desigma_set_layout_child",
    "desigma_delete_node",
];

/// Every engine tool that edits the document. Transports use it to track unsaved edits and the
/// editor bridge to refuse edits mid-gesture, so a new editing tool must be listed here.
pub const EDIT_TOOLS: &[&str] = &[
    "desigma_batch",
    "desigma_create_node",
    "desigma_duplicate_node",
    "desigma_create_page",
    "desigma_update_node",
    "desigma_move_node",
    "desigma_set_appearance",
    "desigma_set_effects",
    "desigma_set_text_style",
    "desigma_create_style",
    "desigma_apply_style",
    "desigma_detach_style",
    "desigma_set_auto_layout",
    "desigma_set_layout_child",
    "desigma_create_variable_collection",
    "desigma_add_variable_mode",
    "desigma_create_variable",
    "desigma_set_variable_value",
    "desigma_bind_variable",
    "desigma_detach_variable",
    "desigma_set_variable_mode",
    "desigma_import_svg",
    "desigma_delete_node",
    "desigma_delete_page",
    // Component, instance and page-rename tools also change the document, so a
    // transport must treat them as edits and the editor bridge must not let
    // them run mid-gesture.
    "desigma_create_component",
    "desigma_create_instance",
    "desigma_detach_instance",
    "desigma_sync_instances",
    "desigma_rename_page",
    "desigma_undo",
    "desigma_redo",
    "desigma_add_keyframe",
    "desigma_apply_animation_preset",
    "desigma_set_timeline",
];

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
        "desigma_create_variable_collection" => create_variable_collection(host, parse(args)?),
        "desigma_add_variable_mode" => add_variable_mode(host, parse(args)?),
        "desigma_create_variable" => create_variable(host, parse(args)?),
        "desigma_set_variable_value" => set_variable_value(host, parse(args)?),
        "desigma_bind_variable" => bind_variable(host, parse(args)?),
        "desigma_detach_variable" => detach_variable(host, parse(args)?),
        "desigma_set_variable_mode" => set_variable_mode(host, parse(args)?),
        "desigma_batch" => batch(host, parse(args)?),
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
                "flutter" => desigma_export::flutter_for(&doc, id),
                "vue" => desigma_export::vue_for(&doc, id),
                "svg" => desigma_export::export_svg(&doc, id, &Default::default()),
                _ => return Err("language must be css, tailwind, react, swiftui, compose, flutter, vue, or svg".into()),
            };
            Ok(json!({"node_id":id,"language":a.language,"code":code}))
        }
        "desigma_create_node" => create(host, parse(args)?),
        "desigma_duplicate_node" => duplicate(host, parse(args)?),
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
            // `Command::Remove` walks up to the parent to splice the node out, and a
            // page root has none — it fails with `NotFound(id)`. `RemovePage` is the
            // page-shaped edit: it drops the id from `Document::pages` and the subtree
            // from `Document::nodes`, and undoes as `InsertPage`.
            host.exec(Command::RemovePage { id })?;
            Ok(json!({"page_id":id,"deleted":true}))
        }
        "desigma_move_node" => {
            let a: MoveNodeArgs = parse(args)?;
            move_node(host, a)
        }
        "desigma_create_page" => {
            let a: CreatePageArgs = parse(args)?;
            create_page(host, a)
        }
        "desigma_set_appearance" => {
            let a: AppearanceArgs = parse(args)?;
            set_appearance(host, a)
        }
        "desigma_set_effects" => set_effects(host, parse(args)?),
        "desigma_set_text_style" => {
            let a: TextStyleArgs = parse(args)?;
            set_text_style(host, a)
        }
        "desigma_create_style" => create_style(host, parse(args)?),
        "desigma_apply_style" => apply_style(host, parse(args)?),
        "desigma_detach_style" => detach_style(host, parse(args)?),
        "desigma_set_auto_layout" => set_auto_layout(host, parse(args)?),
        "desigma_set_layout_child" => set_layout_child(host, parse(args)?),
        "desigma_import_svg" => import_svg(host, parse(args)?),
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
        // ---- Components, instances and page rename (thin wrappers over `desigma_doc::components`) ----
        "desigma_create_component" => {
            let a: CreateComponentArgs = parse(args)?;
            let target = node(host, a.node_id)?.clone();
            if !matches!(target.kind, NodeKind::Frame(_) | NodeKind::Group | NodeKind::Section) {
                return Err("only frame / group / section layers can become a component".into());
            }
            if target.component.source.is_some() {
                return Err("a mirrored layer inside an instance cannot become a component; detach the instance first".into());
            }
            // Validate the name before converting, so a refused name leaves the layer as it was.
            if let Some(name) = &a.name {
                bounded_text("name", name, 1024)?;
            }
            let mut cmd = desigma_doc::create_component(host.doc_mut(), NodeId(a.node_id));
            if let Some(name) = a.name {
                // These kinds convert in place, so the rename joins the same undo step.
                let Command::Update { node } = &mut cmd else {
                    return Err("layer did not convert in place".into());
                };
                node.name = name;
            }
            host.exec(cmd)?;
            Ok(json!({"component_id":a.node_id,"node":host.doc().get(NodeId(a.node_id))}))
        }
        "desigma_create_instance" => {
            let a: CreateInstanceArgs = parse(args)?;
            if node(host, a.parent_id)?.component.source.is_some() {
                return Err("a mirrored layer inside an instance cannot receive an instance; detach the instance first".into());
            }
            let at = desigma_doc::Vec2::new(a.x, a.y);
            let cmd = desigma_doc::create_instance(host.doc_mut(), NodeId(a.component_id), NodeId(a.parent_id), at);
            let instance_id = match &cmd {
                Command::Insert { nodes, .. } => nodes.first().map(|n| n.id.0),
                _ => None,
            };
            let Some(instance_id) = instance_id else {
                return Err("cannot instantiate: component must be a Component and parent must be a container that does not contain it".into());
            };
            host.exec(cmd)?;
            Ok(json!({"instance_id":instance_id,"component_id":a.component_id}))
        }
        "desigma_detach_instance" => {
            let a: NodeArgs = parse(args)?;
            let kind = node(host, a.node_id)?.kind.clone();
            if !matches!(kind, NodeKind::Instance { .. }) {
                return Err("only instances can be detached".into());
            }
            let cmd = desigma_doc::detach_instance(host.doc_mut(), NodeId(a.node_id));
            host.exec_derived(cmd)?;
            Ok(json!({"node_id":a.node_id,"detached":true}))
        }
        "desigma_sync_instances" => {
            let a: SyncArgs = parse(args)?;
            let kind = node(host, a.component_id)?.kind.clone();
            if !matches!(kind, NodeKind::Component(_) | NodeKind::ComponentSet(_)) {
                return Err(format!("node {} is not a main component or component set", a.component_id));
            }
            let cmd = desigma_doc::sync_instances(host.doc_mut(), NodeId(a.component_id));
            // Nothing to follow: executing an empty batch would push an empty undo
            // step and mark the session dirty, so report it as unchanged.
            if is_noop(&cmd) {
                return Ok(json!({"component_id":a.component_id,"synced":true,"changed":false}));
            }
            host.exec_derived(cmd)?;
            Ok(json!({"component_id":a.component_id,"synced":true,"changed":true}))
        }
        "desigma_rename_page" => {
            let a: RenamePageArgs = parse(args)?;
            let name = a.name.trim();
            if name.is_empty() {
                return Err("name must not be empty".into());
            }
            bounded_text("name", name, 1024)?;
            let mut page = node(host, a.page_id)?.clone();
            if !matches!(page.kind, NodeKind::Page { .. }) || page.parent.is_some() {
                return Err(format!("node {} is not a page", a.page_id));
            }
            page.name = name.to_string();
            // Page roots go through `Command::Update` too, so a rename is undoable.
            host.exec(Command::Update { node: Box::new(page) })?;
            Ok(json!({"page_id":a.page_id,"name":name}))
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

/// Copy a non-page layer and its subtree in place, right after the source.
///
/// Structural edits inside an instance are not part of the component override model, so
/// those layers are refused rather than copied and silently lost on the next sync.
fn duplicate<H: Host>(host: &mut H, a: DuplicateArgs) -> Result<Value, String> {
    let src = node(host, a.node_id)?.clone();
    let parent_id = src.parent.ok_or("page roots cannot be duplicated by this tool")?;
    let parent = node(host, parent_id.0)?.clone();
    if !parent.kind.is_container() {
        return Err("duplicate parent must be a container".into());
    }
    if let Some(instance) = desigma_doc::instance_root_of(host.doc(), src.id) {
        if instance != src.id {
            return Err("a layer inside an instance cannot be duplicated; duplicate the instance or edit the main component".into());
        }
    }
    if let Some(offset) = a.offset {
        if !offset.is_finite() || !(0.0..=100_000.0).contains(&offset) {
            return Err("offset must be finite and between 0 and 100000".into());
        }
    }
    let auto_layout = parent.kind.frame_props().is_some_and(|f| f.auto_layout.is_some());
    let offset = a.offset.unwrap_or(if auto_layout { 0.0 } else { 10.0 });
    let index = parent.children.iter().position(|c| *c == src.id).map_or(parent.children.len(), |i| i + 1).min(parent.children.len());
    // duplicate_subtree reserves fresh ids on the document; the Insert below is the
    // undoable edit that puts the copies into the tree.
    let mut nodes = desigma_doc::duplicate_subtree(host.doc_mut(), src.id);
    let count = nodes.len();
    let root = nodes.first_mut().ok_or("cannot duplicate this layer")?;
    let new_id = root.id.0;
    let name = root.name.clone();
    if offset != 0.0 {
        root.transform.tx += offset;
        root.transform.ty += offset;
    }
    host.exec(Command::Insert { parent: parent_id, index, nodes })?;
    Ok(json!({"node_id":new_id,"name":name,"parent_id":parent_id.0,"index":index,"nodes":count,"changed":true}))
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

/// Move one layer under another container, keeping its parent-relative transform.
///
/// Refuses page roots, the layer's own subtree and non-container targets, so the
/// document can never be put into a cycle. Supports undo.
fn move_node<H: Host>(host: &mut H, a: MoveNodeArgs) -> Result<Value, String> {
    let moving = node(host, a.node_id)?.clone();
    if moving.parent.is_none() {
        return Err("page roots cannot be moved by this tool".into());
    }
    if a.node_id == a.parent_id {
        return Err("a layer cannot be moved into itself".into());
    }
    let target = node(host, a.parent_id)?;
    if !target.kind.is_container() {
        return Err(format!("node {} is not a container", a.parent_id));
    }
    if host.doc().is_ancestor(NodeId(a.node_id), NodeId(a.parent_id)) {
        return Err("a layer cannot be moved into its own subtree".into());
    }
    let index = a.index.unwrap_or(target.children.len()).min(target.children.len());
    if moving.parent == Some(NodeId(a.parent_id)) && target.children.get(index) == Some(&NodeId(a.node_id)) {
        return Ok(json!({"node_id":a.node_id,"parent_id":a.parent_id,"index":index,"changed":false}));
    }
    host.exec(Command::Reparent { id: NodeId(a.node_id), parent: NodeId(a.parent_id), index })?;
    Ok(json!({"node_id":a.node_id,"parent_id":a.parent_id,"index":index,"changed":true}))
}

/// Append a new empty page and return it, ready to receive layers.
///
/// Pages live in `Document::pages`, not under another node, so this uses
/// `Command::InsertPage` and stays undoable like every other edit.
fn create_page<H: Host>(host: &mut H, a: CreatePageArgs) -> Result<Value, String> {
    let name = a.name.trim();
    if name.is_empty() {
        return Err("name must not be empty".into());
    }
    bounded_text("name", name, 1024)?;
    let background = match a.background.as_deref() {
        Some(spec) => colour(spec, "background")?,
        None => Color::from_hex("#ffffff").expect("white is a valid colour"),
    };
    let index = host.doc().pages.len();
    // Allocate only after every argument has passed validation.
    let id = host.alloc_id()?;
    let mut page = Node::new(id, NodeKind::Page { background });
    page.name = name.to_string();
    host.exec(Command::InsertPage { index, nodes: vec![page] })?;
    Ok(json!({"page_id":id,"index":index,"name":name,"changed":true}))
}

/// Restyle a layer's fill, stroke, corner radius and opacity.
///
/// A text layer keeps its colour in `TextStyle::fills`, so a `fill` on a text node is
/// written there rather than into the unused `Node::fills`. Supports undo.
fn set_appearance<H: Host>(host: &mut H, a: AppearanceArgs) -> Result<Value, String> {
    let mut node = node(host, a.node_id)?.clone();
    if node.parent.is_none() {
        return Err("page roots cannot be restyled by this tool".into());
    }
    let mut applied: Vec<&str> = Vec::new();
    if let Some(spec) = a.fill.as_deref() {
        let paints = if spec.trim().eq_ignore_ascii_case("none") { Vec::new() } else { vec![Paint::solid(colour(spec, "fill")?)] };
        match &mut node.kind {
            NodeKind::Text(props) => props.style.fills = paints,
            _ => node.fills = paints,
        }
        applied.push("fill");
    }
    if let Some(spec) = a.stroke.as_deref() {
        node.stroke.paints = if spec.trim().eq_ignore_ascii_case("none") { Vec::new() } else { vec![Paint::solid(colour(spec, "stroke")?)] };
        if let Some(width) = a.stroke_width {
            node.stroke.weight = stroke_weight(width)?;
        }
        applied.push("stroke");
    } else if let Some(width) = a.stroke_width {
        node.stroke.weight = stroke_weight(width)?;
        applied.push("stroke_width");
    }
    if let Some(radius) = a.corner_radius {
        if !radius.is_finite() || !(0.0..=1_000_000.0).contains(&radius) {
            return Err("corner_radius must be finite and between 0 and 1000000".into());
        }
        node.corner_radii = [radius; 4];
        applied.push("corner_radius");
    }
    if let Some(opacity) = a.opacity {
        if !opacity.is_finite() || !(0.0..=1.0).contains(&opacity) {
            return Err("opacity must be between 0 and 1".into());
        }
        node.opacity = opacity;
        applied.push("opacity");
    }
    commit(host, node, applied)
}

/// Set a text layer's content and typography (size, weight, family, line height,
/// letter spacing, alignment). Supports undo.
fn set_text_style<H: Host>(host: &mut H, a: TextStyleArgs) -> Result<Value, String> {
    let mut node = node(host, a.node_id)?.clone();
    let mut applied: Vec<&str> = Vec::new();
    let text = match &mut node.kind {
        NodeKind::Text(props) => props,
        _ => return Err("node is not a text node".into()),
    };
    if let Some(content) = a.text.as_deref() {
        bounded_text("text", content, 100_000)?;
        if text.content != content {
            text.content = content.to_string();
            text.runs.clear();
            text.lists.clear();
        }
        applied.push("text");
    }
    if let Some(size) = a.font_size {
        if !size.is_finite() || !(1.0..=1000.0).contains(&size) {
            return Err("font_size must be within 1..=1000".into());
        }
        text.style.font_size = size;
        applied.push("font_size");
    }
    if let Some(weight) = a.font_weight {
        if !(100..=900).contains(&weight) || weight % 100 != 0 {
            return Err("font_weight must be 100..=900 in steps of 100".into());
        }
        text.style.font_weight = weight;
        applied.push("font_weight");
    }
    if let Some(family) = a.font_family.as_deref() {
        if family.trim().is_empty() {
            return Err("font_family must not be empty".into());
        }
        bounded_text("font_family", family, 256)?;
        text.style.font_family = family.to_string();
        applied.push("font_family");
    }
    if let Some(px) = a.line_height {
        if !px.is_finite() || !(0.0..=1000.0).contains(&px) {
            return Err("line_height must be within 0..=1000".into());
        }
        text.style.line_height = LineHeight::Pixels(px);
        applied.push("line_height");
    }
    if let Some(px) = a.letter_spacing {
        if !px.is_finite() || !(-100.0..=100.0).contains(&px) {
            return Err("letter_spacing must be within -100..=100".into());
        }
        text.style.letter_spacing = px;
        applied.push("letter_spacing");
    }
    if let Some(value) = a.align_h.as_deref() {
        text.style.align_h = match value {
            "left" => TextAlignH::Left,
            "center" => TextAlignH::Center,
            "right" => TextAlignH::Right,
            "justified" => TextAlignH::Justified,
            other => return Err(format!("align_h must be left|center|right|justified, got {other}")),
        };
        applied.push("align_h");
    }
    if let Some(value) = a.align_v.as_deref() {
        text.style.align_v = match value {
            "top" => TextAlignV::Top,
            "center" => TextAlignV::Center,
            "bottom" => TextAlignV::Bottom,
            other => return Err(format!("align_v must be top|center|bottom, got {other}")),
        };
        applied.push("align_v");
    }
    if let Some(value) = a.auto_resize.as_deref() {
        text.auto_resize = match value {
            "none" => TextAutoResize::None,
            "width_and_height" => TextAutoResize::WidthAndHeight,
            "height" => TextAutoResize::Height,
            "truncate" => TextAutoResize::Truncate,
            other => return Err(format!("auto_resize must be none|width_and_height|height|truncate, got {other}")),
        };
        applied.push("auto_resize");
    }
    if let Some(lines) = a.max_lines {
        if !(1..=1000).contains(&lines) {
            return Err("max_lines must be within 1..=1000".into());
        }
        text.max_lines = Some(lines);
        applied.push("max_lines");
    }
    commit(host, node, applied)
}

fn reject_style_keys(obj: &serde_json::Map<String, Value>, allowed: &[&str], subject: &str) -> Result<(), String> {
    for key in obj.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(format!("unknown {subject} field '{key}'"));
        }
    }
    Ok(())
}

fn parse_text_style(value: &Value) -> Result<TextStyle, String> {
    let obj = value.as_object().ok_or("text must be an object")?;
    reject_style_keys(obj, &["font_size", "font_weight", "font_family", "line_height", "letter_spacing", "align_h", "align_v"], "text style")?;
    let mut style = TextStyle::default();
    if let Some(value) = obj.get("font_size") {
        let size = value.as_f64().ok_or("font_size must be a number")?;
        if !size.is_finite() || !(1.0..=1000.0).contains(&size) {
            return Err("font_size must be within 1..=1000".into());
        }
        style.font_size = size as f32;
    }
    if let Some(value) = obj.get("font_weight") {
        let weight = value.as_u64().ok_or("font_weight must be an integer")?;
        if !(100..=900).contains(&weight) || weight % 100 != 0 {
            return Err("font_weight must be 100..=900 in steps of 100".into());
        }
        style.font_weight = weight as u16;
    }
    if let Some(value) = obj.get("font_family") {
        let family = value.as_str().ok_or("font_family must be a string")?;
        if family.trim().is_empty() {
            return Err("font_family must not be empty".into());
        }
        bounded_text("font_family", family, 256)?;
        style.font_family = family.to_string();
    }
    if let Some(value) = obj.get("line_height") {
        let px = value.as_f64().ok_or("line_height must be a number")?;
        if !px.is_finite() || !(0.0..=1000.0).contains(&px) {
            return Err("line_height must be within 0..=1000".into());
        }
        style.line_height = LineHeight::Pixels(px as f32);
    }
    if let Some(value) = obj.get("letter_spacing") {
        let px = value.as_f64().ok_or("letter_spacing must be a number")?;
        if !px.is_finite() || !(-100.0..=100.0).contains(&px) {
            return Err("letter_spacing must be within -100..=100".into());
        }
        style.letter_spacing = px as f32;
    }
    if let Some(value) = obj.get("align_h") {
        style.align_h = match value.as_str().ok_or("align_h must be a string")? {
            "left" => TextAlignH::Left,
            "center" => TextAlignH::Center,
            "right" => TextAlignH::Right,
            "justified" => TextAlignH::Justified,
            other => return Err(format!("align_h must be left|center|right|justified, got {other}")),
        };
    }
    if let Some(value) = obj.get("align_v") {
        style.align_v = match value.as_str().ok_or("align_v must be a string")? {
            "top" => TextAlignV::Top,
            "center" => TextAlignV::Center,
            "bottom" => TextAlignV::Bottom,
            other => return Err(format!("align_v must be top|center|bottom, got {other}")),
        };
    }
    Ok(style)
}

fn create_style<H: Host>(host: &mut H, a: CreateStyleArgs) -> Result<Value, String> {
    let name = non_empty_name("name", &a.name)?;
    let description = a.description.unwrap_or_default();
    bounded_text("description", &description, 100_000)?;
    let (style, property) = match a.kind.as_str() {
        "fill" | "stroke" => {
            if a.text.is_some() {
                return Err("text is only valid for kind=text".into());
            }
            let spec = a.color.as_deref().ok_or("color is required for fill/stroke styles")?;
            let color = Color::from_hex(spec).ok_or_else(|| format!("invalid style color '{spec}' (use #RGB, #RRGGBB or #RRGGBBAA)"))?;
            let opacity = a.opacity.unwrap_or(1.0);
            if !opacity.is_finite() || !(0.0..=1.0).contains(&opacity) {
                return Err("opacity must be within 0..=1".into());
            }
            let paint = Paint { opacity, ..Paint::solid(color) };
            let property = if a.kind == "stroke" { StyleProperty::Stroke } else { StyleProperty::Fill };
            (Style::Fill(vec![paint]), property)
        }
        "text" => {
            if a.color.is_some() || a.opacity.is_some() {
                return Err("color/opacity are not valid for kind=text".into());
            }
            let text = a.text.as_ref().ok_or("text is required for kind=text")?;
            (Style::Text(parse_text_style(text)?), StyleProperty::Text)
        }
        other => return Err(format!("kind must be fill, stroke, or text, got {other}")),
    };
    let (key, _) = host.doc().fresh_style_key("MCP style");
    let display_name = name.clone();
    let mut styles = host.doc().styles.clone();
    let mut style_meta = host.doc().style_meta.clone();
    styles.insert(key.clone(), style);
    style_meta.insert(key.clone(), StyleMeta { name, key: None, description, property });
    host.exec(Command::SetStyles { styles, style_meta })?;
    Ok(json!({"style_key":key,"name":display_name,"kind":a.kind,"changed":true}))
}

fn resolve_style(doc: &Document, reference: &str) -> Result<String, String> {
    if doc.styles.contains_key(reference) {
        return Ok(reference.to_string());
    }
    let matches: Vec<String> = doc.style_meta.iter().filter(|(_, meta)| meta.name == reference).map(|(key, _)| key.clone()).collect();
    match matches.as_slice() {
        [key] => Ok(key.clone()),
        [] => Err(format!("style '{reference}' not found")),
        _ => Err(format!("style name '{reference}' is ambiguous; pass a style key")),
    }
}

fn apply_style<H: Host>(host: &mut H, a: ApplyStyleArgs) -> Result<Value, String> {
    let key = resolve_style(host.doc(), &a.style)?;
    let style = host.doc().styles.get(&key).cloned().ok_or_else(|| format!("style '{key}' not found"))?;
    let property = desigma_doc::style_property(host.doc(), &key);
    if matches!(property, StyleProperty::Effect | StyleProperty::Grid) {
        return Err("effect and grid styles are not supported by apply_style yet".into());
    }
    let nodes = target_nodes(host, &a.node_ids)?;
    if property == StyleProperty::Text {
        for id in &nodes {
            if !matches!(node(host, id.0)?.kind, NodeKind::Text(_)) {
                return Err(format!("text style '{key}' requires text layers; node {} is not text", id.0));
            }
        }
    }
    let mut updates = Vec::new();
    for id in &nodes {
        let mut n = node(host, id.0)?.clone();
        match (&style, property) {
            (Style::Fill(paints), StyleProperty::Stroke) => {
                n.stroke.paints = paints.clone();
                n.style_refs.insert("stroke".into(), key.clone());
            }
            (Style::Fill(paints), StyleProperty::Fill) => {
                if let NodeKind::Text(t) = &mut n.kind {
                    t.style.fills = paints.clone();
                } else {
                    n.fills = paints.clone();
                }
                n.style_refs.insert("fill".into(), key.clone());
            }
            (Style::Text(ts), StyleProperty::Text) => {
                if let NodeKind::Text(t) = &mut n.kind {
                    let mut base = ts.clone();
                    base.fills = t.style.fills.clone();
                    t.style = base;
                    for run in &mut t.runs {
                        let mut run_style = ts.clone();
                        run_style.fills = run.style.fills.clone();
                        run.style = run_style;
                    }
                    n.style_refs.insert("text".into(), key.clone());
                }
            }
            _ => {}
        }
        if host.doc().get(*id) != Some(&n) {
            updates.push(Command::Update { node: Box::new(n) });
        }
    }
    let changed = !updates.is_empty();
    if changed {
        host.exec(Command::Batch(updates))?;
    }
    Ok(json!({"node_ids":a.node_ids,"style_key":key,"changed":changed}))
}

fn detach_style<H: Host>(host: &mut H, a: DetachStyleArgs) -> Result<Value, String> {
    let nodes = target_nodes(host, &a.node_ids)?;
    if let Some(property) = a.property.as_deref() {
        if !["fill", "stroke", "text", "effect", "grid"].contains(&property) {
            return Err("property must be fill, stroke, text, effect, or grid".into());
        }
    }
    let mut updates = Vec::new();
    for id in &nodes {
        let mut n = node(host, id.0)?.clone();
        let changed = match a.property.as_deref() {
            Some(property) => n.style_refs.remove(property).is_some(),
            None => {
                !n.style_refs.is_empty() && {
                    n.style_refs.clear();
                    true
                }
            }
        };
        if changed {
            updates.push(Command::Update { node: Box::new(n) });
        }
    }
    let changed = !updates.is_empty();
    if changed {
        host.exec(Command::Batch(updates))?;
    }
    Ok(json!({"node_ids":a.node_ids,"property":a.property,"changed":changed}))
}

fn variable_kind(field: &str) -> Result<VariableKind, String> {
    match field {
        "color" => Ok(VariableKind::Color),
        "number" => Ok(VariableKind::Number),
        "string" => Ok(VariableKind::String),
        "boolean" => Ok(VariableKind::Boolean),
        _ => Err(format!("kind must be color, number, string, or boolean, got {field}")),
    }
}

fn non_empty_name(field: &str, value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(format!("{field} must not be empty"));
    }
    bounded_text(field, value, 1024)?;
    Ok(value.to_string())
}

fn variable_mode_id(store: &VariableStore, collection: &str, key: &str) -> Result<String, String> {
    let c = store.collections.get(collection).ok_or_else(|| format!("variable collection {collection} not found"))?;
    if let Some(mode) = c.modes.iter().find(|m| m.id == key) {
        return Ok(mode.id.clone());
    }
    let matches: Vec<_> = c.modes.iter().filter(|m| m.name == key).collect();
    match matches.as_slice() {
        [mode] => Ok(mode.id.clone()),
        [] => {
            Err(format!("mode '{key}' not found in collection '{}'; modes: {}", c.name, c.modes.iter().map(|m| m.name.as_str()).collect::<Vec<_>>().join(", ")))
        }
        _ => Err(format!("mode name '{key}' is ambiguous in collection '{}'; pass a mode id", c.name)),
    }
}

fn parse_variable_value(kind: VariableKind, value: &Value) -> Result<VariableValue, String> {
    if let Some(obj) = value.as_object() {
        if obj.len() == 1 {
            if let Some(alias) = obj.get("alias") {
                let alias = alias.as_str().ok_or("alias must be a variable id string")?;
                if alias.trim().is_empty() {
                    return Err("alias must not be empty".into());
                }
                return Ok(VariableValue::Alias(alias.to_string()));
            }
        }
        return Err("value must be a literal for the variable kind or {\"alias\":\"variable_id\"}".into());
    }
    match kind {
        VariableKind::Color => {
            let spec = value.as_str().ok_or("color value must be a hex string")?;
            let color = Color::from_hex(spec).ok_or_else(|| format!("invalid color value '{spec}' (use #RGB, #RRGGBB or #RRGGBBAA)"))?;
            Ok(VariableValue::Color(color))
        }
        VariableKind::Number => {
            let number = value.as_f64().ok_or("number value must be a number")?;
            if !number.is_finite() || number.abs() > 1e9 {
                return Err("number value must be finite and within ±1e9".into());
            }
            Ok(VariableValue::Number(number as f32))
        }
        VariableKind::String => Ok(VariableValue::String(value.as_str().ok_or("string value must be a string")?.to_string())),
        VariableKind::Boolean => Ok(VariableValue::Boolean(value.as_bool().ok_or("boolean value must be true or false")?)),
    }
}

fn set_variable_in_store(store: &mut VariableStore, variable: &str, kind: VariableKind, mode: &str, value: &Value) -> Result<(), String> {
    match parse_variable_value(kind, value)? {
        VariableValue::Alias(target) => {
            if store.set_alias(variable, mode, &target) {
                Ok(())
            } else {
                Err("alias must point at a different variable of the same kind without a cycle".into())
            }
        }
        literal => {
            if store.set_value(variable, mode, literal) {
                Ok(())
            } else {
                Err(format!("value does not match variable kind {kind:?}"))
            }
        }
    }
}

/// Installs a working-copy variable store as one undo step, unless it is unchanged.
fn commit_variables<H: Host>(host: &mut H, store: VariableStore) -> Result<bool, String> {
    if store.matches_doc(host.doc()) {
        return Ok(false);
    }
    host.exec(store.command())?;
    Ok(true)
}

fn create_variable_collection<H: Host>(host: &mut H, a: CreateVariableCollectionArgs) -> Result<Value, String> {
    let name = non_empty_name("name", &a.name)?;
    let modes: Vec<String> = match a.modes {
        None => vec!["Mode 1".into()],
        Some(modes) => {
            if modes.is_empty() {
                return Err("modes must contain at least one mode".into());
            }
            let mut out = Vec::new();
            for mode in modes {
                let mode = non_empty_name("mode name", &mode)?;
                if out.contains(&mode) {
                    return Err(format!("duplicate mode name '{mode}'"));
                }
                out.push(mode);
            }
            out
        }
    };
    let mut store = VariableStore::from_doc(host.doc());
    let collection = store.add_collection(&name);
    if let Some(c) = store.collections.get_mut(&collection) {
        if let Some(first) = c.modes.first_mut() {
            first.name = modes[0].clone();
        }
    } else {
        return Err("cannot create variable collection".into());
    }
    for mode in &modes[1..] {
        if store.add_mode(&collection, mode).is_none() {
            return Err(format!("cannot add mode '{mode}'"));
        }
    }
    if let Some(default_mode) = a.default_mode.as_deref() {
        let mode = variable_mode_id(&store, &collection, default_mode)?;
        store.set_default_mode(&collection, &mode);
    }
    let changed = commit_variables(host, store)?;
    Ok(json!({"collection":host.doc().variable_collections.get(&collection),"changed":changed}))
}

fn add_variable_mode<H: Host>(host: &mut H, a: AddVariableModeArgs) -> Result<Value, String> {
    let name = non_empty_name("name", &a.name)?;
    let mut store = VariableStore::from_doc(host.doc());
    let collection = store.collections.get(&a.collection_id).ok_or_else(|| format!("variable collection {} not found", a.collection_id))?;
    if collection.modes.iter().any(|m| m.name == name) {
        return Err(format!("mode '{name}' already exists in collection '{}'", collection.name));
    }
    let mode = store.add_mode(&a.collection_id, &name).ok_or("cannot add variable mode")?;
    let changed = commit_variables(host, store)?;
    let mode = host.doc().variable_collections.get(&a.collection_id).and_then(|c| c.modes.iter().find(|m| m.id == mode));
    Ok(json!({"collection_id":a.collection_id,"mode":mode,"changed":changed}))
}

fn create_variable<H: Host>(host: &mut H, a: CreateVariableArgs) -> Result<Value, String> {
    let name = non_empty_name("name", &a.name)?;
    let kind = variable_kind(&a.kind)?;
    let mut store = VariableStore::from_doc(host.doc());
    let collection_name =
        store.collections.get(&a.collection_id).map(|c| c.name.clone()).ok_or_else(|| format!("variable collection {} not found", a.collection_id))?;
    let variable = store.add_variable(&a.collection_id, &name, kind).ok_or_else(|| format!("cannot create variable in collection '{collection_name}'"))?;
    if let Some(description) = a.description {
        bounded_text("description", &description, 100_000)?;
        if let Some(v) = store.variables.get_mut(&variable) {
            v.description = description;
        }
    }
    if let Some(scopes) = a.scopes {
        let mut out = Vec::new();
        for scope in scopes {
            let scope = scope.trim();
            if scope.is_empty() {
                return Err("scopes must not contain empty strings".into());
            }
            bounded_text("scope", scope, 256)?;
            out.push(scope.to_string());
        }
        if let Some(v) = store.variables.get_mut(&variable) {
            v.scopes = out;
        }
    }
    if let Some(values) = a.values {
        let values = values.as_object().ok_or("values must be an object mapping mode names or ids to values")?;
        let mut seen = Vec::new();
        for (mode_key, value) in values {
            let mode = variable_mode_id(&store, &a.collection_id, mode_key)?;
            if seen.contains(&mode) {
                return Err(format!("mode '{mode_key}' is specified more than once"));
            }
            seen.push(mode.clone());
            set_variable_in_store(&mut store, &variable, kind, &mode, value)?;
        }
    }
    let changed = commit_variables(host, store)?;
    Ok(json!({"variable":host.doc().variables.get(&variable),"changed":changed}))
}

fn set_variable_value<H: Host>(host: &mut H, a: SetVariableValueArgs) -> Result<Value, String> {
    let mut store = VariableStore::from_doc(host.doc());
    let variable = store.variables.get(&a.variable_id).ok_or_else(|| format!("variable {} not found", a.variable_id))?;
    let kind = variable.kind;
    let collection = variable.collection.clone();
    let default_mode = store.collections.get(&collection).map(|c| c.default_mode.clone()).unwrap_or_default();
    let mode = match a.mode.as_deref() {
        Some(key) => variable_mode_id(&store, &collection, key)?,
        None => default_mode,
    };
    set_variable_in_store(&mut store, &a.variable_id, kind, &mode, &a.value)?;
    let changed = commit_variables(host, store)?;
    let value = host.doc().variables.get(&a.variable_id).and_then(|v| v.values.get(&mode));
    Ok(json!({"variable_id":a.variable_id,"mode_id":mode,"value":value,"changed":changed}))
}

/// Validate a variable-tool node list: non-empty, bounded, existing, non-page, deduplicated.
fn target_nodes<H: Host>(host: &H, ids: &[u64]) -> Result<Vec<NodeId>, String> {
    if ids.is_empty() {
        return Err("node_ids must contain at least one node".into());
    }
    if ids.len() > 500 {
        return Err("node_ids accepts at most 500 nodes".into());
    }
    let mut out = Vec::new();
    for id in ids {
        let n = node(host, *id)?;
        if n.parent.is_none() {
            return Err("page roots cannot be targeted by this tool".into());
        }
        if !out.contains(&n.id) {
            out.push(n.id);
        }
    }
    Ok(out)
}

fn bind_variable<H: Host>(host: &mut H, a: BindVariableArgs) -> Result<Value, String> {
    let nodes = target_nodes(host, &a.node_ids)?;
    let expected = desigma_doc::property_kind(&a.property).ok_or_else(|| format!("unknown variable property '{}'", a.property))?;
    let (kind, scopes) =
        host.doc().variables.get(&a.variable_id).map(|v| (v.kind, v.scopes.clone())).ok_or_else(|| format!("variable {} not found", a.variable_id))?;
    if kind != expected {
        return Err(format!("property '{}' accepts {expected:?} variables, not {kind:?}", a.property));
    }
    for id in &nodes {
        let n = node(host, id.0)?;
        if !desigma_doc::scope_allows(&scopes, &a.property, &n.kind) {
            return Err(format!("variable '{}' scope does not allow property '{}' on node {}", a.variable_id, a.property, id.0));
        }
        if host.doc().resolve_variable(*id, &a.variable_id).is_none() {
            return Err(format!("variable '{}' has no value for node {} in its current mode", a.variable_id, id.0));
        }
    }
    let mut updates = Vec::new();
    for id in &nodes {
        let value = host.doc().resolve_variable(*id, &a.variable_id).ok_or_else(|| format!("variable '{}' has no value for node {}", a.variable_id, id.0))?;
        let mut n = node(host, id.0)?.clone();
        n.variables.bindings.insert(a.property.clone(), a.variable_id.clone());
        desigma_doc::set_bound_property(&mut n, &a.property, &value);
        if host.doc().get(*id) != Some(&n) {
            updates.push(Command::Update { node: Box::new(n) });
        }
    }
    let changed = !updates.is_empty();
    if changed {
        host.exec(Command::Batch(updates))?;
    }
    Ok(json!({"node_ids":a.node_ids,"property":a.property,"variable_id":a.variable_id,"changed":changed}))
}

fn detach_variable<H: Host>(host: &mut H, a: DetachVariableArgs) -> Result<Value, String> {
    let nodes = target_nodes(host, &a.node_ids)?;
    if desigma_doc::property_kind(&a.property).is_none() {
        return Err(format!("unknown variable property '{}'", a.property));
    }
    let mut updates = Vec::new();
    for id in &nodes {
        let mut n = node(host, id.0)?.clone();
        if n.variables.bindings.remove(&a.property).is_some() {
            if let Some(value) = host.doc().bound_value(*id, &a.property) {
                desigma_doc::set_bound_property(&mut n, &a.property, &value);
            }
            updates.push(Command::Update { node: Box::new(n) });
        }
    }
    let changed = !updates.is_empty();
    if changed {
        host.exec(Command::Batch(updates))?;
    }
    Ok(json!({"node_ids":a.node_ids,"property":a.property,"changed":changed}))
}

fn set_variable_mode<H: Host>(host: &mut H, a: SetVariableModeArgs) -> Result<Value, String> {
    let nodes = target_nodes(host, &a.node_ids)?;
    let store = VariableStore::from_doc(host.doc());
    if !store.collections.contains_key(&a.collection_id) {
        return Err(format!("variable collection {} not found", a.collection_id));
    }
    let mode = match a.mode.as_deref() {
        Some(key) => Some(variable_mode_id(&store, &a.collection_id, key)?),
        None => None,
    };
    let mut updates = Vec::new();
    for id in &nodes {
        let mut n = node(host, id.0)?.clone();
        let changed = match &mode {
            Some(mode) => {
                n.variables.modes.get(&a.collection_id) != Some(mode) && {
                    n.variables.modes.insert(a.collection_id.clone(), mode.clone());
                    true
                }
            }
            None => n.variables.modes.remove(&a.collection_id).is_some(),
        };
        if changed {
            updates.push(Command::Update { node: Box::new(n) });
        }
    }
    let changed = !updates.is_empty();
    if changed {
        host.exec(Command::Batch(updates))?;
    }
    Ok(json!({"node_ids":a.node_ids,"collection_id":a.collection_id,"mode_id":mode,"changed":changed}))
}

fn layout_direction(value: &str) -> Result<LayoutDirection, String> {
    match value {
        "horizontal" => Ok(LayoutDirection::Horizontal),
        "vertical" => Ok(LayoutDirection::Vertical),
        _ => Err(format!("direction must be horizontal or vertical, got {value}")),
    }
}

fn layout_align(value: &str) -> Result<LayoutAlign, String> {
    match value {
        "start" => Ok(LayoutAlign::Start),
        "center" => Ok(LayoutAlign::Center),
        "end" => Ok(LayoutAlign::End),
        "space_between" => Ok(LayoutAlign::SpaceBetween),
        "baseline" => Ok(LayoutAlign::Baseline),
        _ => Err(format!("align must be start, center, end, space_between, or baseline, got {value}")),
    }
}

fn layout_sizing(value: &str) -> Result<Sizing, String> {
    match value {
        "fixed" => Ok(Sizing::Fixed),
        "hug" => Ok(Sizing::Hug),
        "fill" => Ok(Sizing::Fill),
        _ => Err(format!("sizing must be fixed, hug, or fill, got {value}")),
    }
}

fn layout_number(field: &str, value: f32) -> Result<f32, String> {
    if !value.is_finite() || value < 0.0 {
        return Err(format!("{field} must be finite and non-negative"));
    }
    Ok(value)
}

fn set_auto_layout<H: Host>(host: &mut H, a: AutoLayoutArgs) -> Result<Value, String> {
    let mut node = node(host, a.node_id)?.clone();
    if node.parent.is_none() {
        return Err("page roots cannot be edited by this tool".into());
    }
    let enabled = a.enabled.unwrap_or(true);
    let has_fields = a.direction.is_some()
        || a.wrap.is_some()
        || a.item_spacing.is_some()
        || a.counter_spacing.is_some()
        || a.padding.is_some()
        || a.primary_align.is_some()
        || a.counter_align.is_some()
        || a.primary_sizing.is_some()
        || a.counter_sizing.is_some()
        || a.stroke_included.is_some()
        || a.reverse_z.is_some();
    if !enabled && has_fields {
        return Err("enabled=false cannot be combined with auto layout fields".into());
    }
    let Some(frame) = node.kind.frame_props_mut() else {
        return Err("node does not support auto layout".into());
    };
    if enabled {
        let layout = frame.auto_layout.get_or_insert_with(AutoLayout::default);
        if let Some(value) = a.direction.as_deref() {
            layout.direction = layout_direction(value)?;
        }
        if let Some(value) = a.wrap {
            layout.wrap = value;
        }
        if let Some(value) = a.item_spacing {
            layout.item_spacing = layout_number("item_spacing", value)?;
        }
        if let Some(value) = a.counter_spacing {
            layout.counter_spacing = layout_number("counter_spacing", value)?;
        }
        if let Some(padding) = a.padding {
            if padding.len() != 4 {
                return Err("padding must be [top, right, bottom, left]".into());
            }
            let mut out = [0.0; 4];
            for (i, value) in padding.into_iter().enumerate() {
                out[i] = layout_number("padding", value)?;
            }
            layout.padding = out;
        }
        if let Some(value) = a.primary_align.as_deref() {
            layout.primary_align = layout_align(value)?;
        }
        if let Some(value) = a.counter_align.as_deref() {
            layout.counter_align = layout_align(value)?;
        }
        if let Some(value) = a.primary_sizing.as_deref() {
            layout.primary_sizing = layout_sizing(value)?;
        }
        if let Some(value) = a.counter_sizing.as_deref() {
            layout.counter_sizing = layout_sizing(value)?;
        }
        if let Some(value) = a.stroke_included {
            layout.stroke_included = value;
        }
        if let Some(value) = a.reverse_z {
            layout.reverse_z = value;
        }
    } else {
        frame.auto_layout = None;
    }
    let changed = host.doc().get(NodeId(a.node_id)) != Some(&node);
    if changed {
        host.exec(Command::Update { node: Box::new(node) })?;
    }
    let auto_layout = host.doc().get(NodeId(a.node_id)).and_then(|n| n.kind.frame_props()).and_then(|f| f.auto_layout);
    Ok(json!({"node_id":a.node_id,"auto_layout":auto_layout,"changed":changed}))
}

fn set_layout_child<H: Host>(host: &mut H, a: LayoutChildArgs) -> Result<Value, String> {
    let mut layer = node(host, a.node_id)?.clone();
    let parent = layer.parent.ok_or("page roots cannot be edited by this tool")?;
    if node(host, parent.0)?.kind.frame_props().is_none_or(|f| f.auto_layout.is_none()) {
        return Err("parent must use auto layout".into());
    }
    let grow = match a.grow {
        Some(value) if value.is_finite() && (0.0..=1000.0).contains(&value) => Some(value),
        Some(_) => return Err("grow must be finite and between 0 and 1000".into()),
        None => None,
    };
    let min_w = a.min_w.map(|v| layout_number("min_w", v)).transpose()?;
    let max_w = a.max_w.map(|v| layout_number("max_w", v)).transpose()?;
    let min_h = a.min_h.map(|v| layout_number("min_h", v)).transpose()?;
    let max_h = a.max_h.map(|v| layout_number("max_h", v)).transpose()?;
    if min_w.zip(max_w).is_some_and(|(min, max)| min > max) {
        return Err("min_w must not exceed max_w".into());
    }
    if min_h.zip(max_h).is_some_and(|(min, max)| min > max) {
        return Err("min_h must not exceed max_h".into());
    }
    if let Some(value) = a.horizontal.as_deref() {
        layer.layout_child.horizontal = layout_sizing(value)?;
    }
    if let Some(value) = a.vertical.as_deref() {
        layer.layout_child.vertical = layout_sizing(value)?;
    }
    if let Some(value) = grow {
        layer.layout_child.grow = value;
    }
    if min_w.is_some() {
        layer.layout_child.min_w = min_w;
    }
    if max_w.is_some() {
        layer.layout_child.max_w = max_w;
    }
    if min_h.is_some() {
        layer.layout_child.min_h = min_h;
    }
    if max_h.is_some() {
        layer.layout_child.max_h = max_h;
    }
    if let Some(value) = a.absolute {
        layer.layout_child.absolute = value;
    }
    let changed = host.doc().get(NodeId(a.node_id)) != Some(&layer);
    if changed {
        host.exec(Command::Update { node: Box::new(layer) })?;
    }
    let layout_child = host.doc().get(NodeId(a.node_id)).map(|n| n.layout_child);
    Ok(json!({"node_id":a.node_id,"layout_child":layout_child,"changed":changed}))
}

fn resolve_batch_ref(value: &Value, results: &[Option<u64>]) -> Result<Option<Value>, String> {
    let Some(text) = value.as_str() else { return Ok(None) };
    let Some(rest) = text.strip_prefix('$') else { return Ok(None) };
    let index: usize = rest.parse().map_err(|_| format!("invalid node reference '{text}'"))?;
    let id =
        results.get(index).and_then(|id| *id).ok_or_else(|| format!("node reference '{text}' must point at an earlier operation that returned a node id"))?;
    Ok(Some(json!(id)))
}

fn resolve_batch_refs(args: &mut Value, results: &[Option<u64>]) -> Result<(), String> {
    let Some(obj) = args.as_object_mut() else { return Err("arguments must be an object".into()) };
    for key in ["node_id", "parent_id"] {
        if let Some(value) = obj.get_mut(key) {
            if let Some(resolved) = resolve_batch_ref(value, results)? {
                *value = resolved;
            }
        }
    }
    if let Some(node_ids) = obj.get_mut("node_ids").and_then(Value::as_array_mut) {
        for value in node_ids {
            if let Some(resolved) = resolve_batch_ref(value, results)? {
                *value = resolved;
            }
        }
    }
    Ok(())
}

fn batch<H: Host>(host: &mut H, a: BatchArgs) -> Result<Value, String> {
    if a.operations.is_empty() {
        return Err("operations must contain at least one operation".into());
    }
    if a.operations.len() > 100 {
        return Err("operations accepts at most 100 entries".into());
    }
    for op in &a.operations {
        if !BATCHABLE.contains(&op.tool.as_str()) {
            return Err(format!("tool '{}' is not batchable; use one of: {}", op.tool, BATCHABLE.join(", ")));
        }
    }

    let mut scratch = host.doc().clone();
    let mut batch_host = BatchHost { doc: &mut scratch, commands: Vec::new() };
    let mut results = Vec::with_capacity(a.operations.len());
    let mut summaries = Vec::with_capacity(a.operations.len());
    for (index, op) in a.operations.iter().enumerate() {
        let mut args = op.arguments.clone().unwrap_or_else(|| json!({}));
        resolve_batch_refs(&mut args, &results)?;
        let result = call(&mut batch_host, &op.tool, args).map_err(|e| format!("operation {index} ({}): {e}", op.tool))?;
        let node_id = result.get("node_id").and_then(Value::as_u64).or_else(|| result.get("node").and_then(|n| n.get("id")).and_then(Value::as_u64));
        results.push(node_id);
        summaries.push(json!({"index":index,"node_id":node_id}));
    }
    let commands = batch_host.commands;
    let changed = !commands.is_empty();
    if changed {
        host.exec(Command::Batch(commands))?;
    }
    Ok(json!({"changed":changed,"operations":a.operations.len(),"results":summaries}))
}

fn reject_effect_keys(obj: &serde_json::Map<String, Value>, allowed: &[&str]) -> Result<(), String> {
    for key in obj.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(format!("unknown effect field '{key}'"));
        }
    }
    Ok(())
}

fn effect_bool(obj: &serde_json::Map<String, Value>, key: &str, default: bool) -> Result<bool, String> {
    match obj.get(key) {
        None => Ok(default),
        Some(value) => value.as_bool().ok_or_else(|| format!("{key} must be a boolean")),
    }
}

fn effect_number(obj: &serde_json::Map<String, Value>, key: &str) -> Result<f32, String> {
    let value = obj.get(key).ok_or_else(|| format!("{key} is required"))?;
    let number = value.as_f64().ok_or_else(|| format!("{key} must be a number"))?;
    if !number.is_finite() || number.abs() > 1_000_000.0 {
        return Err(format!("{key} must be finite and within ±1000000"));
    }
    Ok(number as f32)
}

fn effect_optional_number(obj: &serde_json::Map<String, Value>, key: &str, default: f32) -> Result<f32, String> {
    match obj.get(key) {
        Some(_) => effect_number(obj, key),
        None => Ok(default),
    }
}

fn effect_radius(obj: &serde_json::Map<String, Value>) -> Result<f32, String> {
    let radius = effect_number(obj, "radius")?;
    if radius < 0.0 {
        return Err("radius must be non-negative".into());
    }
    Ok(radius)
}

fn effect_offset(obj: &serde_json::Map<String, Value>) -> Result<Vec2, String> {
    let offset = obj.get("offset").ok_or("offset is required")?;
    let (x, y) = if let Some(v) = offset.as_object() {
        for key in v.keys() {
            if key != "x" && key != "y" {
                return Err(format!("unknown offset field '{key}'"));
            }
        }
        (v.get("x").and_then(Value::as_f64).ok_or("offset.x must be a number")?, v.get("y").and_then(Value::as_f64).ok_or("offset.y must be a number")?)
    } else if let Some(v) = offset.as_array() {
        if v.len() != 2 {
            return Err("offset array must be [x, y]".into());
        }
        (v[0].as_f64().ok_or("offset[0] must be a number")?, v[1].as_f64().ok_or("offset[1] must be a number")?)
    } else {
        return Err("offset must be {x,y} or [x,y]".into());
    };
    if !x.is_finite() || !y.is_finite() || x.abs() > 1_000_000.0 || y.abs() > 1_000_000.0 {
        return Err("offset must be finite and within ±1000000".into());
    }
    Ok(Vec2::new(x as f32, y as f32))
}

fn effect_color(obj: &serde_json::Map<String, Value>) -> Result<Color, String> {
    let spec = obj.get("color").and_then(Value::as_str).ok_or("color is required")?;
    Color::from_hex(spec.trim()).ok_or_else(|| format!("invalid effect color '{spec}' (use #RGB, #RRGGBB or #RRGGBBAA)"))
}

fn parse_effect(value: &Value) -> Result<Effect, String> {
    let obj = value.as_object().ok_or("each effect must be an object")?;
    let kind = obj.get("type").and_then(Value::as_str).ok_or("effect type is required")?;
    let visible = effect_bool(obj, "visible", true)?;
    match kind {
        "drop_shadow" => {
            reject_effect_keys(obj, &["type", "color", "offset", "radius", "spread", "visible", "show_behind"])?;
            let spread = effect_optional_number(obj, "spread", 0.0)?;
            Ok(Effect::DropShadow {
                color: effect_color(obj)?,
                offset: effect_offset(obj)?,
                radius: effect_radius(obj)?,
                spread,
                visible,
                show_behind: effect_bool(obj, "show_behind", false)?,
            })
        }
        "inner_shadow" => {
            reject_effect_keys(obj, &["type", "color", "offset", "radius", "spread", "visible"])?;
            let spread = effect_optional_number(obj, "spread", 0.0)?;
            Ok(Effect::InnerShadow { color: effect_color(obj)?, offset: effect_offset(obj)?, radius: effect_radius(obj)?, spread, visible })
        }
        "layer_blur" => {
            reject_effect_keys(obj, &["type", "radius", "visible"])?;
            Ok(Effect::LayerBlur { radius: effect_radius(obj)?, visible })
        }
        "background_blur" => {
            reject_effect_keys(obj, &["type", "radius", "visible"])?;
            Ok(Effect::BackgroundBlur { radius: effect_radius(obj)?, visible })
        }
        other => Err(format!("effect type must be one of: {}, got {other}", EFFECT_TYPES.join(", "))),
    }
}

fn set_effects<H: Host>(host: &mut H, a: SetEffectsArgs) -> Result<Value, String> {
    let mut node = node(host, a.node_id)?.clone();
    if node.parent.is_none() {
        return Err("page roots cannot be edited by this tool".into());
    }
    if a.effects.len() > 100 {
        return Err("effects accepts at most 100 entries".into());
    }
    let effects = a.effects.iter().map(parse_effect).collect::<Result<Vec<_>, _>>()?;
    node.effects = effects;
    let changed = host.doc().get(NodeId(a.node_id)) != Some(&node);
    if changed {
        host.exec(Command::Update { node: Box::new(node) })?;
    }
    let effects = host.doc().get(NodeId(a.node_id)).map(|n| n.effects.clone()).unwrap_or_default();
    Ok(json!({"node_id":a.node_id,"effects":effects,"changed":changed}))
}

fn colour(spec: &str, field: &str) -> Result<Color, String> {
    Color::from_hex(spec.trim()).ok_or_else(|| format!("invalid {field} colour: {spec} (use #RGB, #RRGGBB, #RRGGBBAA or none)"))
}

fn stroke_weight(width: f32) -> Result<f32, String> {
    if !width.is_finite() || !(0.0..=1000.0).contains(&width) {
        return Err("stroke_width must be within 0..=1000".into());
    }
    Ok(width)
}

/// Pushes the edited node onto the undo stack and reports whether anything changed.
fn commit<H: Host>(host: &mut H, node: Node, applied: Vec<&str>) -> Result<Value, String> {
    let id = node.id.0;
    let changed = host.doc().get(node.id) != Some(&node);
    if changed {
        host.exec(Command::Update { node: Box::new(node) })?;
    }
    Ok(json!({"node_id":id,"applied":applied,"changed":changed}))
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

/// `true` when `cmd` has nothing to apply, as an empty batch coming out of the
/// component helpers does.
fn is_noop(cmd: &Command) -> bool {
    matches!(cmd, Command::Batch(v) if v.is_empty())
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
struct CreateVariableCollectionArgs {
    name: String,
    /// Mode names; defaults to one mode named "Mode 1".
    modes: Option<Vec<String>>,
    /// Mode name or id; defaults to the first mode.
    default_mode: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AddVariableModeArgs {
    collection_id: String,
    name: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateVariableArgs {
    collection_id: String,
    name: String,
    /// color | number | string | boolean
    kind: String,
    description: Option<String>,
    /// Figma scopes such as ALL_FILLS, GAP, CORNER_RADIUS; empty means all.
    scopes: Option<Vec<String>>,
    /// Mode name/id -> raw literal or {"alias":"variable_id"}.
    values: Option<Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetVariableValueArgs {
    variable_id: String,
    /// Mode name or id; defaults to the collection's default mode.
    mode: Option<String>,
    value: Value,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BindVariableArgs {
    node_ids: Vec<u64>,
    property: String,
    variable_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DetachVariableArgs {
    node_ids: Vec<u64>,
    property: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetVariableModeArgs {
    node_ids: Vec<u64>,
    collection_id: String,
    /// Mode name or id; null or omitted clears the explicit mode.
    mode: Option<String>,
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
struct DuplicateArgs {
    node_id: u64,
    /// Parent-space offset for the copied root; `None` picks 10, or 0 in auto layout.
    offset: Option<f32>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MoveNodeArgs {
    node_id: u64,
    /// New container (frame, group, section or page root).
    parent_id: u64,
    /// Position among the new parent's children; appended when omitted.
    index: Option<usize>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreatePageArgs {
    name: String,
    /// Page background; `#RRGGBB` (defaults to white).
    background: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetEffectsArgs {
    node_id: u64,
    effects: Vec<Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AppearanceArgs {
    node_id: u64,
    /// `#RGB`, `#RRGGBB`, `#RRGGBBAA` or `none` to clear.
    fill: Option<String>,
    /// `#RGB`, `#RRGGBB`, `#RRGGBBAA` or `none` to clear.
    stroke: Option<String>,
    stroke_width: Option<f32>,
    /// Sets all four corner radii.
    corner_radius: Option<f32>,
    opacity: Option<f32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateStyleArgs {
    name: String,
    /// fill | stroke | text
    kind: String,
    description: Option<String>,
    /// Required for fill/stroke.
    color: Option<String>,
    /// Paint opacity for fill/stroke; defaults to 1.
    opacity: Option<f32>,
    /// Required for text: typography fields (font_size, font_weight, font_family, line_height, letter_spacing, align_h, align_v).
    text: Option<Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ApplyStyleArgs {
    node_ids: Vec<u64>,
    style: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DetachStyleArgs {
    node_ids: Vec<u64>,
    /// fill | stroke | text | effect | grid; omit to clear every style reference.
    property: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TextStyleArgs {
    node_id: u64,
    text: Option<String>,
    font_size: Option<f32>,
    /// CSS weight 100..=900, in steps of 100.
    font_weight: Option<u16>,
    font_family: Option<String>,
    /// Line height in px.
    line_height: Option<f32>,
    letter_spacing: Option<f32>,
    /// left | center | right | justified
    align_h: Option<String>,
    /// top | center | bottom
    align_v: Option<String>,
    /// none | width_and_height | height | truncate
    auto_resize: Option<String>,
    /// Maximum visible lines when auto_resize is truncate; 1..=1000.
    max_lines: Option<u32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AutoLayoutArgs {
    node_id: u64,
    /// false removes auto layout; true (default) creates or updates it.
    enabled: Option<bool>,
    /// horizontal | vertical
    direction: Option<String>,
    wrap: Option<bool>,
    item_spacing: Option<f32>,
    counter_spacing: Option<f32>,
    /// top, right, bottom, left
    padding: Option<Vec<f32>>,
    primary_align: Option<String>,
    counter_align: Option<String>,
    primary_sizing: Option<String>,
    counter_sizing: Option<String>,
    stroke_included: Option<bool>,
    reverse_z: Option<bool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LayoutChildArgs {
    node_id: u64,
    horizontal: Option<String>,
    vertical: Option<String>,
    grow: Option<f32>,
    min_w: Option<f32>,
    max_w: Option<f32>,
    min_h: Option<f32>,
    max_h: Option<f32>,
    absolute: Option<bool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BatchArgs {
    operations: Vec<BatchOp>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BatchOp {
    tool: String,
    #[serde(default)]
    arguments: Option<Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateComponentArgs {
    node_id: u64,
    /// Main component name; the layer keeps its own name when omitted.
    name: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateInstanceArgs {
    component_id: u64,
    /// Container that receives the instance (frame, group, section or page).
    parent_id: u64,
    #[serde(default)]
    x: f32,
    #[serde(default)]
    y: f32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SyncArgs {
    component_id: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RenamePageArgs {
    page_id: u64,
    name: String,
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportSvgArgs {
    pub parent_id: u64,
    /// SVG markup (`<svg …`), at most 8 MiB.
    pub svg: String,
    /// Layer name for the imported frame; defaults to "svg".
    pub name: Option<String>,
    #[serde(default)]
    pub x: f32,
    #[serde(default)]
    pub y: f32,
}

/// Imports SVG markup as a frame under `parent_id`, top-left at (x, y) in the
/// parent's coordinates. One undoable `Command::Insert`; bitmap bytes carried
/// by the SVG are merged into the document as content-addressed assets.
pub fn import_svg<H: Host>(host: &mut H, a: ImportSvgArgs) -> Result<Value, String> {
    if !node(host, a.parent_id)?.kind.is_container() {
        return Err("parent_id must identify a container".into());
    }
    if a.svg.len() > 8 * 1024 * 1024 {
        return Err("svg markup exceeds the 8 MiB limit".into());
    }
    if !desigma_svg::looks_like_svg(&a.svg) {
        return Err("svg must be SVG markup starting with <svg or an XML prolog".into());
    }
    if !a.x.is_finite() || !a.y.is_finite() {
        return Err("x and y must be finite numbers".into());
    }
    let name = match a.name.as_deref().map(str::trim) {
        None | Some("") => "svg".to_string(),
        Some(n) => n.to_string(),
    };
    bounded_text("name", &name, 1024)?;
    let index = host.doc().get(NodeId(a.parent_id)).map_or(0, |n| n.children.len());
    let mut exhausted = false;
    let mut out = desigma_svg::svg_to_nodes_with(
        &a.svg,
        &name,
        &mut || match host.alloc_id() {
            Ok(id) => id,
            Err(_) => {
                exhausted = true;
                NodeId(0)
            }
        },
        &ImportOptions::default(),
    )
    .map_err(|e| format!("cannot import SVG: {e}"))?;
    if exhausted {
        return Err("Document has exhausted node IDs".into());
    }
    let count = out.nodes.len();
    if count > 20_000 {
        return Err(format!("svg produces {count} layers; the limit is 20000"));
    }
    let size = out.nodes.first().map(|n| n.size);
    if let Some(root) = out.nodes.first_mut() {
        root.transform = desigma_doc::Affine::translate(a.x, a.y);
    }
    let root_id = out.nodes.first().map(|n| n.id).ok_or("svg produced no layers")?;
    let images = out.images.len();
    host.exec(Command::Insert { parent: NodeId(a.parent_id), index, nodes: out.nodes })?;
    // Only once the layers that use them are in: images sit outside undo, so bytes added
    // before a failed insert would stay in the document for good.
    host.doc_mut().images.append(&mut out.images);
    Ok(json!({"node_id":root_id,"name":name,"nodes":count,
        "width":size.map(|s| s.x),"height":size.map(|s| s.y),
        "images":images,"changed":true}))
}

/// JSON tool descriptors kept beside the argument validators.
pub fn tools() -> Vec<Value> {
    let id = json!({"type":"integer","minimum":1,"description":"Desigma numeric node ID; obtain from list_nodes/get_document."});
    let coordinate = json!({"type":"number","minimum":-1000000,"maximum":1000000});
    let size = json!({"type":"number","minimum":0,"maximum":1000000});
    let text = json!({"type":"string","maxLength":100000,"description":"Plain text; limited to 100000 UTF-8 bytes."});
    let name = json!({"type":"string","maxLength":1024,"description":"Layer name; limited to 1024 UTF-8 bytes."});
    let variable_ref = json!({"type":"string","minLength":1,"maxLength":1024,"description":"Variable or collection ID returned by the variable tools."});
    let node_ids = json!({"type":"array","items":id.clone(),"minItems":1,"maxItems":500,"description":"Non-page layer IDs."});
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
        "create_variable_collection",
        "Create a variable collection with named modes. The first mode (default: \"Mode 1\") becomes the default; set default_mode to another mode name or id. Supports undo as one step; changes remain in memory until saved.",
        json!({"name":{"type":"string","minLength":1,"maxLength":1024},"modes":{"type":"array","items":{"type":"string","minLength":1,"maxLength":1024},"minItems":1,"maxItems":32},"default_mode":{"type":"string","minLength":1,"maxLength":1024}}),
        vec!["name"],
        false,
        false,
    );
    add(
        "add_variable_mode",
        "Append a mode to a variable collection. Existing variables receive the collection's default-mode value for the new mode. Supports undo as one step; changes remain in memory until saved.",
        json!({"collection_id":variable_ref,"name":{"type":"string","minLength":1,"maxLength":1024}}),
        vec!["collection_id", "name"],
        false,
        false,
    );
    add(
        "create_variable",
        "Create a color, number, string, or boolean variable in a collection. values maps a mode name or id to a literal for the kind (hex string, number, string, or boolean) or {\"alias\":\"variable_id\"}; omitted modes get the kind's default. Supports undo as one step; changes remain in memory until saved.",
        json!({"collection_id":variable_ref,"name":{"type":"string","minLength":1,"maxLength":1024},"kind":{"type":"string","enum":["color","number","string","boolean"]},"description":{"type":"string","maxLength":100000},"scopes":{"type":"array","items":{"type":"string","maxLength":256},"maxItems":64},"values":{"type":"object","additionalProperties":true,"description":"Mode name/id -> literal or {\"alias\":\"variable_id\"}."}}),
        vec!["collection_id", "name", "kind"],
        false,
        false,
    );
    add(
        "set_variable_value",
        "Set one mode's value for a variable. value is a literal for the variable's kind (hex string, number, string, or boolean) or {\"alias\":\"variable_id\"} for a same-kind alias without a cycle. mode defaults to the collection's default mode. Supports undo as one step; changes remain in memory until saved.",
        json!({"variable_id":variable_ref,"mode":{"type":"string","minLength":1,"maxLength":1024},"value":{}}),
        vec!["variable_id", "value"],
        false,
        true,
    );
    add(
        "bind_variable",
        "Bind a variable to one property on up to 500 layers. property uses Figma's boundVariables keys (fills/0, strokes/0, width, itemSpacing, fontSize, ...). The variable kind and scopes must allow the property, and its current value is written into the raw property so detaching keeps the look. One undo step; changes remain in memory until saved.",
        json!({"node_ids":node_ids,"property":{"type":"string","minLength":1,"maxLength":128},"variable_id":variable_ref}),
        vec!["node_ids", "property", "variable_id"],
        false,
        true,
    );
    add(
        "detach_variable",
        "Remove a variable binding from one property on up to 500 layers, writing the value it currently resolves to into the raw property. One undo step; changes remain in memory until saved.",
        json!({"node_ids":node_ids,"property":{"type":"string","minLength":1,"maxLength":128}}),
        vec!["node_ids", "property"],
        false,
        true,
    );
    add(
        "set_variable_mode",
        "Set or clear the explicit mode of a collection on up to 500 layers. mode accepts a mode name or id; null or omitted clears the explicit mode and inherits from the parent. One undo step; changes remain in memory until saved.",
        json!({"node_ids":node_ids,"collection_id":variable_ref,"mode":{"type":"string","minLength":1,"maxLength":1024}}),
        vec!["node_ids", "collection_id"],
        false,
        true,
    );
    add(
        "generate_code",
        "Generate developer handoff code or SVG for a layer using resolved variable values. Returns source text; writes no files.",
        json!({"node_id":id,"language":{"type":"string","enum":["css","tailwind","react","swiftui","compose","flutter","vue","svg"]}}),
        vec!["node_id", "language"],
        true,
        true,
    );
    add("create_node","Create one layer in a container; defaults to x/y=0, width/height=100. Supports undo. Text is only valid for text layers. Changes are in memory until saved.",
        json!({"parent_id":id,"kind":{"type":"string","enum":["frame","rectangle","ellipse","text"]},"name":name,"text":text,"x":coordinate,"y":coordinate,"width":size,"height":size}),vec!["parent_id","kind"],false,false);
    add(
        "duplicate_node",
        "Duplicate a non-page layer and its whole subtree in place, immediately after the source under the same parent. New node IDs; the copy keeps the source name. offset moves the copied root in parent space; it defaults to 10, or 0 when the parent uses auto layout. Refuses page roots and layers inside an instance. Supports undo as one step; changes remain in memory until saved.",
        json!({"node_id":id,"offset":{"type":"number","minimum":0,"maximum":100000,"description":"Parent-space offset for the copied root; defaults to 10, or 0 when the parent uses auto layout."}}),
        vec!["node_id"],
        false,
        false,
    );
    add(
        "batch",
        "Apply 1-100 node edits as one undo step. Each operation is {tool, arguments}; allowed tools are create_node, duplicate_node, update_node, move_node, set_appearance, set_effects, set_text_style, set_auto_layout, set_layout_child, and delete_node. A later operation can reference a node id returned by an earlier operation with \"$INDEX\" in node_id, parent_id, or entries of node_ids. Validation runs on a scratch document; a failed operation leaves the document and history unchanged.",
        json!({"operations":{"type":"array","minItems":1,"maxItems":100,"items":{"type":"object","properties":{"tool":{"type":"string","enum":["desigma_create_node","desigma_duplicate_node","desigma_update_node","desigma_move_node","desigma_set_appearance","desigma_set_effects","desigma_set_text_style","desigma_set_auto_layout","desigma_set_layout_child","desigma_delete_node"]},"arguments":{"type":"object","additionalProperties":true}},"required":["tool"],"additionalProperties":false}}}),
        vec!["operations"],
        false,
        false,
    );
    add("import_svg","Import SVG markup as one frame under parent_id, top-left at (x, y). Real child layers (paths, text, groups) are created rather than a flat bitmap; bitmap bytes inside the SVG become document assets. Supports undo as one step; changes remain in memory until saved.",json!({"parent_id":id,"svg":{"type":"string","maxLength":8388608,"description":"SVG markup; at most 8 MiB. Pass this or path."},"path":{"type":"string","description":"Workspace-relative .svg file to import instead of inline markup; the layer takes the file name unless name is given."},"name":name,"x":coordinate,"y":coordinate}),vec!["parent_id"],false,false);
    add("update_node","Update specified fields of one non-page layer. Coordinates are parent-relative. Replacing text clears text runs and list ranges. Supports undo; changes remain in memory until saved.",
        json!({"node_id":id,"name":name,"text":text,"x":coordinate,"y":coordinate,"width":size,"height":size,"visible":{"type":"boolean"},"opacity":{"type":"number","minimum":0,"maximum":1}}),vec!["node_id"],false,true);
    add(
        "move_node",
        "Move one non-page layer under another container (frame, group, section or page), optionally at a given child index; appended when index is omitted. The layer keeps its parent-relative transform, so absolute position changes when the new parent is transformed. Refuses page roots, the layer's own subtree and non-container targets. Supports undo; changes remain in memory until saved.",
        json!({"node_id":id,"parent_id":id,"index":{"type":"integer","minimum":0}}),
        vec!["node_id","parent_id"],
        false,
        true,
    );
    add(
        "create_page",
        "Append an empty page (background defaults to white) and return its id, ready to receive layers. Pages are not nested under another node, so this is separate from create_node. Supports undo; changes remain in memory until saved.",
        json!({"name":{"type":"string","minLength":1,"maxLength":1024},"background":{"type":"string","maxLength":9,"description":"#RGB, #RRGGBB or #RRGGBBAA; defaults to #ffffff."}}),
        vec!["name"],
        false,
        false,
    );
    add(
        "set_appearance",
        "Restyle one non-page layer: fill, stroke, stroke width, corner radius, opacity. Colours accept #RGB, #RRGGBB, #RRGGBBAA or none. On text layers fill sets the text colour. Supports undo; changes remain in memory until saved.",
        json!({"node_id":id,"fill":{"type":"string","maxLength":9},"stroke":{"type":"string","maxLength":9},"stroke_width":{"type":"number","minimum":0,"maximum":1000},"corner_radius":{"type":"number","minimum":0,"maximum":1000000},"opacity":{"type":"number","minimum":0,"maximum":1}}),
        vec!["node_id"],
        false,
        true,
    );
    let effect_types = EFFECT_TYPES.join(", ");
    let set_effects_description = format!(
        "Replace a non-page layer's effect list; an empty array clears it. Supported types: {effect_types}. drop_shadow/inner_shadow take color, offset ({{x,y}} or [x,y]), radius, optional spread, visible, and (drop only) show_behind; blur types take radius and visible. Colors accept #RGB, #RRGGBB or #RRGGBBAA; radius is non-negative. Supports undo as one step; changes remain in memory until saved."
    );
    add(
        "set_effects",
        &set_effects_description,
        json!({"node_id":id,"effects":{"type":"array","maxItems":100,"items":{"type":"object","additionalProperties":true,"properties":{"type":{"type":"string","enum":EFFECT_TYPES},"color":{"type":"string","maxLength":9},"offset":{"type":["object","array"]},"radius":{"type":"number","minimum":0,"maximum":1000000},"spread":{"type":"number","minimum":-1000000,"maximum":1000000},"visible":{"type":"boolean"},"show_behind":{"type":"boolean"}},"required":["type"]}}}),
        vec!["node_id", "effects"],
        false,
        true,
    );
    add(
        "set_text_style",
        "Set a text layer's content, typography and auto-resize mode: font size, CSS weight, family, line height in px, letter spacing, alignment, auto_resize (none, width_and_height, height, truncate) and max_lines. Supports undo; changes remain in memory until saved.",
        json!({"node_id":id,"text":text,"font_size":{"type":"number","minimum":1,"maximum":1000},"font_weight":{"type":"integer","minimum":100,"maximum":900,"multipleOf":100},"font_family":{"type":"string","maxLength":256},"line_height":{"type":"number","minimum":0,"maximum":1000},"letter_spacing":{"type":"number","minimum":-100,"maximum":100},"align_h":{"type":"string","enum":["left","center","right","justified"]},"align_v":{"type":"string","enum":["top","center","bottom"]},"auto_resize":{"type":"string","enum":["none","width_and_height","height","truncate"]},"max_lines":{"type":"integer","minimum":1,"maximum":1000}}),
        vec!["node_id"],
        false,
        true,
    );
    add(
        "set_auto_layout",
        "Create, update, or remove auto layout on a frame, component, component set, or instance. enabled=false removes it and cannot be combined with layout fields. Spacing and padding are in px and must be non-negative. Supports undo as one step; changes remain in memory until saved.",
        json!({"node_id":id,"enabled":{"type":"boolean"},"direction":{"type":"string","enum":["horizontal","vertical"]},"wrap":{"type":"boolean"},"item_spacing":{"type":"number","minimum":0,"maximum":1000000},"counter_spacing":{"type":"number","minimum":0,"maximum":1000000},"padding":{"type":"array","items":{"type":"number","minimum":0,"maximum":1000000},"minItems":4,"maxItems":4,"description":"top, right, bottom, left"},"primary_align":{"type":"string","enum":["start","center","end","space_between","baseline"]},"counter_align":{"type":"string","enum":["start","center","end","space_between","baseline"]},"primary_sizing":{"type":"string","enum":["fixed","hug","fill"]},"counter_sizing":{"type":"string","enum":["fixed","hug","fill"]},"stroke_included":{"type":"boolean"},"reverse_z":{"type":"boolean"}}),
        vec!["node_id"],
        false,
        true,
    );
    add(
        "set_layout_child",
        "Set one layer's auto-layout child sizing. The parent must use auto layout. Sizing is fixed, hug, or fill; min/max are px and min must not exceed max. Supports undo as one step; changes remain in memory until saved.",
        json!({"node_id":id,"horizontal":{"type":"string","enum":["fixed","hug","fill"]},"vertical":{"type":"string","enum":["fixed","hug","fill"]},"grow":{"type":"number","minimum":0,"maximum":1000},"min_w":{"type":"number","minimum":0,"maximum":1000000},"max_w":{"type":"number","minimum":0,"maximum":1000000},"min_h":{"type":"number","minimum":0,"maximum":1000000},"max_h":{"type":"number","minimum":0,"maximum":1000000},"absolute":{"type":"boolean"}}),
        vec!["node_id"],
        false,
        true,
    );
    add(
        "create_style",
        "Create a shared fill, stroke, or text style. fill/stroke need color and optional opacity; text needs a text object with font_size, font_weight, font_family, line_height, letter_spacing, align_h, align_v. Returns the generated style key. Supports undo; changes remain in memory until saved.",
        json!({"name":{"type":"string","minLength":1,"maxLength":1024},"kind":{"type":"string","enum":["fill","stroke","text"]},"description":{"type":"string","maxLength":100000},"color":{"type":"string","maxLength":9},"opacity":{"type":"number","minimum":0,"maximum":1},"text":{"type":"object","additionalProperties":false,"properties":{"font_size":{"type":"number","minimum":1,"maximum":1000},"font_weight":{"type":"integer","minimum":100,"maximum":900,"multipleOf":100},"font_family":{"type":"string","maxLength":256},"line_height":{"type":"number","minimum":0,"maximum":1000},"letter_spacing":{"type":"number","minimum":-100,"maximum":100},"align_h":{"type":"string","enum":["left","center","right","justified"]},"align_v":{"type":"string","enum":["top","center","bottom"]}}}}),
        vec!["name", "kind"],
        false,
        false,
    );
    add(
        "apply_style",
        "Apply a shared fill, stroke, or text style to 1-500 non-page layers and record the style reference. Text styles require text layers. Supports undo as one step; changes remain in memory until saved.",
        json!({"node_ids":node_ids,"style":{"type":"string","minLength":1,"maxLength":1024}}),
        vec!["node_ids", "style"],
        false,
        true,
    );
    add(
        "detach_style",
        "Remove shared-style references from 1-500 non-page layers, keeping their current values. Omit property to clear every reference. Supports undo as one step; changes remain in memory until saved.",
        json!({"node_ids":node_ids,"property":{"type":"string","enum":["fill","stroke","text","effect","grid"]}}),
        vec!["node_ids"],
        false,
        true,
    );
    add(
        "delete_node",
        "Remove a non-page layer and its entire subtree. Supports undo; changes remain in memory until saved.",
        json!({"node_id":id}),
        vec!["node_id"],
        false,
        false,
    );
    // ---- Components, instances and page rename ----
    add(
        "create_component",
        "Turn a frame / group / section layer into a main component in place, optionally renaming it. Supports undo; changes remain in memory until saved.",
        json!({"node_id":id,"name":name}),
        vec!["node_id"],
        false,
        false,
    );
    add(
        "create_instance",
        "Place an instance of a main component inside another container (frame, group, section or page). Supports undo; changes remain in memory until saved.",
        json!({"component_id":id,"parent_id":id,"x":coordinate,"y":coordinate}),
        vec!["component_id", "parent_id"],
        false,
        false,
    );
    add(
        "detach_instance",
        "Detach an instance in place: its mirrored subtree becomes ordinary layers. Supports undo; changes remain in memory until saved.",
        json!({"node_id":id}),
        vec!["node_id"],
        false,
        false,
    );
    add(
        "sync_instances",
        "Re-apply a main component's current state to every instance of it (needed after editing a component in a way that does not sync, e.g. set_appearance). Supports undo; changes remain in memory until saved.",
        json!({"component_id":id}),
        vec!["component_id"],
        false,
        true,
    );
    add(
        "rename_page",
        "Rename a page; page roots cannot be renamed with update_node. Supports undo; changes remain in memory until saved.",
        json!({"page_id":id,"name":{"type":"string","minLength":1,"maxLength":1024}}),
        vec!["page_id", "name"],
        false,
        true,
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
    fn duplicate_copies_subtree_after_source_with_offset_and_undo() {
        let mut e = Engine::default();
        let frame = create(&mut e, "frame", "Card");
        let inner =
            e.call("desigma_create_node", json!({"parent_id":frame,"kind":"rectangle","name":"Inner","x":4,"y":6})).unwrap()["node"]["id"].as_u64().unwrap();
        let result = e.call("desigma_duplicate_node", json!({"node_id":frame})).unwrap();
        let copy = result["node_id"].as_u64().unwrap();
        assert_eq!(result["nodes"], 2);
        assert_eq!(result["index"], 1);
        assert_eq!(result["parent_id"], 1);
        assert_eq!(e.doc.get(NodeId(1)).unwrap().children, vec![NodeId(frame), NodeId(copy)]);
        let copied = e.doc.get(NodeId(copy)).unwrap();
        assert_eq!(copied.name, "Card");
        assert_eq!(copied.transform.tx, 10.0);
        assert_eq!(copied.transform.ty, 10.0);
        assert_eq!(copied.children.len(), 1);
        let copied_inner = e.doc.get(copied.children[0]).unwrap();
        assert_eq!(copied_inner.name, "Inner");
        assert_eq!(copied_inner.parent, Some(NodeId(copy)));
        assert_eq!(copied_inner.transform.tx, 4.0);
        assert_ne!(copied.children[0], NodeId(inner));
        e.call("desigma_undo", json!({})).unwrap();
        assert!(e.doc.get(NodeId(copy)).is_none());
        assert_eq!(e.doc.get(NodeId(1)).unwrap().children, vec![NodeId(frame)]);
        e.call("desigma_redo", json!({})).unwrap();
        assert!(e.doc.get(NodeId(copy)).is_some());
    }

    #[test]
    fn duplicate_uses_zero_offset_in_auto_layout_and_accepts_explicit_offset() {
        let mut e = Engine::default();
        let row = create(&mut e, "frame", "Row");
        e.doc.get_mut(NodeId(row)).unwrap().kind.frame_props_mut().unwrap().auto_layout = Some(Default::default());
        let item = e.call("desigma_create_node", json!({"parent_id":row,"kind":"rectangle","name":"Item"})).unwrap()["node"]["id"].as_u64().unwrap();
        let zero = e.call("desigma_duplicate_node", json!({"node_id":item})).unwrap();
        assert_eq!(e.doc.get(NodeId(zero["node_id"].as_u64().unwrap())).unwrap().transform.tx, 0.0);
        let shifted = e.call("desigma_duplicate_node", json!({"node_id":item,"offset":4})).unwrap();
        assert_eq!(e.doc.get(NodeId(shifted["node_id"].as_u64().unwrap())).unwrap().transform.tx, 4.0);
    }

    #[test]
    fn duplicate_refuses_page_roots_missing_nodes_and_instance_layers() {
        let mut e = Engine::default();
        let err = e.call("desigma_duplicate_node", json!({"node_id":1})).unwrap_err();
        assert!(err.contains("page roots"), "{err}");
        let err = e.call("desigma_duplicate_node", json!({"node_id":9999})).unwrap_err();
        assert!(err.contains("not found"), "{err}");
        let comp = create(&mut e, "frame", "Button");
        e.call("desigma_create_node", json!({"parent_id":comp,"kind":"rectangle","name":"Label"})).unwrap();
        e.call("desigma_create_component", json!({"node_id":comp})).unwrap();
        let instance = e.call("desigma_create_instance", json!({"component_id":comp,"parent_id":1,"x":0,"y":0})).unwrap();
        let instance_id = instance["instance_id"].as_u64().unwrap();
        let child = e.doc.get(NodeId(instance_id)).unwrap().children[0];
        let err = e.call("desigma_duplicate_node", json!({"node_id":child})).unwrap_err();
        assert!(err.contains("inside an instance"), "{err}");
        let err = e.call("desigma_duplicate_node", json!({"node_id":comp,"offset":-1})).unwrap_err();
        assert!(err.contains("offset"), "{err}");
    }

    #[test]
    fn duplicate_is_annotated_as_a_writing_non_idempotent_tool() {
        let t = tools().into_iter().find(|t| t["name"] == "desigma_duplicate_node").unwrap();
        assert_eq!(t["annotations"]["readOnlyHint"], false);
        assert_eq!(t["annotations"]["idempotentHint"], false);
        assert_eq!(t["inputSchema"]["required"], json!(["node_id"]));
        assert_eq!(t["inputSchema"]["properties"]["offset"]["maximum"], 100000);
    }

    #[test]
    fn variable_store_creates_modes_values_and_aliases() {
        let mut e = Engine::default();
        let collection =
            e.call("desigma_create_variable_collection", json!({"name":"Theme","modes":["Light","Dark"],"default_mode":"Dark"})).unwrap()["collection"].clone();
        let collection_id = collection["id"].as_str().unwrap().to_string();
        let mode_id = |name: &str| collection["modes"].as_array().unwrap().iter().find(|m| m["name"] == name).unwrap()["id"].as_str().unwrap().to_string();
        let light = mode_id("Light");
        let dark = mode_id("Dark");
        assert_eq!(e.doc.variable_collections[&collection_id].default_mode, dark);

        let variable = e
            .call(
                "desigma_create_variable",
                json!({
                    "collection_id":collection_id,
                    "name":"Color/Brand/Primary",
                    "kind":"color",
                    "scopes":["ALL_FILLS"],
                    "values":{"Light":"#2F5BFF","Dark":"#8AA2FF"}
                }),
            )
            .unwrap()["variable"]
            .clone();
        let variable_id = variable["id"].as_str().unwrap().to_string();
        let stored = &e.doc.variables[&variable_id];
        assert_eq!(stored.scopes, vec!["ALL_FILLS"]);
        assert_eq!(stored.values[&light], VariableValue::Color(Color::from_hex("#2F5BFF").unwrap()));
        assert_eq!(stored.values[&dark], VariableValue::Color(Color::from_hex("#8AA2FF").unwrap()));

        e.call("desigma_set_variable_value", json!({"variable_id":variable_id,"mode":"Light","value":"#FF6B4A"})).unwrap();
        assert_eq!(e.doc.variables[&variable_id].values[&light], VariableValue::Color(Color::from_hex("#FF6B4A").unwrap()));
        e.call("desigma_undo", json!({})).unwrap();
        assert_eq!(e.doc.variables[&variable_id].values[&light], VariableValue::Color(Color::from_hex("#2F5BFF").unwrap()));
        e.call("desigma_redo", json!({})).unwrap();
        assert_eq!(e.doc.variables[&variable_id].values[&light], VariableValue::Color(Color::from_hex("#FF6B4A").unwrap()));

        let alias = e
            .call(
                "desigma_create_variable",
                json!({"collection_id":collection_id,"name":"Color/Action","kind":"color","values":{"Light":{"alias":variable_id}}}),
            )
            .unwrap()["variable"]
            .clone();
        let alias_id = alias["id"].as_str().unwrap().to_string();
        assert_eq!(e.doc.variables[&alias_id].values[&light], VariableValue::Alias(variable_id.clone()));

        let added = e.call("desigma_add_variable_mode", json!({"collection_id":collection_id,"name":"High contrast"})).unwrap();
        let hc = added["mode"]["id"].as_str().unwrap().to_string();
        assert_eq!(e.doc.variables[&variable_id].values[&hc], e.doc.variables[&variable_id].values[&dark]);

        let unchanged = e.call("desigma_set_variable_value", json!({"variable_id":variable_id,"mode":"Light","value":"#FF6B4A"})).unwrap();
        assert_eq!(unchanged["changed"], false);
    }

    #[test]
    fn variable_store_rejects_invalid_input_without_touching_the_document() {
        let mut e = Engine::default();
        let collection = e.call("desigma_create_variable_collection", json!({"name":"Theme","modes":["Light","Dark"]})).unwrap()["collection"].clone();
        let collection_id = collection["id"].as_str().unwrap().to_string();
        let variable = e.call("desigma_create_variable", json!({"collection_id":collection_id,"name":"Brand","kind":"color"})).unwrap()["variable"].clone();
        let variable_id = variable["id"].as_str().unwrap().to_string();
        let before = e.doc.to_json().unwrap();
        for (name, args) in [
            ("desigma_create_variable_collection", json!({"name":"Empty","modes":[]})),
            ("desigma_create_variable_collection", json!({"name":"Duplicate","modes":["Light","Light"]})),
            ("desigma_create_variable", json!({"collection_id":"missing","name":"X","kind":"color"})),
            ("desigma_create_variable", json!({"collection_id":collection_id,"name":"X","kind":"pixel"})),
            ("desigma_create_variable", json!({"collection_id":collection_id,"name":"X","kind":"color","values":{"Light":"not-a-color"}})),
            ("desigma_create_variable", json!({"collection_id":collection_id,"name":"X","kind":"color","values":{"Light":{"alias":"missing"}}})),
            ("desigma_create_variable", json!({"collection_id":collection_id,"name":"X","kind":"number","values":{"Light":"12"}})),
            ("desigma_set_variable_value", json!({"variable_id":"missing","value":"#ffffff"})),
            ("desigma_set_variable_value", json!({"variable_id":variable_id,"mode":"Missing","value":"#ffffff"})),
            ("desigma_set_variable_value", json!({"variable_id":variable_id,"value":12})),
            ("desigma_set_variable_value", json!({"variable_id":variable_id,"value":{"alias":variable_id}})),
            ("desigma_add_variable_mode", json!({"collection_id":collection_id,"name":"Light"})),
            ("desigma_add_variable_mode", json!({"collection_id":collection_id,"name":""})),
        ] {
            let err = e.call(name, args).unwrap_err();
            assert!(!err.is_empty(), "{name}");
            assert_eq!(e.doc.to_json().unwrap(), before, "{name} changed the document");
        }
    }

    #[test]
    fn variable_binding_bakes_current_mode_and_detaches() {
        let mut e = Engine::default();
        let collection = e.call("desigma_create_variable_collection", json!({"name":"Theme","modes":["Light","Dark"],"default_mode":"Light"})).unwrap()
            ["collection"]
            .clone();
        let collection_id = collection["id"].as_str().unwrap().to_string();
        let mode = |name: &str| collection["modes"].as_array().unwrap().iter().find(|m| m["name"] == name).unwrap()["id"].as_str().unwrap().to_string();
        let dark = mode("Dark");
        let variable = e
            .call(
                "desigma_create_variable",
                json!({"collection_id":collection_id,"name":"Color/Brand","kind":"color","values":{"Light":"#2F5BFF","Dark":"#8AA2FF"}}),
            )
            .unwrap()["variable"]
            .clone();
        let variable_id = variable["id"].as_str().unwrap().to_string();
        let rect = create(&mut e, "rectangle", "Card");
        let bound = e.call("desigma_bind_variable", json!({"node_ids":[rect],"property":"fills/0","variable_id":variable_id})).unwrap();
        assert_eq!(bound["changed"], true);
        let node = e.doc.get(NodeId(rect)).unwrap();
        assert_eq!(node.variables.bindings["fills/0"], variable_id);
        assert_eq!(node.fills[0].kind, desigma_doc::PaintKind::Solid(Color::from_hex("#2F5BFF").unwrap()));

        e.call("desigma_set_variable_mode", json!({"node_ids":[rect],"collection_id":collection_id,"mode":"Dark"})).unwrap();
        assert_eq!(e.doc.variable_mode(NodeId(rect), &collection_id), Some(dark));
        assert_eq!(e.doc.bound_value(NodeId(rect), "fills/0"), Some(VariableValue::Color(Color::from_hex("#8AA2FF").unwrap())));

        e.call("desigma_detach_variable", json!({"node_ids":[rect],"property":"fills/0"})).unwrap();
        let node = e.doc.get(NodeId(rect)).unwrap();
        assert!(!node.variables.bindings.contains_key("fills/0"));
        assert_eq!(node.fills[0].kind, desigma_doc::PaintKind::Solid(Color::from_hex("#8AA2FF").unwrap()));

        e.call("desigma_undo", json!({})).unwrap();
        let node = e.doc.get(NodeId(rect)).unwrap();
        assert_eq!(node.variables.bindings["fills/0"], variable_id);
        assert_eq!(node.fills[0].kind, desigma_doc::PaintKind::Solid(Color::from_hex("#2F5BFF").unwrap()));
    }

    #[test]
    fn variable_binding_multiple_nodes_is_one_undo_step() {
        let mut e = Engine::default();
        let collection = e.call("desigma_create_variable_collection", json!({"name":"Theme","modes":["Light"]})).unwrap()["collection"].clone();
        let collection_id = collection["id"].as_str().unwrap().to_string();
        let variable = e
            .call("desigma_create_variable", json!({"collection_id":collection_id,"name":"Brand","kind":"color","values":{"Light":"#2F5BFF"}}))
            .unwrap()["variable"]
            .clone();
        let variable_id = variable["id"].as_str().unwrap().to_string();
        let a = create(&mut e, "rectangle", "A");
        let b = create(&mut e, "rectangle", "B");
        e.call("desigma_bind_variable", json!({"node_ids":[a,b],"property":"fills/0","variable_id":variable_id})).unwrap();
        assert!(e.doc.get(NodeId(a)).unwrap().variables.bindings.contains_key("fills/0"));
        assert!(e.doc.get(NodeId(b)).unwrap().variables.bindings.contains_key("fills/0"));
        e.call("desigma_undo", json!({})).unwrap();
        assert!(!e.doc.get(NodeId(a)).unwrap().variables.bindings.contains_key("fills/0"));
        assert!(!e.doc.get(NodeId(b)).unwrap().variables.bindings.contains_key("fills/0"));
        e.call("desigma_redo", json!({})).unwrap();
        assert!(e.doc.get(NodeId(a)).unwrap().variables.bindings.contains_key("fills/0"));
        assert!(e.doc.get(NodeId(b)).unwrap().variables.bindings.contains_key("fills/0"));
    }

    #[test]
    fn variable_binding_rejects_invalid_input_atomically() {
        let mut e = Engine::default();
        let collection = e.call("desigma_create_variable_collection", json!({"name":"Theme","modes":["Light","Dark"],"default_mode":"Light"})).unwrap()
            ["collection"]
            .clone();
        let collection_id = collection["id"].as_str().unwrap().to_string();
        let color = e.call("desigma_create_variable", json!({"collection_id":collection_id,"name":"Color/Brand","kind":"color"})).unwrap()["variable"].clone();
        let color_id = color["id"].as_str().unwrap().to_string();
        let number =
            e.call("desigma_create_variable", json!({"collection_id":collection_id,"name":"Size/Radius","kind":"number"})).unwrap()["variable"].clone();
        let number_id = number["id"].as_str().unwrap().to_string();
        let text_only = e
            .call("desigma_create_variable", json!({"collection_id":collection_id,"name":"Text/Strong","kind":"color","scopes":["TEXT_FILL"]}))
            .unwrap()["variable"]
            .clone();
        let text_only_id = text_only["id"].as_str().unwrap().to_string();
        let rect = create(&mut e, "rectangle", "Card");
        let before = e.doc.to_json().unwrap();
        for (name, args) in [
            ("desigma_bind_variable", json!({"node_ids":[1],"property":"fills/0","variable_id":color_id})),
            ("desigma_bind_variable", json!({"node_ids":[9999],"property":"fills/0","variable_id":color_id})),
            ("desigma_bind_variable", json!({"node_ids":[rect],"property":"banana","variable_id":color_id})),
            ("desigma_bind_variable", json!({"node_ids":[rect],"property":"fills/0","variable_id":number_id})),
            ("desigma_bind_variable", json!({"node_ids":[rect],"property":"fills/0","variable_id":text_only_id})),
            ("desigma_bind_variable", json!({"node_ids":[],"property":"fills/0","variable_id":color_id})),
            ("desigma_bind_variable", json!({"node_ids":vec![rect;501],"property":"fills/0","variable_id":color_id})),
            ("desigma_detach_variable", json!({"node_ids":[1],"property":"fills/0"})),
            ("desigma_detach_variable", json!({"node_ids":[rect],"property":"banana"})),
            ("desigma_set_variable_mode", json!({"node_ids":[rect],"collection_id":"missing"})),
            ("desigma_set_variable_mode", json!({"node_ids":[rect],"collection_id":collection_id,"mode":"Missing"})),
            ("desigma_set_variable_mode", json!({"node_ids":[1],"collection_id":collection_id})),
            ("desigma_set_variable_mode", json!({"node_ids":[],"collection_id":collection_id})),
        ] {
            let err = e.call(name, args).unwrap_err();
            assert!(!err.is_empty(), "{name}");
            assert_eq!(e.doc.to_json().unwrap(), before, "{name} changed the document");
        }
    }

    #[test]
    fn variable_mode_can_be_set_and_cleared() {
        let mut e = Engine::default();
        let collection = e.call("desigma_create_variable_collection", json!({"name":"Theme","modes":["Light","Dark"]})).unwrap()["collection"].clone();
        let collection_id = collection["id"].as_str().unwrap().to_string();
        let dark = collection["modes"].as_array().unwrap().iter().find(|m| m["name"] == "Dark").unwrap()["id"].as_str().unwrap().to_string();
        let rect = create(&mut e, "rectangle", "Card");
        let set = e.call("desigma_set_variable_mode", json!({"node_ids":[rect],"collection_id":collection_id,"mode":"Dark"})).unwrap();
        assert_eq!(set["changed"], true);
        assert_eq!(e.doc.get(NodeId(rect)).unwrap().variables.modes[&collection_id], dark);
        let cleared = e.call("desigma_set_variable_mode", json!({"node_ids":[rect],"collection_id":collection_id})).unwrap();
        assert_eq!(cleared["changed"], true);
        assert!(!e.doc.get(NodeId(rect)).unwrap().variables.modes.contains_key(&collection_id));
    }

    #[test]
    fn variable_tools_are_registered_as_writing_tools() {
        for name in [
            "desigma_create_variable_collection",
            "desigma_add_variable_mode",
            "desigma_create_variable",
            "desigma_set_variable_value",
            "desigma_bind_variable",
            "desigma_detach_variable",
            "desigma_set_variable_mode",
        ] {
            let t = tools().into_iter().find(|t| t["name"] == name).unwrap_or_else(|| panic!("missing {name}"));
            assert_eq!(t["annotations"]["readOnlyHint"], false, "{name}");
            assert_eq!(t["inputSchema"]["additionalProperties"], false, "{name}");
        }
        let set = tools().into_iter().find(|t| t["name"] == "desigma_set_variable_value").unwrap();
        assert_eq!(set["annotations"]["idempotentHint"], true);
        assert_eq!(set["inputSchema"]["required"], json!(["variable_id", "value"]));
        let bind = tools().into_iter().find(|t| t["name"] == "desigma_bind_variable").unwrap();
        assert_eq!(bind["annotations"]["idempotentHint"], true);
        assert_eq!(bind["inputSchema"]["required"], json!(["node_ids", "property", "variable_id"]));
    }

    #[test]
    fn auto_layout_updates_removes_and_validates() {
        let mut e = Engine::default();
        let frame = create(&mut e, "frame", "Row");
        let result = e
            .call(
                "desigma_set_auto_layout",
                json!({
                    "node_id":frame,
                    "direction":"vertical",
                    "wrap":true,
                    "item_spacing":12,
                    "counter_spacing":8,
                    "padding":[16,12,16,12],
                    "primary_align":"center",
                    "counter_align":"space_between",
                    "primary_sizing":"hug",
                    "counter_sizing":"fill",
                    "stroke_included":true,
                    "reverse_z":true
                }),
            )
            .unwrap();
        assert_eq!(result["changed"], true);
        let layout = e.doc.get(NodeId(frame)).unwrap().kind.frame_props().unwrap().auto_layout.unwrap();
        assert_eq!(layout.direction, LayoutDirection::Vertical);
        assert!(layout.wrap);
        assert_eq!(layout.item_spacing, 12.0);
        assert_eq!(layout.counter_spacing, 8.0);
        assert_eq!(layout.padding, [16.0, 12.0, 16.0, 12.0]);
        assert_eq!(layout.primary_align, LayoutAlign::Center);
        assert_eq!(layout.counter_align, LayoutAlign::SpaceBetween);
        assert_eq!(layout.primary_sizing, Sizing::Hug);
        assert_eq!(layout.counter_sizing, Sizing::Fill);
        assert!(layout.stroke_included);
        assert!(layout.reverse_z);

        let removed = e.call("desigma_set_auto_layout", json!({"node_id":frame,"enabled":false})).unwrap();
        assert_eq!(removed["changed"], true);
        assert!(e.doc.get(NodeId(frame)).unwrap().kind.frame_props().unwrap().auto_layout.is_none());
        e.call("desigma_undo", json!({})).unwrap();
        assert!(e.doc.get(NodeId(frame)).unwrap().kind.frame_props().unwrap().auto_layout.is_some());

        let before = e.doc.to_json().unwrap();
        for (name, args) in [
            ("desigma_set_auto_layout", json!({"node_id":1})),
            ("desigma_set_auto_layout", json!({"node_id":frame,"padding":[1,2,3]})),
            ("desigma_set_auto_layout", json!({"node_id":frame,"item_spacing":-1})),
            ("desigma_set_auto_layout", json!({"node_id":frame,"direction":"diagonal"})),
            ("desigma_set_auto_layout", json!({"node_id":frame,"enabled":false,"direction":"vertical"})),
        ] {
            let err = e.call(name, args).unwrap_err();
            assert!(!err.is_empty(), "{name}");
            assert_eq!(e.doc.to_json().unwrap(), before, "{name} changed the document");
        }
    }

    #[test]
    fn layout_child_requires_auto_layout_and_sets_limits() {
        let mut e = Engine::default();
        let parent = create(&mut e, "frame", "Row");
        let child = e.call("desigma_create_node", json!({"parent_id":parent,"kind":"rectangle","name":"Item"})).unwrap()["node"]["id"].as_u64().unwrap();
        let before = e.doc.to_json().unwrap();
        let err = e.call("desigma_set_layout_child", json!({"node_id":child,"horizontal":"fill"})).unwrap_err();
        assert!(err.contains("auto layout"), "{err}");
        assert_eq!(e.doc.to_json().unwrap(), before);
        e.call("desigma_set_auto_layout", json!({"node_id":parent,"direction":"horizontal"})).unwrap();
        let result = e
            .call(
                "desigma_set_layout_child",
                json!({"node_id":child,"horizontal":"fill","vertical":"fixed","grow":0.5,"min_w":100,"max_w":200,"min_h":20,"max_h":80,"absolute":false}),
            )
            .unwrap();
        assert_eq!(result["changed"], true);
        let layout = e.doc.get(NodeId(child)).unwrap().layout_child;
        assert_eq!(layout.horizontal, Sizing::Fill);
        assert_eq!(layout.vertical, Sizing::Fixed);
        assert_eq!(layout.grow, 0.5);
        assert_eq!(layout.min_w, Some(100.0));
        assert_eq!(layout.max_w, Some(200.0));
        assert_eq!(layout.min_h, Some(20.0));
        assert_eq!(layout.max_h, Some(80.0));
        let before = e.doc.to_json().unwrap();
        for (name, args) in [
            ("desigma_set_layout_child", json!({"node_id":1,"horizontal":"fill"})),
            ("desigma_set_layout_child", json!({"node_id":child,"min_w":100,"max_w":50})),
            ("desigma_set_layout_child", json!({"node_id":child,"horizontal":"stretch"})),
            ("desigma_set_layout_child", json!({"node_id":child,"grow":-1})),
        ] {
            let err = e.call(name, args).unwrap_err();
            assert!(!err.is_empty(), "{name}");
            assert_eq!(e.doc.to_json().unwrap(), before, "{name} changed the document");
        }
    }

    #[test]
    fn text_auto_resize_and_max_lines_are_editable() {
        let mut e = Engine::default();
        let text = create(&mut e, "text", "Label");
        let result = e.call("desigma_set_text_style", json!({"node_id":text,"auto_resize":"truncate","max_lines":2})).unwrap();
        assert_eq!(result["changed"], true);
        let props = e.doc.get(NodeId(text)).unwrap().kind.any_text().unwrap();
        assert_eq!(props.auto_resize, TextAutoResize::Truncate);
        assert_eq!(props.max_lines, Some(2));
        let before = e.doc.to_json().unwrap();
        for (name, args) in
            [("desigma_set_text_style", json!({"node_id":text,"auto_resize":"banana"})), ("desigma_set_text_style", json!({"node_id":text,"max_lines":0}))]
        {
            let err = e.call(name, args).unwrap_err();
            assert!(!err.is_empty(), "{name}");
            assert_eq!(e.doc.to_json().unwrap(), before, "{name} changed the document");
        }
        e.call("desigma_undo", json!({})).unwrap();
        assert_eq!(e.doc.get(NodeId(text)).unwrap().kind.any_text().unwrap().auto_resize, TextAutoResize::WidthAndHeight);
    }

    #[test]
    fn layout_tools_are_registered_as_writing_tools() {
        for name in ["desigma_set_auto_layout", "desigma_set_layout_child"] {
            let t = tools().into_iter().find(|t| t["name"] == name).unwrap_or_else(|| panic!("missing {name}"));
            assert_eq!(t["annotations"]["readOnlyHint"], false, "{name}");
            assert_eq!(t["annotations"]["idempotentHint"], true, "{name}");
            assert_eq!(t["inputSchema"]["required"], json!(["node_id"]), "{name}");
        }
    }

    #[test]
    fn batch_applies_node_edits_with_references_and_one_undo_step() {
        let mut e = Engine::default();
        let result = e
            .call(
                "desigma_batch",
                json!({"operations":[
                    {"tool":"desigma_create_node","arguments":{"parent_id":1,"kind":"frame","name":"Card","x":0,"y":0,"width":320,"height":200}},
                    {"tool":"desigma_create_node","arguments":{"parent_id":"$0","kind":"rectangle","name":"Inner","x":12,"y":12,"width":100,"height":40}},
                    {"tool":"desigma_update_node","arguments":{"node_id":"$1","name":"Inner renamed","x":16}},
                    {"tool":"desigma_set_appearance","arguments":{"node_id":"$1","fill":"#FF6B4A","corner_radius":8}},
                    {"tool":"desigma_duplicate_node","arguments":{"node_id":"$0","offset":20}},
                    {"tool":"desigma_update_node","arguments":{"node_id":"$4","name":"Card copy"}},
                    {"tool":"desigma_set_auto_layout","arguments":{"node_id":"$0","direction":"vertical","item_spacing":8,"padding":[12,12,12,12]}}
                ]}),
            )
            .unwrap();
        assert_eq!(result["changed"], true);
        assert_eq!(result["operations"], 7);
        let ids: Vec<u64> = result["results"].as_array().unwrap().iter().map(|r| r["node_id"].as_u64().unwrap()).collect();
        assert_eq!(ids.len(), 7);
        assert_eq!(e.doc.get(NodeId(1)).unwrap().children, vec![NodeId(ids[0]), NodeId(ids[4])]);
        let child = e.doc.get(NodeId(ids[1])).unwrap();
        assert_eq!(child.name, "Inner renamed");
        assert_eq!(child.transform.tx, 16.0);
        assert_eq!(child.fills[0].kind, desigma_doc::PaintKind::Solid(Color::from_hex("#FF6B4A").unwrap()));
        assert_eq!(e.doc.get(NodeId(ids[4])).unwrap().name, "Card copy");
        assert!(e.doc.get(NodeId(ids[0])).unwrap().kind.frame_props().unwrap().auto_layout.is_some());
        assert_eq!(e.history.undo_len(), 1);

        e.call("desigma_undo", json!({})).unwrap();
        assert!(e.doc.get(NodeId(1)).unwrap().children.is_empty());
        assert!(e.doc.get(NodeId(ids[0])).is_none());
        e.call("desigma_redo", json!({})).unwrap();
        assert_eq!(e.doc.get(NodeId(1)).unwrap().children.len(), 2);
    }

    #[test]
    fn batch_rejects_invalid_operations_without_touching_the_document() {
        let mut e = Engine::default();
        let base = create(&mut e, "rectangle", "Base");
        let before = e.doc.to_json().unwrap();
        let before_undo = e.history.undo_len();
        let too_many: Vec<Value> = (0..101).map(|_| json!({"tool":"desigma_update_node","arguments":{"node_id":base}})).collect();
        for (name, args) in [
            ("desigma_batch", json!({"operations":[]})),
            ("desigma_batch", json!({"operations":[{"tool":"desigma_import_svg","arguments":{}}]})),
            ("desigma_batch", json!({"operations":too_many})),
            ("desigma_batch", json!({"operations":[{"tool":"desigma_update_node","arguments":{"node_id":9999}}]})),
            ("desigma_batch", json!({"operations":[{"tool":"desigma_update_node","arguments":{"node_id":"$5"}}]})),
            ("desigma_batch", json!({"operations":[{"tool":"desigma_update_node","arguments":[]}]})),
            ("desigma_batch", json!({"operations":[{"tool":"desigma_update_node","arguments":{"node_id":base,"width":-1}}]})),
        ] {
            let err = e.call(name, args).unwrap_err();
            assert!(!err.is_empty(), "{name}");
            assert_eq!(e.doc.to_json().unwrap(), before, "{name} changed the document");
            assert_eq!(e.history.undo_len(), before_undo, "{name} pushed history");
        }
    }

    #[test]
    fn batch_is_registered_as_a_writing_tool() {
        let t = tools().into_iter().find(|t| t["name"] == "desigma_batch").unwrap();
        assert_eq!(t["annotations"]["readOnlyHint"], false);
        assert_eq!(t["annotations"]["idempotentHint"], false);
        assert_eq!(t["inputSchema"]["required"], json!(["operations"]));
        assert!(t["inputSchema"]["properties"]["operations"]["items"]["properties"]["tool"]["enum"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "desigma_create_node"));
    }

    #[test]
    fn effects_are_set_cleared_and_undoable() {
        let mut e = Engine::default();
        let rect = create(&mut e, "rectangle", "Card");
        let result = e
            .call(
                "desigma_set_effects",
                json!({"node_id":rect,"effects":[
                    {"type":"drop_shadow","color":"#00000040","offset":{"x":0,"y":4},"radius":12,"spread":2,"show_behind":false},
                    {"type":"inner_shadow","color":"#FFFFFF80","offset":[0,0],"radius":4,"visible":false},
                    {"type":"layer_blur","radius":6},
                    {"type":"background_blur","radius":8,"visible":true}
                ]}),
            )
            .unwrap();
        assert_eq!(result["changed"], true);
        let effects = &e.doc.get(NodeId(rect)).unwrap().effects;
        assert_eq!(effects.len(), 4);
        assert!(matches!(&effects[0], Effect::DropShadow { radius, spread, show_behind, .. } if *radius == 12.0 && *spread == 2.0 && !*show_behind));
        assert!(matches!(&effects[1], Effect::InnerShadow { visible, spread, .. } if !*visible && *spread == 0.0));
        assert!(matches!(&effects[2], Effect::LayerBlur { radius, .. } if *radius == 6.0));
        assert!(matches!(&effects[3], Effect::BackgroundBlur { radius, visible } if *radius == 8.0 && *visible));

        e.call("desigma_set_effects", json!({"node_id":rect,"effects":[]})).unwrap();
        assert!(e.doc.get(NodeId(rect)).unwrap().effects.is_empty());
        e.call("desigma_undo", json!({})).unwrap();
        assert_eq!(e.doc.get(NodeId(rect)).unwrap().effects.len(), 4);
        e.call("desigma_redo", json!({})).unwrap();
        assert!(e.doc.get(NodeId(rect)).unwrap().effects.is_empty());
    }

    #[test]
    fn effects_reject_invalid_input_atomically() {
        let mut e = Engine::default();
        let rect = create(&mut e, "rectangle", "Card");
        let before = e.doc.to_json().unwrap();
        let too_many: Vec<Value> = (0..101).map(|_| json!({"type":"layer_blur","radius":2})).collect();
        for (name, args) in [
            ("desigma_set_effects", json!({"node_id":1,"effects":[]})),
            ("desigma_set_effects", json!({"node_id":rect,"effects":[{"type":"glow","radius":4}]})),
            ("desigma_set_effects", json!({"node_id":rect,"effects":[{"type":"drop_shadow","offset":{"x":0,"y":2},"radius":4}]})),
            ("desigma_set_effects", json!({"node_id":rect,"effects":[{"type":"drop_shadow","color":"not-a-color","offset":{"x":0,"y":2},"radius":4}]})),
            ("desigma_set_effects", json!({"node_id":rect,"effects":[{"type":"layer_blur","radius":-1}]})),
            ("desigma_set_effects", json!({"node_id":rect,"effects":[{"type":"layer_blur","radius":4,"extra":true}]})),
            ("desigma_set_effects", json!({"node_id":rect,"effects":[{"type":"layer_blur","radius":4,"offset":"bad"}]})),
            ("desigma_set_effects", json!({"node_id":rect,"effects":[{"type":"drop_shadow","color":"#000000","offset":{"x":0,"y":4,"z":1},"radius":2}]})),
            ("desigma_set_effects", json!({"node_id":rect,"effects":too_many})),
            ("desigma_set_effects", json!({"node_id":rect,"effects":5})),
        ] {
            let err = e.call(name, args).unwrap_err();
            assert!(!err.is_empty(), "{name}");
            assert_eq!(e.doc.to_json().unwrap(), before, "{name} changed the document");
        }
    }

    #[test]
    fn effects_tool_is_registered_as_a_writing_tool() {
        let t = tools().into_iter().find(|t| t["name"] == "desigma_set_effects").unwrap();
        assert_eq!(t["annotations"]["readOnlyHint"], false);
        assert_eq!(t["annotations"]["idempotentHint"], true);
        assert_eq!(t["inputSchema"]["required"], json!(["node_id", "effects"]));
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
        for language in ["css", "tailwind", "react", "swiftui", "compose", "flutter", "vue", "svg"] {
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
            fn doc_mut(&mut self) -> &mut Document {
                &mut self.e.doc
            }
        }
        let mut h = Counting { e: Engine::default(), execs: 0 };
        let id = call(&mut h, "desigma_create_node", json!({"parent_id":1,"kind":"frame","name":"A"})).unwrap()["node"]["id"].as_u64().unwrap();
        call(&mut h, "desigma_update_node", json!({"node_id":id,"x":10.0})).unwrap();
        assert_eq!(h.execs, 2);
        assert!(call(&mut h, "desigma_undo", json!({})).unwrap()["changed"].as_bool().unwrap());
    }
    #[test]
    fn every_document_editing_tool_is_listed_in_edit_tools() {
        // Writing tools that are not document edits: they are handled by the transport itself.
        const OTHER: [&str; 4] = ["desigma_export_image", "desigma_export_animation", "desigma_select_nodes", "desigma_save_file"];
        for t in tools() {
            let name = t["name"].as_str().unwrap();
            if t["annotations"]["readOnlyHint"] == false && !OTHER.contains(&name) {
                assert!(EDIT_TOOLS.contains(&name), "{name} edits the document but is missing from EDIT_TOOLS");
            }
        }
    }

    #[test]
    fn svg_import_into_a_leaf_is_refused_and_keeps_no_image_bytes() {
        let mut e = Engine::default();
        let rect = call(&mut e, "desigma_create_node", json!({"parent_id":1,"kind":"rectangle"})).unwrap()["node"]["id"].as_u64().unwrap();
        let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";
        let svg = format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"4\" height=\"4\"><image width=\"4\" height=\"4\" href=\"data:image/png;base64,{png}\"/></svg>"
        );
        let err = call(&mut e, "desigma_import_svg", json!({"parent_id":rect,"svg":svg})).unwrap_err();
        assert!(err.contains("container"), "{err}");
        assert!(e.doc.images.is_empty());
        let ok = call(&mut e, "desigma_import_svg", json!({"parent_id":1,"svg":svg})).unwrap();
        assert_eq!(ok["images"], 1);
        assert_eq!(e.doc.images.len(), 1);
    }

    #[test]
    fn engine_tool_reference_lists_every_tool() {
        let reference = include_str!("../README.md");
        for tool in tools() {
            let name = tool["name"].as_str().unwrap();
            assert!(reference.contains(name), "{name} is missing from the MCP README tool table");
        }
    }

    #[cfg(feature = "server")]
    #[test]
    fn server_tool_reference_and_root_count_are_current() {
        let reference = include_str!("../README.md");
        let tools = crate::server::tools();
        for tool in &tools {
            let name = tool.name.as_ref();
            assert!(reference.contains(name), "{name} is missing from the MCP README tool table");
        }
        let root = include_str!("../../../README.md");
        let count = tools.len();
        assert!(root.contains(&format!("It provides {count} tools")), "the root README must advertise {count} tools");
    }

    #[test]
    fn export_image_is_annotated_as_a_writing_idempotent_tool() {
        let t = tools().into_iter().find(|t| t["name"] == "desigma_export_image").unwrap();
        assert_eq!(t["annotations"]["readOnlyHint"], false);
        assert_eq!(t["annotations"]["idempotentHint"], true);
    }

    #[test]
    fn shared_styles_create_apply_detach_and_undo() {
        let mut e = Engine::default();
        let rect = create(&mut e, "rectangle", "Card");
        let text = create(&mut e, "text", "Title");

        let fill = e.call("desigma_create_style", json!({"name":"Color/Brand","kind":"fill","color":"#2F5BFF","opacity":0.8})).unwrap();
        let fill_key = fill["style_key"].as_str().unwrap().to_string();
        assert_eq!(e.doc.style_meta[&fill_key].property, StyleProperty::Fill);
        assert!(matches!(e.doc.styles[&fill_key], Style::Fill(_)));

        e.call("desigma_apply_style", json!({"node_ids":[rect,text],"style":fill_key})).unwrap();
        let rn = e.doc.get(NodeId(rect)).unwrap();
        assert_eq!(rn.style_refs.get("fill").map(String::as_str), Some(fill_key.as_str()));
        assert_eq!(rn.fills[0].kind, desigma_doc::PaintKind::Solid(Color::from_hex("#2F5BFF").unwrap()));
        assert_eq!(rn.fills[0].opacity, 0.8);
        let tn = e.doc.get(NodeId(text)).unwrap();
        assert_eq!(tn.style_refs.get("fill").map(String::as_str), Some(fill_key.as_str()));

        e.call("desigma_detach_style", json!({"node_ids":[rect,text],"property":"fill"})).unwrap();
        assert!(!e.doc.get(NodeId(rect)).unwrap().style_refs.contains_key("fill"));
        assert_eq!(e.doc.get(NodeId(rect)).unwrap().fills[0].kind, desigma_doc::PaintKind::Solid(Color::from_hex("#2F5BFF").unwrap()));
        e.call("desigma_undo", json!({})).unwrap();
        assert_eq!(e.doc.get(NodeId(rect)).unwrap().style_refs.get("fill").map(String::as_str), Some(fill_key.as_str()));

        let text_style = e
            .call(
                "desigma_create_style",
                json!({"name":"Type/Headline","kind":"text","text":{"font_size":28,"font_weight":700,"line_height":34,"align_h":"center"}}),
            )
            .unwrap();
        let text_key = text_style["style_key"].as_str().unwrap().to_string();
        e.call("desigma_apply_style", json!({"node_ids":[text],"style":text_key})).unwrap();
        let tn = e.doc.get(NodeId(text)).unwrap();
        assert_eq!(tn.style_refs.get("text").map(String::as_str), Some(text_key.as_str()));
        assert_eq!(tn.kind.any_text().unwrap().style.font_size, 28.0);
        assert_eq!(tn.kind.any_text().unwrap().style.font_weight, 700);
        e.call("desigma_undo", json!({})).unwrap();
        assert!(!e.doc.get(NodeId(text)).unwrap().style_refs.contains_key("text"));

        let stroke = e.call("desigma_create_style", json!({"name":"Stroke/Line","kind":"stroke","color":"#101828"})).unwrap();
        let stroke_key = stroke["style_key"].as_str().unwrap().to_string();
        e.call("desigma_apply_style", json!({"node_ids":[rect],"style":stroke_key})).unwrap();
        let rn = e.doc.get(NodeId(rect)).unwrap();
        assert_eq!(rn.style_refs.get("stroke").map(String::as_str), Some(stroke_key.as_str()));
        assert_eq!(rn.stroke.paints[0].kind, desigma_doc::PaintKind::Solid(Color::from_hex("#101828").unwrap()));
    }

    #[test]
    fn shared_styles_reject_invalid_input_atomically() {
        let mut e = Engine::default();
        let rect = create(&mut e, "rectangle", "Card");
        let text_style = e.call("desigma_create_style", json!({"name":"Type/Body","kind":"text","text":{"font_size":15}})).unwrap();
        let text_key = text_style["style_key"].as_str().unwrap().to_string();
        e.doc.styles.insert("Effect/Glow".into(), Style::Effect(vec![]));
        let before = e.doc.to_json().unwrap();
        for (name, args) in [
            ("desigma_create_style", json!({"name":"Bad","kind":"fill"})),
            ("desigma_create_style", json!({"name":"Bad","kind":"fill","color":"not-a-color"})),
            ("desigma_create_style", json!({"name":"Bad","kind":"fill","color":"#ffffff","opacity":2})),
            ("desigma_create_style", json!({"name":"Bad","kind":"glow"})),
            ("desigma_create_style", json!({"name":"Bad","kind":"text","color":"#ffffff","text":{"font_size":15}})),
            ("desigma_create_style", json!({"name":"Bad","kind":"text"})),
            ("desigma_create_style", json!({"name":"Bad","kind":"text","text":{"font_size":15,"extra":true}})),
            ("desigma_apply_style", json!({"node_ids":[rect],"style":"missing"})),
            ("desigma_apply_style", json!({"node_ids":[rect],"style":text_key})),
            ("desigma_apply_style", json!({"node_ids":[1],"style":text_key})),
            ("desigma_apply_style", json!({"node_ids":[],"style":text_key})),
            ("desigma_apply_style", json!({"node_ids":[rect],"style":"Effect/Glow"})),
            ("desigma_detach_style", json!({"node_ids":[rect],"property":"banana"})),
            ("desigma_detach_style", json!({"node_ids":[],"property":"fill"})),
        ] {
            let err = e.call(name, args).unwrap_err();
            assert!(!err.is_empty(), "{name}");
            assert_eq!(e.doc.to_json().unwrap(), before, "{name} changed the document");
        }
    }

    #[test]
    fn style_tools_are_registered_as_writing_tools() {
        for name in ["desigma_create_style", "desigma_apply_style", "desigma_detach_style"] {
            let t = tools().into_iter().find(|t| t["name"] == name).unwrap_or_else(|| panic!("missing {name}"));
            assert_eq!(t["annotations"]["readOnlyHint"], false, "{name}");
        }
        let apply = tools().into_iter().find(|t| t["name"] == "desigma_apply_style").unwrap();
        assert_eq!(apply["inputSchema"]["required"], json!(["node_ids", "style"]));
    }
}
