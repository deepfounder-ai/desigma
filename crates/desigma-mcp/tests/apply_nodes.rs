use desigma_doc::{Effect, LayoutDirection, NodeId, NodeKind, PaintKind, Sizing};
use desigma_mcp::engine::{validate_document, Engine};
use serde_json::{json, Value};

fn page(e: &mut Engine) -> u64 {
    e.call("desigma_get_document", json!({})).unwrap()["pages"][0]["node_id"].as_u64().unwrap()
}

fn ids(r: &Value) -> Vec<u64> {
    r["nodes"].as_array().unwrap().iter().map(|n| n["node_id"].as_u64().unwrap()).collect()
}

fn card() -> Value {
    json!({
        "kind":"frame","name":"Card","x":40,"y":60,"width":320,"height":"hug","rotation":0,
        "clips_content":true,"corner_radius":[24,24,8,8],"blend":"pass_through",
        "fill":[{"type":"linear","angle":90,"stops":[{"pos":0,"color":"#ff3366"},{"pos":1,"color":"#3366ffcc"}]}],
        "stroke":{"color":"#00000033","width":2,"align":"outside","dashes":[4,2]},
        "effects":[
            {"type":"drop_shadow","color":"#00000040","x":0,"y":8,"blur":24},
            {"type":"inner_shadow","y":1,"blur":2},
            {"type":"layer_blur","blur":0},
            {"type":"background_blur","blur":20},
            {"type":"glass","frost":6}
        ],
        "auto_layout":{"direction":"vertical","gap":12,"padding":[16,20],"primary_align":"start","counter_align":"center"},
        "constraints":{"horizontal":"stretch","vertical":"top"},
        "children":[
            {"kind":"text","name":"Title","text":"Hello","width":"fill",
             "text_style":{"font_size":24,"font_weight":700,"line_height":"120%","letter_spacing":-0.5,"align_h":"center","color":"#111111"}},
            {"kind":"ellipse","name":"Radial","width":80,"height":80,"fill":{"type":"radial","stops":[{"pos":0,"color":"#fff"},{"pos":1,"color":"#000"}]}},
            {"kind":"rectangle","name":"Angular","width":"fill","height":40,"rotation":15,"blend":"multiply",
             "fill":{"type":"angular","stops":[{"pos":0,"color":"#f00"},{"pos":1,"color":"#00f"}]}},
            {"kind":"rectangle","name":"Diamond","width":60,"height":60,
             "fill":{"type":"diamond","stops":[{"pos":0,"color":"#0f0"},{"pos":1,"color":"#000"}],"opacity":0.5}}
        ]
    })
}

#[test]
fn builds_a_full_tree_in_one_undo_step() {
    let mut e = Engine::default();
    let p = page(&mut e);
    let before = e.doc.nodes.clone();
    let undo_before = e.history.can_undo();
    let r = e.call("desigma_apply_nodes", json!({"parent_id":p,"nodes":[card()]})).unwrap();
    assert_eq!(r["created"], 5);
    assert_eq!(r["undo_steps"], 1);
    validate_document(&e.doc).unwrap();
    let ids = ids(&r);
    let c = e.doc.get(NodeId(ids[0])).unwrap();
    assert_eq!(c.corner_radii, [24.0, 24.0, 8.0, 8.0]);
    let NodeKind::Frame(fp) = &c.kind else { panic!("frame") };
    assert!(fp.clips_content);
    let al = fp.auto_layout.unwrap();
    assert_eq!(al.direction, LayoutDirection::Vertical);
    assert_eq!(al.padding, [16.0, 20.0, 16.0, 20.0]);
    assert_eq!(al.counter_sizing, Sizing::Fixed, "a pixel width on a vertical layout is the counter axis");
    assert_eq!(al.primary_sizing, Sizing::Hug);
    assert!(matches!(c.fills[0].kind, PaintKind::LinearGradient { .. }));
    assert_eq!(c.effects.len(), 5);
    assert!(matches!(c.effects[4], Effect::Glass(_)));
    assert_eq!(c.stroke.dashes, vec![4.0, 2.0]);
    let kids: Vec<_> = c.children.iter().map(|k| e.doc.get(*k).unwrap()).collect();
    assert!(matches!(kids[1].fills[0].kind, PaintKind::RadialGradient { .. }));
    assert!(matches!(kids[2].fills[0].kind, PaintKind::AngularGradient { .. }));
    assert!(matches!(kids[3].fills[0].kind, PaintKind::DiamondGradient { .. }));
    assert_eq!(kids[3].fills[0].opacity, 0.5);
    assert!((kids[2].transform.b - 15f32.to_radians().sin()).abs() < 1e-5, "rotation is stored in the transform");
    // auto layout was solved: the fill-width title and the angular bar span the padded width
    assert_eq!(c.size.x, 320.0);
    assert_eq!(kids[0].size.x, 280.0);
    assert_eq!(kids[2].size.x, 280.0);
    assert_eq!(kids[0].transform.ty, 16.0);
    assert!(kids[1].transform.ty > kids[0].transform.ty + kids[0].size.y, "children stack with the gap");
    assert!(c.size.y > 16.0 + 24.0 + 80.0 + 40.0 + 60.0, "the card hugs its content: {}", c.size.y);
    // one undo removes everything, one redo brings it back
    assert!(e.history.can_undo());
    e.call("desigma_undo", json!({})).unwrap();
    assert_eq!(e.doc.nodes, before, "undo restores every node (only the id counter moves on)");
    assert_eq!(e.history.can_undo(), undo_before);
    e.call("desigma_redo", json!({})).unwrap();
    assert!(e.doc.get(NodeId(ids[0])).is_some());
    assert_eq!(e.doc.nodes.len(), 1 + 5);
}

