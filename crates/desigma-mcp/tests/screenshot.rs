use desigma_mcp::server::Session;
use serde_json::json;

#[test]
fn screenshot_writes_a_png_under_the_workspace_by_default() {
    let dir = std::env::temp_dir().join(format!("desigma-shot-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = Session::with_discovery(&dir, None).unwrap();
    let page = s.call("desigma_get_document", json!({})).unwrap()["pages"][0]["node_id"].as_u64().unwrap();
    let r =
        s.call("desigma_apply_nodes", json!({"parent_id":page,"nodes":[{"kind":"rectangle","name":"Box","width":40,"height":30,"fill":"#ff0000"}]})).unwrap();
    let id = r["nodes"][0]["node_id"].as_u64().unwrap();
    let shot = s.call("desigma_screenshot", json!({"node_id":id,"scale":2})).unwrap();
    assert_eq!((shot["width"].as_u64(), shot["height"].as_u64()), (Some(80), Some(60)));
    let path = std::path::PathBuf::from(shot["path"].as_str().unwrap());
    assert!(path.starts_with(dir.canonicalize().unwrap().join(".desigma-screenshots")), "{path:?}");
    assert_eq!(&std::fs::read(&path).unwrap()[1..4], b"PNG");
    let again = s.call("desigma_screenshot", json!({"node_id":id})).unwrap();
    assert_ne!(again["path"], shot["path"], "names rotate");
    assert!(s.call("desigma_screenshot", json!({"node_id":id,"scale":9})).is_err());
    assert!(s.call("desigma_screenshot", json!({"node_id":id,"path":"../out.png"})).is_err());
    std::fs::remove_dir_all(&dir).ok();
}
