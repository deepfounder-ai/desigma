#![cfg(feature = "server")]
//! Real MCP client/server lifecycle across a subprocess, not a mocked dispatcher.
use rmcp::{model::CallToolRequestParams, transport::TokioChildProcess, ServiceExt};
use serde_json::{json, Value};

#[tokio::test]
async fn stdio_handshake_discovery_edit_and_error_recovery() {
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_desigma-mcp"));
        command.arg("--workspace").arg(env!("CARGO_MANIFEST_DIR"));
        // Empty = disabled: this subprocess must never probe a developer's real editor.
        command.env(desigma_mcp::live::ENV_FILE, "");
        let client = ().serve(TokioChildProcess::new(command).unwrap()).await.unwrap();
        assert_eq!(client.peer_info().unwrap().server_info.as_ref().unwrap().name, "desigma-mcp");
        let tools = client.list_all_tools().await.unwrap();
        assert!(tools.iter().any(|t| t.name == "desigma_list_files"));
        let schema = tools.iter().find(|t| t.name == "desigma_delete_page").unwrap().input_schema.clone();
        assert_eq!(schema["properties"]["node_id"]["type"], "integer");
        // 27（上游）+ 5（本地扩展：组件 / 实例 / 页面重命名）
        for name in ["desigma_create_component", "desigma_create_instance", "desigma_detach_instance", "desigma_sync_instances", "desigma_rename_page"] {
            assert!(tools.iter().any(|t| t.name == name), "missing local tool {name}");
        }
        assert_eq!(tools.len(), 32);
        assert!(tools.iter().all(|t| t.annotations.is_some()));
        let call = |name: &'static str, args: Value| CallToolRequestParams::new(name).with_arguments(args.as_object().unwrap().clone());
        let doc = client.call_tool(call("desigma_get_document", json!({}))).await.unwrap();
        assert_eq!(doc.is_error, Some(false));
        let page = doc.structured_content.unwrap()["pages"][0]["node_id"].clone();
        let created = client.call_tool(call("desigma_create_node", json!({"parent_id":page,"kind":"rectangle","name":"MCP test","width":120}))).await.unwrap();
        let id = created.structured_content.unwrap()["node"]["id"].clone();
        let invalid = client.call_tool(call("desigma_update_node", json!({"node_id":id,"width":-1}))).await.unwrap();
        assert_eq!(invalid.is_error, Some(true));
        let code = client.call_tool(call("desigma_generate_code", json!({"node_id":id,"language":"css"}))).await.unwrap();
        assert!(code.structured_content.unwrap()["code"].as_str().unwrap().contains("120px"));
        client.call_tool(call("desigma_undo", json!({}))).await.unwrap();
        let gone = client.call_tool(call("desigma_get_node", json!({"node_id":id}))).await.unwrap();
        assert_eq!(gone.is_error, Some(true));
        client.call_tool(call("desigma_redo", json!({}))).await.unwrap();
        let restored = client.call_tool(call("desigma_get_node", json!({"node_id":id}))).await.unwrap();
        assert_eq!(restored.is_error, Some(false));
        // A document with a single page must refuse to delete it (#39).
        let only_page = client.call_tool(call("desigma_get_document", json!({}))).await.unwrap();
        let pages = only_page.structured_content.as_ref().unwrap()["pages"].as_array().unwrap().len();
        assert_eq!(pages, 1);
        let refused = client.call_tool(call("desigma_delete_page", json!({"node_id":page}))).await.unwrap();
        assert_eq!(refused.is_error, Some(true));
        assert!(refused.structured_content.unwrap()["error"].as_str().unwrap().contains("last page"));
        client.cancel().await.unwrap();
    })
    .await
    .expect("MCP subprocess must finish within 30 seconds");
}

