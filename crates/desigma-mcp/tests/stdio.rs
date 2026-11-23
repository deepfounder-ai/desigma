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
        // 53 upstream tools + 5 for components, instances and page rename
        for name in ["desigma_create_component", "desigma_create_instance", "desigma_detach_instance", "desigma_sync_instances", "desigma_rename_page"] {
            assert!(tools.iter().any(|t| t.name == name), "missing component tool {name}");
        }
        assert_eq!(tools.len(), 58);
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

/// Duplicating a layer copies its subtree, offsets it and undoes as one step.
#[tokio::test]
async fn duplicating_a_layer_copies_its_subtree_and_undoes_as_one_step() {
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_desigma-mcp"));
        command.arg("--workspace").arg(env!("CARGO_MANIFEST_DIR"));
        command.env(desigma_mcp::live::ENV_FILE, "");
        let client = ().serve(TokioChildProcess::new(command).unwrap()).await.unwrap();
        let call = |name: &'static str, args: Value| CallToolRequestParams::new(name).with_arguments(args.as_object().unwrap().clone());
        let doc = client.call_tool(call("desigma_get_document", json!({}))).await.unwrap();
        let page = doc.structured_content.unwrap()["pages"][0]["node_id"].clone();
        let frame =
            client.call_tool(call("desigma_create_node", json!({"parent_id":page,"kind":"frame","name":"Card","width":100,"height":100}))).await.unwrap();
        let frame_id = frame.structured_content.unwrap()["node"]["id"].clone();
        let inner = client.call_tool(call("desigma_create_node", json!({"parent_id":frame_id,"kind":"rectangle","name":"Inner"}))).await.unwrap();
        let inner_id = inner.structured_content.unwrap()["node"]["id"].clone();

        let duplicated = client.call_tool(call("desigma_duplicate_node", json!({"node_id":frame_id}))).await.unwrap();
        assert_eq!(duplicated.is_error, Some(false), "{duplicated:?}");
        let body = duplicated.structured_content.unwrap();
        assert_eq!(body["nodes"], 2);
        let copy_id = body["node_id"].clone();
        let copied = client.call_tool(call("desigma_get_node", json!({"node_id":copy_id}))).await.unwrap();
        let copied = copied.structured_content.unwrap()["node"].clone();
        assert_eq!(copied["name"], "Card");
        assert_eq!(copied["transform"]["tx"], 10.0);
        assert_eq!(copied["parent"], page);
        assert_ne!(copied["children"][0], inner_id);
        let page_node = client.call_tool(call("desigma_get_node", json!({"node_id":page}))).await.unwrap();
        assert_eq!(page_node.structured_content.unwrap()["node"]["children"], json!([frame_id, copy_id]));

        client.call_tool(call("desigma_undo", json!({}))).await.unwrap();
        let gone = client.call_tool(call("desigma_get_node", json!({"node_id":copy_id}))).await.unwrap();
        assert_eq!(gone.is_error, Some(true));
        let listed = client.call_tool(call("desigma_list_nodes", json!({"parent_id":page}))).await.unwrap();
        assert_eq!(listed.structured_content.unwrap()["total"], 1);
        client.cancel().await.unwrap();
    })
    .await
    .expect("MCP subprocess must finish within 30 seconds");
}

/// A batch creates a parent and child with `$0` references in one undo step.
#[tokio::test]
async fn batch_creates_nodes_with_references_over_stdio() {
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_desigma-mcp"));
        command.arg("--workspace").arg(env!("CARGO_MANIFEST_DIR"));
        command.env(desigma_mcp::live::ENV_FILE, "");
        let client = ().serve(TokioChildProcess::new(command).unwrap()).await.unwrap();
        let call = |name: &'static str, args: Value| CallToolRequestParams::new(name).with_arguments(args.as_object().unwrap().clone());
        let batched = client
            .call_tool(call(
                "desigma_batch",
                json!({"operations":[
                    {"tool":"desigma_create_node","arguments":{"parent_id":1,"kind":"frame","name":"Card"}},
                    {"tool":"desigma_create_node","arguments":{"parent_id":"$0","kind":"rectangle","name":"Inner"}}
                ]}),
            ))
            .await
            .unwrap();
        assert_eq!(batched.is_error, Some(false), "{batched:?}");
        let body = batched.structured_content.unwrap();
        let frame_id = body["results"][0]["node_id"].clone();
        let child_id = body["results"][1]["node_id"].clone();
        let child = client.call_tool(call("desigma_get_node", json!({"node_id":child_id}))).await.unwrap();
        assert_eq!(child.structured_content.unwrap()["node"]["parent"], frame_id);
        client.call_tool(call("desigma_undo", json!({}))).await.unwrap();
        let gone = client.call_tool(call("desigma_get_node", json!({"node_id":child_id}))).await.unwrap();
        assert_eq!(gone.is_error, Some(true));
        client.cancel().await.unwrap();
    })
    .await
    .expect("MCP subprocess must finish within 30 seconds");
}

