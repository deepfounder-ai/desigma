#![cfg(feature = "server")]
use desigma_mcp::server::Session;
use serde_json::json;
use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    thread,
};

/// `SystemTime` alone can read back the same value across threads (its
/// resolution is coarser than a nanosecond on some platforms); this counter
/// guarantees distinct names for concurrently-running tests regardless.
fn unique_suffix() -> u128 {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    nanos * 1_000_000 + u128::from(COUNTER.fetch_add(1, Ordering::Relaxed))
}

/// Writes a discovery file pointing at a fake editor on `port`, returns its path.
fn write_discovery(port: u16, token: &str) -> PathBuf {
    let file = std::env::temp_dir().join(format!("desigma-mcp-bridge-{}-{}.json", std::process::id(), unique_suffix()));
    fs::write(&file, format!(r#"{{"version":1,"port":{port},"token":"{token}","pid":1,"document":"D","path":null}}"#)).unwrap();
    file
}

struct Workspace(PathBuf);
impl Workspace {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!("desigma-mcp-{}-{}", std::process::id(), unique_suffix()));
        fs::create_dir(&p).unwrap();
        Self(p)
    }
}
impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn saved_file_roundtrip_and_unsaved_changes_guard() {
    let dir = Workspace::new();
    let mut s = Session::with_discovery(&dir.0, None).unwrap();
    let d = s.call("desigma_get_document", json!({})).unwrap();
    let page = d["pages"][0]["node_id"].clone();
    let node = s.call("desigma_create_node", json!({"parent_id":page,"kind":"text","name":"Heading","text":"Hello MCP"})).unwrap();
    assert!(s.call("desigma_new_document", json!({"name":"Oops"})).is_err());
    s.call("desigma_save_file", json!({"path":"first.desigma"})).unwrap();
    s.call("desigma_update_node", json!({"node_id":node["node"]["id"],"text":"Changed"})).unwrap();
    assert!(s.call("desigma_open_file", json!({"path":"first.desigma"})).is_err());
    s.call("desigma_open_file", json!({"path":"first.desigma","discard_changes":true})).unwrap();
    assert_eq!(s.call("desigma_get_node", json!({"node_id":node["node"]["id"]})).unwrap()["node"], node["node"]);
    let original = fs::read(dir.0.join("first.desigma")).unwrap();
    assert!(s.call("desigma_save_file", json!({"path":"first.desigma"})).is_err());
    assert_eq!(fs::read(dir.0.join("first.desigma")).unwrap(), original);
}

#[test]
fn save_can_overwrite_the_file_it_opened_when_asked() {
    let dir = Workspace::new();
    let mut s = Session::with_discovery(&dir.0, None).unwrap();
    let page = s.call("desigma_get_document", json!({})).unwrap()["pages"][0]["node_id"].clone();
    s.call("desigma_create_node", json!({"parent_id":page,"kind":"text","name":"T","text":"one"})).unwrap();
    s.call("desigma_save_file", json!({"path":"iter.desigma"})).unwrap();
    s.call("desigma_open_file", json!({"path":"iter.desigma"})).unwrap();
    let doc = s.call("desigma_get_document", json!({})).unwrap();
    assert_eq!(doc["path"], "iter.desigma", "the session reports the file it came from: {doc}");
    s.call("desigma_create_node", json!({"parent_id":page,"kind":"rectangle","name":"R"})).unwrap();
    // without the flag the existing file is kept
    assert!(s.call("desigma_save_file", json!({"path":"iter.desigma"})).unwrap_err().contains("overwrite=true"));
    s.call("desigma_save_file", json!({"path":"iter.desigma","overwrite":true})).unwrap();
    let saved = fs::read_to_string(dir.0.join("iter.desigma")).unwrap();
    assert!(saved.contains("\"R\""), "the overwrite holds the new layer");
    let leftovers: Vec<_> = fs::read_dir(&dir.0).unwrap().flatten().map(|e| e.file_name()).filter(|n| n.to_string_lossy().ends_with(".tmp")).collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
    // overwriting never follows a symlink out of the workspace
    #[cfg(unix)]
    {
        let outside = Workspace::new();
        let target = outside.0.join("victim.desigma");
        fs::write(&target, "keep").unwrap();
        std::os::unix::fs::symlink(&target, dir.0.join("link.desigma")).unwrap();
        assert!(s.call("desigma_save_file", json!({"path":"link.desigma","overwrite":true})).is_err());
        assert_eq!(fs::read_to_string(&target).unwrap(), "keep");
    }
}

