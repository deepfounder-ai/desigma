//! Modal dialogs shared by the home screen and the editor: name prompts,
//! confirmations, the project picker, keyboard shortcuts and About.

use super::Section;
use crate::app::App;
use crate::theme;
use crate::widgets::{self, menu_panel};
use desigma_library::Library;
use egui::{Align2, Color32, Context, Id, Key, Modifiers, Order, Pos2, Rect, Sense, Vec2};

#[derive(Clone, Debug, PartialEq)]
pub enum Intent {
    NewProject,
    RenameProject(String),
    NewFolder(String, Option<String>),
    RenameFolder(String, String),
    RenameFile(String),
    MoveFile(String),
    /// Store the open document as a new library file.
    SaveToLibrary,
    EmptyTrash,
    /// Settings: the name comments are posted under.
    AuthorName,
    /// Settings: the Agents panel's server URL.
    AgentsUrl,
    /// Names the version history entry with this number.
    NameVersion(u32),
    OpenGithub,
    /// Opens a dev link (annotation / Dev Mode link) after confirmation.
    OpenDevLink(String),
    Shortcuts,
    About,
    /// Leaving the open file while sharing it or joined to it.
    LeaveSession,
}

/// One row of the project picker: a project or one of its folders.
#[derive(Clone, Debug, PartialEq)]
pub struct Choice {
    pub project: String,
    pub folder: Option<String>,
    pub label: String,
    pub depth: usize,
}

#[derive(Clone, Debug)]
pub struct Dialog {
    pub intent: Intent,
    pub title: String,
    /// Body rows: `(text, "")` is a wrapped paragraph, `(key, text)` a two-column row.
    pub lines: Vec<(String, String)>,
    /// Text field content (for prompts).
    pub input: Option<String>,
    pub choices: Vec<Choice>,
    pub picked: Option<usize>,
    pub ok: String,
    pub danger: bool,
    /// No Cancel button (information only).
    pub info: bool,
    fresh: bool,
    scroll: f32,
}

const W: f32 = 440.0;
const PAD: f32 = 20.0;
const BTN_H: f32 = 32.0;
const ROW_H: f32 = 28.0;
const MAX_CHOICES: usize = 8;

impl Dialog {
    fn base(intent: Intent, title: &str, ok: &str) -> Self {
        Self {
            intent,
            title: title.into(),
            lines: vec![],
            input: None,
            choices: vec![],
            picked: None,
            ok: ok.into(),
            danger: false,
            info: false,
            fresh: true,
            scroll: 0.0,
        }
    }

    pub fn prompt(intent: Intent, title: &str, initial: &str, ok: &str) -> Self {
        Self { input: Some(initial.into()), ..Self::base(intent, title, ok) }
    }

    pub fn confirm(intent: Intent, title: &str, body: &[&str], ok: &str) -> Self {
        Self { lines: body.iter().map(|l| (l.to_string(), String::new())).collect(), ..Self::base(intent, title, ok) }
    }

    pub fn info(intent: Intent, title: &str, lines: Vec<(String, String)>) -> Self {
        Self { lines, info: true, ..Self::base(intent, title, "Close") }
    }

    /// A project / folder picker listing every project and its folder tree.
    pub fn pick(intent: Intent, title: &str, ok: &str, lib: &Library) -> Self {
        Self { choices: choices(lib), ..Self::base(intent, title, ok) }
    }

    /// Preselects a project (and folder) in the picker.
    pub fn pick_where(&mut self, project: &str, folder: Option<&str>) {
        self.picked = self.choices.iter().position(|c| c.project == project && c.folder.as_deref() == folder);
    }

    pub fn can_submit(&self) -> bool {
        self.info || (self.input.as_ref().is_none_or(|t| !t.trim().is_empty()) && (self.choices.is_empty() || self.picked.is_some()))
    }
}

/// Projects with their folders, depth-first, for the picker.
pub fn choices(lib: &Library) -> Vec<Choice> {
    fn walk(out: &mut Vec<Choice>, project: &desigma_library::Project, parent: Option<&str>, depth: usize) {
        let mut kids: Vec<_> = project.folders.iter().filter(|f| f.parent.as_deref() == parent).collect();
        kids.sort_by_key(|f| f.name.to_lowercase());
        for f in kids {
            out.push(Choice { project: project.id.clone(), folder: Some(f.id.clone()), label: f.name.clone(), depth });
            walk(out, project, Some(&f.id), depth + 1);
        }
    }
    let mut out = vec![];
    for p in lib.list_projects() {
        out.push(Choice { project: p.id.clone(), folder: None, label: p.name.clone(), depth: 0 });
        walk(&mut out, &p, None, 1);
    }
    out
}