/// Effects can be replaced and cleared over stdio.
#[tokio::test]
async fn effects_round_trip_over_stdio() {
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_desigma-mcp"));
        command.arg("--workspace").arg(env!("CARGO_MANIFEST_DIR"));
        command.env(desigma_mcp::live::ENV_FILE, "");
        let client = ().serve(TokioChildProcess::new(command).unwrap()).await.unwrap();
        let call = |name: &'static str, args: Value| CallToolRequestParams::new(name).with_arguments(args.as_object().unwrap().clone());
        let rect = client
            .call_tool(call("desigma_create_node", json!({"parent_id":1,"kind":"rectangle","name":"Card"})))
            .await
            .unwrap();
        let rect_id = rect.structured_content.unwrap()["node"]["id"].clone();
        let set = client
            .call_tool(call(
                "desigma_set_effects",
                json!({"node_id":rect_id.clone(),"effects":[{"type":"drop_shadow","color":"#00000040","offset":{"x":0,"y":4},"radius":12},{"type":"layer_blur","radius":6}]}),
            ))
            .await
            .unwrap();
        assert_eq!(set.is_error, Some(false), "{set:?}");
        assert_eq!(set.structured_content.unwrap()["effects"].as_array().unwrap().len(), 2);
        let clear = client.call_tool(call("desigma_set_effects", json!({"node_id":rect_id,"effects":[]}))).await.unwrap();
        assert_eq!(clear.is_error, Some(false), "{clear:?}");
        assert!(clear.structured_content.unwrap()["effects"].as_array().unwrap().is_empty());
        client.cancel().await.unwrap();
    })
    .await
    .expect("MCP subprocess must finish within 30 seconds");
}

