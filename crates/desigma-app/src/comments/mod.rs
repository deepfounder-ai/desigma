//! Comments: local, single-user notes pinned to the canvas.
//!
//! Threads live in `Document::comments` and every change goes through
//! `Command::SetComments`, one undo step each. The author name is an app setting
//! ("Your name"). See `model` for the pure logic, `pins` for the canvas tool and
//! pins, `thread` for the popovers and `panel` for the right-hand list.
//!
//! Two plain-text conventions need no schema: an opening message that starts with
//! `[ ]`, `[/]` or `[x]` makes the thread a to-do, in-progress or done task (pins take
//! the status colour), and `@name` tokens are shown as mentions.

pub mod fixture;
pub mod import;
pub mod model;
pub mod panel;
pub mod pins;
pub mod settings;
pub mod thread;
pub mod time;
pub mod ui;

use crate::app::App;
use crate::canvas::tools::Tool;
use desigma_doc::{Command, CommentAnchor, CommentThread};

/// A new thread being written: pinned, but not posted yet.
pub struct Composer {
    pub anchor: CommentAnchor,
    pub text: String,
    pub fresh: bool,
}

/// A message being edited in place.
pub struct Editing {
    pub thread: String,
    pub comment: String,
    pub text: String,
    pub fresh: bool,
}

/// The "…" menu of one message.
pub struct MsgMenu {
    pub thread: String,
    pub comment: String,
    pub anchor: egui::Rect,
}

#[derive(Default)]
pub struct CommentsState {
    /// The author name (loaded lazily from the settings).
    pub author: Option<String>,
    /// Draw resolved threads on the canvas too.
    pub show_resolved: bool,
    /// The panel lists every page instead of the current one.
    pub all_pages: bool,
    /// The panel lists resolved threads instead of open ones.
    pub list_resolved: bool,
    pub search: String,
    pub composer: Option<Composer>,
    /// Id of the thread whose popover is open.
    pub open: Option<String>,
    pub reply: String,
    pub editing: Option<Editing>,
    pub menu: Option<MsgMenu>,
    pub scroll: f32,
    /// Focus the reply box on the next frame (after opening a thread).
    pub focus_reply: bool,
    /// Pin rects drawn last frame, so the canvas click can tell a pin from empty space.
    pub pin_rects: Vec<egui::Rect>,
    /// The press that closed a popover must not also start a new comment on release.
    pub swallow_click: bool,
}

impl CommentsState {
    /// A popover is open and handles Escape.
    pub fn captures_escape(&self) -> bool {
        self.composer.is_some() || self.open.is_some()
    }

    pub fn close_popovers(&mut self) {
        self.composer = None;
        self.open = None;
        self.editing = None;
        self.menu = None;
        self.reply.clear();
    }
}

impl App {
    /// The name comments are posted under: the saved setting, else the OS user name.
    pub fn author(&mut self) -> String {
        if let Some(a) = self.comments.author.clone() {
            return a;
        }
        let name = self.lib.as_ref().and_then(|l| settings::load_author(l.root())).unwrap_or_else(settings::os_user_name);
        self.comments.author = Some(name.clone());
        name
    }

    /// Sets (and stores) the author name; an empty name goes back to the OS user name.
    pub fn set_author(&mut self, name: &str) {
        let name = name.trim();
        if let Some(lib) = &self.lib {
            if let Err(e) = settings::save_author(lib.root(), name) {
                self.show_toast(e, true);
            }
        }
        self.comments.author = Some(if name.is_empty() { settings::os_user_name() } else { name.to_string() });
    }

    /// Opens the "Your name" prompt.
    pub fn ask_author_name(&mut self) {
        let current = self.author();
        self.dialog = Some(crate::home::Dialog::prompt(crate::home::Intent::AuthorName, "Your name", &current, "Save"));
    }

    /// Replaces all threads as one undo step.
    fn commit_comments(&mut self, new: Vec<CommentThread>) -> bool {
        if self.history.viewing.is_some() {
            self.show_toast("Viewing an old version is read-only", true);
            return false;
        }
        if new == self.doc.comments {
            return false;
        }
        match self.hist.exec(&mut self.doc, Command::SetComments { comments: new }) {
            Ok(()) => {
                self.changed();
                true
            }
            Err(e) => {
                self.show_toast(format!("Edit failed: {e}"), true);
                false
            }
        }
    }

    /// Posts a new thread at `anchor`; returns its id.
    pub fn post_comment(&mut self, anchor: CommentAnchor, text: &str) -> Option<String> {
        if text.trim().is_empty() {
            return None;
        }
        let author = self.author();
        let id = desigma_library::new_id();
        let t = CommentThread { id: id.clone(), anchor, messages: vec![model::comment(desigma_library::new_id(), &author, text)], resolved: false };
        let new = model::add_thread(&self.doc.comments, t);
        self.commit_comments(new).then_some(id)
    }

