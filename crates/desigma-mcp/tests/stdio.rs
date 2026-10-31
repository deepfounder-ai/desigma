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
        assert_eq!(tools.len(), 23);
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
