//! Opening something over a document that has work in it: File → Open, a dropped file, and a
//! file or `desigma://join/...` link handed over by the OS or a second launch all go through
//! [`App::request_open`]. When the open would throw work away, a Save / Discard / Cancel dialog
//! comes first. Library files autosave, so they never count as unsaved. "Restart to update" (see
//! `updater`) asks the same question before it closes the window.

use crate::app::App;
use crate::comments::ui::{button, button_width};
use crate::home::session::doc_hash;
use crate::instance::Open;
use crate::{theme, widgets};
use egui::{Align2, Color32, Context, Id, Key, Modifiers, Order, Pos2, Rect, Sense, Vec2};

impl App {
    /// Edits since the document was opened or last saved to a file (library files autosave).
    pub fn edits_unsaved(&self) -> bool {
        if self.lib_file.is_some() {
            return false;
        }
        match self.file_hash {
            Some(h) => h != doc_hash(&self.doc),
            None => self.hist.can_undo(),
        }
    }

    /// Whether leaving the document now would lose work: a live session or unsaved edits.
    pub fn has_unsaved_work(&self) -> bool {
        self.collab.active() || self.edits_unsaved()
    }

    /// Opens `open` now, or asks first when that would discard unsaved work.
    pub fn request_open(&mut self, open: Open) {
        if self.has_unsaved_work() {
            self.pending_open = Some(open);
        } else {
            self.perform_open(open);
        }
    }

    fn perform_open(&mut self, open: Open) {
        match open {
            Open::Link(link) => self.open_join(Some(link)),
            Open::File(path) => self.open_path(path),
        }
    }

    /// What a confirmed question leads to: the pending open, or else the restart.
    fn proceed(&mut self, open: Option<Open>) {
        match open {
            Some(open) => self.perform_open(open),
            None => self.confirm_restart(),
        }
    }
}

enum Answer {
    Save,
    Discard,
    Cancel,
}

pub fn dialog(app: &mut App, ctx: &Context) {
    let open = app.pending_open.clone();
    if open.is_none() && !app.updater.pending_restart && !app.pending_close {
        return;
    }
    let screen = ctx.content_rect();
    let esc = ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
    const W: f32 = 420.0;
    const PAD: f32 = 20.0;
    let body = match (app.collab.active(), &open, app.updater.pending_restart) {
        (true, Some(_), _) => "You are in a live session. Opening this leaves it.",
        (true, None, _) => "You are in a live session. Closing this window leaves it.",
        (false, Some(Open::Link(_)), _) => "Joining a session replaces this document.",
        (false, Some(Open::File(_)), _) => "Opening this file replaces the current document.",
        (false, None, true) => "Restarting to install the update closes this document.",
        (false, None, false) => "Closing this window discards unsaved changes.",
    };
    let rect = Rect::from_center_size(screen.center() - Vec2::new(0.0, 24.0), Vec2::new(W, PAD + 24.0 + 12.0 + 44.0 + 32.0 + PAD));
    let mut answer = esc.then_some(Answer::Cancel);
    egui::Area::new(Id::new("unsaved-open")).order(Order::Foreground).fixed_pos(screen.min).interactable(true).show(ctx, |ui| {
        ui.allocate_rect(screen, Sense::click_and_drag());
        let p = ui.painter().clone();
        p.rect_filled(screen, 0.0, Color32::from_black_alpha(140));
        widgets::menu_panel(&p, rect, theme::MENU_RADIUS);
        let x0 = rect.left() + PAD;
        let y = rect.top() + PAD;
        p.text(Pos2::new(x0, y + 12.0), Align2::LEFT_CENTER, crate::i18n::tr("Discard unsaved changes?"), theme::font_large_strong(), theme::TEXT);
        let g = p.layout(crate::i18n::tr(body).into(), theme::font_body(), theme::TEXT_SECONDARY, W - PAD * 2.0);
        p.galley(Pos2::new(x0, y + 36.0), g, theme::TEXT_SECONDARY);
        let by = rect.bottom() - PAD - 32.0;
        let mut right = rect.right() - PAD;
        let buttons = [
            ("unsaved-save", "Save", true, Answer::Save),
            ("unsaved-discard", "Discard", false, Answer::Discard),
            ("unsaved-cancel", "Cancel", false, Answer::Cancel),
        ];
        for (id, text, primary, a) in buttons {
            let w = button_width(&p, text) + 16.0;
            let r = Rect::from_min_size(Pos2::new(right - w, by), Vec2::new(w, 32.0));
            if button(ui, r, id, text, primary, true).clicked() {
                answer = Some(a);
            }
            widgets::record(ui, id, r);
            right = r.left() - 8.0;
        }
    });
    match answer {
        None => {}
        Some(Answer::Cancel) => {
            app.pending_open = None;
            app.updater.pending_restart = false;
            app.pending_close = false;
        }
        Some(Answer::Discard) => {
            let closing = app.pending_close && open.is_none() && !app.updater.pending_restart;
            app.pending_open = None;
            app.updater.pending_restart = false;
            app.pending_close = false;
            if closing {
                app.force_close = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            } else {
                app.proceed(open);
            }
        }
        Some(Answer::Save) => {
            let closing = app.pending_close && open.is_none() && !app.updater.pending_restart;
            app.pending_open = None;
            app.updater.pending_restart = false;
            app.pending_close = false;
            app.save(false);
            // a cancelled Save as… (or a failed save) keeps the work and the document
            if !app.edits_unsaved() {
                if closing {
                    app.force_close = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                } else {
                    app.proceed(open);
                }
            }
        }
    }
}
