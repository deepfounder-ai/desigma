use desigma_doc::{Command, Document, NodeId};
use desigma_mcp::apply_nodes::apply_nodes;
use desigma_mcp::engine::{validate_document, Engine, Host};
use serde_json::json;

fn page(e: &mut Engine) -> u64 {
    e.call("desigma_get_document", json!({})).unwrap()["pages"][0]["node_id"].as_u64().unwrap()
}

fn id(r: &serde_json::Value, i: usize) -> u64 {
    r["nodes"][i]["node_id"].as_u64().unwrap()
}

#[test]
fn growing_a_container_relayouts_hugging_ancestors() {
    let mut e = Engine::default();
    let p = page(&mut e);
    let r = e
        .call(
            "desigma_apply_nodes",
            json!({"parent_id":p,"nodes":[{"kind":"frame","name":"Screen","width":300,"height":"hug","auto_layout":{"direction":"vertical","gap":10},"children":[
                {"kind":"frame","name":"Card","width":"fill","height":"hug","auto_layout":{"direction":"vertical","gap":0},
                 "children":[{"kind":"rectangle","name":"A","width":"fill","height":50}]},
                {"kind":"rectangle","name":"Footer","width":"fill","height":20}]}]}),
        )
        .unwrap();
    let (screen, card, footer) = (id(&r, 0), id(&r, 1), id(&r, 3));
    assert_eq!(e.doc.get(NodeId(screen)).unwrap().size.y, 80.0);
    e.call("desigma_apply_nodes", json!({"parent_id":card,"nodes":[{"kind":"rectangle","name":"B","width":"fill","height":30}]})).unwrap();
    validate_document(&e.doc).unwrap();
    assert_eq!(e.doc.get(NodeId(card)).unwrap().size.y, 80.0);
    assert_eq!(e.doc.get(NodeId(footer)).unwrap().transform.ty, 90.0, "the sibling below moved down");
    assert_eq!(e.doc.get(NodeId(screen)).unwrap().size.y, 110.0, "the hugging ancestor grew");
}

#[test]
fn replace_children_removes_reorders_and_adds_in_one_step() {
    let mut e = Engine::default();
    let p = page(&mut e);
    let r = e
        .call(
            "desigma_apply_nodes",
            json!({"parent_id":p,"nodes":[{"kind":"frame","name":"List","width":100,"height":"hug","auto_layout":{"direction":"vertical","gap":0},"children":[
                {"kind":"rectangle","name":"One","width":"fill","height":10},
                {"kind":"rectangle","name":"Two","width":"fill","height":20},
                {"kind":"rectangle","name":"Three","width":"fill","height":30}]}]}),
        )
        .unwrap();
    let (list, one, two, three) = (id(&r, 0), id(&r, 1), id(&r, 2), id(&r, 3));
    let r = e
        .call(
            "desigma_apply_nodes",
            json!({"nodes":[{"node_id":list,"replace_children":true,"children":[
                {"node_id":three},{"kind":"rectangle","name":"New","width":"fill","height":5},{"node_id":one}]}]}),
        )
        .unwrap();
    assert_eq!((r["removed"].as_u64(), r["undo_steps"].as_u64()), (Some(1), Some(1)));
    validate_document(&e.doc).unwrap();
    let names: Vec<String> = e.doc.get(NodeId(list)).unwrap().children.iter().map(|c| e.doc.get(*c).unwrap().name.clone()).collect();
    assert_eq!(names, ["Three", "New", "One"]);
    assert!(e.doc.get(NodeId(two)).is_none());
    assert_eq!(e.doc.get(NodeId(list)).unwrap().size.y, 45.0);
    assert_eq!(e.doc.get(NodeId(one)).unwrap().transform.ty, 35.0);
    e.call("desigma_undo", json!({})).unwrap();
    assert_eq!(e.doc.get(NodeId(list)).unwrap().children, vec![NodeId(one), NodeId(two), NodeId(three)]);
    assert!(e.call("desigma_apply_nodes", json!({"nodes":[{"node_id":list,"replace_children":true}]})).is_err());
    assert!(e.call("desigma_apply_nodes", json!({"nodes":[{"node_id":list,"replace_children":true,"children":[{"node_id":one},{"node_id":one}]}]})).is_err());
    let none = e.call("desigma_apply_nodes", json!({"nodes":[{"node_id":list,"replace_children":true,"children":[]}]})).unwrap();
    assert_eq!(none["removed"], 3);
}

#[test]
fn parent_removed_by_replace_children_is_a_per_node_error() {
    let mut e = Engine::default();
    let p = page(&mut e);
    let r = e
        .call(
            "desigma_apply_nodes",
            json!({"parent_id":p,"nodes":[{"kind":"frame","name":"A","width":100,"height":100,"children":[{"kind":"frame","name":"B","width":10,"height":10}]}]}),
        )
        .unwrap();
    let (a, b) = (id(&r, 0), id(&r, 1));
    let before = e.doc.nodes.clone();
    let err = e
        .call(
            "desigma_apply_nodes",
            json!({"nodes":[
                {"node_id":a,"replace_children":true,"children":[]},
                {"kind":"rectangle","parent_id":b,"width":5,"height":5}]}),
        )
        .unwrap_err();
    assert!(err.contains("nodes[1]") && err.contains("replace_children"), "{err}");
    assert_eq!(e.doc.nodes, before);
    assert!(e.doc.get(NodeId(b)).is_some());
}

/// An engine whose edits leave the document invalid, to reach the validation rollback.
struct Corrupting(Engine);
impl Host for Corrupting {
    fn doc(&self) -> &Document {
        self.0.doc()
    }
    fn exec(&mut self, cmd: Command) -> Result<(), String> {
        self.0.exec(cmd)?;
        let page = self.0.doc.pages[0];
        if let Some(last) = self.0.doc.get(page).and_then(|p| p.children.last().copied()) {
            self.0.doc.pages.push(last);
        }
        Ok(())
    }
    fn undo(&mut self) -> Result<bool, String> {
        let r = self.0.undo();
        self.0.doc.pages.truncate(1);
        r
    }
    fn redo(&mut self) -> Result<bool, String> {
        self.0.redo()
    }
    fn discard_redo(&mut self) {
        self.0.discard_redo()
    }
    fn can_undo(&self) -> bool {
        self.0.can_undo()
    }
    fn can_redo(&self) -> bool {
        self.0.can_redo()
    }
    fn alloc_id(&mut self) -> Result<NodeId, String> {
        self.0.alloc_id()
    }
    fn doc_mut(&mut self) -> &mut Document {
        self.0.doc_mut()
    }
}

#[test]
fn validation_rollback_leaves_nothing_to_redo() {
    let mut h = Corrupting(Engine::default());
    let p = page(&mut h.0);
    let err = apply_nodes(&mut h, json!({"parent_id":p,"nodes":[{"kind":"rectangle","width":5,"height":5}]})).unwrap_err();
    assert!(err.contains("rolled back"), "{err}");
    assert!(!h.can_redo(), "a rolled back step must not be redoable");
    assert!(!h.0.doc.nodes.values().any(|n| n.kind.default_name() == "Rectangle"));
}