/// Moving a layer keeps the document acyclic and is undoable (#32 follow-up).
#[tokio::test]
async fn moving_a_layer_reparents_it_and_refuses_cycles() {
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_desigma-mcp"));
        command.arg("--workspace").arg(env!("CARGO_MANIFEST_DIR"));
        command.env(desigma_mcp::live::ENV_FILE, "");
        let client = ().serve(TokioChildProcess::new(command).unwrap()).await.unwrap();
        let call = |name: &'static str, args: Value| CallToolRequestParams::new(name).with_arguments(args.as_object().unwrap().clone());
        let doc = client.call_tool(call("desigma_get_document", json!({}))).await.unwrap();
        let page = doc.structured_content.unwrap()["pages"][0]["node_id"].clone();
        let frame =
            client.call_tool(call("desigma_create_node", json!({"parent_id":page,"kind":"frame","name":"card","width":400,"height":200}))).await.unwrap();
        let frame_id = frame.structured_content.unwrap()["node"]["id"].clone();
        let rect = client
            .call_tool(call("desigma_create_node", json!({"parent_id":page,"kind":"rectangle","name":"row","x":32.0,"y":48.0,"width":120,"height":40})))
            .await
            .unwrap();
        let id = rect.structured_content.unwrap()["node"]["id"].clone();

        let moved = client.call_tool(call("desigma_move_node", json!({"node_id":id,"parent_id":frame_id}))).await.unwrap();
        assert_eq!(moved.is_error, Some(false), "{moved:?}");
        assert_eq!(moved.structured_content.as_ref().unwrap()["changed"], true);
        let node = client.call_tool(call("desigma_get_node", json!({"node_id":id}))).await.unwrap();
        let node = node.structured_content.unwrap()["node"].clone();
        assert_eq!(node["parent"], frame_id);
        assert_eq!(node["transform"]["tx"], 32.0, "transform stays parent-relative");
        let frame_node = client.call_tool(call("desigma_get_node", json!({"node_id":frame_id}))).await.unwrap();
        assert_eq!(frame_node.structured_content.unwrap()["node"]["children"], json!([id]));

        let same = client.call_tool(call("desigma_move_node", json!({"node_id":id,"parent_id":frame_id,"index":0}))).await.unwrap();
        assert_eq!(same.structured_content.unwrap()["changed"], false, "no-op move reports no change");

        let into_self = client.call_tool(call("desigma_move_node", json!({"node_id":id,"parent_id":id}))).await.unwrap();
        assert_eq!(into_self.is_error, Some(true));
        let into_leaf = client.call_tool(call("desigma_move_node", json!({"node_id":frame_id,"parent_id":id}))).await.unwrap();
        assert_eq!(into_leaf.is_error, Some(true), "rectangles are not containers");
        let page_root = client.call_tool(call("desigma_move_node", json!({"node_id":page,"parent_id":frame_id}))).await.unwrap();
        assert_eq!(page_root.is_error, Some(true));
        let missing = client.call_tool(call("desigma_move_node", json!({"node_id":999999,"parent_id":frame_id}))).await.unwrap();
        assert_eq!(missing.is_error, Some(true));

        client.call_tool(call("desigma_undo", json!({}))).await.unwrap();
        let restored = client.call_tool(call("desigma_get_node", json!({"node_id":id}))).await.unwrap();
        assert_eq!(restored.structured_content.unwrap()["node"]["parent"], page);
        client.cancel().await.unwrap();
    })
    .await
    .expect("MCP subprocess must finish within 30 seconds");
}

