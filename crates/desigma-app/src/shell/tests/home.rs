//! UI tests for the projects library: the home screen, opening / renaming / saving
//! library files, trash, drag and drop, and the removed account / plan labels.

use super::*;
use crate::home::{Act, DropTo, Intent, Section};
use desigma_doc::Document;
use desigma_library::Library;
use std::path::PathBuf;

struct TmpDir(PathBuf);

impl TmpDir {
    fn new() -> Self {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let p = std::env::temp_dir().join(format!("desigma_home_ui_{}_{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        Self(p)
    }
}

impl Drop for TmpDir {
    fn drop(&mut self) {
        // The thumbnail thread may still be writing; ignore failures.
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn home_sim() -> (Sim, TmpDir) {
    let dir = TmpDir::new();
    let s = Sim::with(Options { home: true, library: Some(dir.0.clone()), ..Default::default() });
    (s, dir)
}

fn lib(s: &Sim) -> Library {
    s.app().lib.clone().expect("a library")
}

fn click_rect(s: &mut Sim, name: &str) {
    let r = s.rect_of(name);
    s.click(r.center());
}

fn double_click(s: &mut Sim, p: Pos2) {
    s.click(p);
    s.click(p);
}

fn run_action(s: &mut Sim, a: crate::panels::menus::Action) {
    let ctx = s.ctx.clone();
    crate::panels::menus::run(&mut s.shell.app, &ctx, a);
}

fn file_id(s: &Sim, name: &str) -> String {
    lib(s).all_files().into_iter().find(|f| f.name == name).unwrap_or_else(|| panic!("no library file {name}")).id
}

fn open(s: &mut Sim, name: &str) {
    let id = file_id(s, name);
    crate::home::apply(&mut s.shell.app, Act::Open(id));
    s.frame(vec![]);
    s.frame(vec![]);
}

/// Frames until `secs` of UI time have passed.
fn wait(s: &mut Sim, secs: f64) {
    let end = s.t + secs;
    while s.t < end {
        s.frame(vec![]);
    }
}

#[test]
fn first_launch_shows_the_home_screen_with_the_getting_started_project() {
    let (s, _d) = home_sim();
    let home = s.app().home.as_ref().expect("home screen");
    assert_eq!(home.section, Section::Recents);
    let names: Vec<_> = home.snap.projects.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["Getting started"]);
    assert_eq!(home.snap.files.len(), 1);
    assert_eq!(home.snap.files[0].name, "Welcome");
    // the seeded file carries the demo document and already has a thumbnail
    let l = lib(&s);
    assert!(l.load_file(&home.snap.files[0].id).unwrap().nodes.len() > 10);
    assert!(l.thumbnail_path(&home.snap.files[0]).is_some());
    let card = s.rect_of("home-card-Welcome");
    assert!(card.width() > 150.0 && card.min.x > theme::HOME_SIDEBAR_W);
    s.rect_of("home-project-Getting started");
    s.rect_of("home-new-file");
    s.rect_of("home-import");
    assert_eq!(s.shell.title, theme::APP_TITLE);
}

#[test]
fn new_design_file_creates_and_opens_an_untitled_library_file() {
    let (mut s, _d) = home_sim();
    click_rect(&mut s, "home-new-file");
    assert!(s.app().home.is_none(), "the editor opens");
    let lf = s.app().lib_file.clone().expect("a library file is open");
    assert_eq!((lf.name.as_str(), lf.project.as_str()), ("Untitled", "Getting started"));
    assert_eq!(s.app().doc.name, "Untitled");
    assert_eq!(lib(&s).all_files().len(), 2);
    assert!(lib(&s).load_file(&lf.id).is_ok());
}

#[test]
fn double_clicking_a_card_opens_it_and_single_click_selects() {
    let (mut s, _d) = home_sim();
    let card = s.rect_of("home-card-Welcome");
    s.click(card.center());
    assert_eq!(s.app().home.as_ref().unwrap().selected.as_deref(), Some(file_id(&s, "Welcome").as_str()));
    assert!(s.app().home.is_some());
    let before = lib(&s).file(&file_id(&s, "Welcome")).unwrap().opened;
    double_click(&mut s, card.center());
    assert!(s.app().home.is_none());
    assert_eq!(s.app().lib_file.as_ref().unwrap().name, "Welcome");
    assert!(s.app().doc.nodes.len() > 10, "the demo document loaded");
    assert!(lib(&s).file(&file_id(&s, "Welcome")).unwrap().opened > before, "opening updates Recents");
}

#[test]
fn the_editor_header_shows_project_and_file_and_none_of_the_account_labels() {
    let (mut s, _d) = home_sim();
    open(&mut s, "Welcome");
    let r = s.rect_of("file-header");
    assert!(r.left() >= theme::RAIL_WIDTH, "the header sits in the left panel");
    // the second line is the save state, not a plan
    assert_eq!(s.app().lib_file.as_ref().unwrap().project, "Getting started");
    assert!(!s.app().autosave.pending());
}

#[test]
fn renaming_in_the_header_persists_to_the_library() {
    let (mut s, _d) = home_sim();
    open(&mut s, "Welcome");
    let id = s.app().lib_file.as_ref().unwrap().id.clone();
    let hit = s.rect_of("file-header");
    double_click(&mut s, Pos2::new(hit.left() + 90.0, hit.center().y));
    assert!(s.app().renaming_file, "double-click starts renaming");
    s.frame(vec![]);
    s.frame(vec![]);
    s.type_text("Launch page");
    s.key(Key::Enter);
    s.frame(vec![]);
    assert!(!s.app().renaming_file);
    assert_eq!(lib(&s).file(&id).unwrap().name, "Launch page");
    assert_eq!(s.app().lib_file.as_ref().unwrap().name, "Launch page");
    assert_eq!(s.app().doc.name, "Launch page");
}

#[test]
fn renaming_a_file_from_the_home_dialog_persists() {
    let (mut s, _d) = home_sim();
    let id = file_id(&s, "Welcome");
    crate::home::apply(&mut s.shell.app, Act::RenameFile(id.clone()));
    s.frame(vec![]);
    s.frame(vec![]);
    assert!(matches!(s.app().dialog.as_ref().map(|d| &d.intent), Some(Intent::RenameFile(_))));
    s.type_text("Intro");
    s.frame(vec![]);
    click_rect(&mut s, "dialog-ok");
    assert!(s.app().dialog.is_none());
    assert_eq!(lib(&s).file(&id).unwrap().name, "Intro");
    assert!(s.app().home.as_ref().unwrap().snap.files.iter().any(|f| f.name == "Intro"), "the grid refreshed");
}

#[test]
fn right_click_menu_moves_a_file_to_the_trash_and_trash_restores_it() {
    let (mut s, _d) = home_sim();
    let card = s.rect_of("home-card-Welcome");
    s.right_click(card.center());
    assert!(s.app().home.as_ref().unwrap().menu.is_some(), "context menu open");
    // "Move to trash" is the 8th row, after three separators
    let pos = card.center();
    let y = pos.y + theme::MENU_PADDING + 7.0 * theme::MENU_ITEM_HEIGHT + 3.0 * crate::widgets::menu::SEP_H + theme::MENU_ITEM_HEIGHT / 2.0;
    s.click(Pos2::new(pos.x + 60.0, y));
    s.frame(vec![]);
    let l = lib(&s);
    assert!(l.all_files().is_empty());
    assert_eq!(l.list_trash().len(), 1);
    assert!(s.app().home.as_ref().unwrap().snap.files.is_empty());
    // Trash view: restore
    click_rect(&mut s, "home-trash");
    assert_eq!(s.app().home.as_ref().unwrap().section, Section::Trash);
    s.frame(vec![]);
    click_rect(&mut s, "home-restore-Welcome");
    s.frame(vec![]);
    assert_eq!(l.all_files().len(), 1);
    assert!(l.list_trash().is_empty());
    assert!(s.app().home.as_ref().unwrap().snap.trash.is_empty());
}

#[test]
fn emptying_the_trash_needs_a_confirmation() {
    let (mut s, _d) = home_sim();
    let id = file_id(&s, "Welcome");
    crate::home::apply(&mut s.shell.app, Act::Trash(id));
    s.frame(vec![]);
    click_rect(&mut s, "home-trash");
    s.frame(vec![]);
    click_rect(&mut s, "home-empty-trash");
    s.frame(vec![]);
    s.frame(vec![]);
    assert_eq!(s.app().dialog.as_ref().map(|d| d.intent.clone()), Some(Intent::EmptyTrash));
    assert_eq!(lib(&s).list_trash().len(), 1, "nothing is deleted before confirming");
    // Cancel keeps everything
    click_rect(&mut s, "dialog-cancel");
    assert!(s.app().dialog.is_none());
    assert_eq!(lib(&s).list_trash().len(), 1);
    // Confirm empties
    click_rect(&mut s, "home-empty-trash");
    s.frame(vec![]);
    s.frame(vec![]);
    click_rect(&mut s, "dialog-ok");
    assert!(lib(&s).list_trash().is_empty());
    s.frame(vec![]);
    assert!(s.app().home.as_ref().unwrap().snap.trash.is_empty());
}

#[test]
fn dragging_a_card_onto_a_project_moves_the_file() {
    let (mut s, _d) = home_sim();
    let l = lib(&s);
    let other = l.create_project("Marketing").unwrap();
    s.shell.app.home.as_mut().unwrap().dirty = true;
    s.frame(vec![]);
    s.frame(vec![]);
    let from = s.rect_of("home-card-Welcome").center();
    let to = s.rect_of("home-project-Marketing").center();
    s.drag(from, to);
    s.frame(vec![]);
    let f = l.all_files().into_iter().find(|f| f.name == "Welcome").unwrap();
    assert_eq!(f.project, other.id);
    // and into a folder via the action used by the folder rows
    let folder = l.create_folder(&other.id, None, "Q4").unwrap();
    crate::home::apply(&mut s.shell.app, Act::Drop(f.id.clone(), DropTo::Folder(other.id.clone(), folder.id.clone())));
    assert_eq!(l.file(&f.id).unwrap().folder.as_deref(), Some(folder.id.as_str()));
}

#[test]
fn search_filters_files_and_the_sidebar_navigates_sections() {
    let (mut s, _d) = home_sim();
    let l = lib(&s);
    let p = l.list_projects()[0].id.clone();
    let doc = Document::new("x");
    l.create_file(&p, None, "Checkout flow", &doc).unwrap();
    s.shell.app.home.as_mut().unwrap().dirty = true;
    s.frame(vec![]);
    s.shell.app.home.as_mut().unwrap().search = "check".into();
    s.frame(vec![]);
    let files = s.app().home.as_ref().unwrap().files();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].name, "Checkout flow");
    assert_eq!(s.app().home.as_ref().unwrap().title(), "Results for \u{201c}check\u{201d}");
    // choosing a section clears the search
    click_rect(&mut s, "home-starred");
    let h = s.app().home.as_ref().unwrap();
    assert_eq!((h.section.clone(), h.search.as_str()), (Section::Starred, ""));
    click_rect(&mut s, "home-project-Getting started");
    assert!(matches!(s.app().home.as_ref().unwrap().section, Section::Project(..)));
    assert_eq!(s.app().home.as_ref().unwrap().files().len(), 2);
}

#[test]
fn back_to_files_flushes_edits_and_returns_to_the_home_screen() {
    let (mut s, _d) = home_sim();
    open(&mut s, "Welcome");
    let id = s.app().lib_file.as_ref().unwrap().id.clone();
    let before = lib(&s).load_file(&id).unwrap().nodes.len();
    s.key(Key::R);
    s.mods = Modifiers::COMMAND;
    s.drag(s.screen(300.0, 800.0), s.screen(380.0, 860.0));
    s.mods = Modifiers::NONE;
    assert_eq!(s.count(), before + 1);
    assert_eq!(lib(&s).load_file(&id).unwrap().nodes.len(), before, "not saved yet");
    run_action(&mut s, crate::panels::menus::Action::BackToFiles);
    s.frame(vec![]);
    assert!(s.app().home.is_some());
    assert_eq!(lib(&s).load_file(&id).unwrap().nodes.len(), before + 1, "leaving the editor saved the edit");
}

#[test]
fn autosave_runs_after_the_idle_delay_and_ctrl_s_saves_at_once() {
    let (mut s, _d) = home_sim();
    open(&mut s, "Welcome");
    let id = s.app().lib_file.as_ref().unwrap().id.clone();
    let before = lib(&s).load_file(&id).unwrap().nodes.len();
    s.key(Key::R);
    s.mods = Modifiers::COMMAND;
    s.drag(s.screen(300.0, 800.0), s.screen(380.0, 860.0));
    s.mods = Modifiers::NONE;
    wait(&mut s, 0.8);
    assert!(s.app().autosave.pending(), "a save is pending");
    assert_eq!(lib(&s).load_file(&id).unwrap().nodes.len(), before, "not before the 1.5s idle delay");
    wait(&mut s, 1.2);
    assert_eq!(lib(&s).load_file(&id).unwrap().nodes.len(), before + 1, "autosaved");
    assert_eq!(s.app().autosave.saves, 1);
    assert!(!s.app().autosave.pending());
    // Ctrl+S is immediate
    s.mods = Modifiers::COMMAND;
    s.drag(s.screen(500.0, 800.0), s.screen(560.0, 860.0));
    s.mods = Modifiers::NONE;
    s.key(Key::R);
    s.mods = Modifiers::COMMAND;
    s.drag(s.screen(600.0, 800.0), s.screen(660.0, 860.0));
    s.mods = Modifiers::NONE;
    let now = s.count();
    s.cmd_key(Key::S);
    assert_eq!(lib(&s).load_file(&id).unwrap().nodes.len(), now);
    assert!(s.app().toast.as_ref().is_some_and(|t| t.text.starts_with("Saved to")));
}

#[test]
fn unchanged_documents_are_never_rewritten_by_autosave() {
    let (mut s, _d) = home_sim();
    open(&mut s, "Welcome");
    let id = s.app().lib_file.as_ref().unwrap().id.clone();
    let m0 = lib(&s).file(&id).unwrap().modified;
    // zooming changes nothing in the document
    s.shell.app.zoom_to(2.0);
    wait(&mut s, 3.0);
    assert_eq!(lib(&s).file(&id).unwrap().modified, m0);
    assert_eq!(s.app().autosave.saves, 0);
}

#[test]
fn external_files_keep_their_name_and_offer_save_to_library() {
    let dir = TmpDir::new();
    std::fs::create_dir_all(&dir.0).unwrap();
    let ext = dir.0.join("outside.desigma");
    let mut d = Document::new("Outside");
    d.add_page("P");
    std::fs::write(&ext, d.to_json().unwrap()).unwrap();
    let mut s = Sim::with(Options { file: Some(ext.clone()), library: Some(dir.0.join("lib")), ..Default::default() });
    assert!(s.app().home.is_none(), "a file argument opens the editor");
    assert!(s.app().lib_file.is_none());
    assert_eq!(s.app().doc.name, "Outside");
    click_rect(&mut s, "save-to-library");
    s.frame(vec![]);
    assert_eq!(s.app().dialog.as_ref().map(|d| d.intent.clone()), Some(Intent::SaveToLibrary));
    click_rect(&mut s, "dialog-ok");
    assert!(s.app().dialog.is_none());
    let lf = s.app().lib_file.clone().expect("now a library file");
    assert_eq!(lf.name, "Outside");
    assert_eq!(lib(&s).file(&lf.id).unwrap().name, "Outside");
    assert!(ext.exists(), "the original is untouched");
}

#[test]
fn importing_a_file_adds_it_to_the_project() {
    let (mut s, d) = home_sim();
    let svg = d.0.join("logo.svg");
    std::fs::write(&svg, r##"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><rect width="8" height="8" fill="#f00"/></svg>"##).unwrap();
    let p = lib(&s).list_projects()[0].id.clone();
    assert_eq!(s.shell.app.import_into_library(&[svg], &p, None), 1);
    s.frame(vec![]);
    s.frame(vec![]);
    assert!(s.app().home.as_ref().unwrap().snap.files.iter().any(|f| f.name == "logo"));
}

#[test]
fn account_and_plan_labels_are_gone_from_the_ui_sources() {
    // The product has no accounts, plans or cloud; guard against the Figma labels coming back.
    let sources = [
        ("left.rs", include_str!("../../panels/left.rs")),
        ("right.rs", include_str!("../../panels/right.rs")),
        ("rail.rs", include_str!("../../panels/rail.rs")),
        ("menus.rs", include_str!("../../panels/menus.rs")),
        ("toolbar.rs", include_str!("../../panels/toolbar.rs")),
    ];
    // ("Agents" left the list when the rail got a real Agents panel: `crate::agents`.)
    let banned = ["\"Drafts\"", "\"Free\"", "\"Share\"", "Help and account", "Upgrade", "\"Team\"", "\"Pro\"", "avatar"];
    for (file, src) in sources {
        for b in banned {
            assert!(!src.contains(b), "{file} still mentions {b}");
        }
    }
}

#[test]
fn help_menu_offers_shortcuts_about_and_a_confirmed_github_link() {
    let (mut s, _d) = home_sim();
    open(&mut s, "Welcome");
    run_action(&mut s, crate::panels::menus::Action::Shortcuts);
    s.frame(vec![]);
    assert_eq!(s.app().dialog.as_ref().map(|d| d.intent.clone()), Some(Intent::Shortcuts));
    s.key(Key::Escape);
    assert!(s.app().dialog.is_none());
    run_action(&mut s, crate::panels::menus::Action::AboutDesigma);
    s.frame(vec![]);
    assert_eq!(s.app().dialog.as_ref().map(|d| d.intent.clone()), Some(Intent::About));
    click_rect(&mut s, "dialog-ok");
    assert!(s.app().dialog.is_none());
    // the GitHub link asks first and is cancellable
    run_action(&mut s, crate::panels::menus::Action::OpenGithub);
    s.frame(vec![]);
    assert_eq!(s.app().dialog.as_ref().map(|d| d.intent.clone()), Some(Intent::OpenGithub));
    click_rect(&mut s, "dialog-cancel");
    assert!(s.app().dialog.is_none());
    assert!(s.app().toast.as_ref().is_none_or(|t| !t.error));
}

#[test]
fn library_option_sets_the_root_and_home_option_shows_the_home_screen() {
    let dir = TmpDir::new();
    let s = Sim::with(Options { home: true, library: Some(dir.0.clone()), screenshot: Some(dir.0.join("shot.png")), ..Default::default() });
    assert_eq!(lib(&s).root(), dir.0.as_path());
    assert!(s.app().home.is_some());
    assert!(dir.0.join("projects").exists(), "the library was created under --library");
    // without --home a screenshot run stays in the editor and never touches a library
    let e = Sim::with(Options { screenshot: Some(dir.0.join("e.png")), ..Default::default() });
    assert!(e.app().home.is_none() && e.app().lib.is_none());
}
