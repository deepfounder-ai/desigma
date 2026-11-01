use super::*;
use serde_json::{json, Value};

fn call(port: u16, token: &str, name: &str, args: Value) -> (u16, Value) {
    let r = ureq::post(&format!("http://127.0.0.1:{port}/{token}/call"))
        .set("X-Desigma-Token", token)
        .set("Content-Type", "application/json")
        .send_string(&json!({"name":name,"args":args}).to_string());
    let (status, response) = match r {
        Ok(ok) => (ok.status(), ok),
        Err(ureq::Error::Status(code, response)) => (code, response),
        Err(e) => panic!("{e}"),
    };
    (status, serde_json::from_reader(response.into_reader()).unwrap())
}

/// Runs `f` on a worker while pumping frames until it finishes.
fn with_frames<T: Send + 'static>(s: &mut Sim, f: impl FnOnce() -> T + Send + 'static) -> T {
    let (send, receive) = std::sync::mpsc::channel();
    std::thread::spawn(move || send.send(f()).unwrap());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);
    loop {
        s.frame(vec![]);
        if let Ok(v) = receive.try_recv() {
            return v;
        }
        assert!(std::time::Instant::now() < deadline, "bridge request did not complete");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// One bridge call over HTTP while frames are pumped.
fn send(s: &mut Sim, name: &str, args: Value) -> (u16, Value) {
    let b = s.app().bridge.bridge.as_ref().unwrap();
    let (port, token, name) = (b.port(), b.token().to_string(), name.to_string());
    with_frames(s, move || call(port, &token, &name, args))
}

#[test]
fn bridge_edits_live_document_with_user_undo_and_reports_selection() {
    let mut s = Sim::new();
    let ctx = s.ctx.clone();
    s.shell.app.start_bridge(&ctx).unwrap();
    let (port, token) = {
        let b = s.app().bridge.bridge.as_ref().unwrap();
        (b.port(), b.token().to_string())
    };
    let page = s.app().page.0;
    let before = s.app().doc.nodes.len();
    let (status, created) = {
        let t = token.clone();
        with_frames(&mut s, move || {
            call(port, &t, "desigma_create_node", json!({"parent_id":page,"kind":"rectangle","name":"From MCP","width":50,"height":30}))
        })
    };
    assert_eq!(status, 200, "{created}");
    let id = created["result"]["node"]["id"].as_u64().unwrap();
    assert_eq!(s.app().doc.nodes.len(), before + 1);
    assert!(s.app().hist.can_undo(), "edit must land in the user's undo stack");
    let step = |s: &mut Sim, name: &'static str| {
        let t = token.clone();
        let (status, body) = with_frames(s, move || call(port, &t, name, json!({})));
        assert_eq!(status, 200, "{body}");
        body["result"]["changed"].as_bool().unwrap()
    };
    assert!(step(&mut s, "desigma_undo"));
    assert_eq!(s.app().doc.nodes.len(), before);
    assert!(s.app().hist.can_redo());
    assert!(step(&mut s, "desigma_redo"));
    assert!(s.app().doc.get(NodeId(id)).is_some());
    assert!(!step(&mut s, "desigma_redo"), "nothing left to redo");
    let (_, sel) = {
        let t = token.clone();
        with_frames(&mut s, move || call(port, &t, "desigma_select_nodes", json!({"node_ids":[id]})))
    };
    assert_eq!(sel["result"]["selected"], 1);
    assert_eq!(s.app().selection, vec![NodeId(id)]);
    let (_, got) = {
        let t = token.clone();
        with_frames(&mut s, move || call(port, &t, "desigma_get_selection", json!({})))
    };
    assert_eq!(got["result"]["nodes"][0]["name"], "From MCP");
    let (_, doc) = {
        let t = token.clone();
        with_frames(&mut s, move || call(port, &t, "desigma_get_document", json!({})))
    };
    assert_eq!(doc["result"]["session"], "editor");
    s.shell.app.undo();
    assert_eq!(s.app().doc.nodes.len(), before);
    s.shell.app.stop_bridge();
    assert!(s.app().bridge.bridge.is_none());
}

#[test]
fn bridge_refuses_mutations_while_a_gesture_runs() {
    let mut s = Sim::new();
    let ctx = s.ctx.clone();
    s.shell.app.start_bridge(&ctx).unwrap();
    let (port, token) = {
        let b = s.app().bridge.bridge.as_ref().unwrap();
        (b.port(), b.token().to_string())
    };
    // hold the middle button over the canvas: a real pan stays active while the request arrives
    let p = s.app().canvas_rect.center();
    s.mv(p);
    s.frame(vec![Event::PointerButton { pos: p, button: PointerButton::Middle, pressed: true, modifiers: Modifiers::NONE }]);
    assert!(matches!(s.app().canvas.inter, crate::canvas::Interaction::Pan));
    let page = s.app().page.0;
    let before = s.app().doc.nodes.len();
    let (status, body) = with_frames(&mut s, move || call(port, &token, "desigma_create_node", json!({"parent_id":page,"kind":"frame"})));
    assert_eq!(status, 422);
    assert!(body["error"].as_str().unwrap().contains("busy"), "{body}");
    assert_eq!(s.app().doc.nodes.len(), before);
}