#[test]
fn paths_and_bad_imports_cannot_replace_the_session() {
    let dir = Workspace::new();
    let outside = Workspace::new();
    fs::write(outside.0.join("outside.desigma"), "{}").unwrap();
    let mut s = Session::with_discovery(&dir.0, None).unwrap();
    let before = s.call("desigma_get_document", json!({})).unwrap();
    assert!(s.call("desigma_open_file", json!({"path":outside.0.join("outside.desigma")})).is_err());
    assert!(s.call("desigma_save_file", json!({"path":outside.0.join("escape.desigma")})).is_err());
    assert!(s.call("desigma_save_file", json!({"path":"../escape.desigma"})).is_err());
    assert!(s.call("desigma_save_file", json!({"path":"bad.json"})).is_err());
    fs::write(dir.0.join("bad.desigma"), "not JSON").unwrap();
    assert!(s.call("desigma_open_file", json!({"path":"bad.desigma"})).is_err());
    let mut broken = s.engine.doc.clone();
    let id = broken.pages[0];
    broken.get_mut(id).unwrap().parent = Some(id);
    fs::write(dir.0.join("cycle.desigma"), broken.to_json().unwrap()).unwrap();
    assert!(s.call("desigma_open_file", json!({"path":"cycle.desigma"})).is_err());
    assert_eq!(s.call("desigma_get_document", json!({})).unwrap(), before);
    assert!(!outside.0.join("escape.desigma").exists());
}

#[test]
fn imported_stale_id_counter_does_not_overwrite_existing_nodes() {
    let dir = Workspace::new();
    let mut s = Session::with_discovery(&dir.0, None).unwrap();
    let mut native = serde_json::to_value(&s.engine.doc).unwrap();
    native["next_id"] = json!(0);
    fs::write(dir.0.join("stale.desigma"), native.to_string()).unwrap();
    let doc = s.call("desigma_open_file", json!({"path":"stale.desigma"})).unwrap();
    let page = doc["pages"][0]["node_id"].clone();
    let created = s.call("desigma_create_node", json!({"parent_id":page,"kind":"rectangle"})).unwrap();
    assert_ne!(created["node"]["id"], page);
    assert_eq!(s.call("desigma_get_document", json!({})).unwrap()["node_count"], 2);
    s.call("desigma_delete_node", json!({"node_id":created["node"]["id"]})).unwrap();
    let next = s.call("desigma_create_node", json!({"parent_id":page,"kind":"rectangle"})).unwrap();
    assert!(next["node"]["id"].as_u64().unwrap() > created["node"]["id"].as_u64().unwrap());

    let mut exhausted = serde_json::to_value(&s.engine.doc).unwrap();
    exhausted["next_id"] = json!(u64::MAX);
    fs::write(dir.0.join("exhausted.desigma"), exhausted.to_string()).unwrap();
    s.call("desigma_open_file", json!({"path":"exhausted.desigma","discard_changes":true})).unwrap();
    let before = s.engine.doc.to_json().unwrap();
    assert!(s.call("desigma_create_node", json!({"parent_id":page,"kind":"rectangle"})).unwrap_err().contains("exhausted"));
    assert_eq!(s.engine.doc.to_json().unwrap(), before);
}

#[test]
fn export_image_writes_png_inside_workspace_only() {
    let dir = Workspace::new();
    let mut s = Session::with_discovery(&dir.0, None).unwrap();
    let page = s.call("desigma_get_document", json!({})).unwrap()["pages"][0]["node_id"].clone();
    let id = s.call("desigma_create_node", json!({"parent_id":page,"kind":"rectangle","width":40,"height":20})).unwrap()["node"]["id"].clone();
    let out = s.call("desigma_export_image", json!({"node_id":id,"scale":2,"path":"shot.png"})).unwrap();
    assert_eq!((out["width"].as_u64().unwrap(), out["height"].as_u64().unwrap()), (80, 40));
    let bytes = fs::read(dir.0.join("shot.png")).unwrap();
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
    // overwrite of a .png is allowed; anything else is not
    s.call("desigma_export_image", json!({"node_id":id,"path":"shot.png"})).unwrap();
    assert!(s.call("desigma_export_image", json!({"node_id":id,"path":"shot.jpg"})).is_err());
    assert!(s.call("desigma_export_image", json!({"node_id":id,"path":"../shot.png"})).is_err());
    assert!(s.call("desigma_export_image", json!({"node_id":id,"path":"missing/shot.png"})).is_err());
}

