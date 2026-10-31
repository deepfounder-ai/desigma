//! Watching the open document for changes made by other programs.
//!
//! desigma reads a file once when it opens it. Without this module a file rewritten on disk
//! (by a code generator, a sync client, or a second editor) stays invisible until the user
//! reopens it — and a stray Cmd-S in the editor silently overwrites the newer file.
//!
//! We poll the file's stamp (mtime + size) once a second, which is cheap and works on every
//! platform without extra dependencies:
//! * no unsaved edits -> reload silently and say so in a toast;
//! * unsaved edits -> ask: Reload (throw mine away) / Keep mine (stop watching this version).

use crate::app::App;
use egui::{Align2, Color32, Context, Id, Order, Pos2, Rect, Sense, Vec2};
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

    /// Keep the in-memory document; stop treating the on-disk version as news.
    pub fn keep_local_changes(&mut self) {
        if let Some(w) = self.ext_watch.as_mut() {
            w.asked = false;
            w.next_check = Instant::now() + Duration::from_millis(1000);
        }
    }
}

/// The "this file changed on disk" question, shown while an external change meets unsaved edits.
pub fn dialog(app: &mut App, ctx: &Context) {
    let Some(w) = app.ext_watch.as_ref() else { return };
    if !w.asked {
        return;
    }
    let path = w.path.clone();
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let screen = ctx.content_rect();
    const W: f32 = 460.0;
    const PAD: f32 = 20.0;
    let body = "It was changed by another program while you had unsaved edits. Reloading throws your edits away; keeping yours stops this prompt, but the next save overwrites the file on disk.";
    let rect = Rect::from_center_size(screen.center() - Vec2::new(0.0, 24.0), Vec2::new(W, PAD + 24.0 + 12.0 + 56.0 + 32.0 + PAD));
    let mut answer: Option<Answer> = None;
    egui::Area::new(Id::new("external-change")).order(Order::Foreground).fixed_pos(screen.min).interactable(true).show(ctx, |ui| {
        ui.allocate_rect(screen, Sense::click_and_drag());
        let p = ui.painter().clone();
        p.rect_filled(screen, 0.0, Color32::from_black_alpha(140));
        crate::widgets::menu_panel(&p, rect, crate::theme::MENU_RADIUS);
        let x0 = rect.left() + PAD;
        let y = rect.top() + PAD;
        p.text(Pos2::new(x0, y + 12.0), Align2::LEFT_CENTER, crate::i18n::tr("File changed on disk"), crate::theme::font_large_strong(), crate::theme::TEXT);
        let g = p.layout(format!("{name} — {}", crate::i18n::tr(body)), crate::theme::font_body(), crate::theme::TEXT_SECONDARY, W - PAD * 2.0);
        p.galley(Pos2::new(x0, y + 36.0), g, crate::theme::TEXT_SECONDARY);
        let by = rect.bottom() - PAD - 32.0;
        let mut right = rect.right() - PAD;
        let buttons = [
            ("ext-keep", "Keep my changes", false, Answer::Keep),
            ("ext-reload", "Reload", true, Answer::Reload),
        ];
        for (id, text, primary, a) in buttons {
            let w = crate::comments::ui::button_width(&p, text) + 16.0;
            let r = Rect::from_min_size(Pos2::new(right - w, by), Vec2::new(w, 32.0));
            if crate::comments::ui::button(ui, r, id, text, primary, true).clicked() {
                answer = Some(a);
            }
            crate::widgets::record(ui, id, r);
            right = r.left() - 8.0;
        }
    });
    match answer {
        Some(Answer::Reload) => app.reload_watched(),
        Some(Answer::Keep) => app.keep_local_changes(),
        None => {}
    }
}

enum Answer {
    Reload,
    Keep,
}
