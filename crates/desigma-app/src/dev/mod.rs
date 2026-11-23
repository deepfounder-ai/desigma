//! Dev mode: code languages, Ready for dev status, annotations, dev links and Compare changes.

pub mod annot;
pub mod color;
pub mod compare;
pub mod data;
pub mod highlight;
pub mod list;
pub mod preset;
pub mod sections;

use crate::app::App;
use crate::home::dialog::{Dialog, Intent};
use desigma_doc::*;

/// Code languages of the Inspect panel.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Language {
    #[default]
    Css,
    Tailwind,
    SwiftUi,
    Compose,
    React,
    Flutter,
    Vue,
}

impl Language {
    pub const ALL: [Language; 7] = [Language::Css, Language::Tailwind, Language::SwiftUi, Language::Compose, Language::React, Language::Flutter, Language::Vue];

    pub fn label(self) -> &'static str {
        match self {
            Self::Css => "CSS",
            Self::Tailwind => "Tailwind",
            Self::SwiftUi => "SwiftUI",
            Self::Compose => "Compose",
            Self::React => "React",
            Self::Flutter => "Flutter",
            Self::Vue => "Vue",
        }
    }

    /// Code for `node`. `react_tailwind` switches React from inline styles to Tailwind classes.
    pub fn generate(self, doc: &Document, node: NodeId, react_tailwind: bool) -> String {
        let (code, motion, comment) = match self {
            Self::Css => (desigma_export::css_for(doc, node), desigma_export::css_animation_for(doc, node), "/* Animation (Motion) */"),
            Self::Tailwind => (desigma_export::tailwind_for(doc, node), String::new(), ""),
            Self::SwiftUi => (desigma_export::swiftui_for(doc, node), desigma_export::swiftui_animation_for(doc, node), "// Animation (Motion)"),
            Self::Compose => (desigma_export::compose_for(doc, node), desigma_export::compose_animation_for(doc, node), "// Animation (Motion)"),
            Self::React => (desigma_export::react_with(doc, node, &desigma_export::ReactOptions { tailwind: react_tailwind }), String::new(), ""),
            Self::Flutter => (desigma_export::flutter_for(doc, node), String::new(), ""),
            Self::Vue => (desigma_export::vue_for(doc, node), String::new(), ""),
        };
        if motion.is_empty() {
            code
        } else {
            format!(
                "{code}

{comment}
{motion}"
            )
        }
    }
}

/// Which list the Dev mode left panel shows under the pages.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LeftTab {
    #[default]
    ReadyForDev,
    Layers,
}

/// The annotation being added or edited in the Inspect panel.
#[derive(Clone, Debug, PartialEq)]
pub struct AnnotEdit {
    pub node: NodeId,
    /// `None` adds a new annotation.
    pub index: Option<usize>,
    pub text: String,
    pub props: Vec<String>,
}

/// The dev link being added in the Inspect panel.
#[derive(Clone, Debug, PartialEq)]
pub struct LinkEdit {
    pub node: NodeId,
    pub label: String,
    pub url: String,
}

#[derive(Default)]
pub struct DevState {
    pub lang: Language,
    pub color_format: color::ColorFormat,
    pub react_tailwind: bool,
    pub left_tab: LeftTab,
    pub annot: Option<AnnotEdit>,
    pub link: Option<LinkEdit>,
    pub compare: Option<compare::CompareView>,
    /// Time of the last status refresh (seconds), to throttle fingerprinting while dragging.
    pub last_refresh: f64,
    /// The document changed since the last refresh.
    pub stale: bool,
}

/// Ready for dev items can be marked on these layers.
pub fn markable(app: &App, id: NodeId) -> bool {
    app.doc.get(id).is_some_and(|n| can_mark(&n.kind))
}

impl App {
    /// Re-evaluates the Ready for dev / Changed statuses after edits (throttled to 4 per second
    /// unless `force`).
    pub fn dev_refresh(&mut self, force: bool) {
        if !has_tracked(&self.doc) {
            self.dev.stale = false;
            return;
        }
        if !force && self.now - self.dev.last_refresh < 0.25 {
            return;
        }
        self.dev.last_refresh = self.now;
        self.dev.stale = false;
        let now = desigma_library::now_rfc3339();
        if !refresh_statuses(&mut self.doc, &now).is_empty() {
            self.history.dirty = true;
        }
    }

    /// Per-frame hook: keeps statuses current while the document changes.
    pub fn dev_tick(&mut self, ctx: &egui::Context) {
        if self.canvas.dirty && has_tracked(&self.doc) {
            self.dev.stale = true;
        }
        if self.dev.stale {
            self.dev_refresh(false);
            if self.dev.stale {
                ctx.request_repaint_after(std::time::Duration::from_millis(260));
            }
        }
    }

