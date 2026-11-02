//! The projects library UI: the home screen (file browser) shown at launch and
//! from File -> "Back to files", the dialogs it shares with the editor, and the
//! glue that opens library files in the editor.
//!
//! The data lives in `desigma-library`; this module only draws it and turns
//! clicks into [`Act`]s that [`apply`] runs against the library.

pub mod dialog;
mod main_view;
pub(crate) mod session;
mod sidebar;
pub mod thumb;
pub mod time;

pub use dialog::{Dialog, Intent};
pub(crate) use session::doc_hash;
pub use session::{Autosave, LibFile};

use crate::app::App;
use crate::theme;
use desigma_library::{FileEntry, Library, Project, TrashItem};
use egui::{Pos2, Rect, Ui, Vec2};
use std::collections::HashSet;

pub const GITHUB_URL: &str = "https://github.com/deepfounder-ai/desigma";

#[derive(Clone, Debug, PartialEq)]
pub enum Section {
    Recents,
    Starred,
    /// A project, optionally narrowed to one folder.
    Project(String, Option<String>),
    Trash,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewMode {
    Grid,
    List,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sort {
    LastOpened,
    LastModified,
    Name,
}

impl Sort {
    pub const ALL: [Sort; 3] = [Sort::LastOpened, Sort::LastModified, Sort::Name];
    pub fn label(self) -> &'static str {
        match self {
            Sort::LastOpened => "Last opened",
            Sort::LastModified => "Last modified",
            Sort::Name => "Name",
        }
    }
}

/// What a context menu was opened on.
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    File(String),
    Project(String),
    Folder(String, String),
}

/// Where a dragged file can be dropped.
#[derive(Clone, Debug, PartialEq)]
pub enum DropTo {
    Project(String),
    Folder(String, String),
}

/// Everything the home screen shows, read from disk when something changes.
#[derive(Default)]
pub struct Snapshot {
    pub projects: Vec<Project>,
    pub files: Vec<FileEntry>,
    pub trash: Vec<TrashItem>,
}

impl Snapshot {
    pub fn project_name(&self, id: &str) -> &str {
        self.projects.iter().find(|p| p.id == id).map(|p| p.name.as_str()).unwrap_or("")
    }
}

pub struct Home {
    pub section: Section,
    pub view: ViewMode,
    pub sort: Sort,
    pub search: String,
    pub snap: Snapshot,
    /// The snapshot is stale; reload it before drawing.
    pub dirty: bool,
    /// Projects whose folders are collapsed in the sidebar.
    pub collapsed: HashSet<String>,
    pub selected: Option<String>,
    pub menu: Option<(Target, Pos2)>,
    pub sort_menu: Option<Rect>,
    /// File being dragged: (id, name).
    pub drag: Option<(String, String)>,
    /// Drop zones drawn this frame.
    pub drop_rects: Vec<(Rect, DropTo)>,
    pub scroll: f32,
    pub side_scroll: f32,
    pub thumbs: thumb::Textures,
    /// Design file or board, per file.
    pub kinds: thumb::Kinds,
}

impl Default for Home {
    fn default() -> Self {
        Self::new()
    }
}

impl Home {
    pub fn new() -> Self {
        Self {
            section: Section::Recents,
            view: ViewMode::Grid,
            sort: Sort::LastOpened,
            search: String::new(),
            snap: Snapshot::default(),
            dirty: true,
            collapsed: HashSet::new(),
            selected: None,
            menu: None,
            sort_menu: None,
            drag: None,
            drop_rects: vec![],
            scroll: 0.0,
            side_scroll: 0.0,
            thumbs: Default::default(),
            kinds: Default::default(),
        }
    }

    pub fn refresh(&mut self, lib: &Library) {
        let projects = lib.list_projects();
        let files = projects.iter().flat_map(|p| lib.files_in(&p.id, None).unwrap_or_default()).collect();
        self.snap = Snapshot { projects, files, trash: lib.list_trash() };
        self.dirty = false;
        // A section pointing at something that no longer exists falls back to Recents.
        if let Section::Project(p, f) = &self.section {
            let gone = match self.snap.projects.iter().find(|x| &x.id == p) {
                None => true,
                Some(pr) => f.as_ref().is_some_and(|f| !pr.folders.iter().any(|x| &x.id == f)),
            };
            if gone {
                self.section = Section::Recents;
            }
        }
    }

