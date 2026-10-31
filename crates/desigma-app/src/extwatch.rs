//! Watching the open document for changes made by other programs.
//!
//! desigma reads a file once when it opens it. Without this module a file rewritten on disk
//! (by a code generator, a sync client, or a second editor) stays invisible until the user
//! reopens it — and a stray Cmd-S in the editor silently overwrites the newer file.
//!
//! We poll the file's stamp (mtime + size) once a second, which is cheap and works on every
//! platform without extra dependencies:
//! * no unsaved edits -> reload silently and say so in a toast;
//! * unsaved edits -> ask: Reload (throw mine away) / Keep mine (stop watching this
//!   version) / Keep both (write mine to a "(my copy)" file, then take the newer one).

use crate::app::App;
use egui::{Align2, Context, Id, Order, Pos2, Rect, Vec2};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

/// Identity of a file version: modification time plus length (both change on a rewrite).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Stamp {
    mtime: Option<SystemTime>,
    len: u64,
}

pub fn stamp(path: &Path) -> Option<Stamp> {
    let meta = std::fs::metadata(path).ok()?;
    Some(Stamp { mtime: meta.modified().ok(), len: meta.len() })
}

/// The file we are watching, the version we last read or wrote, and whether the user has been
/// asked about a change and not answered yet.
pub struct Watch {
    pub path: PathBuf,
    pub seen: Stamp,
    pub next_check: Instant,
    pub asked: bool,
}

impl Watch {
    fn at(path: &Path) -> Option<Self> {
        let seen = stamp(path)?;
        Some(Self { path: path.to_path_buf(), seen, next_check: Instant::now(), asked: false })
    }
}

impl App {
    /// Start watching `path` as of its current contents (after opening, saving, or reloading).
    pub fn watch_external(&mut self, path: &Path) {
        // Only real files on disk can change underneath us; library files autosave instead.
        self.ext_watch = if self.lib_file.is_none() { Watch::at(path) } else { None };
    }

    pub fn stop_watching(&mut self) {
        self.ext_watch = None;
    }

    /// Called once per frame. Cheap: the stamp is only read once a second.
    pub fn poll_external_change(&mut self, ctx: &Context) {
        let Some(w) = self.ext_watch.as_ref() else { return };
        if w.asked || Instant::now() < w.next_check {
            return;
        }
        let path = w.path.clone();
        let seen = w.seen;
        if let Some(w) = self.ext_watch.as_mut() {
            w.next_check = Instant::now() + Duration::from_millis(1000);
        }
        let Some(now) = stamp(&path) else { return };
        if now == seen {
            return;
        }
        let dirty = self.edits_unsaved();
        if let Some(w) = self.ext_watch.as_mut() {
            // Remember the version we just saw so a dismissed prompt does not come back every second.
            w.seen = now;
            w.asked = dirty;
        }
        if dirty {
            ctx.request_repaint();
        } else {
            self.reload_watched();
        }
    }

    /// Reload the watched file from disk, keeping the user's viewport where it was.
    pub fn reload_watched(&mut self) {
        let Some(w) = self.ext_watch.as_ref() else { return };
        let path = w.path.clone();
        let view = self.view;
        match crate::io::load_path_noted(&path) {
            Ok((doc, _notes)) => {
                self.replace_document(doc, Some(path.clone()));
                self.view = view;
                self.show_toast(format!("Reloaded {} (changed on disk)", path.display()), false);
            }
            Err(e) => self.show_toast(e, true),
        }
        self.watch_external(&path);
    }

    /// Write the in-memory document beside the file as "<name> (my copy).desigma", then take the
    /// version on disk. The one answer that cannot lose anything.
    pub fn save_copy_and_reload(&mut self) {
        let Some(w) = self.ext_watch.as_ref() else { return };
        let path = w.path.clone();
        if let Some(w) = self.ext_watch.as_mut() {
            w.asked = false;
        }
        let copy = unique_copy_path(&path);
        match crate::io::save_path(&self.doc, &copy) {
            Ok(saved) => {
                self.show_toast(format!("Saved your edits to {}", saved.display()), false);
                self.reload_watched();
            }
            Err(e) => self.show_toast(e, true),
        }
    }