#[test]
fn errors_name_the_node_and_change_nothing() {
    let mut e = Engine::default();
    let p = page(&mut e);
    let before = e.doc.to_json().unwrap();
    let err = e
        .call(
            "desigma_apply_nodes",
            json!({"parent_id":p,"nodes":[{"kind":"frame","name":"Ok","children":[
                {"kind":"rectangle","name":"BadColour","fill":"#12"},
                {"kind":"text","name":"Typo","txt":"x"},
                {"kind":"ellipse","name":"BadStop","fill":{"type":"linear","stops":[{"pos":2,"color":"#fff"},{"pos":0,"color":"#000"}]}},
                {"kind":"rectangle","name":"Hug","width":"hug"},
                {"kind":"rectangle","name":"Glow","effects":[{"type":"layer_blur","x":3}]}
            ]},{"name":"NoKind"}]}),
        )
        .unwrap_err();
    for needle in [
        "nodes[0].children[0] \"BadColour\": fill:",
        "nodes[0].children[1] \"Typo\": unknown field `txt`",
        "nodes[0].children[2] \"BadStop\": fill: stop pos",
        "nodes[0].children[3] \"Hug\": width: \"hug\" needs",
        "nodes[0].children[4] \"Glow\": effects[0]: x is not valid on layer_blur",
        "nodes[1] \"NoKind\"",
    ] {
        assert!(err.contains(needle), "missing `{needle}` in:\n{err}");
    }
    assert!(err.contains("nothing was changed (6 problems)"), "{err}");
    assert_eq!(e.doc.to_json().unwrap(), before, "no node and no id was spent");
    assert!(!e.history.can_undo());
    let bad_parent = e.call("desigma_apply_nodes", json!({"parent_id":99999,"nodes":[{"kind":"frame"}]})).unwrap_err();
    assert!(bad_parent.contains("parent 99999 is missing"), "{bad_parent}");
}

#[test]
fn updates_change_only_the_passed_fields_and_reach_children() {
    let mut e = Engine::default();
    let p = page(&mut e);
    let r = e.call("desigma_apply_nodes", json!({"parent_id":p,"nodes":[card()]})).unwrap();
    let ids = ids(&r);
    let (card_id, title_id) = (ids[0], ids[1]);
    let r = e
        .call(
            "desigma_apply_nodes",
            json!({"nodes":[{"node_id":card_id,"name":"Card v2","effects":[],"children":[
                {"node_id":title_id,"text":"Bye","text_style":{"font_size":30}},
                {"kind":"rectangle","name":"Added","width":"fill","height":10}
            ]}]}),
        )
        .unwrap();
    assert_eq!((r["created"].as_u64(), r["updated"].as_u64()), (Some(1), Some(5)), "card, title and the three siblings the taller title pushed down");
    let c = e.doc.get(NodeId(card_id)).unwrap();
    assert_eq!(c.name, "Card v2");
    assert!(c.effects.is_empty());
    assert_eq!(c.corner_radii, [24.0, 24.0, 8.0, 8.0], "untouched fields stay");
    assert_eq!(c.children.len(), 5);
    let NodeKind::Text(t) = &e.doc.get(NodeId(title_id)).unwrap().kind else { panic!("text") };
    assert_eq!((t.content.as_str(), t.style.font_size, t.style.font_weight), ("Bye", 30.0, 700));
    let added = e.doc.get(c.children[4]).unwrap();
    assert_eq!(added.size.x, 280.0, "the new bar fills the card");
    // a no-op call changes nothing and records no undo step
    e.call("desigma_undo", json!({})).unwrap();
    let r = e.call("desigma_apply_nodes", json!({"nodes":[{"node_id":card_id,"name":"Card"}]})).unwrap();
    assert_eq!(r["changed"], false);
    // wrong kind and strangers are refused per node
    let err = e.call("desigma_apply_nodes", json!({"nodes":[{"node_id":card_id,"kind":"text"},{"node_id":999}]})).unwrap_err();
    assert!(err.contains("kind cannot be changed") && err.contains("node 999 not found"), "{err}");
}