impl App {
    /// File -> "Save to library...": pick a project for the open document.
    pub fn begin_save_to_library(&mut self) {
        crate::decks::ui::flush_notes(self);
        let Some(lib) = self.lib.clone() else {
            self.show_toast("The projects library is unavailable", true);
            return;
        };
        let name = if self.doc.name.trim().is_empty() { "Untitled".to_string() } else { self.doc.name.clone() };
        let mut d = Dialog::pick(Intent::SaveToLibrary, "Save to library", "Save", &lib);
        d.input = Some(name);
        if d.choices.is_empty() {
            match lib.create_project("My project") {
                Ok(_) => d.choices = choices(&lib),
                Err(e) => {
                    self.show_toast(e.to_string(), true);
                    return;
                }
            }
        }
        d.picked = Some(0);
        self.dialog = Some(d);
    }

    pub fn show_shortcuts(&mut self) {
        let rows = [
            ("Ctrl+S", "Save"),
            ("Ctrl+Shift+S", "Save as\u{2026}"),
            ("Ctrl+O", "Open a file"),
            ("Ctrl+K", "Command palette"),
            ("Ctrl+Z / Ctrl+Shift+Z", "Undo / redo"),
            ("Ctrl+D", "Duplicate"),
            ("Ctrl+G / Ctrl+Shift+G", "Group / ungroup"),
            ("Shift+A", "Add auto layout"),
            ("Shift+1 / Shift+2", "Zoom to fit / selection"),
            ("Shift+0", "Zoom to 100%"),
            ("Ctrl+\\", "Show or hide the UI"),
            ("Ctrl+Alt+Enter", "Present"),
        ];
        let lines = rows.iter().map(|(k, a)| (k.to_string(), a.to_string())).collect();
        self.dialog = Some(Dialog::info(Intent::Shortcuts, "Keyboard shortcuts", lines));
    }

    pub fn show_about(&mut self) {
        let mut lines = vec![
            ("desigma is a free, open-source design editor written in Rust.".to_string(), String::new()),
            (
                if crate::RELEASE_BUILD {
                    format!("Version {}", crate::VERSION)
                } else {
                    format!("Version {} (development build, it does not check for updates)", crate::VERSION)
                },
                String::new(),
            ),
            ("There are no accounts, plans or cloud. Your files stay on this computer.".to_string(), String::new()),
        ];
        if let Some(lib) = &self.lib {
            lines.push((format!("Library: {}", lib.root().display()), String::new()));
        }
        self.dialog = Some(Dialog::info(Intent::About, "About desigma", lines));
    }

    pub fn confirm_open_github(&mut self) {
        self.dialog = Some(Dialog::confirm(
            Intent::OpenGithub,
            "Open the GitHub repository?",
            &["This opens the desigma source code page in your web browser.", super::GITHUB_URL],
            "Open",
        ));
    }
}