    pub fn reply_comment(&mut self, thread: &str, text: &str) -> bool {
        if text.trim().is_empty() {
            return false;
        }
        let author = self.author();
        let c = model::comment(desigma_library::new_id(), &author, text);
        match model::reply(&self.doc.comments, thread, c) {
            Some(new) => self.commit_comments(new),
            None => false,
        }
    }

    pub fn edit_comment_text(&mut self, thread: &str, comment: &str, text: &str) -> bool {
        match model::edit_comment(&self.doc.comments, thread, comment, text) {
            Some(new) => self.commit_comments(new),
            None => false,
        }
    }

    /// Sets the task marker of the thread's opening message: an ordinary text edit,
    /// so it is one undo step and syncs like any other.
    pub fn set_comment_task(&mut self, thread: &str, status: model::TaskStatus) -> bool {
        let Some(first) = self.doc.comments.iter().find(|t| t.id == thread).and_then(|t| t.messages.first()) else { return false };
        let (id, text) = (first.id.clone(), model::with_status(&first.text, status));
        self.edit_comment_text(thread, &id, &text)
    }

    pub fn delete_comment_message(&mut self, thread: &str, comment: &str) -> bool {
        match model::delete_comment(&self.doc.comments, thread, comment) {
            Some(new) => self.commit_comments(new),
            None => false,
        }
    }

    pub fn delete_comment_thread(&mut self, thread: &str) -> bool {
        match model::delete_thread(&self.doc.comments, thread) {
            Some(new) => self.commit_comments(new),
            None => false,
        }
    }

    pub fn resolve_comment_thread(&mut self, thread: &str, resolved: bool) -> bool {
        match model::set_resolved(&self.doc.comments, thread, resolved) {
            Some(new) => self.commit_comments(new),
            None => false,
        }
    }

    /// Switches to the thread's page, centres its pin and opens it.
    pub fn goto_comment(&mut self, id: &str) {
        let Some(t) = self.doc.comments.iter().find(|t| t.id == id).cloned() else { return };
        let page = model::thread_page(&self.doc, &t);
        if page != self.page {
            self.switch_page(page);
        }
        if let Some(p) = model::pin_point(&self.doc, &t.anchor) {
            let z = self.view.zoom.max(0.5);
            let size = self.canvas_rect.size();
            self.view.zoom = z;
            // the bubble sits above-right of its tip: centre on the bubble
            let bubble = crate::theme::COMMENT_PIN / 2.0 / z;
            self.view.offset = desigma_doc::Vec2::new(p.x + bubble - size.x / 2.0 / z, p.y - bubble - size.y / 2.0 / z);
            self.changed();
        }
        self.comments.composer = None;
        self.comments.editing = None;
        self.comments.reply.clear();
        self.comments.open = Some(id.to_string());
        self.comments.focus_reply = true;
    }

    /// Merges a Figma REST comments file (`GET /v1/files/:key/comments`) into the document.
    pub fn import_comments_text(&mut self, text: &str) -> Result<import::Merged, String> {
        let (threads, skipped) = import::parse_threads(&self.doc, text)?;
        let (merged, mut summary) = import::merge(&self.doc.comments, threads);
        summary.skipped = skipped;
        if summary.threads + summary.replies > 0 {
            self.commit_comments(merged);
        }
        Ok(summary)
    }

    pub fn import_comments_file(&mut self, path: &std::path::Path) {
        let result = std::fs::read_to_string(path).map_err(|e| e.to_string()).and_then(|t| self.import_comments_text(&t));
        match result {
            Ok(m) if m.threads + m.replies == 0 => self.show_toast("Those comments are already in this file", false),
            Ok(m) => {
                let skipped = if m.skipped > 0 { format!(" ({} skipped)", m.skipped) } else { String::new() };
                self.show_toast(format!("Imported {} comment threads, {} replies{skipped}", m.threads, m.replies), false)
            }
            Err(e) => self.show_toast(format!("Cannot import comments: {e}"), true),
        }
    }

    /// The right panel lists comments while the comment tool is active.
    pub fn comment_mode(&self) -> bool {
        self.tool == Tool::Comment
    }
}

/// `--panel comments`: comment mode with the first thread open (for screenshots).
pub fn preset(app: &mut App) {
    app.set_tool(Tool::Comment);
    app.comments.open = app.doc.comments.first().map(|t| t.id.clone());
}

#[cfg(test)]
mod tests;