/// New pages are appended to the document and can receive layers (#32 follow-up).
#[tokio::test]
async fn creating_a_page_appends_it_and_accepts_layers() {
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_desigma-mcp"));
        command.arg("--workspace").arg(env!("CARGO_MANIFEST_DIR"));
        command.env(desigma_mcp::live::ENV_FILE, "");
        let client = ().serve(TokioChildProcess::new(command).unwrap()).await.unwrap();
        let call = |name: &'static str, args: Value| CallToolRequestParams::new(name).with_arguments(args.as_object().unwrap().clone());
        let before = client.call_tool(call("desigma_get_document", json!({}))).await.unwrap();
        let before = before.structured_content.unwrap();
        let pages_before = before["pages"].as_array().unwrap().len();

        let created = client.call_tool(call("desigma_create_page", json!({"name":"Visitors","background":"#f5f7fb"}))).await.unwrap();
        assert_eq!(created.is_error, Some(false), "{created:?}");
        let page_id = created.structured_content.unwrap()["page_id"].clone();

        let after = client.call_tool(call("desigma_get_document", json!({}))).await.unwrap();
        let after = after.structured_content.unwrap();
        let pages = after["pages"].as_array().unwrap();
        assert_eq!(pages.len(), pages_before + 1);
        assert_eq!(pages.last().unwrap()["node_id"], page_id);
        assert_eq!(pages.last().unwrap()["name"], "Visitors");

        let rect =
            client.call_tool(call("desigma_create_node", json!({"parent_id":page_id,"kind":"rectangle","name":"card","width":120,"height":40}))).await.unwrap();
        assert_eq!(rect.is_error, Some(false), "{rect:?}");

        let blank = client.call_tool(call("desigma_create_page", json!({"name":"   "}))).await.unwrap();
        assert_eq!(blank.is_error, Some(true), "blank names must be refused");
        let bad_bg = client.call_tool(call("desigma_create_page", json!({"name":"x","background":"nope"}))).await.unwrap();
        assert_eq!(bad_bg.is_error, Some(true));

        client.call_tool(call("desigma_undo", json!({}))).await.unwrap();
        client.call_tool(call("desigma_undo", json!({}))).await.unwrap();
        let undone = client.call_tool(call("desigma_get_document", json!({}))).await.unwrap();
        assert_eq!(undone.structured_content.unwrap()["pages"].as_array().unwrap().len(), pages_before);
        client.cancel().await.unwrap();
    })
    .await
    .expect("MCP subprocess must finish within 30 seconds");
}

/// Deleting a page takes its layers with it, and undo restores both (#39).
#[tokio::test]
async fn deleting_a_page_removes_its_layers_and_undo_restores_them() {
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_desigma-mcp"));
        command.arg("--workspace").arg(env!("CARGO_MANIFEST_DIR"));
        command.env(desigma_mcp::live::ENV_FILE, "");
        let client = ().serve(TokioChildProcess::new(command).unwrap()).await.unwrap();
        let call = |name: &'static str, args: Value| CallToolRequestParams::new(name).with_arguments(args.as_object().unwrap().clone());
        let pages = |doc: &Value| doc["pages"].as_array().unwrap().len();

        let before = client.call_tool(call("desigma_get_document", json!({}))).await.unwrap();
        let pages_before = pages(&before.structured_content.unwrap());

        let created = client.call_tool(call("desigma_create_page", json!({"name":"Scratch"}))).await.unwrap();
        let page_id = created.structured_content.unwrap()["page_id"].clone();
        let rect =
            client.call_tool(call("desigma_create_node", json!({"parent_id":page_id,"kind":"rectangle","name":"card","width":120,"height":40}))).await.unwrap();
        let rect_id = rect.structured_content.unwrap()["node"]["id"].clone();

        let deleted = client.call_tool(call("desigma_delete_page", json!({"node_id":page_id}))).await.unwrap();
        assert_eq!(deleted.is_error, Some(false), "{deleted:?}");
        let after = client.call_tool(call("desigma_get_document", json!({}))).await.unwrap();
        assert_eq!(pages(&after.structured_content.unwrap()), pages_before, "the page is gone from the tab strip");
        let gone = client.call_tool(call("desigma_get_node", json!({"node_id":rect_id}))).await.unwrap();
        assert_eq!(gone.is_error, Some(true), "its layers went with it, not orphaned");

        // One undo brings the page and its layer back together.
        client.call_tool(call("desigma_undo", json!({}))).await.unwrap();
        let back = client.call_tool(call("desigma_get_document", json!({}))).await.unwrap();
        assert_eq!(pages(&back.structured_content.unwrap()), pages_before + 1);
        let node = client.call_tool(call("desigma_get_node", json!({"node_id":rect_id}))).await.unwrap();
        assert_eq!(node.is_error, Some(false), "{node:?}");
        client.cancel().await.unwrap();
    })
    .await
    .expect("MCP subprocess must finish within 30 seconds");
}