/// Variable collections, modes, values and aliases round-trip over stdio.
#[tokio::test]
async fn variables_round_trip_over_stdio() {
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_desigma-mcp"));
        command.arg("--workspace").arg(env!("CARGO_MANIFEST_DIR"));
        command.env(desigma_mcp::live::ENV_FILE, "");
        let client = ().serve(TokioChildProcess::new(command).unwrap()).await.unwrap();
        let call = |name: &'static str, args: Value| CallToolRequestParams::new(name).with_arguments(args.as_object().unwrap().clone());

        let collection =
            client.call_tool(call("desigma_create_variable_collection", json!({"name":"Theme","modes":["Light","Dark"],"default_mode":"Dark"}))).await.unwrap();
        assert_eq!(collection.is_error, Some(false), "{collection:?}");
        let collection = collection.structured_content.unwrap()["collection"].clone();
        let collection_id = collection["id"].clone();
        let light = collection["modes"].as_array().unwrap().iter().find(|m| m["name"] == "Light").unwrap()["id"].clone();
        let dark = collection["modes"].as_array().unwrap().iter().find(|m| m["name"] == "Dark").unwrap()["id"].clone();

        let variable = client
            .call_tool(call(
                "desigma_create_variable",
                json!({
                    "collection_id":collection_id,
                    "name":"Color/Brand/Primary",
                    "kind":"color",
                    "values":{"Light":"#2F5BFF","Dark":"#8AA2FF"}
                }),
            ))
            .await
            .unwrap();
        assert_eq!(variable.is_error, Some(false), "{variable:?}");
        let variable = variable.structured_content.unwrap()["variable"].clone();
        let variable_id = variable["id"].clone();
        assert!(variable["values"].get(light.as_str().unwrap()).is_some());
        assert!(variable["values"].get(dark.as_str().unwrap()).is_some());

        let set = client.call_tool(call("desigma_set_variable_value", json!({"variable_id":variable_id,"mode":"Light","value":"#FF6B4A"}))).await.unwrap();
        assert_eq!(set.is_error, Some(false), "{set:?}");
        assert_eq!(set.structured_content.unwrap()["changed"], true);

        let alias = client
            .call_tool(call(
                "desigma_create_variable",
                json!({"collection_id":collection_id,"name":"Color/Action","kind":"color","values":{"Light":{"alias":variable_id}}}),
            ))
            .await
            .unwrap();
        assert_eq!(alias.is_error, Some(false), "{alias:?}");
        let alias = alias.structured_content.unwrap()["variable"].clone();
        let alias_id = alias["id"].clone();
        let vars = client.call_tool(call("desigma_get_variables", json!({}))).await.unwrap();
        let vars = vars.structured_content.unwrap();
        assert_eq!(vars["variables"][alias_id.as_str().unwrap()]["values"][light.as_str().unwrap()]["Alias"], variable_id);

        let node = client.call_tool(call("desigma_create_node", json!({"parent_id":1,"kind":"rectangle","name":"Bound"}))).await.unwrap();
        let node_id = node.structured_content.unwrap()["node"]["id"].clone();
        let bound = client
            .call_tool(call("desigma_bind_variable", json!({"node_ids":[node_id.clone()],"property":"fills/0","variable_id":variable_id.clone()})))
            .await
            .unwrap();
        assert_eq!(bound.is_error, Some(false), "{bound:?}");
        let effective = client.call_tool(call("desigma_get_variables", json!({"node_id":node_id.clone()}))).await.unwrap();
        assert_eq!(effective.structured_content.unwrap()["effective_bindings"][0]["property"], "fills/0");
        let mode = client
            .call_tool(call("desigma_set_variable_mode", json!({"node_ids":[node_id.clone()],"collection_id":collection_id,"mode":"Dark"})))
            .await
            .unwrap();
        assert_eq!(mode.is_error, Some(false), "{mode:?}");
        let detached = client.call_tool(call("desigma_detach_variable", json!({"node_ids":[node_id],"property":"fills/0"}))).await.unwrap();
        assert_eq!(detached.is_error, Some(false), "{detached:?}");

        let added = client.call_tool(call("desigma_add_variable_mode", json!({"collection_id":collection_id,"name":"High contrast"}))).await.unwrap();
        assert_eq!(added.is_error, Some(false), "{added:?}");
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

/// The component, instance and page-rename tools, round trip through a real
/// MCP subprocess: the main component and its instances, detach, sync, rename.
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

        // A frame becomes a main component in place, and can be renamed.
        let card =
            client.call_tool(call("desigma_create_node", json!({"parent_id":page,"kind":"frame","name":"card","width":400,"height":200}))).await.unwrap();
        let card_id = card.structured_content.unwrap()["node"]["id"].clone();
        let comp = client.call_tool(call("desigma_create_component", json!({"node_id":card_id,"name":"CardMain"}))).await.unwrap();
        assert_eq!(comp.is_error, Some(false), "{comp:?}");
        let comp_sc = comp.structured_content.unwrap();
        assert_eq!(comp_sc["component_id"], card_id);
        assert_eq!(comp_sc["node"]["name"], "CardMain");

        // Negative: a rectangle is not a layer that can become a component.
        let rect =
            client.call_tool(call("desigma_create_node", json!({"parent_id":page,"kind":"rectangle","name":"row","width":100,"height":40}))).await.unwrap();
        let rect_id = rect.structured_content.unwrap()["node"]["id"].clone();
        let refused = client.call_tool(call("desigma_create_component", json!({"node_id":rect_id}))).await.unwrap();
        assert_eq!(refused.is_error, Some(true));

        // Instances go into a frame or straight onto a page.
        let host =
            client.call_tool(call("desigma_create_node", json!({"parent_id":page,"kind":"frame","name":"host","width":600,"height":400}))).await.unwrap();
        let host_id = host.structured_content.unwrap()["node"]["id"].clone();
        // Negative: the parent is the component itself, which would refer to itself.
        let bad_parent = client.call_tool(call("desigma_create_instance", json!({"component_id":card_id,"parent_id":card_id,"x":0.0,"y":0.0}))).await.unwrap();
        assert_eq!(bad_parent.is_error, Some(true), "a component cannot be instantiated inside itself");
        // Negative: a plain layer is not a main component.
        let bad_component =
            client.call_tool(call("desigma_create_instance", json!({"component_id":rect_id,"parent_id":host_id,"x":0.0,"y":0.0}))).await.unwrap();
        assert_eq!(bad_component.is_error, Some(true), "a rectangle is not a main component");
        let inst = client.call_tool(call("desigma_create_instance", json!({"component_id":card_id,"parent_id":host_id,"x":12.0,"y":24.0}))).await.unwrap();
        assert_eq!(inst.is_error, Some(false), "{inst:?}");
        let instance_id = inst.structured_content.unwrap()["instance_id"].clone();
        assert!(instance_id.as_u64().is_some());

        // Sync: the main component's current state reaches every instance.
        let synced = client.call_tool(call("desigma_sync_instances", json!({"component_id":card_id}))).await.unwrap();
        assert_eq!(synced.is_error, Some(false), "{synced:?}");
        assert_eq!(synced.structured_content.unwrap()["synced"], true);
        // Negative: a plain layer is not a component to sync from.
        let bad_sync = client.call_tool(call("desigma_sync_instances", json!({"component_id":rect_id}))).await.unwrap();
        assert_eq!(bad_sync.is_error, Some(true));

        // Detach turns the instance into ordinary layers, and undoes.
        let detached = client.call_tool(call("desigma_detach_instance", json!({"node_id":instance_id}))).await.unwrap();
        assert_eq!(detached.is_error, Some(false), "{detached:?}");
        assert_eq!(detached.structured_content.unwrap()["detached"], true);
        client.call_tool(call("desigma_undo", json!({}))).await.unwrap();
        let still_instance = client.call_tool(call("desigma_get_node", json!({"node_id":instance_id}))).await.unwrap();
        assert_eq!(still_instance.is_error, Some(false), "undoing the detach brings the instance back");

        // Renaming a page undoes.
        let before = client.call_tool(call("desigma_get_document", json!({}))).await.unwrap();
        let old_name = before.structured_content.unwrap()["pages"][0]["name"].clone();
        let renamed = client.call_tool(call("desigma_rename_page", json!({"page_id":page,"name":"Renamed"}))).await.unwrap();
        assert_eq!(renamed.is_error, Some(false), "{renamed:?}");
        let after = client.call_tool(call("desigma_get_document", json!({}))).await.unwrap();
        assert_eq!(after.structured_content.unwrap()["pages"][0]["name"], "Renamed");
        client.call_tool(call("desigma_undo", json!({}))).await.unwrap();
        let undone = client.call_tool(call("desigma_get_document", json!({}))).await.unwrap();
        assert_eq!(undone.structured_content.unwrap()["pages"][0]["name"], old_name);
        // Negative: a blank name is refused, as create_page refuses it.
        let blank = client.call_tool(call("desigma_rename_page", json!({"page_id":page,"name":"   "}))).await.unwrap();
        assert_eq!(blank.is_error, Some(true));
        // Negative: a non-page node cannot be renamed as a page.
        let not_page = client.call_tool(call("desigma_rename_page", json!({"page_id":rect_id,"name":"x"}))).await.unwrap();
        assert_eq!(not_page.is_error, Some(true));

        client.cancel().await.unwrap();
    })
    .await
    .expect("MCP subprocess must finish within 30 seconds");
}