    fn dev_exec(&mut self, cmds: Vec<Command>) {
        if cmds.is_empty() {
            return;
        }
        if self.history.viewing.is_some() {
            self.show_toast("Viewing an old version is read-only", true);
            return;
        }
        let cmd = if cmds.len() == 1 { cmds.into_iter().next().unwrap() } else { Command::Batch(cmds) };
        match self.hist.exec(&mut self.doc, cmd) {
            Ok(()) => self.changed(),
            Err(e) => self.show_toast(format!("Edit failed: {e}"), true),
        }
    }

    /// Saves a named version of the open library file to diff against later.
    fn save_marking_version(&mut self, ids: &[NodeId]) -> Option<u32> {
        let (lib, lf) = (self.lib.clone()?, self.lib_file.clone()?);
        let names: Vec<String> = ids.iter().filter_map(|i| self.doc.get(*i)).map(|n| n.name.clone()).take(3).collect();
        let label = format!("Ready for dev: {}", names.join(", "));
        let v = lib.save_version_described(&lf.id, &self.doc, Some(label), None, false).ok()?;
        self.history.dirty = true;
        Some(v.n)
    }

    /// Marks layers Ready for dev: stores the status, a fingerprint and the comparison snapshot
    /// (a named version in the file's history for library files, else inline).
    pub fn dev_mark_ready(&mut self, ids: &[NodeId]) {
        let ids: Vec<NodeId> = ids.iter().copied().filter(|i| markable(self, *i)).collect();
        if ids.is_empty() {
            self.show_toast("Select a frame or section to mark it Ready for dev", true);
            return;
        }
        let version = if self.history.viewing.is_none() { self.save_marking_version(&ids) } else { None };
        let now = desigma_library::now_rfc3339();
        let cmds: Vec<Command> = ids.iter().filter_map(|i| mark_ready(&self.doc, *i, &now, version)).collect();
        self.dev_exec(cmds);
        self.show_toast(if ids.len() == 1 { "Marked Ready for dev".to_string() } else { format!("Marked {} layers Ready for dev", ids.len()) }, false);
    }

    pub fn dev_mark_completed(&mut self, ids: &[NodeId]) {
        let now = desigma_library::now_rfc3339();
        let cmds: Vec<Command> = ids.iter().filter(|i| markable(self, **i)).filter_map(|i| mark_completed(&self.doc, *i, &now)).collect();
        if cmds.is_empty() {
            return;
        }
        self.dev_exec(cmds);
        self.show_toast("Marked completed", false);
    }

    pub fn dev_clear_status(&mut self, ids: &[NodeId]) {
        let cmds: Vec<Command> =
            ids.iter().filter(|i| self.doc.get(**i).is_some_and(|n| n.dev.status.is_some())).filter_map(|i| clear_status(&self.doc, *i)).collect();
        self.dev_exec(cmds);
    }

    /// Edits the dev data of a layer as one undo step.
    pub fn dev_edit(&mut self, id: NodeId, f: impl FnOnce(&mut DevInfo)) {
        let Some(n) = self.doc.get(id) else { return };
        let mut dev = n.dev.clone();
        f(&mut dev);
        if dev == n.dev {
            return;
        }
        if let Some(c) = set_dev(&self.doc, id, dev) {
            self.dev_exec(vec![c]);
        }
    }

    /// Asks for confirmation, then opens a dev link in the browser.
    pub fn dev_open_link(&mut self, label: &str, url: &str) {
        if !crate::present::url_allowed(url) {
            self.show_toast("Blocked: only http(s) and mailto links open", true);
            return;
        }
        let title = if label.trim().is_empty() { "Open this link?".to_string() } else { format!("Open \"{}\"?", label.trim()) };
        self.dialog =
            Some(Dialog::confirm(Intent::OpenDevLink(url.trim().to_string()), &title, &["This opens the link in your web browser.", url.trim()], "Open"));
    }

    /// The old document to compare `id` against, and the old subtree.
    pub fn dev_snapshot(&self, id: NodeId) -> Option<(Document, Vec<Node>)> {
        let n = self.doc.get(id)?;
        if let (Some(v), Some(lib), Some(lf)) = (n.dev.snapshot_version, &self.lib, &self.lib_file) {
            if let Ok(old) = lib.load_version(&lf.id, v) {
                if old.get(id).is_some() {
                    let nodes = subtree_nodes(&old, id);
                    return Some((old, nodes));
                }
            }
        }
        let nodes = parse_snapshot(n.dev.snapshot.as_ref()?)?;
        // Put the snapshot in a copy of the document in place of the current subtree.
        let mut d = self.doc.clone();
        let parent = n.parent?;
        let index = d.get(parent)?.children.iter().position(|c| *c == id).unwrap_or(0);
        d.remove(id).ok()?;
        let mut ins = nodes.clone();
        ins.first_mut()?.parent = Some(parent);
        Command::Insert { parent, index, nodes: ins }.apply(&mut d).ok()?;
        Some((d, nodes))
    }
}