/// Restyling and typography are their own tools and land on the undo stack (#32 follow-up).
#[tokio::test]
async fn styling_and_typography_are_editable() {
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_desigma-mcp"));
        command.arg("--workspace").arg(env!("CARGO_MANIFEST_DIR"));
        command.env(desigma_mcp::live::ENV_FILE, "");
        let client = ().serve(TokioChildProcess::new(command).unwrap()).await.unwrap();
        let call = |name: &'static str, args: Value| CallToolRequestParams::new(name).with_arguments(args.as_object().unwrap().clone());
        let doc = client.call_tool(call("desigma_get_document", json!({}))).await.unwrap();
        let page = doc.structured_content.unwrap()["pages"][0]["node_id"].clone();
        let rect = client
            .call_tool(call("desigma_create_node", json!({"parent_id":page,"kind":"rectangle","name":"accent","width":120,"height":40})))
            .await
            .unwrap();
        let id = rect.structured_content.unwrap()["node"]["id"].clone();

        let styled = client
            .call_tool(call(
                "desigma_set_appearance",
                json!({"node_id":id,"fill":"#2563eb","stroke":"#e4e9f3","stroke_width":1,"corner_radius":8,"opacity":0.5}),
            ))
            .await
            .unwrap();
        assert_eq!(styled.is_error, Some(false), "{styled:?}");
        let applied = styled.structured_content.as_ref().unwrap()["applied"].clone();
        assert_eq!(applied, json!(["fill", "stroke", "corner_radius", "opacity"]));
        let node = client.call_tool(call("desigma_get_node", json!({"node_id":id}))).await.unwrap();
        let node = node.structured_content.unwrap()["node"].clone();
        assert_eq!(node["corner_radii"], json!([8.0, 8.0, 8.0, 8.0]));
        assert_eq!(node["opacity"], 0.5);
        assert_eq!(node["stroke"]["weight"], 1.0);
        let red = node["fills"][0]["kind"]["Solid"]["r"].as_f64().unwrap() * 255.0;
        assert!((red - 37.0).abs() < 0.6, "fill red channel: {red}");
        let blue = node["fills"][0]["kind"]["Solid"]["b"].as_f64().unwrap() * 255.0;
        assert!((blue - 235.0).abs() < 0.6, "fill blue channel: {blue}");

        let cleared = client.call_tool(call("desigma_set_appearance", json!({"node_id":id,"fill":"none"}))).await.unwrap();
        assert_eq!(cleared.is_error, Some(false), "{cleared:?}");
        let node = client.call_tool(call("desigma_get_node", json!({"node_id":id}))).await.unwrap();
        assert_eq!(node.structured_content.unwrap()["node"]["fills"], json!([]));

        let invalid = client.call_tool(call("desigma_set_appearance", json!({"node_id":id,"fill":"nope"}))).await.unwrap();
        assert_eq!(invalid.is_error, Some(true));
        let bad_radius = client.call_tool(call("desigma_set_appearance", json!({"node_id":id,"corner_radius":-2.0}))).await.unwrap();
        assert_eq!(bad_radius.is_error, Some(true));
        let missing = client.call_tool(call("desigma_set_appearance", json!({"node_id":999999,"fill":"#ffffff"}))).await.unwrap();
        assert_eq!(missing.is_error, Some(true));

        let text = client
            .call_tool(call("desigma_create_node", json!({"parent_id":page,"kind":"text","name":"label","text":"Hi"})))
            .await
            .unwrap();
        let tid = text.structured_content.unwrap()["node"]["id"].clone();
        let typed = client
            .call_tool(call(
                "desigma_set_text_style",
                json!({"node_id":tid,"text":"Hello","font_size":18.0,"font_weight":600,"line_height":24.0,"align_h":"center","align_v":"center","font_family":"Inter"}),
            ))
            .await
            .unwrap();
        assert_eq!(typed.is_error, Some(false), "{typed:?}");
        let tnode = client.call_tool(call("desigma_get_node", json!({"node_id":tid}))).await.unwrap();
        let tnode = tnode.structured_content.unwrap()["node"].clone();
        let tstyle = &tnode["kind"]["Text"];
        assert_eq!(tstyle["content"], "Hello");
        assert_eq!(tstyle["style"]["font_size"], 18.0);
        assert_eq!(tstyle["style"]["font_weight"], 600);
        assert_eq!(tstyle["style"]["font_family"], "Inter");
        assert_eq!(tstyle["style"]["align_h"], "Center");
        let text_fill = client.call_tool(call("desigma_set_appearance", json!({"node_id":tid,"fill":"#0b1220"}))).await.unwrap();
        assert_eq!(text_fill.is_error, Some(false), "{text_fill:?}");
        let tnode = client.call_tool(call("desigma_get_node", json!({"node_id":tid}))).await.unwrap();
        let tnode = tnode.structured_content.unwrap()["node"].clone();
        assert_eq!(tnode["fills"], json!([]));
        let text_red = tnode["kind"]["Text"]["style"]["fills"][0]["kind"]["Solid"]["r"].as_f64().unwrap() * 255.0;
        assert!((text_red - 11.0).abs() < 0.6, "text colour red channel: {text_red}");

        let bad_weight = client.call_tool(call("desigma_set_text_style", json!({"node_id":tid,"font_weight":550}))).await.unwrap();
        assert_eq!(bad_weight.is_error, Some(true));
        let wrong_kind = client.call_tool(call("desigma_set_text_style", json!({"node_id":id,"font_size":20.0}))).await.unwrap();
        assert_eq!(wrong_kind.is_error, Some(true));

        let undone = client.call_tool(call("desigma_undo", json!({}))).await.unwrap();
        assert_eq!(undone.is_error, Some(false), "{undone:?}");
        client.cancel().await.unwrap();
    })
    .await
    .expect("MCP subprocess must finish within 30 seconds");
}

