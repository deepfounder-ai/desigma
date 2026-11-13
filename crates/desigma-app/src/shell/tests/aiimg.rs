//! AI images end to end against a local stand-in for OpenRouter: the settings dialog, generating,
//! editing a region, and a failing request (see `aiimg`).

use super::*;
use base64::Engine;
use desigma_doc::PaintKind;
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};

type Seen = Arc<Mutex<Vec<(String, String, Value)>>>;

fn png(w: u32, h: u32, c: [u8; 4]) -> Vec<u8> {
    crate::aiimg::compose::encode_png(&image::RgbaImage::from_pixel(w, h, image::Rgba(c))).unwrap()
}

/// A tiny HTTP server on the loopback interface. It records `(path, authorization, body)` of
/// every request and answers with what `answer` gives for the path: `(status, body)`.
fn server(answer: impl Fn(&str) -> (u16, Value) + Send + 'static) -> (String, Seen) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let seen: Seen = Default::default();
    let log = seen.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let mut buf = Vec::new();
            let mut chunk = [0u8; 8192];
            let (head_end, length) = loop {
                let Ok(n) = stream.read(&mut chunk) else { break (0, 0) };
                if n == 0 {
                    break (0, 0);
                }
                buf.extend_from_slice(&chunk[..n]);
                if let Some(at) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&buf[..at]).to_lowercase();
                    let length = head.lines().find_map(|l| l.strip_prefix("content-length:")).and_then(|v| v.trim().parse::<usize>().ok()).unwrap_or(0);
                    break (at + 4, length);
                }
            };
            if head_end == 0 {
                continue;
            }
            while buf.len() < head_end + length {
                let Ok(n) = stream.read(&mut chunk) else { break };
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&chunk[..n]);
            }
            let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
            let path = head.split_whitespace().nth(1).unwrap_or("").to_string();
            let auth = head.lines().find_map(|l| l.to_lowercase().strip_prefix("authorization:").map(|v| v.trim().to_string())).unwrap_or_default();
            let body = serde_json::from_slice(&buf[head_end..]).unwrap_or(Value::Null);
            log.lock().unwrap().push((path.clone(), auth, body));
            let (status, reply) = answer(&path);
            let reply = reply.to_string();
            let _ =
                write!(stream, "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}", reply.len());
        }
    });
    (base, seen)
}

fn image_answer(bytes: &[u8]) -> Value {
    json!({"data":[{"b64_json": base64::engine::general_purpose::STANDARD.encode(bytes), "media_type":"image/png"}], "usage":{"cost":0.04}})
}

