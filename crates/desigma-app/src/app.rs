//! Application state shared by all panels and the canvas.

use crate::canvas::tools::{Tool, GROUPS};
use crate::canvas::CanvasState;
use crate::clipboard::Clip;
use crate::fonts::FontLoader;
use crate::panels::layers::LayersState;
use crate::panels::properties::PropsState;
use crate::theme;
use desigma_doc::{Document, History, NodeId, Vec2};
use std::path::PathBuf;

/// Page-space point shown at the top-left of the canvas, plus zoom.
#[derive(Clone, Copy, Debug)]
pub struct View {
    pub offset: Vec2,
    pub zoom: f32,
}

impl Default for View {
    fn default() -> Self {
        Self { offset: Vec2::ZERO, zoom: 1.0 }
    }
}

// `View` is Copy, but these keep `&self` so call sites read `view.to_screen(..)` uniformly.
#[allow(clippy::wrong_self_convention)]
impl View {
    pub fn to_screen(&self, origin: egui::Pos2, p: Vec2) -> egui::Pos2 {
        egui::pos2(origin.x + (p.x - self.offset.x) * self.zoom, origin.y + (p.y - self.offset.y) * self.zoom)
    }
    pub fn to_page(&self, origin: egui::Pos2, s: egui::Pos2) -> Vec2 {
        Vec2::new((s.x - origin.x) / self.zoom + self.offset.x, (s.y - origin.y) / self.zoom + self.offset.y)
    }
    /// Zooms keeping the page point under `screen` fixed.
    pub fn zoom_at(&mut self, origin: egui::Pos2, screen: egui::Pos2, new_zoom: f32) {
        let z = new_zoom.clamp(theme::ZOOM_MIN, theme::ZOOM_MAX);
        let page = self.to_page(origin, screen);
        self.zoom = z;
        self.offset = Vec2::new(page.x - (screen.x - origin.x) / z, page.y - (screen.y - origin.y) / z);
    }
    /// Fits `bounds` (page space) into a canvas of `size` px with padding.
    pub fn fit(bounds: desigma_doc::Rect, size: egui::Vec2) -> View {
        let pad = theme::FIT_PADDING;
        let zw = (size.x - 2.0 * pad).max(10.0) / bounds.w.max(1.0);
        let zh = (size.y - 2.0 * pad).max(10.0) / bounds.h.max(1.0);
        let z = zw.min(zh).clamp(theme::ZOOM_MIN, theme::ZOOM_MAX);
        let cx = bounds.x + bounds.w / 2.0;
        let cy = bounds.y + bounds.h / 2.0;
        View { zoom: z, offset: Vec2::new(cx - size.x / 2.0 / z, cy - size.y / 2.0 / z) }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RailItem {
    File,
    Assets,
    Tools,
    Variables,
    Plugins,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RightTab {
    Design,
    Prototype,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MenuKind {
    Main,
    FileMenu,
    Zoom,
    ToolGroup(usize),
    FileName,
    /// The "Actions…" command palette (Ctrl+K).
    Palette,
}

#[derive(Default)]
pub struct UiState {
    pub menu: Option<MenuKind>,
    pub menu_anchor: Option<egui::Rect>,
    pub left_collapsed: bool,
    pub rail: Option<RailItem>,
    pub right_tab: Option<RightTab>,
    pub dev_mode: bool,
    /// Selection context menu position (canvas or layers right-click).
    pub ctx_menu: Option<egui::Pos2>,
    /// Anchor of the instance "more" menu and of the add-property menu in the inspector.
    pub inst_menu: Option<egui::Rect>,
    pub prop_menu: Option<egui::Rect>,
    /// Command palette query and highlighted row.
    pub palette_query: String,
    pub palette_sel: usize,
    /// View toggles from the main menu.
    pub pixel_grid: bool,
    pub rulers: bool,
    /// Layout grids of frames are hidden (Ctrl+Shift+4).
    pub hide_layout_grids: bool,
    /// Snap to pixel grid is off (it is on by default, as in Figma).
    pub no_pixel_snap: bool,
}

pub struct Toast {
    pub text: String,
    pub until: f64,
    pub error: bool,
}

pub struct App {
    pub doc: Document,
    pub hist: History,
    pub page: NodeId,
    pub selection: Vec<NodeId>,
    pub hover: Option<NodeId>,
    pub hover_layer: Option<NodeId>,
    pub tool: Tool,
    /// Last picked tool index per toolbar group.
    pub group_pick: [usize; GROUPS.len()],
    pub view: View,
    pub canvas_rect: egui::Rect,
    pub canvas: CanvasState,
    pub layers: LayersState,
    pub props: PropsState,
    pub vars: crate::variables::VarsState,
    pub assets: crate::panels::assets::AssetsState,
    /// Shared libraries (publishing, enabled libraries, updates).
    pub libs: crate::libraries::LibrariesState,
    pub plugins: crate::plugins::PluginsState,
    pub bridge: crate::bridge::BridgeState,
    pub proto: crate::proto::ProtoState,
    /// Find and replace panel state (Ctrl+F).
    pub find: crate::find::FindState,
    pub ui: UiState,
    pub clip: Option<Clip>,
    pub paste_count: u32,
    pub toast: Option<Toast>,
    pub file_path: Option<PathBuf>,
    pub now: f64,
    pub keep_tool: bool,
    pub first_fit_done: bool,
    pub fonts: FontLoader,
    /// Document snapshot while a numeric field is being scrubbed (see `widgets::field::SCRUB_FLAG`).
    pub scrub: Option<std::collections::BTreeMap<NodeId, desigma_doc::Node>>,
    /// Zoom (as a factor) to apply on the first frame instead of fitting the page (`--zoom`).
    pub startup_zoom: Option<f32>,
    /// The projects library (None until opened; tests run without one).
    pub lib: Option<desigma_library::Library>,
    /// The library file open in the editor, if any (autosaved, shown in the header).
    pub lib_file: Option<crate::home::LibFile>,
    /// Some while the home screen (file browser) is showing instead of the editor.
    pub home: Option<crate::home::Home>,
    /// The open modal dialog (prompts, confirmations, shortcuts, About).
    pub dialog: Option<crate::home::Dialog>,
    pub autosave: crate::home::Autosave,
    /// The header file name is being renamed (double-click).
    pub renaming_file: bool,
    /// Comment threads UI (composer, open thread, panel filters).
    pub comments: crate::comments::CommentsState,
    /// Version history of the open library file.
    pub history: crate::history::HistoryState,
    /// Dev mode: code language, annotation / link editors, Compare changes.
    pub dev: crate::dev::DevState,
    /// Board (whiteboard) mode: tool settings, magnet dots and popovers.
    pub board: crate::board::BoardState,
    pub decks: crate::decks::DecksState,
    /// Update notifications (banner, dialog, background checks).
    pub updater: crate::updater::UpdaterState,
    /// Motion (Beta) mode state shared by the inspector, timeline and playback.
    pub motion: crate::motion::MotionUi,
    /// Motion inspector: easing editor, animation library and clip popovers.
    pub motion_ins: crate::motion::inspector::InspectorState,
    /// Live collaboration: sharing, joining, other people's presence.
    pub collab: crate::collab::CollabState,
    /// Hash of the document as opened or last saved to a file; `None` until then (see `unsaved.rs`).
    pub file_hash: Option<u64>,
    /// An open waiting for the answer to "Discard unsaved changes?".
    pub pending_open: Option<crate::instance::Open>,
    /// Watching the open file for changes made by other programs (see `extwatch.rs`).
    pub ext_watch: Option<crate::extwatch::Watch>,
}

impl App {
    pub fn new(doc: Document) -> Self {
        let page = doc.pages.first().copied().unwrap_or(NodeId(0));
        Self {
            doc,
            hist: History::default(),
            page,
            selection: vec![],
            hover: None,
            hover_layer: None,
            tool: Tool::Move,
            group_pick: [0; GROUPS.len()],
            view: View::default(),
            canvas_rect: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0)),
            canvas: CanvasState::default(),
            layers: LayersState::default(),
            props: PropsState::default(),
            vars: Default::default(),
            assets: Default::default(),
            libs: Default::default(),
            plugins: Default::default(),
            bridge: Default::default(),
            proto: Default::default(),
            find: Default::default(),
            ui: UiState { pixel_grid: true, ..UiState::default() },
            clip: None,
            paste_count: 0,
            toast: None,
            file_path: None,
            now: 0.0,
            keep_tool: false,
            first_fit_done: false,
            fonts: FontLoader::default(),
            scrub: None,
            startup_zoom: None,
            lib: None,
            lib_file: None,
            home: None,
            dialog: None,
            autosave: Default::default(),
            renaming_file: false,
            comments: Default::default(),
            history: Default::default(),
            dev: Default::default(),
            board: Default::default(),
            decks: Default::default(),
            updater: Default::default(),
            motion: Default::default(),
            motion_ins: Default::default(),
            collab: Default::default(),
            file_hash: None,
            pending_open: None,
            ext_watch: None,
        }
    }

    /// Registers fonts finished loading in the background and re-renders.
    pub fn poll_fonts(&mut self) {
        let Some(done) = self.fonts.poll() else { return };
        if done.loaded > 0 {
            self.changed();
        }
        // A guest first asks the host for the fonts the shared file uses.
        let pairs: Vec<_> = done.failed.into_iter().zip(done.failed_requests).collect();
        let rest = self.ask_host_for_fonts(pairs.iter().map(|(_, r)| r.clone()).collect());
        let failed: Vec<String> = pairs.into_iter().filter(|(_, r)| rest.contains(r)).map(|(msg, _)| msg).collect();
        self.toast_fonts_unavailable(&failed);
    }

    /// "Font unavailable: Family 400: reason (+2 more)".
    pub fn toast_fonts_unavailable(&mut self, failed: &[String]) {
        if let Some(first) = failed.first() {
            let more = failed.len() - 1;
            let suffix = if more > 0 { format!(" (+{more} more)") } else { String::new() };
            self.show_toast(format!("Font unavailable: {first}{suffix}"), true);
        }
    }

    pub fn show_toast(&mut self, text: impl Into<String>, error: bool) {
        self.toast = Some(Toast { text: text.into(), until: self.now + theme::TOAST_SECONDS, error });
    }

    /// Marks the document as modified so the canvas texture is re-rendered. Not for view
    /// changes (pan, zoom): the renderers follow the viewport by themselves, and a revision bump
    /// re-indexes the page and makes autosave hash the whole document.
    pub fn changed(&mut self) {
        self.canvas.dirty = true;
    }

    /// The document as drawn: in Motion mode the page evaluated at the playhead (animated
    /// positions, sizes and rotations), else the stored document. Selection boxes, handles and the
    /// motion gizmo are placed with it so they sit on what is on screen.
    pub fn shown_doc(&self) -> &desigma_doc::Document {
        self.canvas.motion.document(self.motion.enabled, &self.doc)
    }

    pub fn is_selected(&self, id: NodeId) -> bool {
        self.selection.contains(&id)
    }

    pub fn select_only(&mut self, id: NodeId) {
        self.selection = vec![id];
    }

    pub fn clear_selection(&mut self) {
        self.selection.clear();
    }

    /// Drops selection/hover entries that no longer exist or left the page.
    pub fn sanitize(&mut self) {
        let doc = &self.doc;
        let page = self.page;
        self.selection.retain(|id| doc.get(*id).is_some() && doc.page_of(*id) == Some(page));
        if self.hover.map(|h| doc.get(h).is_none()).unwrap_or(false) {
            self.hover = None;
        }
        if !doc.nodes.contains_key(&self.page) {
            self.page = doc.pages.first().copied().unwrap_or(NodeId(0));
        }
    }

    pub fn title(&self) -> String {
        let name = if self.doc.name.is_empty() { "Untitled" } else { &self.doc.name };
        format!("{name} - {}", theme::APP_TITLE)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn view_roundtrip_and_zoom_anchor() {
        let mut v = View { offset: Vec2::new(100.0, 50.0), zoom: 2.0 };
        let origin = egui::pos2(300.0, 10.0);
        let screen = egui::pos2(500.0, 200.0);
        let before = v.to_page(origin, screen);
        v.zoom_at(origin, screen, 4.0);
        let after = v.to_page(origin, screen);
        assert!((before.x - after.x).abs() < 1e-3 && (before.y - after.y).abs() < 1e-3);
        let s = v.to_screen(origin, before);
        assert!((s.x - screen.x).abs() < 1e-3);
    }

    #[test]
    fn fit_centers_bounds() {
        let v = View::fit(desigma_doc::Rect::new(0.0, 0.0, 1000.0, 500.0), egui::vec2(800.0, 600.0));
        assert!(v.zoom < 1.0);
        let c = v.to_screen(egui::Pos2::ZERO, Vec2::new(500.0, 250.0));
        assert!((c.x - 400.0).abs() < 1.0 && (c.y - 300.0).abs() < 1.0);
    }

    #[test]
    fn zoom_is_clamped() {
        let mut v = View::default();
        v.zoom_at(egui::Pos2::ZERO, egui::Pos2::ZERO, 10_000.0);
        assert_eq!(v.zoom, theme::ZOOM_MAX);
    }
}