/// A file session must keep instance edits as overrides, let main-component
/// edits reach their instances in the same undo step, and not push an empty
/// undo step when a sync has nothing to do.
#[tokio::test]
async fn instance_overrides_survive_sync_and_main_edits_follow() {
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_desigma-mcp"));
        command.arg("--workspace").arg(env!("CARGO_MANIFEST_DIR"));
        command.env(desigma_mcp::live::ENV_FILE, "");
        let client = ().serve(TokioChildProcess::new(command).unwrap()).await.unwrap();
        let call = |name: &'static str, args: Value| CallToolRequestParams::new(name).with_arguments(args.as_object().unwrap().clone());

        let doc = client.call_tool(call("desigma_get_document", json!({}))).await.unwrap();
        let page = doc.structured_content.unwrap()["pages"][0]["node_id"].clone();

        // Main component: a frame with the rectangles `a` and `b` inside.
        let card =
            client.call_tool(call("desigma_create_node", json!({"parent_id":page,"kind":"frame","name":"card","width":200,"height":100}))).await.unwrap();
        let card_id = card.structured_content.unwrap()["node"]["id"].clone();
        let main_a =
            client.call_tool(call("desigma_create_node", json!({"parent_id":card_id,"kind":"rectangle","name":"a","width":40,"height":40}))).await.unwrap();
        let main_a_id = main_a.structured_content.unwrap()["node"]["id"].clone();
        let main_b = client
            .call_tool(call("desigma_create_node", json!({"parent_id":card_id,"kind":"rectangle","name":"b","x":50.0,"width":40,"height":40})))
            .await
            .unwrap();
        let main_b_id = main_b.structured_content.unwrap()["node"]["id"].clone();
        let comp = client.call_tool(call("desigma_create_component", json!({"node_id":card_id}))).await.unwrap();
        assert_eq!(comp.is_error, Some(false), "{comp:?}");

        let host =
            client.call_tool(call("desigma_create_node", json!({"parent_id":page,"kind":"frame","name":"host","width":600,"height":400}))).await.unwrap();
        let host_id = host.structured_content.unwrap()["node"]["id"].clone();
        let inst = client.call_tool(call("desigma_create_instance", json!({"component_id":card_id,"parent_id":host_id}))).await.unwrap();
        assert_eq!(inst.is_error, Some(false), "{inst:?}");
        let instance_id = inst.structured_content.unwrap()["instance_id"].clone();

        // The mirrored children keep their names, so they can be addressed.
        let kids = client.call_tool(call("desigma_list_nodes", json!({"parent_id":instance_id}))).await.unwrap();
        let kids = kids.structured_content.unwrap()["nodes"].as_array().cloned().unwrap();
        let mirror = |name: &str| kids.iter().find(|n| n["name"] == name).expect("mirrored child")["node_id"].clone();
        let mirror_a_id = mirror("a");
        let mirror_b_id = mirror("b");

        // An edit inside the instance is an override, not a lost edit.
        let edited = client.call_tool(call("desigma_set_appearance", json!({"node_id":mirror_a_id,"fill":"#00ff00"}))).await.unwrap();
        assert_eq!(edited.is_error, Some(false), "{edited:?}");
        let overridden = client.call_tool(call("desigma_get_node", json!({"node_id":mirror_a_id}))).await.unwrap();
        let overridden = overridden.structured_content.unwrap()["node"].clone();
        let green = overridden["fills"].clone();
        assert!(
            overridden["component"]["overrides"].as_array().unwrap().iter().any(|f| f == "Fills"),
            "the edit must be recorded as an override: {overridden}"
        );

        // A manual sync keeps it and does not clobber the fill.
        let synced = client.call_tool(call("desigma_sync_instances", json!({"component_id":card_id}))).await.unwrap();
        assert_eq!(synced.is_error, Some(false), "{synced:?}");
        let after_sync = client.call_tool(call("desigma_get_node", json!({"node_id":mirror_a_id}))).await.unwrap();
        assert_eq!(after_sync.structured_content.unwrap()["node"]["fills"], green, "sync must keep the override");

        // Editing the main reaches the instances without an explicit sync, and
        // still leaves the overridden child alone.
        let main_edited = client.call_tool(call("desigma_set_appearance", json!({"node_id":main_b_id,"fill":"#0000ff"}))).await.unwrap();
        assert_eq!(main_edited.is_error, Some(false), "{main_edited:?}");
        let followed = client.call_tool(call("desigma_get_node", json!({"node_id":mirror_b_id}))).await.unwrap();
        let main_now = client.call_tool(call("desigma_get_node", json!({"node_id":main_b_id}))).await.unwrap();
        assert_eq!(
            followed.structured_content.unwrap()["node"]["fills"],
            main_now.structured_content.unwrap()["node"]["fills"],
            "the instance follows an edit to the main component"
        );
        let kept = client.call_tool(call("desigma_get_node", json!({"node_id":mirror_a_id}))).await.unwrap();
        assert_eq!(kept.structured_content.unwrap()["node"]["fills"], green, "the override survives the main edit");
        let main_a_now = client.call_tool(call("desigma_get_node", json!({"node_id":main_a_id}))).await.unwrap();
        assert_ne!(main_a_now.structured_content.unwrap()["node"]["fills"], green, "the override stays on the instance only");

        // A component nobody instanced: nothing to sync, nothing to undo.
        let lonely =
            client.call_tool(call("desigma_create_node", json!({"parent_id":page,"kind":"frame","name":"lonely","width":80,"height":80}))).await.unwrap();
        let lonely_id = lonely.structured_content.unwrap()["node"]["id"].clone();
        client.call_tool(call("desigma_create_component", json!({"node_id":lonely_id}))).await.unwrap();
        let nothing = client.call_tool(call("desigma_sync_instances", json!({"component_id":lonely_id}))).await.unwrap();
        assert_eq!(nothing.is_error, Some(false), "{nothing:?}");
        assert_eq!(nothing.structured_content.unwrap()["changed"], false, "an empty sync must not push an undo step");
        client.call_tool(call("desigma_undo", json!({}))).await.unwrap();
        let reverted = client.call_tool(call("desigma_get_node", json!({"node_id":lonely_id}))).await.unwrap();
        let kind = reverted.structured_content.unwrap()["node"]["kind"].clone();
        assert!(kind.get("Frame").is_some(), "undo must reach the conversion, not a no-op sync: {kind}");

        client.cancel().await.unwrap();
    })
    .await
    .expect("MCP subprocess must finish within 30 seconds");
}

