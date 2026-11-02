//! A file rewritten on disk is picked up: silently when there are no edits, after a question
//! when there are (see `extwatch.rs`).

use super::*;
use desigma_doc::Document;
use std::path::{Path, PathBuf};

fn write_doc(path: &Path, name: &str) {
    let mut d = Document::new(name);
    d.add_page("Page 1");
    crate::io::save_path(&d, path).expect("write test document");
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("desigma-extwatch-{tag}-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    dir.join("doc.desigma")
}

#[test]
fn a_clean_document_reloads_itself_when_the_file_changes() {
    let path = scratch("clean");
    write_doc(&path, "Before");
    let mut s = Sim::new();
    s.shell.app.open_path(path.clone());
    assert_eq!(s.app().doc.name, "Before");
    assert!(s.app().ext_watch.is_some(), "opening a file starts watching it");

    write_doc(&path, "Changed on disk");
    s.frame(vec![]);
    assert_eq!(s.app().doc.name, "Changed on disk", "no edits: the newer file wins");
    assert!(!s.app().ext_watch.as_ref().unwrap().asked);
}

#[test]
fn unsaved_edits_ask_before_the_disk_version_replaces_them() {
    let path = scratch("dirty");
    write_doc(&path, "Before");
    let mut s = Sim::new();
    s.shell.app.open_path(path.clone());
    // An edit in the editor: the document no longer matches the file we opened.
    s.shell.app.doc.name = "Mine".into();

    write_doc(&path, "Theirs, longer");
    s.frame(vec![]);
    s.frame(vec![]);
    assert_eq!(s.app().doc.name, "Mine", "my edits stay until I answer");
    assert!(s.app().ext_watch.as_ref().unwrap().asked, "the question is up");

    s.click(s.rect_of("ext-keep").center());
    s.frame(vec![]);
    assert_eq!(s.app().doc.name, "Mine");
    assert!(!s.app().ext_watch.as_ref().unwrap().asked, "keeping mine silences the prompt");
}

#[test]
fn unsaved_edits_can_be_thrown_away_by_reloading() {
    let path = scratch("reload");
    write_doc(&path, "Before");
    let mut s = Sim::new();
    s.shell.app.open_path(path.clone());
    s.shell.app.doc.name = "Mine".into();

    write_doc(&path, "Theirs");
    s.frame(vec![]);
    s.frame(vec![]);
    assert!(s.app().ext_watch.as_ref().unwrap().asked);
    s.click(s.rect_of("ext-reload").center());
    s.frame(vec![]);
    assert_eq!(s.app().doc.name, "Theirs");
}

#[test]
fn unsaved_edits_can_be_kept_as_a_copy_while_taking_the_disk_version() {
    let path = scratch("both");
    write_doc(&path, "Before");
    let mut s = Sim::new();
    s.shell.app.open_path(path.clone());
    s.shell.app.doc.name = "Mine".into();

    write_doc(&path, "Theirs");
    s.frame(vec![]);
    s.frame(vec![]);
    assert!(s.app().ext_watch.as_ref().unwrap().asked);
    s.click(s.rect_of("ext-both").center());
    s.frame(vec![]);

    assert_eq!(s.app().doc.name, "Theirs", "the disk version is open again");
    let copy = path.with_file_name("doc (my copy).desigma");
    assert!(copy.exists(), "my edits were written beside the file");
    let (doc, _notes) = crate::io::load_path_noted(&copy).expect("read the copy");
    assert_eq!(doc.name, "Mine");
}

#[test]
fn active_collab_session_skips_external_file_polling() {
    let path = scratch("collab");
    write_doc(&path, "Original");
    let mut s = Sim::new();
    s.shell.app.open_path(path.clone());

    // Mark collaboration active by setting joining state.
    struct DummyGuest;
    impl crate::collab::link::GuestLink for DummyGuest {
        fn drain(&self) -> Vec<crate::collab::link::GuestEv> {
            vec![]
        }
        fn send(&self, _bytes: Vec<u8>) {}
        fn send_presence(&self, _bytes: &[u8]) {}
        fn leave(&self) {}
    }
    s.shell.app.collab.joining = Some(crate::collab::Joining { link: Box::new(DummyGuest), role: desigma_collab::Role::Editor, redial: None });
    assert!(s.shell.app.collab.active(), "collab must be active");

    write_doc(&path, "FromOutside");
    s.frame(vec![]);
    s.frame(vec![]);

    // Even though the file changed on disk, the active collab session prevented reloading.
    assert_eq!(s.app().doc.name, "Original", "must not reload while collab is active");
    assert!(!s.app().ext_watch.as_ref().unwrap().asked);
}

#[test]
fn cloud_sync_touch_with_identical_content_does_not_reload() {
    let path = scratch("cloud_sync");
    write_doc(&path, "UnchangedContent");
    let mut s = Sim::new();
    s.shell.app.open_path(path.clone());
    assert_eq!(s.app().doc.name, "UnchangedContent");

    // Clear toast to start clean.
    s.shell.app.toast = None;

    // Re-write the exact same document bytes, but with a later mtime.
    write_doc(&path, "UnchangedContent");
    s.frame(vec![]);
    s.frame(vec![]);

    // The document name and state are unchanged, and NO reload toast was emitted.
    assert_eq!(s.app().doc.name, "UnchangedContent");
    let reloaded_toast = s.shell.app.toast.as_ref().is_some_and(|t| t.text.contains("Reloaded"));
    assert!(!reloaded_toast, "should not trigger reload toast when content hash matches");
}

#[test]
fn cloud_sync_touch_does_not_ask_about_unsaved_edits() {
    let path = scratch("cloud_sync_dirty");
    write_doc(&path, "Saved");
    let mut s = Sim::new();
    s.shell.app.open_path(path.clone());
    s.shell.app.doc.name = "Mine".into();

    // The same bytes again, later: a sync client touching our own save.
    write_doc(&path, "Saved");
    s.frame(vec![]);
    s.frame(vec![]);
    assert_eq!(s.app().doc.name, "Mine");
    assert!(!s.app().ext_watch.as_ref().unwrap().asked, "nothing changed on disk, so nothing to ask");
}