    pub fn searching(&self) -> bool {
        !self.search.trim().is_empty()
    }

    /// Files of the current section (or search), sorted.
    pub fn files(&self) -> Vec<FileEntry> {
        let q = self.search.trim().to_lowercase();
        let mut v: Vec<FileEntry> = if !q.is_empty() {
            self.snap.files.iter().filter(|f| f.name.to_lowercase().contains(&q)).cloned().collect()
        } else {
            match &self.section {
                Section::Recents => self.snap.files.clone(),
                Section::Starred => self.snap.files.iter().filter(|f| f.starred).cloned().collect(),
                Section::Project(p, folder) => self.snap.files.iter().filter(|f| &f.project == p && &f.folder == folder).cloned().collect(),
                Section::Trash => vec![],
            }
        };
        match self.sort {
            Sort::LastOpened => v.sort_by_key(|f| std::cmp::Reverse(f.opened)),
            Sort::LastModified => v.sort_by_key(|f| std::cmp::Reverse(f.modified)),
            Sort::Name => v.sort_by_key(|f| f.name.to_lowercase()),
        }
        v
    }

    /// Sub-folders shown as tiles in a project view.
    pub fn folders(&self) -> Vec<(String, String)> {
        if self.searching() {
            return vec![];
        }
        let Section::Project(p, parent) = &self.section else { return vec![] };
        let Some(pr) = self.snap.projects.iter().find(|x| &x.id == p) else { return vec![] };
        let mut v: Vec<(String, String)> = pr.folders.iter().filter(|f| &f.parent == parent).map(|f| (f.id.clone(), f.name.clone())).collect();
        v.sort_by_key(|(_, n)| n.to_lowercase());
        v
    }

    pub fn title(&self) -> String {
        if self.searching() {
            return format!("Results for \u{201c}{}\u{201d}", self.search.trim());
        }
        match &self.section {
            Section::Recents => "Recents".into(),
            Section::Starred => "Starred".into(),
            Section::Trash => "Trash".into(),
            Section::Project(p, _) => self.snap.project_name(p).to_string(),
        }
    }

    pub fn file(&self, id: &str) -> Option<&FileEntry> {
        self.snap.files.iter().find(|f| f.id == id)
    }
}

/// Something the user asked for on the home screen.
#[derive(Clone, Debug, PartialEq)]
pub enum Act {
    Go(Section),
    Open(String),
    OpenWindow(String),
    NewFile,
    NewBoard,
    Import,
    /// Opens the Join session dialog.
    JoinSession,
    /// Opens the Claude connection dialog.
    Mcp,
    NewProject,
    RenameProject(String),
    DeleteProject(String),
    NewFolder(String, Option<String>),
    RenameFolder(String, String),
    FolderToTop(String, String),
    RenameFile(String),
    Duplicate(String),
    MoveTo(String),
    Star(String, bool),
    Reveal(String),
    Trash(String),
    Restore(String),
    EmptyTrash,
    Drop(String, DropTo),
    SetView(ViewMode),
    SetSort(Sort),
}

/// Draws the home screen filling `ui`, then runs what the user did.
pub fn show(app: &mut App, ui: &mut Ui) {
    let Some(mut h) = app.home.take() else { return };
    let Some(lib) = app.lib.clone() else { return };
    if h.dirty {
        h.refresh(&lib);
    }
    let full = ui.max_rect();
    ui.painter().rect_filled(full, 0.0, theme::BG);
    let side = Rect::from_min_max(full.min, Pos2::new(full.left() + theme::HOME_SIDEBAR_W, full.bottom()));
    let main = Rect::from_min_max(Pos2::new(side.right(), full.top()), full.max);
    h.drop_rects.clear();
    let mut acts = Vec::new();
    sidebar::show(&mut h, ui, side, &mut acts);
    main_view::show(&lib, &mut h, ui, main, &mut acts);
    menus(&mut h, ui.ctx(), &mut acts);
    drag_ghost(&mut h, ui, &mut acts);
    // Escape clears the search.
    if h.searching() && app.dialog.is_none() && ui.input(|i| i.key_pressed(egui::Key::Escape)) && h.menu.is_none() && h.sort_menu.is_none() {
        h.search.clear();
    }
    app.home = Some(h);
    for a in acts {
        apply(app, a);
    }
}