/// Shared fill styles can be created, applied and detached over stdio.
#[tokio::test]
async fn shared_styles_round_trip_over_stdio() {
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_desigma-mcp"));
        command.arg("--workspace").arg(env!("CARGO_MANIFEST_DIR"));
        command.env(desigma_mcp::live::ENV_FILE, "");
        let client = ().serve(TokioChildProcess::new(command).unwrap()).await.unwrap();
        let call = |name: &'static str, args: Value| CallToolRequestParams::new(name).with_arguments(args.as_object().unwrap().clone());
        let rect = client.call_tool(call("desigma_create_node", json!({"parent_id":1,"kind":"rectangle","name":"Card"}))).await.unwrap();
        let rect_id = rect.structured_content.unwrap()["node"]["id"].clone();
        let style = client.call_tool(call("desigma_create_style", json!({"name":"Color/Brand","kind":"fill","color":"#2F5BFF"}))).await.unwrap();
        assert_eq!(style.is_error, Some(false), "{style:?}");
        let style_key = style.structured_content.unwrap()["style_key"].clone();
        let applied = client.call_tool(call("desigma_apply_style", json!({"node_ids":[rect_id.clone()],"style":style_key.clone()}))).await.unwrap();
        assert_eq!(applied.is_error, Some(false), "{applied:?}");
        let node = client.call_tool(call("desigma_get_node", json!({"node_id":rect_id.clone()}))).await.unwrap();
        assert_eq!(node.structured_content.unwrap()["node"]["style_refs"]["fill"], style_key);
        let detached = client.call_tool(call("desigma_detach_style", json!({"node_ids":[rect_id],"property":"fill"}))).await.unwrap();
        assert_eq!(detached.is_error, Some(false), "{detached:?}");
        client.cancel().await.unwrap();
    })
    .await
    .expect("MCP subprocess must finish within 30 seconds");
}