    /// Keep the in-memory document; stop treating the on-disk version as news.
    pub fn keep_local_changes(&mut self) {
        if let Some(w) = self.ext_watch.as_mut() {
            w.asked = false;
            w.next_check = Instant::now() + Duration::from_millis(1000);
        }
    }
}

/// `<dir>/<stem> (my copy).<ext>`, adding a number if that name is taken.
fn unique_copy_path(path: &Path) -> PathBuf {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "copy".into());
    let ext = path.extension().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "desigma".into());
    let mut n = 1;
    loop {
        let name = if n == 1 {
            format!("{stem} (my copy).{ext}")
        } else {
            format!("{stem} (my copy {n}).{ext}")
        };
        let candidate = dir.join(name);
        if !candidate.exists() {
            return candidate;
        }
        n += 1;
    }
}

/// The "this file changed on disk" question, shown as a card in the bottom-right corner
/// (the same place desktop apps put non-blocking notices) so it does not cover the canvas.
pub fn dialog(app: &mut App, ctx: &Context) {
    let Some(w) = app.ext_watch.as_ref() else { return };
    if !w.asked {
        return;
    }
    let path = w.path.clone();
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let screen = ctx.content_rect();
    const W: f32 = 400.0;
    const PAD: f32 = 16.0;
    const BTN: f32 = 32.0;
    let body = crate::i18n::tr("Another program changed this file on disk. You have unsaved edits.");
    let r = Rect::from_min_size(
        Pos2::new(screen.right() - W - 20.0, screen.bottom() - 20.0 - 206.0),
        Vec2::new(W, 206.0),
    );
    let mut answer: Option<Answer> = None;
    egui::Area::new(Id::new("external-change")).order(Order::Foreground).fixed_pos(r.min).interactable(true).show(ctx, |ui| {
        let p = ui.painter().clone();
        crate::widgets::elevation_shadow(&p, r, crate::theme::MENU_RADIUS);
        crate::widgets::menu_panel(&p, r, crate::theme::MENU_RADIUS);
        let x0 = r.left() + PAD;
        let y = r.top() + PAD;
        p.text(Pos2::new(x0, y + 8.0), Align2::LEFT_CENTER, crate::i18n::tr("File changed on disk"),
               crate::theme::font_body_strong(), crate::theme::TEXT);
        let g = p.layout(body.to_string(), crate::theme::font_small(), crate::theme::TEXT_SECONDARY, W - PAD * 2.0);
        p.galley(Pos2::new(x0, y + 30.0), g, crate::theme::TEXT_SECONDARY);
        let g = p.layout(name, crate::theme::font_small(), crate::theme::TEXT_TERTIARY, W - PAD * 2.0);
        p.galley(Pos2::new(x0, y + 74.0), g, crate::theme::TEXT_TERTIARY);

        // Bottom row: the answer that cannot lose anything.
        let primary = Rect::from_min_size(Pos2::new(x0, r.bottom() - PAD - BTN), Vec2::new(W - PAD * 2.0, BTN));
        if crate::comments::ui::button(ui, primary, "ext-both", "Keep both (save a copy)", true, true).clicked() {
            answer = Some(Answer::Both);
        }
        crate::widgets::record(ui, "ext-both", primary);

        // Row above: the two destructive/plain answers, right aligned.
        let by = primary.top() - 8.0 - BTN;
        let mut right = r.right() - PAD;
        for (id, text, a) in [("ext-keep", "Keep my changes", Answer::Keep), ("ext-reload", "Reload", Answer::Reload)] {
            let label = crate::i18n::tr(text);
            let w = crate::comments::ui::button_width(&p, label) + 16.0;
            let br = Rect::from_min_size(Pos2::new(right - w, by), Vec2::new(w, BTN));
            if crate::comments::ui::button(ui, br, id, label, false, true).clicked() {
                answer = Some(a);
            }
            crate::widgets::record(ui, id, br);
            right = br.left() - 8.0;
        }
    });
    match answer {
        Some(Answer::Reload) => app.reload_watched(),
        Some(Answer::Keep) => app.keep_local_changes(),
        Some(Answer::Both) => app.save_copy_and_reload(),
        None => {}
    }
}

enum Answer {
    Reload,
    Keep,
    Both,
}