fn menus(h: &mut Home, ctx: &egui::Context, acts: &mut Vec<Act>) {
    use crate::widgets::menu::{popup_menu, MenuItem, MenuResult};
    if let Some(anchor) = h.sort_menu {
        let items: Vec<MenuItem> = Sort::ALL.iter().map(|s| MenuItem::new(s.label()).checked(*s == h.sort)).collect();
        match popup_menu(ctx, egui::Id::new("home-sort-menu"), anchor.left_bottom() + Vec2::new(0.0, 4.0), 160.0, &items, Some(anchor)) {
            MenuResult::Picked(i) => {
                h.sort_menu = None;
                acts.push(Act::SetSort(Sort::ALL[i]));
            }
            MenuResult::Closed => h.sort_menu = None,
            MenuResult::Open => {}
        }
    }
    let Some((target, pos)) = h.menu.clone() else { return };
    let entries = context_entries(h, &target);
    let items: Vec<MenuItem> = entries.iter().map(|e| e.0.clone()).collect();
    match popup_menu(ctx, egui::Id::new("home-context-menu"), pos, theme::MENU_WIDTH, &items, None) {
        MenuResult::Picked(i) => {
            h.menu = None;
            acts.push(entries[i].1.clone());
        }
        MenuResult::Closed => h.menu = None,
        MenuResult::Open => {}
    }
}

/// Menu rows for a right-click target, with the action each one runs.
pub fn context_entries(h: &Home, target: &Target) -> Vec<(crate::widgets::menu::MenuItem, Act)> {
    use crate::widgets::menu::MenuItem;
    match target {
        Target::File(id) => {
            let starred = h.file(id).is_some_and(|f| f.starred);
            vec![
                (MenuItem::new("Open"), Act::Open(id.clone())),
                (MenuItem::new("Open in new window").sep(), Act::OpenWindow(id.clone())),
                (MenuItem::new("Rename"), Act::RenameFile(id.clone())),
                (MenuItem::new("Duplicate"), Act::Duplicate(id.clone())),
                (MenuItem::new("Move to\u{2026}").sep(), Act::MoveTo(id.clone())),
                (MenuItem::new(if starred { "Remove star" } else { "Star" }), Act::Star(id.clone(), !starred)),
                (MenuItem::new("Show in folder").sep(), Act::Reveal(id.clone())),
                (MenuItem::new("Move to trash"), Act::Trash(id.clone())),
            ]
        }
        Target::Project(id) => vec![
            (MenuItem::new("New folder"), Act::NewFolder(id.clone(), None)),
            (MenuItem::new("Rename").sep(), Act::RenameProject(id.clone())),
            (MenuItem::new("Move to trash"), Act::DeleteProject(id.clone())),
        ],
        Target::Folder(p, f) => {
            let nested = h.snap.projects.iter().find(|x| &x.id == p).and_then(|pr| pr.folders.iter().find(|x| &x.id == f)).is_some_and(|x| x.parent.is_some());
            let mut v = vec![
                (MenuItem::new("New subfolder"), Act::NewFolder(p.clone(), Some(f.clone()))),
                (MenuItem::new("Rename"), Act::RenameFolder(p.clone(), f.clone())),
            ];
            if nested {
                v.push((MenuItem::new("Move to top level"), Act::FolderToTop(p.clone(), f.clone())));
            }
            v
        }
    }
}

/// Shows the dragged file under the pointer and drops it on release.
fn drag_ghost(h: &mut Home, ui: &Ui, acts: &mut Vec<Act>) {
    let Some((id, name)) = h.drag.clone() else { return };
    let (ptr, released) = ui.input(|i| (i.pointer.hover_pos().or(i.pointer.latest_pos()), i.pointer.primary_released()));
    let Some(ptr) = ptr else {
        h.drag = None;
        return;
    };
    let p = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("home-drag")));
    let font = theme::font_body();
    let w = crate::widgets::text_width(&p, &name, &font).min(220.0) + 24.0;
    let r = Rect::from_min_size(ptr + Vec2::new(12.0, 8.0), Vec2::new(w, 28.0));
    p.rect_filled(r, theme::cr(theme::RADIUS_MEDIUM), theme::BG_BRAND);
    let shown = crate::widgets::elide(&p, &name, &font, 200.0);
    p.text(r.left_center() + Vec2::new(12.0, 0.0), egui::Align2::LEFT_CENTER, shown, font, egui::Color32::WHITE);
    ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    if released {
        if let Some((_, to)) = h.drop_rects.iter().rev().find(|(r, _)| r.contains(ptr)) {
            acts.push(Act::Drop(id, to.clone()));
        }
        h.drag = None;
    }
}