/// Draws the open dialog (if any) and runs its outcome. Call once per frame.
pub fn show(app: &mut App, ctx: &Context) {
    let Some(mut d) = app.dialog.take() else { return };
    let screen = ctx.content_rect();
    let font = theme::font_body();
    let (enter, esc) = ctx.input_mut(|i| (i.consume_key(Modifiers::NONE, Key::Enter), i.consume_key(Modifiers::NONE, Key::Escape)));

    // ---- measure
    let measure = ctx.layer_painter(egui::LayerId::new(Order::Foreground, Id::new("dialog-measure")));
    let text_w = W - PAD * 2.0;
    let mut lines_h = 0.0;
    for (a, b) in &d.lines {
        lines_h += if b.is_empty() { measure.layout(a.clone(), font.clone(), Color32::WHITE, text_w).size().y + 8.0 } else { 22.0 };
    }
    let visible_choices = d.choices.len().min(MAX_CHOICES);
    let mut h = PAD + 24.0 + 12.0 + lines_h;
    if d.input.is_some() {
        h += 32.0 + 12.0;
    }
    if !d.choices.is_empty() {
        h += visible_choices as f32 * ROW_H + 12.0;
    }
    h += BTN_H + PAD;
    let rect = Rect::from_center_size(screen.center() - Vec2::new(0.0, 24.0), Vec2::new(W, h));

    let mut submit = enter && d.can_submit();
    let mut cancel = esc;
    egui::Area::new(Id::new("dialog")).order(Order::Foreground).fixed_pos(screen.min).interactable(true).show(ctx, |ui| {
        let scrim = ui.allocate_rect(screen, Sense::click_and_drag());
        let p = ui.painter().clone();
        p.rect_filled(screen, 0.0, Color32::from_black_alpha(140));
        if scrim.clicked() && !rect.contains(scrim.interact_pointer_pos().unwrap_or(Pos2::ZERO)) {
            cancel = true;
        }
        menu_panel(&p, rect, theme::MENU_RADIUS);
        let x0 = rect.left() + PAD;
        let mut y = rect.top() + PAD;
        p.text(Pos2::new(x0, y + 12.0), Align2::LEFT_CENTER, &d.title, theme::font_large_strong(), theme::TEXT);
        y += 24.0 + 12.0;
        for (a, b) in &d.lines {
            if b.is_empty() {
                let galley = p.layout(a.clone(), font.clone(), theme::TEXT_SECONDARY, text_w);
                let hh = galley.size().y;
                p.galley(Pos2::new(x0, y), galley, theme::TEXT_SECONDARY);
                y += hh + 8.0;
            } else {
                p.text(Pos2::new(x0, y + 11.0), Align2::LEFT_CENTER, a, font.clone(), theme::TEXT);
                p.text(Pos2::new(x0 + 190.0, y + 11.0), Align2::LEFT_CENTER, b, font.clone(), theme::TEXT_SECONDARY);
                y += 22.0;
            }
        }
        if let Some(text) = d.input.as_mut() {
            let r = Rect::from_min_size(Pos2::new(x0, y), Vec2::new(text_w, 32.0));
            p.rect_filled(r, theme::cr(theme::RADIUS_MEDIUM), theme::BG_SECONDARY);
            let id = Id::new("dialog-input");
            let te = egui::TextEdit::singleline(text)
                .id(id)
                .font(font.clone())
                .text_color(theme::TEXT)
                .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(10, 0)))
                .vertical_align(egui::Align::Center)
                .min_size(r.size())
                .desired_width(r.width());
            let out = ui.put(r, te);
            if d.fresh {
                out.request_focus();
                let mut state = egui::TextEdit::load_state(ui.ctx(), id).unwrap_or_default();
                let n = text.chars().count();
                state.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(n))));
                state.store(ui.ctx(), id);
                d.fresh = false;
            }
            y += 32.0 + 12.0;
        }
        if !d.choices.is_empty() {
            let list = Rect::from_min_size(Pos2::new(x0, y), Vec2::new(text_w, visible_choices as f32 * ROW_H));
            p.rect_filled(list, theme::cr(theme::RADIUS_MEDIUM), theme::BG_SECONDARY);
            let content = d.choices.len() as f32 * ROW_H;
            crate::panels::wheel_scroll(ui, list, &mut d.scroll, content);
            let scroll = d.scroll;
            let picked = d.picked;
            let mut new_pick = None;
            crate::panels::clipped(ui, list, |ui| {
                let p = ui.painter().clone();
                for (i, c) in d.choices.iter().enumerate() {
                    let row = Rect::from_min_size(Pos2::new(list.left(), list.top() + i as f32 * ROW_H - scroll), Vec2::new(list.width(), ROW_H));
                    if row.bottom() < list.top() || row.top() > list.bottom() {
                        continue;
                    }
                    let r = ui.interact(row, Id::new(("dialog-choice", i)), Sense::click());
                    if picked == Some(i) {
                        p.rect_filled(row.shrink2(Vec2::new(4.0, 1.0)), theme::cr(theme::RADIUS_MEDIUM), theme::BG_BRAND);
                    } else if r.hovered() {
                        p.rect_filled(row.shrink2(Vec2::new(4.0, 1.0)), theme::cr(theme::RADIUS_MEDIUM), theme::BG_HOVER);
                    }
                    let ix = row.left() + 12.0 + c.depth as f32 * 16.0;
                    crate::icons::paint(
                        &p,
                        if c.folder.is_some() { crate::icons::Icon::Folder } else { crate::icons::Icon::Frame },
                        Pos2::new(ix + 8.0, row.center().y),
                        16.0,
                        theme::ICON,
                    );
                    p.text(Pos2::new(ix + 22.0, row.center().y), Align2::LEFT_CENTER, &c.label, font.clone(), theme::TEXT);
                    if r.clicked() {
                        new_pick = Some(i);
                    }
                    if r.double_clicked() {
                        new_pick = Some(i);
                        submit = true;
                    }
                }
            });
            if new_pick.is_some() {
                d.picked = new_pick;
            }
            y += list.height() + 12.0;
        }
        // ---- buttons
        let ok_w = widgets::text_width(&p, &d.ok, &theme::font_body_strong()).max(48.0) + 32.0;
        let ok_r = Rect::from_min_size(Pos2::new(rect.right() - PAD - ok_w, y), Vec2::new(ok_w, BTN_H));
        let enabled = d.can_submit();
        let resp = ui.interact(ok_r, Id::new("dialog-ok"), if enabled { Sense::click() } else { Sense::hover() });
        let (base, hover, press) =
            if d.danger { (theme::DANGER, theme::DANGER, theme::DANGER) } else { (theme::BG_BRAND, theme::BG_BRAND_HOVER, theme::BG_BRAND_HOVER) };
        let bg = if !enabled {
            theme::BG_TERTIARY
        } else if resp.is_pointer_button_down_on() {
            press
        } else if resp.hovered() {
            hover
        } else {
            base
        };
        p.rect_filled(ok_r, theme::cr(theme::RADIUS_MEDIUM), bg);
        p.text(ok_r.center(), Align2::CENTER_CENTER, &d.ok, theme::font_body_strong(), if enabled { Color32::WHITE } else { theme::TEXT_DISABLED });
        widgets::record(ui, "dialog-ok", ok_r);
        if resp.clicked() {
            submit = true;
        }
        if !d.info {
            let cw = widgets::text_width(&p, "Cancel", &font) + 32.0;
            let cr = Rect::from_min_size(Pos2::new(ok_r.left() - 8.0 - cw, y), Vec2::new(cw, BTN_H));
            let cresp = ui.interact(cr, Id::new("dialog-cancel"), Sense::click());
            widgets::button_bg(&p, cr, &cresp, false, theme::cr(theme::RADIUS_MEDIUM));
            p.text(cr.center(), Align2::CENTER_CENTER, "Cancel", font.clone(), theme::TEXT);
            widgets::record(ui, "dialog-cancel", cr);
            if cresp.clicked() {
                cancel = true;
            }
        }
    });

    if d.info && (submit || cancel) {
        return;
    }
    if cancel {
        return;
    }
    if submit && d.can_submit() {
        finish(app, d);
        return;
    }
    app.dialog = Some(d);
}

