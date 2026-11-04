use desigma_doc::{NodeId, NodeKind, PaintKind, VariableValue};
use desigma_mcp::engine::{validate_document, Engine};
use serde_json::json;

fn page(e: &mut Engine) -> u64 {
    e.call("desigma_get_document", json!({})).unwrap()["pages"][0]["node_id"].as_u64().unwrap()
}

fn tokens(e: &mut Engine) -> serde_json::Value {
    e.call(
        "desigma_set_variables",
        json!({"collections":[{"name":"Brand","modes":["Light","Dark"],"variables":[
            {"name":"Color/Primary","type":"color","values":{"Light":"#3366ff","Dark":"#88aaff"}},
            {"name":"Space/M","type":"number","value":16},
            {"name":"Radius/L","type":"number","value":12},
            {"name":"Flag","type":"boolean","value":true},
            {"name":"Title","type":"string","value":"Hello"}]}]}),
    )
    .unwrap()
}

#[test]
fn creates_and_updates_collections_modes_and_values_by_name() {
    let mut e = Engine::default();
    let r = tokens(&mut e);
    assert_eq!(r["undo_steps"], 1);
    assert_eq!(e.doc.variables.len(), 5);
    let coll = e.doc.variable_collections.values().next().unwrap();
    assert_eq!(coll.modes.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(), ["Light", "Dark"]);
    // Repeating is a no-op; a changed value and a new variable update in place.
    assert_eq!(tokens(&mut e)["changed"], false);
    let r = e
        .call(
            "desigma_set_variables",
            json!({"collections":[{"name":"Brand","variables":[{"name":"Space/M","type":"number","value":20},{"name":"Space/S","type":"number","value":8}]}]}),
        )
        .unwrap();
    assert_eq!(r["changed"], true);
    assert_eq!(e.doc.variables.len(), 6);
    let m = e.doc.variables.values().find(|v| v.name == "Space/M").unwrap();
    assert!(m.values.values().all(|v| *v == VariableValue::Number(20.0)));
    let err = e
        .call("desigma_set_variables", json!({"collections":[{"name":"Brand","variables":[{"name":"Space/M","type":"color","value":"#fff"},{"name":"X","type":"number","values":{"Nope":1}}]}]}))
        .unwrap_err();
    assert!(err.contains("different type") && err.contains("unknown mode"), "{err}");
    assert_eq!(e.doc.variables.len(), 6, "a failed call changes nothing");
}

#[test]
fn apply_nodes_binds_variables_by_name() {
    let mut e = Engine::default();
    let p = page(&mut e);
    tokens(&mut e);
    let r = e
        .call(
            "desigma_apply_nodes",
            json!({"parent_id":p,"nodes":[{"kind":"frame","name":"Card","width":"hug","height":"hug","fill":"#000000",
                "auto_layout":{"direction":"vertical","gap":1,"padding":1},
                "bind":{"fill":"Color/Primary","gap":"Space/M","padding":"Space/M","radius":"Radius/L"},
                "children":[{"kind":"text","name":"T","text":"x","bind":{"text":"Title"}}]}]}),
        )
        .unwrap();
    validate_document(&e.doc).unwrap();
    let id = r["nodes"][0]["node_id"].as_u64().unwrap();
    let card = e.doc.get(NodeId(id)).unwrap();
    assert!(matches!(card.fills[0].kind, PaintKind::Solid(c) if c.b > 0.9 && c.r < 0.3));
    assert_eq!(card.corner_radii, [12.0; 4]);
    assert!(card.variables.bindings.contains_key("itemSpacing"));
    let al = card.kind.frame_props().unwrap().auto_layout.unwrap();
    assert_eq!((al.item_spacing, al.padding), (16.0, [16.0; 4]));
    assert!(card.size.x >= 32.0, "layout uses the bound padding");
    let NodeKind::Text(t) = &e.doc.get(card.children[0]).unwrap().kind else { panic!("text") };
    assert_eq!(t.content, "Hello");

    // Wrong kind, unknown name and a missing target all fail with a reason.
    let bad = e.call("desigma_bind_variable", json!({"node_id":id,"bindings":{"gap":"Color/Primary","radius":"Nope"}})).unwrap_err();
    assert!(bad.contains("needs a Number") && bad.contains("no variable named"), "{bad}");
    let r = e.call("desigma_bind_variable", json!({"node_id":id,"bindings":{"gap":null}})).unwrap();
    assert_eq!(r["changed"], true);
    assert!(!e.doc.get(NodeId(id)).unwrap().variables.bindings.contains_key("itemSpacing"));
}