/// Runs one action against the library, reporting failures as toasts.
pub fn apply(app: &mut App, act: Act) {
    let Some(lib) = app.lib.clone() else { return };
    fn ok<T>(app: &mut App, r: desigma_library::Result<T>) -> Option<T> {
        match r {
            Ok(v) => Some(v),
            Err(e) => {
                app.show_toast(e.to_string(), true);
                None
            }
        }
    }
    let mut changed = true;
    match act {
        Act::Go(section) => {
            if let Some(h) = app.home.as_mut() {
                h.section = section;
                h.search.clear();
                h.selected = None;
                h.scroll = 0.0;
            }
            changed = false;
        }
        Act::SetView(v) => {
            if let Some(h) = app.home.as_mut() {
                h.view = v;
            }
            changed = false;
        }
        Act::SetSort(s) => {
            if let Some(h) = app.home.as_mut() {
                h.sort = s;
            }
            changed = false;
        }
        Act::Open(id) => app.open_library_file(&id),
        Act::OpenWindow(id) => {
            let exe = std::env::current_exe();
            match exe.and_then(|e| std::process::Command::new(e).arg("--library").arg(lib.root()).arg("--open-id").arg(&id).spawn()) {
                Ok(_) => app.show_toast("Opened in a new window", false),
                Err(e) => app.show_toast(format!("Cannot open a new window: {e}"), true),
            }
        }
        Act::NewFile => {
            let target = match app.home_target() {
                Some(t) => Some(t),
                None => app.home.as_ref().and_then(|h| h.snap.projects.first().map(|p| (p.id.clone(), None))),
            };
            app.new_library_file(target);
        }
        Act::NewBoard => {
            let target = match app.home_target() {
                Some(t) => Some(t),
                None => app.home.as_ref().and_then(|h| h.snap.projects.first().map(|p| (p.id.clone(), None))),
            };
            app.new_library_board(target);
        }
        Act::JoinSession => app.open_join(None),
        Act::Mcp => {
            app.mcp_error = None;
            app.mcp_dialog = true;
            changed = false;
        }
        Act::Import => {
            let paths = rfd::FileDialog::new()
                .add_filter("Design files", &["fig", "pen", "json", "svg", "desigma"])
                .add_filter("Figma file", &["fig"])
                .add_filter("Pencil file", &["pen"])
                .add_filter("SVG", &["svg"])
                .add_filter("JSON", &["json"])
                .add_filter("desigma", &["desigma"])
                .pick_files();
            if let Some(paths) = paths {
                let target = match app.home_target() {
                    Some(t) => Some(t),
                    None => app.home.as_ref().and_then(|h| h.snap.projects.first().map(|p| (p.id.clone(), None))),
                };
                let target = match target {
                    Some(t) => Some(t),
                    None => ok(app, lib.create_project("My project")).map(|p| (p.id, None)),
                };
                if let Some((p, f)) = target {
                    app.import_into_library(&paths, &p, f.as_deref());
                }
            }
        }
        Act::NewProject => app.dialog = Some(Dialog::prompt(Intent::NewProject, "New project", "", "Create")),
        Act::RenameProject(id) => {
            let name = lib.project(&id).map(|p| p.name).unwrap_or_default();
            app.dialog = Some(Dialog::prompt(Intent::RenameProject(id), "Rename project", &name, "Rename"));
            changed = false;
        }
        Act::DeleteProject(id) => {
            let name = lib.project(&id).map(|p| p.name).unwrap_or_default();
            if ok(app, lib.delete_project(&id)).is_some() {
                if app.lib_file.as_ref().is_some_and(|f| f.project_id == id) {
                    app.lib_file = None;
                }
                app.show_toast(format!("Moved {name} to the trash"), false);
            }
        }
        Act::NewFolder(p, parent) => {
            app.dialog = Some(Dialog::prompt(Intent::NewFolder(p, parent), "New folder", "", "Create"));
            changed = false;
        }
        Act::RenameFolder(p, f) => {
            let name = lib.project(&p).ok().and_then(|pr| pr.folders.into_iter().find(|x| x.id == f)).map(|x| x.name).unwrap_or_default();
            app.dialog = Some(Dialog::prompt(Intent::RenameFolder(p, f), "Rename folder", &name, "Rename"));
            changed = false;
        }
        Act::FolderToTop(p, f) => {
            ok(app, lib.move_folder(&p, &f, None));
        }
        Act::RenameFile(id) => {
            let name = lib.file(&id).map(|f| f.name).unwrap_or_default();
            app.dialog = Some(Dialog::prompt(Intent::RenameFile(id), "Rename file", &name, "Rename"));
            changed = false;
        }
        Act::Duplicate(id) => {
            if ok(app, lib.duplicate_file(&id)).is_some() {
                app.show_toast("Duplicated", false);
            }
        }
        Act::MoveTo(id) => {
            let mut d = Dialog::pick(Intent::MoveFile(id.clone()), "Move to\u{2026}", "Move", &lib);
            if let Ok(f) = lib.file(&id) {
                d.pick_where(&f.project, f.folder.as_deref());
            }
            app.dialog = Some(d);
            changed = false;
        }
        Act::Star(id, on) => {
            ok(app, lib.star(&id, on));
        }
        Act::Reveal(id) => {
            changed = false;
            if let Ok(f) = lib.file(&id) {
                let dir = lib.path("projects").join(&f.project).join("files");
                if let Err(e) = reveal(&dir) {
                    app.show_toast(e, true);
                }
            }
        }
        Act::Trash(id) => {
            let name = lib.file(&id).map(|f| f.name).unwrap_or_default();
            if ok(app, lib.trash_file(&id)).is_some() {
                if app.lib_file.as_ref().is_some_and(|f| f.id == id) {
                    app.lib_file = None;
                }
                app.show_toast(format!("Moved {name} to the trash"), false);
            }
        }
        Act::Restore(id) => {
            if ok(app, lib.restore(&id)).is_some() {
                app.show_toast("Restored", false);
            }
        }
        Act::EmptyTrash => {
            let n = app.home.as_ref().map(|h| h.snap.trash.len()).unwrap_or(0);
            let body = if n == 1 { "1 item will be permanently deleted." } else { "These items will be permanently deleted." };
            let mut d = Dialog::confirm(Intent::EmptyTrash, "Empty the trash?", &[body, "This cannot be undone."], "Empty trash");
            d.danger = true;
            app.dialog = Some(d);
            changed = false;
        }
        Act::Drop(id, to) => {
            let (project, folder) = match to {
                DropTo::Project(p) => (p, None),
                DropTo::Folder(p, f) => (p, Some(f)),
            };
            if ok(app, lib.move_file(&id, &project, folder.as_deref())).is_some() {
                if let Some(lf) = app.lib_file.as_mut().filter(|f| f.id == id) {
                    lf.project_id = project.clone();
                    lf.project = lib.project(&project).map(|p| p.name).unwrap_or_default();
                }
                app.show_toast("Moved", false);
            }
        }
    }
    if changed {
        if let Some(h) = app.home.as_mut() {
            h.dirty = true;
        }
    }
}

/// Opens a folder in the system file manager.
pub fn reveal(dir: &std::path::Path) -> Result<(), String> {
    #[cfg(feature = "open-url")]
    {
        open::that(dir).map_err(|e| format!("Cannot open {}: {e}", dir.display()))
    }
    #[cfg(not(feature = "open-url"))]
    {
        Err(format!("The file is in {}", dir.display()))
    }
}

/// Opens a web link in the browser.
pub fn open_link(url: &str) -> Result<(), String> {
    #[cfg(feature = "open-url")]
    {
        open::that(url).map_err(|e| format!("Could not open link: {e}"))
    }
    #[cfg(not(feature = "open-url"))]
    {
        Err(format!("Open {url} in your browser"))
    }
}
