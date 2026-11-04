use desigma_doc::{NodeId, NodeKind, OverrideField, PaintKind};
use desigma_mcp::engine::{validate_document, Engine};
use serde_json::json;

fn page(e: &mut Engine) -> u64 {
    e.call("desigma_get_document", json!({})).unwrap()["pages"][0]["node_id"].as_u64().unwrap()
}

fn button(e: &mut Engine, p: u64) -> u64 {
    let r = e
        .call(
            "desigma_apply_nodes",
            json!({"parent_id":p,"nodes":[{"kind":"frame","name":"Button","width":"hug","height":"hug",
                "auto_layout":{"direction":"horizontal","padding":[8,16],"gap":8},"fill":"#3366ff",
                "children":[
                    {"kind":"rectangle","name":"Icon","width":16,"height":16,"fill":"#ffffff"},
                    {"kind":"text","name":"Label","text":"Click","text_style":{"font_size":14,"color":"#ffffff"}}]}]}),
        )
        .unwrap();
    r["nodes"][0]["node_id"].as_u64().unwrap()
}

fn child(e: &Engine, parent: u64, name: &str) -> NodeId {
    let p = e.doc.get(NodeId(parent)).unwrap();
    *p.children.iter().find(|c| e.doc.get(**c).unwrap().name == name).unwrap()
}

#[test]
fn creates_a_component_and_instances_with_overrides() {
    let mut e = Engine::default();
    let p = page(&mut e);
    let b = button(&mut e, p);
    let c = e.call("desigma_create_component", json!({"node_id":b})).unwrap();
    assert_eq!(c["component_id"], b);
    assert!(matches!(e.doc.get(NodeId(b)).unwrap().kind, NodeKind::Component(_)));
    assert!(e.call("desigma_create_component", json!({"node_id":b})).is_err());

    let r = e
        .call(
            "desigma_create_instance",
            json!({"component_id":b,"parent_id":p,"x":300,"y":40,
                "overrides":{"Label":{"text":"Buy now","fill":"#ff0000"},"Icon":{"visible":false}}}),
        )
        .unwrap();
    assert_eq!(r["undo_steps"], 1);
    validate_document(&e.doc).unwrap();
    let i = r["instance_id"].as_u64().unwrap();
    let inst = e.doc.get(NodeId(i)).unwrap();
    assert!(matches!(inst.kind, NodeKind::Instance { component: Some(c), .. } if c == NodeId(b)));
    assert_eq!((inst.transform.tx, inst.transform.ty), (300.0, 40.0));
    let label = e.doc.get(child(&e, i, "Label")).unwrap();
    let NodeKind::Text(t) = &label.kind else { panic!("text") };
    assert_eq!(t.content, "Buy now");
    assert!(matches!(t.style.fills[0].kind, PaintKind::Solid(c) if c.r > 0.9 && c.g < 0.1));
    assert!(label.component.overrides.contains(&OverrideField::Characters));
    let icon = e.doc.get(child(&e, i, "Icon")).unwrap();
    assert!(!icon.visible && icon.component.overrides.contains(&OverrideField::Visible));
    // The main component is untouched.
    assert!(e.doc.get(child(&e, b, "Icon")).unwrap().visible);

    let undo = e.call("desigma_undo", json!({})).unwrap();
    assert_eq!(undo["changed"], true);
    assert!(e.doc.get(NodeId(i)).is_none());
}

#[test]
fn instance_of_works_inside_apply_nodes_trees() {
    let mut e = Engine::default();
    let p = page(&mut e);
    let b = button(&mut e, p);
    e.call("desigma_create_component", json!({"node_id":b})).unwrap();
    let r = e
        .call(
            "desigma_apply_nodes",
            json!({"parent_id":p,"nodes":[{"kind":"frame","name":"Row","width":"hug","height":"hug",
                "auto_layout":{"direction":"horizontal","gap":10},
                "children":[{"instance_of":b,"overrides":{"Label":{"text":"A"}}},{"instance_of":b,"overrides":{"Label":{"text":"A much longer label"}}}]}]}),
        )
        .unwrap();
    assert_eq!(r["created"], 1 + 2 * 3);
    validate_document(&e.doc).unwrap();
    let row = r["nodes"][0]["node_id"].as_u64().unwrap();
    let kids = e.doc.get(NodeId(row)).unwrap().children.clone();
    let (a, b2) = (e.doc.get(kids[0]).unwrap(), e.doc.get(kids[1]).unwrap());
    assert!(b2.size.x > a.size.x, "text override is refitted by the layout");
    assert!((b2.transform.tx - (a.size.x + 10.0)).abs() < 0.01);
    let bad =
        e.call("desigma_apply_nodes", json!({"parent_id":p,"nodes":[{"instance_of":row},{"instance_of":b,"overrides":{"Nope":{"text":"x"}}}]})).unwrap_err();
    assert!(bad.contains("not a main component") && bad.contains("no layer named \"Nope\""), "{bad}");
}

