//! Non-blocking native file dialogs so Wayland/winit event pumping continues
//! while `xdg-desktop-portal` or OS file pickers are open.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, TryRecvError};

use crate::app::App;

enum DialogResult {
    Single(Option<PathBuf>),
    Multi(Vec<PathBuf>),
}

type DialogCallback = Box<dyn FnOnce(&mut App, DialogResult)>;

/// Active background file dialog awaiting a user choice.
pub struct PendingFileDialog {
    rx: Receiver<DialogResult>,
    on_done: DialogCallback,
}

impl App {
    /// Opens a single-file or folder picker on a background thread and invokes
    /// `on_done` on the UI thread when the user confirms a path.
    pub fn pick_file_async<F, C>(&mut self, dialog: F, on_done: C)
    where
        F: FnOnce() -> Option<PathBuf> + Send + 'static,
        C: FnOnce(&mut App, PathBuf) + 'static,
    {
        self.spawn_file_dialog(
            move || DialogResult::Single(dialog()),
            move |app, res| {
                if let DialogResult::Single(Some(path)) = res {
                    on_done(app, path);
                }
            },
        );
    }

    /// Opens a multi-file picker on a background thread and invokes `on_done`
    /// on the UI thread when at least one path is selected.
    pub fn pick_files_async<F, C>(&mut self, dialog: F, on_done: C)
    where
        F: FnOnce() -> Vec<PathBuf> + Send + 'static,
        C: FnOnce(&mut App, Vec<PathBuf>) + 'static,
    {
        self.spawn_file_dialog(
            move || DialogResult::Multi(dialog()),
            move |app, res| {
                if let DialogResult::Multi(paths) = res {
                    if !paths.is_empty() {
                        on_done(app, paths);
                    }
                }
            },
        );
    }

    fn spawn_file_dialog<F, C>(&mut self, dialog: F, on_done: C)
    where
        F: FnOnce() -> DialogResult + Send + 'static,
        C: FnOnce(&mut App, DialogResult) + 'static,
    {
        if self.file_dialog.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(dialog());
        });
        self.file_dialog = Some(PendingFileDialog { rx, on_done: Box::new(on_done) });
    }

    /// Polls any active background file dialog and dispatches its callback.
    pub fn poll_file_dialog(&mut self, ctx: &egui::Context) {
        let Some(pending) = self.file_dialog.as_ref() else { return };
        match pending.rx.try_recv() {
            Ok(result) => {
                let Some(pending) = self.file_dialog.take() else { return };
                (pending.on_done)(self, result);
                ctx.request_repaint();
            }
            Err(TryRecvError::Empty) => {
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
            }
            Err(TryRecvError::Disconnected) => {
                self.file_dialog = None;
            }
        }
    }
}