#[test]
fn image_fills_store_the_bytes_and_validate_only_changes_nothing() {
    let mut e = Engine::default();
    let p = page(&mut e);
    let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";
    let spec = json!({"parent_id":p,"nodes":[{"kind":"rectangle","fill":{"type":"image","image_base64":png,"scale_mode":"fit"}}]});
    let mut dry = spec.clone();
    dry["validate_only"] = json!(true);
    let r = e.call("desigma_apply_nodes", dry).unwrap();
    assert_eq!((r["valid"].as_bool(), r["would_create"].as_u64()), (Some(true), Some(1)));
    assert!(e.doc.images.is_empty() && e.doc.nodes.len() == 1);
    e.call("desigma_apply_nodes", spec).unwrap();
    assert_eq!(e.doc.images.len(), 1);
    let n = e.doc.nodes.values().find(|n| n.name == "Rectangle").unwrap();
    let PaintKind::Image { image_ref, .. } = &n.fills[0].kind else { panic!("image") };
    assert!(e.doc.images.contains_key(image_ref));
    let err =
        e.call("desigma_apply_nodes", json!({"parent_id":p,"nodes":[{"kind":"rectangle","fill":{"type":"image","image_base64":"aGVsbG8="}}]})).unwrap_err();
    assert!(err.contains("PNG, JPEG, GIF or WebP"), "{err}");
}

#[test]
fn a_300_node_screen_builds_fast() {
    let mut e = Engine::default();
    let p = page(&mut e);
    let rows: Vec<Value> = (0..49)
        .map(|i| {
            json!({"kind":"frame","name":format!("Row {i}"),"width":"fill","height":"hug","corner_radius":12,
            "fill":{"type":"linear","stops":[{"pos":0,"color":"#ffffff"},{"pos":1,"color":"#eeeeee"}]},
            "effects":[{"type":"drop_shadow","blur":8}],
            "auto_layout":{"direction":"horizontal","gap":8,"padding":12,"counter_align":"center"},
            "children":[
                {"kind":"ellipse","name":"Avatar","width":32,"height":32},
                {"kind":"text","name":"Name","text":format!("Person {i}"),"width":"fill","text_style":{"font_size":14}},
                {"kind":"rectangle","name":"Badge","width":24,"height":16,"corner_radius":8},
                {"kind":"line","name":"Rule","width":10},
                {"kind":"rectangle","name":"Dot","width":6,"height":6}
            ]})
        })
        .collect();
    let screen = json!({"kind":"frame","name":"Screen","width":390,"height":844,"clips_content":true,
        "auto_layout":{"direction":"vertical","gap":8,"padding":16},"children":rows});
    let start = std::time::Instant::now();
    let r = e.call("desigma_apply_nodes", json!({"parent_id":p,"nodes":[screen]})).unwrap();
    let elapsed = start.elapsed();
    assert_eq!(r["created"], 1 + 49 * 6);
    println!("apply_nodes: {} nodes built in {:?}", r["created"], elapsed);
    assert!(elapsed.as_secs_f32() < 10.0, "{elapsed:?}");
    validate_document(&e.doc).unwrap();
    let screen = e.doc.get(NodeId(ids(&r)[0])).unwrap();
    let row = e.doc.get(screen.children[1]).unwrap();
    assert_eq!(row.size.x, 358.0);
    assert!(row.transform.ty > 16.0);
}