#[test]
fn override_instance_and_swap() {
    let mut e = Engine::default();
    let p = page(&mut e);
    let b = button(&mut e, p);
    e.call("desigma_create_component", json!({"node_id":b})).unwrap();
    let other = e.call("desigma_apply_nodes", json!({"parent_id":p,"nodes":[{"kind":"ellipse","name":"Dot","width":16,"height":16}]})).unwrap();
    let dot = other["nodes"][0]["node_id"].as_u64().unwrap();
    let comp = e.call("desigma_create_component", json!({"node_id":dot})).unwrap()["component_id"].as_u64().unwrap();
    assert_ne!(comp, dot, "a shape is wrapped in a component frame");
    let i = e.call("desigma_create_instance", json!({"component_id":b,"parent_id":p})).unwrap()["instance_id"].as_u64().unwrap();
    let r = e.call("desigma_override_instance", json!({"instance_id":i,"overrides":{"Label":{"text":"Hi"}}})).unwrap();
    assert!(r["updated"].as_u64().unwrap() >= 1);
    let NodeKind::Text(t) = &e.doc.get(child(&e, i, "Label")).unwrap().kind else { panic!("text") };
    assert_eq!(t.content, "Hi");
    assert!(e.call("desigma_override_instance", json!({"instance_id":b,"overrides":{}})).is_err(), "main component is not an instance");
    assert!(e
        .call("desigma_override_instance", json!({"instance_id":i,"overrides":{"Icon":{"swap":comp}}}))
        .unwrap_err()
        .contains("swap only works on an instance"));
    // A card holding a button instance; a new card swaps that button for the dot.
    let card = e
        .call(
            "desigma_apply_nodes",
            json!({"parent_id":p,"nodes":[{"kind":"frame","name":"Card","width":200,"height":80,"children":[{"instance_of":b,"name":"Action"}]}]}),
        )
        .unwrap()["nodes"][0]["node_id"]
        .as_u64()
        .unwrap();
    e.call("desigma_create_component", json!({"node_id":card})).unwrap();
    let c2 = e.call("desigma_create_instance", json!({"component_id":card,"parent_id":p,"overrides":{"Action":{"swap":comp}}})).unwrap();
    validate_document(&e.doc).unwrap();
    let action = e.doc.get(e.doc.get(NodeId(c2["instance_id"].as_u64().unwrap())).unwrap().children[0]).unwrap();
    assert!(matches!(action.kind, NodeKind::Instance { component: Some(c), .. } if c == NodeId(comp)));
    let c3 = e.call("desigma_create_instance", json!({"component_id":card,"parent_id":p})).unwrap()["instance_id"].as_u64().unwrap();
    e.call("desigma_override_instance", json!({"instance_id":c3,"overrides":{"Action":{"swap":comp}}})).unwrap();
    let action = e.doc.get(e.doc.get(NodeId(c3)).unwrap().children[0]).unwrap();
    assert!(matches!(action.kind, NodeKind::Instance { component: Some(c), .. } if c == NodeId(comp)));
}

#[test]
fn override_instance_is_all_or_nothing_and_one_undo_step() {
    let mut e = Engine::default();
    let p = page(&mut e);
    let b = button(&mut e, p);
    e.call("desigma_create_component", json!({"node_id":b})).unwrap();
    let dot = e.call("desigma_apply_nodes", json!({"parent_id":p,"nodes":[{"kind":"ellipse","name":"Dot","width":16,"height":16}]})).unwrap()["nodes"][0]
        ["node_id"]
        .as_u64()
        .unwrap();
    let comp = e.call("desigma_create_component", json!({"node_id":dot})).unwrap()["component_id"].as_u64().unwrap();
    let card = e
        .call(
            "desigma_apply_nodes",
            json!({"parent_id":p,"nodes":[{"kind":"frame","name":"Card","width":200,"height":80,"children":[{"instance_of":b,"name":"Action"}]}]}),
        )
        .unwrap()["nodes"][0]["node_id"]
        .as_u64()
        .unwrap();
    e.call("desigma_create_component", json!({"node_id":card})).unwrap();
    let i = e.call("desigma_create_instance", json!({"component_id":card,"parent_id":p})).unwrap()["instance_id"].as_u64().unwrap();
    let before = e.doc.to_json().unwrap();
    let nodes_before = e.doc.nodes.clone();
    let steps = e.history.undo_len();

    // A valid swap next to an invalid override: nothing may change.
    let err = e.call("desigma_override_instance", json!({"instance_id":i,"overrides":{"Action":{"swap":comp},"Nope":{"visible":false}}})).unwrap_err();
    assert!(err.contains("Nope"), "{err}");
    assert_eq!(e.doc.to_json().unwrap(), before);
    assert_eq!(e.history.undo_len(), steps);
    let err = e.call("desigma_override_instance", json!({"instance_id":i,"overrides":{"Action":{"swap":comp,"opacity":7}}})).unwrap_err();
    assert!(err.contains("opacity"), "{err}");
    assert_eq!(e.doc.to_json().unwrap(), before);
    assert_eq!(e.history.undo_len(), steps);

    // A valid pair is one undo step.
    let r = e.call("desigma_override_instance", json!({"instance_id":i,"overrides":{"Action":{"swap":comp,"opacity":0.5}}})).unwrap();
    assert_eq!(r["swapped"], 1);
    assert_eq!(e.history.undo_len(), steps + 1);
    validate_document(&e.doc).unwrap();
    e.call("desigma_undo", json!({})).unwrap();
    assert_eq!(e.doc.nodes, nodes_before);
}

#[test]
fn override_instance_rejects_a_non_number_swap() {
    let mut e = Engine::default();
    let p = page(&mut e);
    let b = button(&mut e, p);
    e.call("desigma_create_component", json!({"node_id":b})).unwrap();
    let i = e.call("desigma_create_instance", json!({"component_id":b,"parent_id":p})).unwrap()["instance_id"].as_u64().unwrap();
    for bad in [json!("12"), json!(1.5), json!(-3), json!(null)] {
        let err = e.call("desigma_override_instance", json!({"instance_id":i,"overrides":{"Label":{"swap":bad}}})).unwrap_err();
        assert!(err.contains("swap must be"), "{err}");
    }
}