/// Variant lifecycle: combine, copy/move add, rename, remove, and undo.
#[tokio::test]
async fn component_variants_round_trip_over_stdio() {
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_desigma-mcp"));
        command.arg("--workspace").arg(env!("CARGO_MANIFEST_DIR"));
        command.env(desigma_mcp::live::ENV_FILE, "");
        let client = ().serve(TokioChildProcess::new(command).unwrap()).await.unwrap();
        let call = |name: &'static str, args: Value| CallToolRequestParams::new(name).with_arguments(args.as_object().unwrap().clone());
        let page = client.call_tool(call("desigma_get_document", json!({}))).await.unwrap().structured_content.unwrap()["pages"][0]["node_id"].clone();

        let small =
            client.call_tool(call("desigma_create_node", json!({"parent_id":page,"kind":"frame","name":"Size=Small","width":100,"height":40}))).await.unwrap();
        let small_id = small.structured_content.unwrap()["node"]["id"].clone();
        let large =
            client.call_tool(call("desigma_create_node", json!({"parent_id":page,"kind":"frame","name":"Size=Large","width":120,"height":40}))).await.unwrap();
        let large_id = large.structured_content.unwrap()["node"]["id"].clone();
        for id in [&small_id, &large_id] {
            client.call_tool(call("desigma_create_component", json!({"node_id":id}))).await.unwrap();
        }
        let combined = client.call_tool(call("desigma_combine_as_variants", json!({"component_ids":[small_id,large_id],"name":"Button"}))).await.unwrap();
        assert_eq!(combined.is_error, Some(false), "{combined:?}");
        let combined = combined.structured_content.unwrap();
        let set_id = combined["set_id"].clone();
        assert_eq!(combined["properties"].as_array().unwrap().len(), 1);
        let set = client.call_tool(call("desigma_get_node", json!({"node_id":set_id}))).await.unwrap().structured_content.unwrap()["node"].clone();
        assert_eq!(set["name"], "Button");
        assert_eq!(set["children"].as_array().unwrap().len(), 2);

        let instance = client.call_tool(call("desigma_create_instance", json!({"component_id":small_id,"parent_id":page}))).await.unwrap();
        assert_eq!(instance.is_error, Some(false), "{instance:?}");
        let instance_id = instance.structured_content.unwrap()["instance_id"].clone();

        let copied = client
            .call_tool(call("desigma_add_variant", json!({"set_id":set_id,"source_id":small_id,"mode":"copy","values":{"Size":"Medium"}})))
            .await
            .unwrap();
        assert_eq!(copied.is_error, Some(false), "{copied:?}");
        let copied = copied.structured_content.unwrap();
        let medium_id = copied["variant_id"].clone();
        assert_eq!(copied["name"], "Size=Medium");
        assert_ne!(medium_id, small_id);
        assert_eq!(
            client.call_tool(call("desigma_get_node", json!({"node_id":small_id}))).await.unwrap().structured_content.unwrap()["node"]["parent"],
            set_id
        );
        let instance_after = client.call_tool(call("desigma_get_node", json!({"node_id":instance_id}))).await.unwrap();
        assert_eq!(instance_after.structured_content.unwrap()["node"]["kind"]["Instance"]["component"], small_id);
        let selected = client.call_tool(call("desigma_set_instance_property", json!({"node_id":instance_id,"name":"Size","value":"Medium"}))).await.unwrap();
        assert_eq!(selected.is_error, Some(false), "{selected:?}");
        let selected_node =
            client.call_tool(call("desigma_get_node", json!({"node_id":instance_id}))).await.unwrap().structured_content.unwrap()["node"].clone();
        assert_eq!(selected_node["kind"]["Instance"]["component"], medium_id, "the instance follows the variant property");
        assert_eq!(selected_node["component"]["values"]["Size"]["Variant"], "Medium");

        let standalone =
            client.call_tool(call("desigma_create_node", json!({"parent_id":page,"kind":"frame","name":"Icon","width":32,"height":32}))).await.unwrap();
        let standalone_id = standalone.structured_content.unwrap()["node"]["id"].clone();
        client.call_tool(call("desigma_create_component", json!({"node_id":standalone_id}))).await.unwrap();
        let moved = client
            .call_tool(call("desigma_add_variant", json!({"set_id":set_id,"source_id":standalone_id,"mode":"move","values":{"Size":"Icon"}})))
            .await
            .unwrap();
        assert_eq!(moved.is_error, Some(false), "{moved:?}");
        let moved_id = moved.structured_content.unwrap()["variant_id"].clone();
        assert_eq!(
            client.call_tool(call("desigma_get_node", json!({"node_id":moved_id}))).await.unwrap().structured_content.unwrap()["node"]["parent"],
            set_id
        );

        let removed = client.call_tool(call("desigma_remove_variant", json!({"variant_id":moved_id}))).await.unwrap();
        assert_eq!(removed.is_error, Some(false), "{removed:?}");
        let set = client.call_tool(call("desigma_get_node", json!({"node_id":set_id}))).await.unwrap().structured_content.unwrap()["node"].clone();
        assert_eq!(set["children"].as_array().unwrap().len(), 3);
        let options = set["component"]["props"][0]["kind"]["Variant"]["options"].clone();
        assert!(!options.as_array().unwrap().iter().any(|option| option == "Icon"), "removed option must be pruned");

        let large_instance = client.call_tool(call("desigma_create_instance", json!({"component_id":large_id,"parent_id":page}))).await.unwrap();
        assert_eq!(large_instance.is_error, Some(false), "{large_instance:?}");
        let blocked = client.call_tool(call("desigma_remove_variant", json!({"variant_id":large_id}))).await.unwrap();
        assert_eq!(blocked.is_error, Some(true), "a variant with an instance must be refused");
        assert!(blocked.structured_content.unwrap()["error"].as_str().unwrap().contains("instance"));

        // A set with a single variant cannot lose its last variant.
        let lonely =
            client.call_tool(call("desigma_create_node", json!({"parent_id":page,"kind":"frame","name":"Only","width":40,"height":40}))).await.unwrap();
        let lonely_id = lonely.structured_content.unwrap()["node"]["id"].clone();
        client.call_tool(call("desigma_create_component", json!({"node_id":lonely_id}))).await.unwrap();
        let lonely_set = client.call_tool(call("desigma_combine_as_variants", json!({"component_ids":[lonely_id]}))).await.unwrap();
        assert_eq!(lonely_set.is_error, Some(false), "{lonely_set:?}");
        let last = client.call_tool(call("desigma_remove_variant", json!({"variant_id":lonely_id}))).await.unwrap();
        assert_eq!(last.is_error, Some(true), "the last variant must be refused");

        let renamed = client.call_tool(call("desigma_rename_variant", json!({"variant_id":medium_id,"values":{"Size":"Wide"}}))).await.unwrap();
        assert_eq!(renamed.is_error, Some(false), "{renamed:?}");
        assert_eq!(renamed.structured_content.unwrap()["name"], "Size=Wide");
        let set = client.call_tool(call("desigma_get_node", json!({"node_id":set_id}))).await.unwrap().structured_content.unwrap()["node"].clone();
        let options = set["component"]["props"][0]["kind"]["Variant"]["options"].clone();
        assert!(options.as_array().unwrap().iter().any(|option| option == "Wide"));
        assert!(!options.as_array().unwrap().iter().any(|option| option == "Medium"));
        let renamed_instance =
            client.call_tool(call("desigma_get_node", json!({"node_id":instance_id}))).await.unwrap().structured_content.unwrap()["node"].clone();
        assert_eq!(renamed_instance["component"]["values"]["Size"]["Variant"], "Wide", "explicit instance selection follows the rename");
        assert_eq!(renamed_instance["kind"]["Instance"]["component"], medium_id, "rename must not swap the instance");

        client.call_tool(call("desigma_undo", json!({}))).await.unwrap();
        let restored = client.call_tool(call("desigma_get_node", json!({"node_id":medium_id}))).await.unwrap().structured_content.unwrap()["node"].clone();
        assert_eq!(restored["name"], "Size=Medium", "renaming is one undo step");

        let raw_rename = client.call_tool(call("desigma_update_node", json!({"node_id":medium_id,"name":"Size=Other"}))).await.unwrap();
        assert_eq!(raw_rename.is_error, Some(true), "update_node must not bypass variant properties");

        client.cancel().await.unwrap();
    })
    .await
    .expect("MCP subprocess must finish within 30 seconds");
}

