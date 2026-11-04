use desigma_mcp::engine::Engine;
use serde_json::json;

#[test]
fn tree_is_scoped_compact_and_assets_list_everything() {
    let mut e = Engine::default();
    let p = e.call("desigma_get_document", json!({})).unwrap()["pages"][0]["node_id"].as_u64().unwrap();
    e.call(
        "desigma_set_variables",
        json!({"collections":[{"name":"Brand","modes":["Light","Dark"],"variables":[
        {"name":"Color/Primary","type":"color","values":{"Light":"#3366ff","Dark":"#88aaff"}}]}]}),
    )
    .unwrap();
    let r = e
        .call(
            "desigma_apply_nodes",
            json!({"parent_id":p,"nodes":[{"kind":"frame","name":"Card","width":200,"height":100,"fill":"#ffffff",
                "children":[{"kind":"frame","name":"Inner","width":50,"height":50,"fill":{"type":"linear","stops":[{"pos":0,"color":"#ff0000"},{"pos":1,"color":"#0000ff"}]},
                    "children":[{"kind":"text","name":"Deep","text":"hello","bind":{}}]}]}]}),
        )
        .unwrap();
    let card = r["nodes"][0]["node_id"].as_u64().unwrap();
    e.call("desigma_create_component", json!({"node_id":card})).unwrap();
    let t = e.call("desigma_get_tree", json!({"node_id":card,"depth":1})).unwrap();
    let root = &t["tree"][0];
    assert_eq!((root["name"].as_str(), root["kind"].as_str(), root["w"].as_f64()), (Some("Card"), Some("Component"), Some(200.0)));
    assert_eq!(root["fill"], "#FFFFFF");
    let inner = &root["children"][0];
    assert_eq!(inner["fill"], "linear(FF0000>0000FF)");
    assert_eq!(inner["child_count"], 1);
    assert!(inner.get("children").is_none(), "depth 1 stops below the first level");
    let all = e.call("desigma_get_tree", json!({})).unwrap();
    assert_eq!(all["tree"][0]["children"][0]["name"], "Card");
    assert!(e.call("desigma_get_tree", json!({"node_id":999999})).is_err());

    let a = e.call("desigma_get_assets", json!({})).unwrap();
    assert_eq!(a["components"][0]["name"], "Card");
    assert_eq!(a["components"][0]["layers"][0], "Inner");
    assert_eq!(a["variables"][0]["name"], "Color/Primary");
    assert_eq!(a["variables"][0]["values"]["Dark"], "#88AAFF");
    assert_eq!(a["variable_collections"][0]["modes"].as_array().unwrap().len(), 2);
    assert_eq!(e.call("desigma_get_assets", json!({"query":"zzz"})).unwrap()["components"].as_array().unwrap().len(), 0);
}