/// Runs the confirmed dialog.
pub fn finish(app: &mut App, d: Dialog) {
    if crate::history::finish_dialog(app, &d)
        || crate::dev::finish_dialog(app, &d)
        || crate::collab::finish_dialog(app, &d)
        || crate::agents::finish_dialog(app, &d)
    {
        return;
    }
    let Some(lib) = app.lib.clone() else { return };
    let text = d.input.clone().unwrap_or_default().trim().to_string();
    let choice = d.picked.and_then(|i| d.choices.get(i)).cloned();
    let mut dirty = true;
    let res: desigma_library::Result<()> = match d.intent.clone() {
        Intent::NewProject => lib.create_project(&text).map(|p| {
            if let Some(h) = app.home.as_mut() {
                h.section = Section::Project(p.id, None);
                h.search.clear();
            }
        }),
        Intent::RenameProject(id) => lib.rename_project(&id, &text).map(|_| {
            if let Some(f) = app.lib_file.as_mut().filter(|f| f.project_id == id) {
                f.project = text.clone();
            }
        }),
        Intent::NewFolder(p, parent) => lib.create_folder(&p, parent.as_deref(), &text).map(|f| {
            if let Some(h) = app.home.as_mut() {
                h.collapsed.remove(&p);
                h.section = Section::Project(p, Some(f.id));
            }
        }),
        Intent::RenameFolder(p, f) => lib.rename_folder(&p, &f, &text),
        Intent::RenameFile(id) => lib.rename_file(&id, &text).map(|_| {
            if app.lib_file.as_ref().is_some_and(|f| f.id == id) {
                app.rename_library_file(&text);
            }
        }),
        Intent::MoveFile(id) => match choice {
            Some(c) => lib.move_file(&id, &c.project, c.folder.as_deref()).map(|_| {
                if let Some(lf) = app.lib_file.as_mut().filter(|f| f.id == id) {
                    lf.project_id = c.project.clone();
                    lf.project = lib.project(&c.project).map(|p| p.name).unwrap_or_default();
                }
            }),
            None => Ok(()),
        },
        Intent::SaveToLibrary => {
            dirty = false;
            if let Some(c) = choice {
                app.save_to_library_as(&c.project, c.folder.as_deref(), &text);
            }
            Ok(())
        }
        Intent::EmptyTrash => lib.empty_trash().inspect(|_| app.show_toast("Trash emptied", false)),
        Intent::OpenGithub => {
            dirty = false;
            if let Err(e) = super::open_link(super::GITHUB_URL) {
                app.show_toast(e, true);
            }
            Ok(())
        }
        Intent::Shortcuts | Intent::About | Intent::LeaveSession | Intent::AuthorName | Intent::AgentsUrl | Intent::NameVersion(_) | Intent::OpenDevLink(_) => {
            dirty = false;
            Ok(())
        }
    };
    if let Err(e) = res {
        app.show_toast(e.to_string(), true);
    }
    if dirty {
        if let Some(h) = app.home.as_mut() {
            h.dirty = true;
        }
    }
}