/// Component properties: add, bind, set, rename, delete, and derived-variant refusal.
#[tokio::test]
async fn component_properties_round_trip_over_stdio() {
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_desigma-mcp"));
        command.arg("--workspace").arg(env!("CARGO_MANIFEST_DIR"));
        command.env(desigma_mcp::live::ENV_FILE, "");
        let client = ().serve(TokioChildProcess::new(command).unwrap()).await.unwrap();
        let call = |name: &'static str, args: Value| CallToolRequestParams::new(name).with_arguments(args.as_object().unwrap().clone());
        let page = client.call_tool(call("desigma_get_document", json!({}))).await.unwrap().structured_content.unwrap()["pages"][0]["node_id"].clone();

        let card =
            client.call_tool(call("desigma_create_node", json!({"parent_id":page,"kind":"frame","name":"Card","width":200,"height":100}))).await.unwrap();
        let card_id = card.structured_content.unwrap()["node"]["id"].clone();
        let icon =
            client.call_tool(call("desigma_create_node", json!({"parent_id":card_id,"kind":"rectangle","name":"Icon","width":20,"height":20}))).await.unwrap();
        let icon_id = icon.structured_content.unwrap()["node"]["id"].clone();
        let label = client
            .call_tool(call("desigma_create_node", json!({"parent_id":card_id,"kind":"text","name":"Label","text":"Label","width":120,"height":24})))
            .await
            .unwrap();
        let label_id = label.structured_content.unwrap()["node"]["id"].clone();
        client.call_tool(call("desigma_create_component", json!({"node_id":card_id}))).await.unwrap();

        let show = client
            .call_tool(call("desigma_add_component_property", json!({"owner_id":card_id,"name":"Show icon","kind":"boolean","default":true})))
            .await
            .unwrap();
        assert_eq!(show.is_error, Some(false), "{show:?}");
        let caption =
            client.call_tool(call("desigma_add_component_property", json!({"owner_id":card_id,"name":"Label","kind":"text","default":"Label"}))).await.unwrap();
        assert_eq!(caption.is_error, Some(false), "{caption:?}");

        let bind_show =
            client.call_tool(call("desigma_bind_component_property", json!({"node_id":icon_id,"field":"visible","property":"Show icon"}))).await.unwrap();
        assert_eq!(bind_show.is_error, Some(false), "{bind_show:?}");
        let bind_label =
            client.call_tool(call("desigma_bind_component_property", json!({"node_id":label_id,"field":"characters","property":"Label"}))).await.unwrap();
        assert_eq!(bind_label.is_error, Some(false), "{bind_label:?}");

        let instance = client.call_tool(call("desigma_create_instance", json!({"component_id":card_id,"parent_id":page}))).await.unwrap();
        assert_eq!(instance.is_error, Some(false), "{instance:?}");
        let instance_id = instance.structured_content.unwrap()["instance_id"].clone();
        let kids = client.call_tool(call("desigma_list_nodes", json!({"parent_id":instance_id}))).await.unwrap().structured_content.unwrap()["nodes"]
            .as_array()
            .cloned()
            .unwrap();
        let mirror = |name: &str| kids.iter().find(|node| node["name"] == name).expect("mirrored child")["node_id"].clone();
        let icon_i = mirror("Icon");
        let label_i = mirror("Label");

        let hidden = client.call_tool(call("desigma_set_instance_property", json!({"node_id":instance_id,"name":"Show icon","value":false}))).await.unwrap();
        assert_eq!(hidden.is_error, Some(false), "{hidden:?}");
        let icon_node = client.call_tool(call("desigma_get_node", json!({"node_id":icon_i}))).await.unwrap().structured_content.unwrap()["node"].clone();
        assert_eq!(icon_node["visible"], false);

        let changed = client.call_tool(call("desigma_set_instance_property", json!({"node_id":instance_id,"name":"Label","value":"Pay"}))).await.unwrap();
        assert_eq!(changed.is_error, Some(false), "{changed:?}");
        let label_node = client.call_tool(call("desigma_get_node", json!({"node_id":label_i}))).await.unwrap().structured_content.unwrap()["node"].clone();
        assert_eq!(label_node["kind"]["Text"]["content"], "Pay");
        let cleared = client.call_tool(call("desigma_set_instance_property", json!({"node_id":instance_id,"name":"Label"}))).await.unwrap();
        assert_eq!(cleared.is_error, Some(false), "{cleared:?}");
        let label_node = client.call_tool(call("desigma_get_node", json!({"node_id":label_i}))).await.unwrap().structured_content.unwrap()["node"].clone();
        assert_eq!(label_node["kind"]["Text"]["content"], "Label");

        let renamed = client
            .call_tool(call("desigma_update_component_property", json!({"owner_id":card_id,"name":"Label","new_name":"Caption","default":"Caption"})))
            .await
            .unwrap();
        assert_eq!(renamed.is_error, Some(false), "{renamed:?}");
        let def = client.call_tool(call("desigma_get_node", json!({"node_id":card_id}))).await.unwrap().structured_content.unwrap()["node"].clone();
        let defs = def["component"]["props"].clone();
        assert!(defs.as_array().unwrap().iter().any(|def| def["name"] == "Caption"));
        assert!(defs.as_array().unwrap().iter().all(|def| def["name"] != "Label"));

        let mismatch = client.call_tool(call("desigma_set_instance_property", json!({"node_id":instance_id,"name":"Show icon","value":"yes"}))).await.unwrap();
        assert_eq!(mismatch.is_error, Some(true), "type mismatch must fail");

        let alt = client.call_tool(call("desigma_create_node", json!({"parent_id":page,"kind":"frame","name":"Alt","width":20,"height":20}))).await.unwrap();
        let alt_id = alt.structured_content.unwrap()["node"]["id"].clone();
        client.call_tool(call("desigma_create_component", json!({"node_id":alt_id}))).await.unwrap();
        let swap = client
            .call_tool(call(
                "desigma_add_component_property",
                json!({"owner_id":card_id,"name":"Icon swap","kind":"instance_swap","default":alt_id,"preferred":[alt_id]}),
            ))
            .await
            .unwrap();
        assert_eq!(swap.is_error, Some(false), "{swap:?}");
        let swapped = client.call_tool(call("desigma_set_instance_property", json!({"node_id":instance_id,"name":"Icon swap","value":alt_id}))).await.unwrap();
        assert_eq!(swapped.is_error, Some(false), "{swapped:?}");

        let variant_prop = client.call_tool(call("desigma_add_component_property", json!({"owner_id":card_id,"name":"State","kind":"variant"}))).await.unwrap();
        assert_eq!(variant_prop.is_error, Some(true), "variant properties are derived");

        client.call_tool(call("desigma_delete_component_property", json!({"owner_id":card_id,"name":"Icon swap"}))).await.unwrap();
        let after_delete =
            client.call_tool(call("desigma_set_instance_property", json!({"node_id":instance_id,"name":"Icon swap","value":alt_id}))).await.unwrap();
        assert_eq!(after_delete.is_error, Some(true), "deleted property cannot be set");

        client.cancel().await.unwrap();
    })
    .await
    .expect("MCP subprocess must finish within 30 seconds");
}

