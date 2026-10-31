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
