//! Runs the example from docs/agents.md so the guide stays true.
use desigma_doc::NodeKind;
use desigma_mcp::engine::{validate_document, Engine};
use serde_json::json;

#[test]
fn the_agent_guide_example_works() {
    let mut e = Engine::default();
    let page = e.call("desigma_get_document", json!({})).unwrap()["pages"][0]["node_id"].as_u64().unwrap();
    e.call(
        "desigma_set_variables",
        json!({"collections":[{"name":"Brand","modes":["Light","Dark"],"variables":[
        {"name":"Color/Primary","type":"color","values":{"Light":"#3366ff","Dark":"#88aaff"}},
        {"name":"Color/Surface","type":"color","values":{"Light":"#ffffff","Dark":"#1c1c1e"}},
        {"name":"Space/M","type":"number","value":16},
        {"name":"Radius/L","type":"number","value":16}]}]}),
    )
    .unwrap();
    let b = e
        .call(
            "desigma_apply_nodes",
            json!({"parent_id":page,"nodes":[{"kind":"frame","name":"Button","width":"hug","height":"hug","corner_radius":10,"fill":"#3366ff",
        "bind":{"fill":"Color/Primary"},"auto_layout":{"direction":"horizontal","padding":[12,20],"counter_align":"center"},
        "children":[{"kind":"text","name":"Label","text":"Button","text_style":{"font_size":15,"font_weight":600,"color":"#ffffff"}}]}]}),
        )
        .unwrap();
    let button = b["nodes"][0]["node_id"].as_u64().unwrap();
    e.call("desigma_create_component", json!({"node_id":button})).unwrap();
    let r = e
        .call(
            "desigma_apply_nodes",
            json!({"parent_id":page,"nodes":[{"kind":"frame","name":"Login card","x":80,"y":80,"width":360,"height":"hug",
        "fill":"#ffffff","corner_radius":16,"effects":[{"type":"drop_shadow","color":"#00000026","y":8,"blur":24}],
        "auto_layout":{"direction":"vertical","gap":16,"padding":24},"bind":{"fill":"Color/Surface","gap":"Space/M","radius":"Radius/L"},
        "children":[
            {"kind":"text","name":"Title","text":"Welcome back","width":"fill","text_style":{"font_size":24,"font_weight":700}},
            {"kind":"rectangle","name":"Email field","width":"fill","height":44,"corner_radius":8,"fill":"#f2f2f7","stroke":{"color":"#d1d1d6","width":1}},
            {"instance_of":button,"name":"Sign in","width":"fill","overrides":{"Label":{"text":"Sign in"}}},
            {"instance_of":button,"name":"Create account","width":"fill","overrides":{"Label":{"text":"Create account"}}}]}]}),
        )
        .unwrap();
    validate_document(&e.doc).unwrap();
    let ids: Vec<u64> = r["nodes"].as_array().unwrap().iter().map(|n| n["node_id"].as_u64().unwrap()).collect();
    let (card, title, field, sign_in) = (ids[0], ids[1], ids[2], ids[3]);
    let tree = e.call("desigma_get_tree", json!({"node_id":card,"depth":1})).unwrap();
    assert_eq!(tree["tree"][0]["children"].as_array().unwrap().len(), 4);
    let r = e
        .call(
            "desigma_apply_nodes",
            json!({"nodes":[{"node_id":card,"replace_children":true,"children":[
        {"node_id":title},{"node_id":field},{"node_id":sign_in},
        {"kind":"text","name":"Forgot","text":"Forgot password?","text_style":{"font_size":13,"color":"#3366ff"}}]}]}),
        )
        .unwrap();
    assert_eq!((r["removed"].as_u64(), r["undo_steps"].as_u64()), (Some(1), Some(1)));
    validate_document(&e.doc).unwrap();
    let c = e.doc.get(desigma_doc::NodeId(card)).unwrap();
    assert_eq!(c.children.len(), 4);
    assert!(matches!(e.doc.get(c.children[2]).unwrap().kind, NodeKind::Instance { .. }));
}
