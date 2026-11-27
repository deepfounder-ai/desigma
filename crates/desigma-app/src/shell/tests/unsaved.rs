//! Opening a file or link over unsaved work asks first (see `unsaved.rs`).

use super::*;
use crate::collab::link::HostEv;
use crate::collab::tests::{MemHost, Net};
use crate::instance::Open;
use desigma_collab::Role;

fn click_named(s: &mut Sim, name: &str) {
    let r = s.rect_of(name);
    s.click(r.center());
}

#[test]
fn opens_over_unsaved_work_ask_first() {
    let mut s = Sim::new();
    // a clean document is replaced without a question
    s.shell.app.request_open(Open::Link("desigma://join/abc".into()));
    assert!(s.app().pending_open.is_none() && s.app().collab.join.is_some());
    s.shell.app.collab.join = None;
    // an edited one waits for the answer
    s.shell.app.file_hash = Some(crate::home::session::doc_hash(&s.shell.app.doc));
    s.shell.app.doc.name = "Edited".into();
    s.shell.app.request_open(Open::Link("desigma://join/abc".into()));
    s.frame(vec![]);
    s.frame(vec![]);
    assert!(s.app().pending_open.is_some() && s.app().collab.join.is_none());
    click_named(&mut s, "unsaved-cancel");
    s.frame(vec![]);
    assert!(s.app().pending_open.is_none() && s.app().collab.join.is_none(), "cancel keeps everything");
    s.shell.app.request_open(Open::Link("desigma://join/abc".into()));
    s.frame(vec![]);
    s.frame(vec![]);
    click_named(&mut s, "unsaved-discard");
    s.frame(vec![]);
    assert!(s.app().pending_open.is_none() && s.app().collab.join.is_some(), "discard goes ahead");
}

#[test]
fn a_live_session_also_asks_before_opening_something() {
    let mut s = Sim::new();
    let net = Net::default();
    s.shell.app.start_sharing_with(Box::new(MemHost(net.clone())), false, false, Role::Editor);
    net.borrow_mut().host.push_back(HostEv::Ready { link: "desigma://join/abcdefghijklmnop".into() });
    for _ in 0..4 {
        s.frame(vec![]);
    }
    s.shell.app.request_open(Open::File(std::env::temp_dir().join("x.desigma")));
    assert!(s.app().pending_open.is_some());
    s.frame(vec![]);
    s.frame(vec![]);
    click_named(&mut s, "unsaved-cancel");
    s.frame(vec![]);
    assert!(s.app().pending_open.is_none() && s.app().collab.hosting());
}

fn run_close_request(s: &mut Sim) -> Vec<egui::ViewportCommand> {
    s.t += 0.05;
    let mut raw = egui::RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1600.0, 1088.0))),
        time: Some(s.t),
        modifiers: s.mods,
        ..Default::default()
    };
    raw.viewports.entry(egui::ViewportId::ROOT).or_default().events.push(egui::ViewportEvent::Close);
    let ctx = s.ctx.clone();
    let shell = &mut s.shell;
    eframe::App::raw_input_hook(shell, &ctx, &mut raw);
    let out = ctx.run_ui(raw, |ui| {
        eframe::App::logic(shell, ui.ctx(), &mut eframe::Frame::_new_kittest());
        shell.draw(ui)
    });
    s.cursor = out.platform_output.cursor_icon;
    s.shapes = out.shapes;
    out.viewport_output.get(&egui::ViewportId::ROOT).map(|v| v.commands.clone()).unwrap_or_default()
}

#[test]
fn closing_clean_document_does_not_prompt() {
    let mut s = Sim::new();
    let cmds = run_close_request(&mut s);
    assert!(!s.app().pending_close);
    assert!(!cmds.contains(&egui::ViewportCommand::CancelClose));
}

#[test]
fn closing_unsaved_document_prompts_and_cancel_keeps_open() {
    let mut s = Sim::new();
    s.shell.app.file_hash = Some(crate::home::session::doc_hash(&s.shell.app.doc));
    s.shell.app.doc.name = "Edited".into();
    let cmds = run_close_request(&mut s);
    assert!(cmds.contains(&egui::ViewportCommand::CancelClose));
    assert!(s.app().pending_close);
    s.frame(vec![]);
    click_named(&mut s, "unsaved-cancel");
    s.frame(vec![]);
    assert!(!s.app().pending_close);
    assert!(!s.app().force_close);
}

#[test]
fn closing_unsaved_document_discard_forces_close() {
    let mut s = Sim::new();
    s.shell.app.file_hash = Some(crate::home::session::doc_hash(&s.shell.app.doc));
    s.shell.app.doc.name = "Edited".into();
    let cmds = run_close_request(&mut s);
    assert!(cmds.contains(&egui::ViewportCommand::CancelClose));
    assert!(s.app().pending_close);
    s.frame(vec![]);
    click_named(&mut s, "unsaved-discard");
    s.frame(vec![]);
    assert!(!s.app().pending_close);
    assert!(s.app().force_close);
    let cmds2 = run_close_request(&mut s);
    assert!(!cmds2.contains(&egui::ViewportCommand::CancelClose));
}

#[test]
fn closing_unsaved_document_save_with_path_saves_and_closes() {
    let mut s = Sim::new();
    let path = std::env::temp_dir().join(format!("desigma_close_test_{}.desigma", std::process::id()));
    let _ = std::fs::remove_file(&path);
    s.shell.app.file_path = Some(path.clone());
    s.shell.app.file_hash = Some(crate::home::session::doc_hash(&s.shell.app.doc));
    s.shell.app.doc.name = "Edited".into();
    let cmds = run_close_request(&mut s);
    assert!(cmds.contains(&egui::ViewportCommand::CancelClose));
    assert!(s.app().pending_close);
    s.frame(vec![]);
    click_named(&mut s, "unsaved-save");
    s.frame(vec![]);
    assert!(!s.app().pending_close);
    assert!(!s.app().edits_unsaved());
    assert!(s.app().force_close);
    assert!(path.exists());
    let _ = std::fs::remove_file(&path);
}