/// Renaming one dimension keeps an option that another variant still uses.
#[tokio::test]
async fn renaming_a_shared_variant_value_keeps_the_option() {
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_desigma-mcp"));
        command.arg("--workspace").arg(env!("CARGO_MANIFEST_DIR"));
        command.env(desigma_mcp::live::ENV_FILE, "");
        let client = ().serve(TokioChildProcess::new(command).unwrap()).await.unwrap();
        let call = |name: &'static str, args: Value| CallToolRequestParams::new(name).with_arguments(args.as_object().unwrap().clone());
        let page = client.call_tool(call("desigma_get_document", json!({}))).await.unwrap().structured_content.unwrap()["pages"][0]["node_id"].clone();
        let mut ids = Vec::new();
        for name in ["Size=Small, State=Default", "Size=Small, State=Hover", "Size=Large, State=Default"] {
            let frame =
                client.call_tool(call("desigma_create_node", json!({"parent_id":page,"kind":"frame","name":name,"width":100,"height":40}))).await.unwrap();
            let id = frame.structured_content.unwrap()["node"]["id"].clone();
            client.call_tool(call("desigma_create_component", json!({"node_id":id}))).await.unwrap();
            ids.push(id);
        }
        let combined = client.call_tool(call("desigma_combine_as_variants", json!({"component_ids":ids,"name":"Button"}))).await.unwrap();
        assert_eq!(combined.is_error, Some(false), "{combined:?}");
        let combined = combined.structured_content.unwrap();
        let first = combined["variant_ids"][0].clone();
        let set_id = combined["set_id"].clone();
        let renamed = client.call_tool(call("desigma_rename_variant", json!({"variant_id":first,"values":{"Size":"Small","State":"Idle"}}))).await.unwrap();
        assert_eq!(renamed.is_error, Some(false), "{renamed:?}");
        let set = client.call_tool(call("desigma_get_node", json!({"node_id":set_id}))).await.unwrap();
        let set = set.structured_content.unwrap()["node"].clone();
        let defs = set["component"]["props"].as_array().unwrap();
        let state = defs.iter().find(|def| def["name"] == "State").unwrap();
        let options = state["kind"]["Variant"]["options"].as_array().unwrap();
        assert!(options.iter().any(|option| option == "Default"), "another variant still uses Default: {options:?}");
        assert!(options.iter().any(|option| option == "Hover"));
        assert!(options.iter().any(|option| option == "Idle"));
        assert_eq!(state["default"]["Variant"], "Idle", "the renamed variant keeps the default through its new value");
        client.cancel().await.unwrap();
    })
    .await
    .expect("MCP subprocess must finish within 30 seconds");
}