/// SVG markup becomes real layers under the requested parent, in one undo step (#32 follow-up).
#[tokio::test]
async fn importing_svg_creates_layers_and_undoes_as_one_step() {
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_desigma-mcp"));
        command.arg("--workspace").arg(env!("CARGO_MANIFEST_DIR"));
        command.env(desigma_mcp::live::ENV_FILE, "");
        let client = ().serve(TokioChildProcess::new(command).unwrap()).await.unwrap();
        let call = |name: &'static str, args: Value| CallToolRequestParams::new(name).with_arguments(args.as_object().unwrap().clone());

        let svg = "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"24\" height=\"24\"><rect x=\"2\" y=\"2\" width=\"20\" height=\"20\" fill=\"#3366ff\"/><circle cx=\"12\" cy=\"12\" r=\"4\" fill=\"#ffffff\"/></svg>";
        let created = client
            .call_tool(call("desigma_import_svg", json!({"parent_id":1,"svg":svg,"name":"icon","x":5,"y":6})))
            .await
            .unwrap();
        assert_eq!(created.is_error, Some(false), "{created:?}");
        let info = created.structured_content.unwrap();
        assert_eq!(info["name"], "icon");
        assert_eq!(info["width"], 24.0);
        assert_eq!(info["height"], 24.0);
        assert!(info["nodes"].as_u64().unwrap() >= 3, "{info:?}");
        let root = info["node_id"].as_u64().unwrap();

        let node = client.call_tool(call("desigma_get_node", json!({"node_id":root}))).await.unwrap();
        let node = node.structured_content.unwrap()["node"].clone();
        assert_eq!(node["name"], "icon");
        assert_eq!(node["transform"]["tx"], 5.0);
        assert_eq!(node["transform"]["ty"], 6.0);
        assert!(node["children"].as_array().is_some_and(|c| c.len() >= 2), "{node:?}");

        let not_svg = client.call_tool(call("desigma_import_svg", json!({"parent_id":1,"svg":"hello"}))).await.unwrap();
        assert_eq!(not_svg.is_error, Some(true));
        let no_parent = client
            .call_tool(call("desigma_import_svg", json!({"parent_id":999999,"svg":svg})))
            .await
            .unwrap();
        assert_eq!(no_parent.is_error, Some(true));
        let escape = client
            .call_tool(call("desigma_import_svg", json!({"parent_id":1,"path":"../Cargo.toml"})))
            .await
            .unwrap();
        assert_eq!(escape.is_error, Some(true), "paths outside the workspace must be refused");

        // an import is an unsaved edit: starting over must ask first (#56)
        let fresh = client.call_tool(call("desigma_new_document", json!({"name":"x"}))).await.unwrap();
        assert_eq!(fresh.is_error, Some(true), "{fresh:?}");

        let undone = client.call_tool(call("desigma_undo", json!({}))).await.unwrap();
        assert_eq!(undone.is_error, Some(false));
        let gone = client.call_tool(call("desigma_get_node", json!({"node_id":root}))).await.unwrap();
        assert_eq!(gone.is_error, Some(true), "one undo must remove the whole import");
        client.cancel().await.unwrap();
    })
    .await
    .expect("MCP subprocess must finish within 30 seconds");
}