/// Runs dialogs owned by Dev mode. Returns true when `d` was one of them.
pub fn finish_dialog(app: &mut App, d: &Dialog) -> bool {
    match &d.intent {
        Intent::OpenDevLink(url) => {
            if let Err(e) = crate::home::open_link(url) {
                app.show_toast(e, true);
            }
            true
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app_with_frame() -> (App, NodeId, NodeId) {
        let mut d = Document::new("t");
        let page = d.add_page("p");
        let f = d.alloc_id();
        let f = d.insert(page, Node::frame(f, Rect::new(0.0, 0.0, 100.0, 100.0))).unwrap();
        let r = d.alloc_id();
        let r = d.insert(f, Node::rectangle(r, Rect::new(10.0, 10.0, 20.0, 20.0))).unwrap();
        (App::new(d), f, r)
    }

    #[test]
    fn mark_change_compare_and_undo() {
        let (mut app, f, r) = app_with_frame();
        app.dev_mark_ready(&[f]);
        assert_eq!(app.doc.get(f).unwrap().dev.status, Some(DevStatus::ReadyForDev));
        // no library: the snapshot is inline
        assert!(app.doc.get(f).unwrap().dev.snapshot.is_some());
        app.edit_nodes(&[r], |n| n.size.x = 44.0);
        app.dev_refresh(true);
        assert_eq!(app.doc.get(f).unwrap().dev.status, Some(DevStatus::Changed));
        let (old, nodes) = app.dev_snapshot(f).unwrap();
        assert_eq!(old.get(r).unwrap().size.x, 20.0, "the snapshot document has the old subtree");
        let diff = diff_subtree(&nodes, &app.doc, f);
        assert_eq!(diff.len(), 1);
        assert_eq!(diff[0].id, r);
        // undo the edit: back to Ready for dev
        app.undo();
        app.dev_refresh(true);
        assert_eq!(app.doc.get(f).unwrap().dev.status, Some(DevStatus::ReadyForDev));
        // undo the marking
        app.undo();
        assert!(app.doc.get(f).unwrap().dev.is_empty());
    }

    #[test]
    fn marking_ignores_unmarkable_layers() {
        let (mut app, _, r) = app_with_frame();
        app.dev_mark_ready(&[r]);
        assert!(app.doc.get(r).unwrap().dev.status.is_none());
    }

    #[test]
    fn annotations_and_links_are_undoable_edits() {
        let (mut app, f, _) = app_with_frame();
        app.dev_edit(f, |d| d.annotations.push(Annotation { text: "Round the corners".into(), properties: vec!["Width".into()], category: None }));
        app.dev_edit(f, |d| d.links.push(DevLink { label: "PR".into(), url: "https://github.com/a/b/pull/1".into() }));
        assert_eq!(app.doc.get(f).unwrap().dev.annotations.len(), 1);
        assert_eq!(app.doc.get(f).unwrap().dev.links.len(), 1);
        app.dev_edit(f, |d| d.annotations[0].text = "Square corners".into());
        assert_eq!(app.doc.get(f).unwrap().dev.annotations[0].text, "Square corners");
        app.undo();
        assert_eq!(app.doc.get(f).unwrap().dev.annotations[0].text, "Round the corners");
        app.undo();
        app.undo();
        assert!(app.doc.get(f).unwrap().dev.is_empty());
        app.redo();
        assert_eq!(app.doc.get(f).unwrap().dev.annotations.len(), 1);
    }

    #[test]
    fn links_open_only_after_confirmation() {
        let (mut app, _, _) = app_with_frame();
        app.dev_open_link("Storybook", "https://sb.example.com/x");
        let d = app.dialog.as_ref().expect("a confirmation dialog");
        assert_eq!(d.intent, Intent::OpenDevLink("https://sb.example.com/x".into()));
        assert!(d.lines.iter().any(|(l, _)| l.contains("sb.example.com")));
        // unsafe schemes never reach the dialog
        app.dialog = None;
        app.dev_open_link("x", "file:///etc/passwd");
        assert!(app.dialog.is_none());
        assert!(app.toast.as_ref().is_some_and(|t| t.error));
    }

    #[test]
    fn languages_generate_code() {
        let (app, f, _) = app_with_frame();
        for l in Language::ALL {
            assert!(!l.generate(&app.doc, f, false).is_empty(), "{}", l.label());
        }
        assert!(Language::React.generate(&app.doc, f, true).contains("className"));
        assert!(Language::React.generate(&app.doc, f, false).contains("style={{"));
    }
}