/// Overwrite is allowed for a real .png this session already wrote, but a
/// symlink at the leaf must never be followed: that would let a workspace
/// file silently overwrite something outside it.
#[cfg(unix)]
#[test]
fn export_image_refuses_to_write_through_a_symlink() {
    let dir = Workspace::new();
    let outside = Workspace::new();
    let target = outside.0.join("outside.png");
    fs::write(&target, b"untouched").unwrap();
    std::os::unix::fs::symlink(&target, dir.0.join("link.png")).unwrap();
    let mut s = Session::with_discovery(&dir.0, None).unwrap();
    let page = s.call("desigma_get_document", json!({})).unwrap()["pages"][0]["node_id"].clone();
    let id = s.call("desigma_create_node", json!({"parent_id":page,"kind":"rectangle"})).unwrap()["node"]["id"].clone();
    assert!(s.call("desigma_export_image", json!({"node_id":id,"path":"link.png"})).is_err());
    assert_eq!(fs::read(&target).unwrap(), b"untouched");
}

/// Discovery is per-`Session` (`with_discovery`), not a global env var, so
/// this test runs like any other: no shared state, safe in parallel, and a
/// real developer's editor is never at risk (`Session::new`/the default
/// `DESIGMA_BRIDGE_FILE` path are simply not used here).
#[test]
fn session_routes_to_the_editor_when_its_bridge_answers() {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    thread::spawn(move || {
        for s in l.incoming() {
            let mut s = s.unwrap();
            let mut buf = vec![0; 8192];
            let n = s.read(&mut buf).unwrap();
            let req = String::from_utf8_lossy(&buf[..n]).to_string();
            let body = if req.starts_with("GET /tok/ping") {
                r#"{"ok":true,"document":"D","path":null}"#.to_string()
            } else {
                r#"{"result":{"echo":true}}"#.to_string()
            };
            write!(s, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
        }
    });
    let file = write_discovery(port, "tok");

    let dir = Workspace::new();
    let mut s = Session::with_discovery(&dir.0, Some(file.clone())).unwrap();
    let v = s.call("desigma_get_document", json!({})).unwrap();
    assert_eq!(v["echo"], true); // answered by the fake editor, not the local engine
    assert!(s.call("desigma_export_image", json!({"node_id":1,"path":"../x.png"})).is_err()); // workspace rule still applies

    let _ = fs::remove_file(&file);
}

const TIMEOUT: &str = "Editor did not answer within 5 s; the call may still have been applied — read the document before retrying";

/// After a successful ping, a failed call must surface as an error, never a
/// silent fallback to the local file session.
#[test]
fn live_call_failure_after_successful_ping_is_an_error_not_a_fallback() {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    thread::spawn(move || {
        for s in l.incoming() {
            let mut s = s.unwrap();
            let mut buf = vec![0; 8192];
            let n = s.read(&mut buf).unwrap();
            let req = String::from_utf8_lossy(&buf[..n]).to_string();
            let (status, body) = if req.starts_with("GET /tok/ping") {
                ("200 OK", r#"{"ok":true,"document":"D","path":null}"#.to_string())
            } else if req.contains("desigma_get_node") {
                ("422 Unprocessable Entity", r#"{"error":"node 7 not found"}"#.to_string())
            } else if req.contains("desigma_delete_node") {
                ("503 Service Unavailable", format!(r#"{{"error":"{TIMEOUT}"}}"#))
            } else {
                ("503 Service Unavailable", r#"{"error":"editor busy"}"#.to_string())
            };
            write!(s, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
        }
    });
    let file = write_discovery(port, "tok");

    let dir = Workspace::new();
    let mut s = Session::with_discovery(&dir.0, Some(file.clone())).unwrap();
    let before = s.engine.doc.nodes.len();

    let get_node_err = s.call("desigma_get_node", json!({"node_id":7})).unwrap_err();
    assert!(get_node_err.contains("node 7 not found"), "{get_node_err}");

    let create_err = s.call("desigma_create_node", json!({"parent_id":1,"kind":"rectangle"})).unwrap_err();
    assert!(create_err.contains("Editor did not answer"), "{create_err}");
    assert_eq!(s.engine.doc.nodes.len(), before, "a failed live call must not fall back to a local edit");
    // the editor's own 503 message reaches the agent unchanged, not as "Editor did not answer: Editor did not answer…"
    assert_eq!(s.call("desigma_delete_node", json!({"node_id":7})).unwrap_err(), TIMEOUT);

    let _ = fs::remove_file(&file);
}

/// A fake editor that answers the ping and every call with `{"session":"editor"}`.
fn fake_editor() -> u16 {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    thread::spawn(move || {
        for s in l.incoming() {
            let mut s = s.unwrap();
            let mut buf = vec![0; 8192];
            let n = s.read(&mut buf).unwrap();
            let body = if buf[..n].starts_with(b"GET /tok/ping") { r#"{"ok":true}"# } else { r#"{"result":{"session":"editor"}}"# };
            write!(s, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
        }
    });
    port
}

#[test]
fn editor_session_whose_bridge_goes_away_errors_instead_of_falling_back() {
    let file = write_discovery(fake_editor(), "tok");
    let dir = Workspace::new();
    let mut s = Session::with_discovery(&dir.0, Some(file.clone())).unwrap();
    assert_eq!(s.call("desigma_get_document", json!({})).unwrap()["session"], "editor");
    fs::remove_file(&file).unwrap();
    let before = s.engine.doc.nodes.len();
    let err = s.call("desigma_create_node", json!({"parent_id":1,"kind":"rectangle"})).unwrap_err();
    assert!(err.starts_with("Editor bridge is off"), "{err}");
    assert!(s.call("desigma_get_document", json!({})).is_err(), "still latched to the editor");
    assert_eq!(s.engine.doc.nodes.len(), before, "the local file session must stay untouched");
}

#[test]
fn file_session_switches_to_a_late_editor_only_without_unsaved_edits() {
    let file = write_discovery(fake_editor(), "tok");
    let discovery = fs::read(&file).unwrap();
    fs::remove_file(&file).unwrap();
    let dir = Workspace::new();
    let mut clean = Session::with_discovery(&dir.0, Some(file.clone())).unwrap();
    let mut dirty = Session::with_discovery(&dir.0, Some(file.clone())).unwrap();
    let d = clean.call("desigma_get_document", json!({})).unwrap();
    assert_eq!(d["session"], "file");
    for tool in ["desigma_get_selection", "desigma_select_nodes"] {
        let err = clean.call(tool, json!({"node_ids":[1]})).unwrap_err();
        assert!(err.starts_with("No editor connected"), "{tool}: {err}");
    }
    let page = dirty.call("desigma_get_document", json!({})).unwrap()["pages"][0]["node_id"].clone();
    dirty.call("desigma_create_node", json!({"parent_id":page,"kind":"rectangle"})).unwrap();

    fs::write(&file, discovery).unwrap();
    let err = dirty.call("desigma_get_document", json!({})).unwrap_err();
    assert!(err.starts_with("An editor appeared"), "{err}");
    assert_eq!(clean.call("desigma_get_document", json!({})).unwrap()["session"], "editor");

    dirty.call("desigma_save_file", json!({"path":"saved.desigma"})).unwrap();
    assert_eq!(dirty.call("desigma_get_document", json!({})).unwrap()["session"], "editor");

    let _ = fs::remove_file(&file);
}

#[test]
fn list_files_shows_library_designs_and_outside_copies() {
    let dir = Workspace::new();
    let lib = desigma_library::Library::open(&dir.0).unwrap();
    let project = lib.create_project("Inbox").unwrap();
    let mut doc = desigma_doc::Document::new("Poster");
    doc.add_page("Page 1");
    lib.create_file(&project.id, None, "Poster", &doc).unwrap();
    let shared = dir.0.join(desigma_library::SHARED_DIR);
    fs::create_dir(&shared).unwrap();
    fs::write(shared.join("From Downloads.desigma"), doc.to_json().unwrap()).unwrap();

    let mut session = Session::with_discovery(&dir.0, None).unwrap();
    let listed = session.call("desigma_list_files", json!({})).unwrap();
    let files = listed["files"].as_array().unwrap();
    assert!(files.iter().any(|f| f["name"] == "Poster" && f["project"] == "Inbox"), "{listed}");
    let outside = files.iter().find(|f| f["name"] == "From Downloads").unwrap();
    assert_eq!(outside["path"], "shared/From Downloads.desigma");
    session.call("desigma_open_file", json!({"path": outside["path"]})).unwrap();
    assert_eq!(session.call("desigma_get_document", json!({})).unwrap()["name"], "Poster");
}