/// Requests written in one burst run, and are answered, in the order they were sent (#31):
/// a create followed by a read must see the created layer.
#[test]
fn batched_requests_run_in_order() {
    use std::io::{BufRead, BufReader, Write};
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_desigma-mcp"))
        .arg("--workspace")
        .arg(env!("CARGO_MANIFEST_DIR"))
        .env(desigma_mcp::live::ENV_FILE, "")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let mut batch = String::new();
    let mut line = |v: Value| batch.push_str(&format!("{v}\n"));
    line(
        json!({"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"batch","version":"1"}}}),
    );
    line(json!({"jsonrpc":"2.0","method":"notifications/initialized"}));
    const N: u64 = 60;
    // The page of a new document is node 1 in a file session.
    for i in 1..=N {
        let (name, args) = if i % 2 == 1 {
            ("desigma_create_node", json!({"parent_id":1,"kind":"rectangle","name":format!("r{i}")}))
        } else {
            ("desigma_list_nodes", json!({"parent_id":1}))
        };
        line(json!({"jsonrpc":"2.0","id":i,"method":"tools/call","params":{"name":name,"arguments":args}}));
    }
    let mut stdin = child.stdin.take().unwrap();
    stdin.write_all(batch.as_bytes()).unwrap();
    stdin.flush().unwrap();
    let mut out = BufReader::new(child.stdout.take().unwrap());
    let mut ids = Vec::new();
    let mut buf = String::new();
    while ids.len() < N as usize + 1 {
        buf.clear();
        assert!(out.read_line(&mut buf).unwrap() > 0, "server closed early after {ids:?}");
        let v: Value = serde_json::from_str(&buf).unwrap();
        let Some(id) = v["id"].as_u64() else { continue };
        ids.push(id);
        if id > 0 && id % 2 == 0 {
            // list_nodes right after the (id-1)th create sees exactly id/2 rectangles
            let text = v["result"]["structuredContent"].to_string();
            let created = (1..id).filter(|k| k % 2 == 1).count();
            assert_eq!(text.matches("\"kind\":\"Rectangle\"").count(), created, "response {id} saw a different tree: {text}");
        }
    }
    drop(stdin);
    let _ = child.kill();
    let _ = child.wait();
    assert_eq!(ids, (0..=N).collect::<Vec<_>>(), "responses come back in request order");
}

/// 本地扩展工具（组件 / 实例 / 页面重命名）的完整往返：上游没有这 5 个，改动必须自证。
#[tokio::test]
async fn components_instances_and_page_rename_round_trip() {
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_desigma-mcp"));
        command.arg("--workspace").arg(env!("CARGO_MANIFEST_DIR"));
        command.env(desigma_mcp::live::ENV_FILE, "");
        let client = ().serve(TokioChildProcess::new(command).unwrap()).await.unwrap();
        let call = |name: &'static str, args: Value| CallToolRequestParams::new(name).with_arguments(args.as_object().unwrap().clone());

        let doc = client.call_tool(call("desigma_get_document", json!({}))).await.unwrap();
        let page = doc.structured_content.unwrap()["pages"][0]["node_id"].clone();

        // 组件：frame 就地转主组件，可改名
        let card = client
            .call_tool(call("desigma_create_node", json!({"parent_id":page,"kind":"frame","name":"card","width":400,"height":200})))
            .await
            .unwrap();
        let card_id = card.structured_content.unwrap()["node"]["id"].clone();
        let comp = client
            .call_tool(call("desigma_create_component", json!({"node_id":card_id,"name":"CardMain"})))
            .await
            .unwrap();
        assert_eq!(comp.is_error, Some(false), "{comp:?}");
        let comp_sc = comp.structured_content.unwrap();
        assert_eq!(comp_sc["component_id"], card_id);
        assert_eq!(comp_sc["node"]["name"], "CardMain");

        // 负向：矩形不能变成组件
        let rect = client
            .call_tool(call("desigma_create_node", json!({"parent_id":page,"kind":"rectangle","name":"row","width":100,"height":40})))
            .await
            .unwrap();
        let rect_id = rect.structured_content.unwrap()["node"]["id"].clone();
        let refused = client.call_tool(call("desigma_create_component", json!({"node_id":rect_id}))).await.unwrap();
        assert_eq!(refused.is_error, Some(true));

        // 实例：挂到另一个 frame（直接挂 Page 会被拒）
        let host = client
            .call_tool(call("desigma_create_node", json!({"parent_id":page,"kind":"frame","name":"host","width":600,"height":400})))
            .await
            .unwrap();
        let host_id = host.structured_content.unwrap()["node"]["id"].clone();
        // 负向：父级就是组件自己 —— 会产生自引用，必须被拒
        let bad_parent = client
            .call_tool(call("desigma_create_instance", json!({"component_id":card_id,"parent_id":card_id,"x":0.0,"y":0.0})))
            .await
            .unwrap();
        assert_eq!(bad_parent.is_error, Some(true), "组件不能实例化到自己里面");
        // 负向：非组件的普通图层不能当组件实例化
        let bad_component = client
            .call_tool(call("desigma_create_instance", json!({"component_id":rect_id,"parent_id":host_id,"x":0.0,"y":0.0})))
            .await
            .unwrap();
        assert_eq!(bad_component.is_error, Some(true), "矩形不是主组件，应当被拒");
        let inst = client
            .call_tool(call("desigma_create_instance", json!({"component_id":card_id,"parent_id":host_id,"x":12.0,"y":24.0})))
            .await
            .unwrap();
        assert_eq!(inst.is_error, Some(false), "{inst:?}");
        let instance_id = inst.structured_content.unwrap()["instance_id"].clone();
        assert!(instance_id.as_u64().is_some());

        // 同步：把主组件当前状态刷到所有实例
        let synced = client.call_tool(call("desigma_sync_instances", json!({"component_id":card_id}))).await.unwrap();
        assert_eq!(synced.is_error, Some(false), "{synced:?}");
        assert_eq!(synced.structured_content.unwrap()["synced"], true);
        // 负向：非组件不能同步
        let bad_sync = client.call_tool(call("desigma_sync_instances", json!({"component_id":rect_id}))).await.unwrap();
        assert_eq!(bad_sync.is_error, Some(true));

        // 分离：实例变普通图层，且可撤回
        let detached = client.call_tool(call("desigma_detach_instance", json!({"node_id":instance_id}))).await.unwrap();
        assert_eq!(detached.is_error, Some(false), "{detached:?}");
        assert_eq!(detached.structured_content.unwrap()["detached"], true);
        client.call_tool(call("desigma_undo", json!({}))).await.unwrap();
        let still_instance = client.call_tool(call("desigma_get_node", json!({"node_id":instance_id}))).await.unwrap();
        assert_eq!(still_instance.is_error, Some(false), "撤回分离后实例应回来");

        // 页面重命名：可撤回
        let before = client.call_tool(call("desigma_get_document", json!({}))).await.unwrap();
        let old_name = before.structured_content.unwrap()["pages"][0]["name"].clone();
        let renamed = client.call_tool(call("desigma_rename_page", json!({"page_id":page,"name":"Renamed"}))).await.unwrap();
        assert_eq!(renamed.is_error, Some(false), "{renamed:?}");
        let after = client.call_tool(call("desigma_get_document", json!({}))).await.unwrap();
        assert_eq!(after.structured_content.unwrap()["pages"][0]["name"], "Renamed");
        client.call_tool(call("desigma_undo", json!({}))).await.unwrap();
        let undone = client.call_tool(call("desigma_get_document", json!({}))).await.unwrap();
        assert_eq!(undone.structured_content.unwrap()["pages"][0]["name"], old_name);
        // 负向：非页面节点不能当页面改名
        let not_page = client.call_tool(call("desigma_rename_page", json!({"page_id":rect_id,"name":"x"}))).await.unwrap();
        assert_eq!(not_page.is_error, Some(true));

        client.cancel().await.unwrap();
    })
    .await
    .expect("MCP subprocess must finish within 30 seconds");
}