/// Frames until `done`, while the request thread works.
fn wait(s: &mut Sim, what: &str, done: impl Fn(&Sim) -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !done(s) {
        assert!(std::time::Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(std::time::Duration::from_millis(5));
        s.frame(vec![]);
    }
}

fn ready(s: &mut Sim, base: &str) {
    s.shell.app.ai.base = base.to_string();
    s.shell.app.ai.model = "test/model".into();
    crate::aiimg::set_test_key("sk-test");
}

fn image_ref(s: &Sim, id: NodeId) -> String {
    match &s.app().doc.get(id).unwrap().fills[0].kind {
        PaintKind::Image { image_ref, .. } => image_ref.clone(),
        other => panic!("not an image fill: {other:?}"),
    }
}

fn pixels(s: &Sim, id: NodeId) -> image::RgbaImage {
    crate::aiimg::compose::decode(&s.app().doc.images[&image_ref(s, id)]).unwrap()
}

#[test]
fn settings_store_the_key_and_the_model_picked_from_openrouters_list() {
    let (base, seen) = server(|_| {
        (
            200,
            json!({"data":[
                {"id":"acme/edit-pro","name":"Acme Edit Pro","architecture":{"input_modalities":["text","image"],"output_modalities":["image"]}},
                {"id":"acme/text-only","name":"Acme Draw","architecture":{"input_modalities":["text"],"output_modalities":["image"]}},
            ]}),
        )
    });
    let mut s = Sim::new();
    s.shell.app.ai.base = base;
    let ctx = s.ctx.clone();
    // generating without a key leads to the settings, saying why
    s.shell.app.open_ai_generate(&ctx);
    s.frame(vec![]);
    assert!(s.app().ai.prompt.is_none());
    assert!(s.app().ai.settings.as_ref().unwrap().error.as_ref().unwrap().contains("key"));

    s.frame(vec![]);
    s.type_text("sk-or-secret");
    s.click(s.rect_of("ai-load").center());
    wait(&mut s, "the model list", |s| matches!(s.app().ai.settings.as_ref().unwrap().models, crate::aiimg::Models::Loaded(_)));
    assert_eq!(seen.lock().unwrap()[0].0, "/images/models");
    assert_eq!(seen.lock().unwrap()[0].1, "bearer sk-or-secret", "the typed key is used for the listing");
    s.frame(vec![]);
    // only the model that takes reference images is offered
    s.click(s.rect_of("ai-model-row-0").center());
    assert_eq!(s.app().ai.settings.as_ref().unwrap().model, "acme/edit-pro");
    s.click(s.rect_of("ai-save").center());
    assert!(s.app().ai.settings.is_none());
    assert_eq!(s.app().ai.model, "acme/edit-pro");
    assert!(s.app().ai_key_saved());

    // the document and its JSON never hold the key
    assert!(!s.app().doc.to_json().unwrap().contains("sk-or-secret"));
}

#[test]
fn generating_places_an_image_and_one_undo_removes_it() {
    let (base, seen) = server(|_| (200, image_answer(&png(64, 32, [200, 30, 30, 255]))));
    let mut s = Sim::new();
    ready(&mut s, &base);
    let ctx = s.ctx.clone();
    let before = s.count();
    s.shell.app.selection.clear();
    s.shell.app.open_ai_generate(&ctx);
    s.frame(vec![]);
    s.frame(vec![]);
    s.type_text("a red banner");
    s.click(s.rect_of("ai-aspect-16:9").center());
    s.click(s.rect_of("ai-generate").center());
    assert!(s.app().ai.prompt.as_ref().unwrap().running);
    wait(&mut s, "the image", |s| s.app().ai.prompt.is_none());

    let (path, auth, body) = seen.lock().unwrap()[0].clone();
    assert_eq!((path.as_str(), auth.as_str()), ("/images", "bearer sk-test"));
    assert_eq!(body["model"], "test/model");
    assert_eq!(body["prompt"], "a red banner");
    assert_eq!(body["aspect_ratio"], "16:9");
    assert!(body.get("input_references").is_none(), "nothing was selected");

    assert_eq!(s.count(), before + 1);
    let id = s.app().selection[0];
    assert_eq!(pixels(&s, id).get_pixel(5, 5).0, [200, 30, 30, 255]);
    assert_eq!(s.app().doc.get(id).unwrap().ai_versions[0].prompt, "a red banner", "a generated layer starts its history");
    s.cmd_key(Key::Z);
    s.cmd_key(Key::Z);
    assert_eq!(s.count(), before, "undo removes the generated layer");
}

#[test]
fn selected_layers_travel_as_references_and_an_empty_shape_is_filled() {
    let (base, seen) = server(|_| (200, image_answer(&png(40, 40, [0, 120, 0, 255]))));
    let mut s = Sim::new();
    ready(&mut s, &base);
    let ctx = s.ctx.clone();
    let page = s.app().page;
    let add = |s: &mut Sim, kids: bool| {
        let id = s.shell.app.doc.alloc_id();
        let id = s.shell.app.doc.insert(page, desigma_doc::Node::frame(id, desigma_doc::Rect::new(0.0, 0.0, 300.0, 100.0))).unwrap();
        if kids {
            let k = s.shell.app.doc.alloc_id();
            s.shell.app.doc.insert(id, desigma_doc::Node::rectangle(k, desigma_doc::Rect::new(10.0, 10.0, 50.0, 50.0))).unwrap();
        }
        id
    };
    let (with_content, empty) = (add(&mut s, true), add(&mut s, false));

    s.shell.app.selection = vec![with_content];
    s.shell.app.open_ai_generate(&ctx);
    assert_eq!(s.app().ai.prompt.as_ref().unwrap().references.len(), 1);
    s.frame(vec![]);
    s.frame(vec![]);
    s.type_text("more like this");
    s.click(s.rect_of("ai-generate").center());
    wait(&mut s, "the image", |s| s.app().ai.prompt.is_none());
    let url = seen.lock().unwrap()[0].2["input_references"][0]["image_url"]["url"].as_str().unwrap().to_string();
    assert!(url.starts_with("data:image/png;base64,"), "{}", &url[..40.min(url.len())]);

    s.shell.app.selection = vec![empty];
    s.shell.app.open_ai_generate(&ctx);
    let p = s.app().ai.prompt.as_ref().unwrap();
    assert!(matches!(p.target, crate::aiimg::Target::Fill(id) if id == empty));
    assert_eq!(crate::aiimg::compose::ASPECTS[p.aspect], "16:9", "3:1 snaps to the widest ratio offered");
    s.frame(vec![]);
    s.frame(vec![]);
    s.type_text("a green field");
    s.click(s.rect_of("ai-generate").center());
    wait(&mut s, "the fill", |s| s.app().ai.prompt.is_none());
    assert_eq!(pixels(&s, empty).get_pixel(1, 1).0, [0, 120, 0, 255]);
    assert_eq!(s.app().doc.get(empty).unwrap().size.x, 300.0, "the shape keeps its size");
}

#[test]
fn editing_changes_only_what_the_brush_marked_and_keeps_a_history() {
    // the model answers with an all-white picture: only the chosen part may turn white
    let (base, seen) = server(|_| (200, image_answer(&png(100, 50, [255, 255, 255, 255]))));
    let mut s = Sim::new();
    ready(&mut s, &base);
    let ctx = s.ctx.clone();
    let at = s.app().view.to_page(s.app().canvas_rect.min, s.app().canvas_rect.center());
    let id = crate::canvas::interact::place_image_bytes(&mut s.shell.app, at, "photo", png(200, 100, [20, 20, 20, 255])).unwrap();
    let original = image_ref(&s, id);

    s.shell.app.selection = vec![id];
    s.shell.app.open_ai_edit(&ctx);
    s.frame(vec![]);
    s.frame(vec![]);
    let r = s.rect_of("ai-region");
    let at = |fx: f32, fy: f32| Pos2::new(r.left() + r.width() * fx, r.top() + r.height() * fy);
    s.click(s.rect_of("ai-brush-2").center());
    assert_eq!(s.app().ai.prompt.as_ref().unwrap().brush, 2);
    s.click(s.rect_of("ai-brush-1").center());
    // one stroke along the middle of the right half
    s.drag(at(0.6, 0.5), at(0.9, 0.5));
    let region = s.app().ai.prompt.as_ref().unwrap().region().expect("the stroke marks a part");
    // the preview is 200 px wide and the brush 22 px: the mark reaches 0.11 beyond the stroke
    assert!(region.x0 > 0.45 && region.x0 < 0.55 && region.x1 > 0.9 && region.y0 > 0.2 && region.y1 < 0.8, "{region:?}");
    // Clear wipes the mark; paint it again
    s.click(s.rect_of("ai-clear").center());
    assert!(s.app().ai.prompt.as_ref().unwrap().region().is_none());
    s.drag(at(0.6, 0.5), at(0.9, 0.5));
    s.click(s.rect_of("ai-prompt-text").center());
    s.type_text("make it snow");
    s.click(s.rect_of("ai-generate").center());
    // while it runs the dialog steps aside for a small bar with Cancel
    assert!(s.app().ai.prompt.as_ref().unwrap().running);
    wait(&mut s, "the edit", |s| s.app().ai.prompt.is_none());

    let body = seen.lock().unwrap()[0].2.clone();
    let prompt = body["prompt"].as_str().unwrap();
    assert!(prompt.starts_with("make it snow") && prompt.contains("of the width"), "{prompt}");
    assert_eq!(body["aspect_ratio"], "16:9");
    assert!(body["input_references"][0]["image_url"]["url"].as_str().unwrap().starts_with("data:image/png;base64,"));

    let out = pixels(&s, id);
    assert_eq!(out.dimensions(), (200, 100), "the picture keeps its size");
    for (x, y) in [(20, 50), (150, 10), (150, 90)] {
        assert_eq!(out.get_pixel(x, y).0, [20, 20, 20, 255], "outside the stroke at {x},{y}");
    }
    assert_eq!(out.get_pixel(150, 50).0, [255, 255, 255, 255], "the middle of the stroke is the model's");
    let generated = image_ref(&s, id);
    assert_ne!(generated, original);

    // the layer remembers both pictures
    let history: Vec<(String, String)> = s.app().doc.get(id).unwrap().ai_versions.iter().map(|v| (v.image_ref.clone(), v.prompt.clone())).collect();
    assert_eq!(history, [(original.clone(), String::new()), (generated.clone(), "make it snow".to_string())]);

    // the dialog shows them; a click on the first brings the original back and keeps the history
    s.shell.app.selection = vec![id];
    s.shell.app.open_ai_edit(&ctx);
    s.frame(vec![]);
    s.frame(vec![]);
    let shown: Vec<bool> = s.app().ai.prompt.as_ref().unwrap().versions.iter().map(|v| v.current).collect();
    assert_eq!(shown, [false, true]);
    s.click(s.rect_of("ai-version-0").center());
    assert_eq!(image_ref(&s, id), original, "the original picture is shown again");
    assert_eq!(s.app().doc.get(id).unwrap().ai_versions.len(), 2);
    let shown: Vec<bool> = s.app().ai.prompt.as_ref().expect("the dialog stays open").versions.iter().map(|v| v.current).collect();
    assert_eq!(shown, [true, false]);
    s.click(s.rect_of("ai-cancel").center());

    s.cmd_key(Key::Z);
    assert_eq!(image_ref(&s, id), generated, "undo takes the restore back");
    s.cmd_key(Key::Z);
    assert_eq!(image_ref(&s, id), original, "undo takes the generation back");
    assert!(s.app().doc.get(id).unwrap().ai_versions.is_empty(), "and its history entry");
}

#[test]
fn a_failed_request_keeps_the_dialog_with_the_reason_and_changes_nothing() {
    let (base, _seen) = server(|_| (402, json!({"error":{"message":"Insufficient credits","code":402}})));
    let mut s = Sim::new();
    ready(&mut s, &base);
    let ctx = s.ctx.clone();
    let before = s.app().doc.to_json().unwrap();
    s.shell.app.selection.clear();
    s.shell.app.open_ai_generate(&ctx);
    s.frame(vec![]);
    s.frame(vec![]);
    s.type_text("anything");
    s.click(s.rect_of("ai-generate").center());
    wait(&mut s, "the failure", |s| s.app().ai.prompt.as_ref().is_some_and(|p| !p.running));
    let p = s.app().ai.prompt.as_ref().unwrap();
    let e = p.error.as_ref().unwrap();
    assert!(e.contains("out of credits") && e.contains("Insufficient credits"), "{e}");
    assert_eq!(p.text, "anything", "the prompt is not lost");
    assert_eq!(s.app().doc.to_json().unwrap(), before);

    // Cancel closes it; a late answer is dropped
    s.click(s.rect_of("ai-cancel").center());
    assert!(s.app().ai.prompt.is_none());
}