#[test]
fn preference_toggle_starts_the_bridge_saves_the_flag_and_the_pill_turns_it_off() {
    use crate::comments::settings;
    use crate::panels::menus::{self, Action};
    let dir = std::env::temp_dir().join(format!("desigma_bridge_pref_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut s = Sim::with(Options { library: Some(dir.clone()), ..Default::default() });
    let ctx = s.ctx.clone();
    menus::run(&mut s.shell.app, &ctx, Action::ToggleAgentBridge);
    assert!(s.app().bridge.bridge.is_some());
    assert_eq!(settings::load_flag(&dir, crate::bridge::FLAG), Some(true));
    assert!(dir.join(crate::bridge::DISCOVERY_FILE).exists());
    s.frame(vec![]);
    let off = s.rect_of("bridge-off");
    s.click(off.center());
    assert!(s.app().bridge.bridge.is_none());
    assert_eq!(settings::load_flag(&dir, crate::bridge::FLAG), Some(false));
    assert!(!dir.join(crate::bridge::DISCOVERY_FILE).exists());
    menus::run(&mut s.shell.app, &ctx, Action::ToggleAgentBridge);
    assert!(s.app().bridge.bridge.is_some());
    menus::run(&mut s.shell.app, &ctx, Action::ToggleAgentBridge);
    assert!(s.app().bridge.bridge.is_none());
    assert_eq!(settings::load_flag(&dir, crate::bridge::FLAG), Some(false));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn bridge_answers_while_the_window_is_hidden() {
    let mut s = Sim::new();
    let ctx = s.ctx.clone();
    s.shell.app.start_bridge(&ctx).unwrap();
    let b = s.app().bridge.bridge.as_ref().unwrap();
    let (port, token) = (b.port(), b.token().to_string());
    let (send, receive) = std::sync::mpsc::channel();
    std::thread::spawn(move || send.send(call(port, &token, "desigma_get_document", json!({}))).unwrap());
    // a minimized window gets `logic` only, never `ui`
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);
    let (status, body) = loop {
        eframe::App::logic(&mut s.shell, &ctx, &mut eframe::Frame::_new_kittest());
        if let Ok(v) = receive.try_recv() {
            break v;
        }
        assert!(std::time::Instant::now() < deadline, "hidden editor never answered");
        std::thread::sleep(std::time::Duration::from_millis(5));
    };
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["result"]["session"], "editor");
}

#[test]
fn bridge_edits_are_laid_out_and_selection_follows_the_page() {
    let mut s = Sim::new();
    let ctx = s.ctx.clone();
    s.shell.app.start_bridge(&ctx).unwrap();
    // the demo's vertical auto-layout card (padding 20) places a new child; created at 0,0
    let card = s.by_name("Card");
    let h = s.app().doc.get(card).unwrap().size.y;
    let (status, body) = send(&mut s, "desigma_create_node", json!({"parent_id":card.0,"kind":"rectangle","width":50,"height":30}));
    assert_eq!(status, 200, "{body}");
    let child = NodeId(body["result"]["node"]["id"].as_u64().unwrap());
    let t = s.app().doc.get(child).unwrap().transform;
    assert!((t.tx - 20.0).abs() < 1e-3 && t.ty > 20.0, "child not laid out: {t:?}");
    assert!(s.app().doc.get(card).unwrap().size.y > h + 30.0, "hug card did not grow");
    // removing it shrinks the card back
    assert_eq!(send(&mut s, "desigma_delete_node", json!({"node_id":child.0})).0, 200);
    assert!((s.app().doc.get(card).unwrap().size.y - h).abs() < 1e-3);

    let first = s.app().page;
    let second = s.shell.app.doc.add_page("Second");
    let (_, body) = send(&mut s, "desigma_create_node", json!({"parent_id":second.0,"kind":"rectangle"}));
    let other = NodeId(body["result"]["node"]["id"].as_u64().unwrap());
    let (status, body) = send(&mut s, "desigma_select_nodes", json!({"node_ids":[other.0]}));
    assert_eq!(status, 200, "{body}");
    assert_eq!((s.app().page, s.app().selection.clone()), (second, vec![other]));
    let (status, body) = send(&mut s, "desigma_select_nodes", json!({"node_ids":[card.0, other.0]}));
    assert_eq!(status, 422);
    assert!(body["error"].as_str().unwrap().contains("one page"), "{body}");
    assert_eq!(send(&mut s, "desigma_select_nodes", json!({"node_ids":[]})).0, 422);
    assert_eq!(s.app().selection, vec![other], "a rejected call keeps the selection");
    let (status, _) = send(&mut s, "desigma_select_nodes", json!({"node_ids":[card.0, card.0],"reveal":false}));
    assert_eq!(status, 200);
    assert_eq!((s.app().page, s.app().selection.clone()), (first, vec![card]));
}

#[test]
fn bridge_file_tools_act_on_the_editor_document() {
    let dir = std::env::temp_dir().join(format!("desigma_bridge_files_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = Sim::new();
    let ctx = s.ctx.clone();
    s.shell.app.start_bridge(&ctx).unwrap();
    let page = s.app().page.0;
    let (_, body) = send(&mut s, "desigma_create_node", json!({"parent_id":page,"kind":"rectangle","width":40,"height":20}));
    let id = body["result"]["node"]["id"].as_u64().unwrap();

    let png = dir.join("shot.png");
    let (status, body) = send(&mut s, "desigma_export_image", json!({"node_id":id,"path":png}));
    assert_eq!(status, 200, "{body}");
    assert!(std::fs::read(&png).unwrap().starts_with(b"\x89PNG"));

    // save writes a copy: the editor keeps its own path, and nothing is overwritten
    let path = s.app().file_path.clone();
    let copy = dir.join("copy.desigma");
    let (status, body) = send(&mut s, "desigma_save_file", json!({"path":copy}));
    assert_eq!(status, 200, "{body}");
    assert!(copy.is_file());
    assert_eq!(s.app().file_path, path);
    assert_eq!(send(&mut s, "desigma_save_file", json!({"path":copy})).0, 422);

    // the user's undo stack guards open and new
    assert!(s.app().hist.can_undo());
    let (status, body) = send(&mut s, "desigma_open_file", json!({"path":copy}));
    assert_eq!(status, 422);
    assert!(body["error"].as_str().unwrap().contains("ask the user to save"), "{body}");
    let (status, body) = send(&mut s, "desigma_open_file", json!({"path":copy,"discard_changes":true}));
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["result"]["session"], "editor");
    assert_eq!(body["result"]["path"], json!(copy));
    assert_eq!(body["result"]["has_undo"], false);
    assert!(s.app().doc.get(NodeId(id)).is_some());

    assert_eq!(send(&mut s, "desigma_new_document", json!({"name":" "})).0, 422);
    assert_eq!(send(&mut s, "desigma_new_document", json!({"name":"x".repeat(1025)})).0, 422);
    let (status, body) = send(&mut s, "desigma_new_document", json!({"name":"Fresh"}));
    assert_eq!(status, 200, "{body}");
    assert_eq!((&body["result"]["session"], &body["result"]["name"]), (&json!("editor"), &json!("Fresh")));
    assert_eq!(s.app().doc.name, "Fresh");

    // the home screen has no editable document
    s.shell.app.home = Some(crate::home::Home::new());
    let err = s.shell.app.bridge_call("desigma_get_document", json!({})).unwrap_err();
    assert!(err.contains("busy"), "{err}");
    s.shell.app.home = None;
    let _ = std::fs::remove_dir_all(&dir);
}

/// The nine tools added after the bridge's busy-gate was written must be
/// refused during an active gesture exactly like the originals.
#[test]
fn bridge_refuses_the_newer_mutating_tools_while_a_gesture_runs() {
    let mut s = Sim::new();
    let ctx = s.ctx.clone();
    s.shell.app.start_bridge(&ctx).unwrap();
    let (port, token) = {
        let b = s.app().bridge.bridge.as_ref().unwrap();
        (b.port(), b.token().to_string())
    };
    // hold the middle button over the canvas: a real pan stays active while the requests arrive
    let p = s.app().canvas_rect.center();
    s.mv(p);
    s.frame(vec![Event::PointerButton { pos: p, button: PointerButton::Middle, pressed: true, modifiers: Modifiers::NONE }]);
    assert!(matches!(s.app().canvas.inter, crate::canvas::Interaction::Pan));
    let before = s.app().doc.nodes.len();
    for tool in [
        "desigma_create_page",
        "desigma_move_node",
        "desigma_set_appearance",
        "desigma_set_text_style",
        "desigma_import_svg",
        "desigma_delete_page",
        "desigma_add_keyframe",
        "desigma_apply_animation_preset",
        "desigma_set_timeline",
    ] {
        let t = token.clone();
        let (status, body) = with_frames(&mut s, move || call(port, &t, tool, json!({})));
        assert_eq!(status, 422, "{tool}: {body}");
        assert!(body["error"].as_str().unwrap().contains("busy"), "{tool}: {body}");
    }
    assert_eq!(s.app().doc.nodes.len(), before);
}
