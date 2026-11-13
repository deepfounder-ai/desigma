//! The canvas: renders the page into a texture, handles viewport gestures,
//! tools and interactions, and paints the selection overlays.

pub mod builder;
pub mod create;
pub mod draw;
pub mod gpu;
pub mod grids;
pub mod handles;
pub mod hit;
pub mod interact;
pub mod polyfill;
pub mod proto;
pub mod reorder;
pub mod rulers;
pub mod snap;
pub mod textsession;
pub mod tiles;
pub mod tools;
pub mod vector;

use crate::app::App;
use crate::edit::Tx;
use desigma_doc::{Affine, NodeId, Rect, Vec2 as DV};
use egui::{Context, Rect as ERect, Sense, Ui};
use handles::Handle;

pub struct Pending {
    pub node: NodeId,
    pub start_screen: egui::Pos2,
    pub start_page: DV,
    pub shift: bool,
    pub was_selected: bool,
}

pub struct MoveState {
    pub tx: Tx,
    pub ids: Vec<NodeId>,
    pub start: DV,
    pub orig_bbox: Rect,
    pub targets: Vec<Rect>,
    pub moved: bool,
    /// Auto-layout reorder target while dragging a child inside its frame.
    pub reorder: Option<reorder::Insertion>,
}

pub enum ResizeKind {
    Single { id: NodeId, world0: Affine, t0: Affine, size0: DV },
    Multi { bbox0: Rect, nodes: Vec<(NodeId, Rect, Affine, DV)> },
}

pub struct ResizeState {
    pub tx: Tx,
    pub handle: Handle,
    pub kind: ResizeKind,
    pub targets: Vec<Rect>,
}

pub struct RotateState {
    pub tx: Tx,
    pub id: NodeId,
    pub center: DV,
    pub start_angle: f32,
    pub t0: Affine,
    pub size: DV,
    pub rot0: f32,
    /// Multi-selection: every rotated node with its world transform at drag start.
    pub group: Vec<(NodeId, Affine)>,
}

pub struct DrawState {
    pub tx: Tx,
    pub tool: tools::Tool,
    pub id: NodeId,
    pub parent: NodeId,
    pub start: DV,
    pub targets: Vec<Rect>,
    pub dragged: bool,
}

#[derive(Default)]
pub enum Interaction {
    #[default]
    None,
    Pan,
    Pending(Pending),
    Marquee {
        start: DV,
        cur: DV,
        base: Vec<NodeId>,
    },
    Move(MoveState),
    Resize(ResizeState),
    Rotate(RotateState),
    Draw(DrawState),
    Pencil(Vec<DV>),
    /// Wiping strokes away with the eraser: they go when the button is released.
    Erase(EraseState),
    PenDrag,
    /// Dragging a vertex/handle in vector edit mode (state lives in `CanvasState::vedit`).
    VectorDrag,
    /// Brushing over regions with the shape builder (state lives in `CanvasState::builder`).
    Builder,
    /// Creating or moving a ruler guide.
    Guide(rulers::GuideDrag),
    /// Dragging a connector out of a board node (or from empty canvas).
    Connect(crate::board::connect::ConnectDrag),
    /// Dragging the divider between two table columns or rows.
    Table(crate::board::table::TableDrag),
}

/// Strokes the eraser has touched so far and where it was last.
#[derive(Default)]
pub struct EraseState {
    pub hits: Vec<NodeId>,
    pub last: DV,
}

pub use textsession::TextSession;

pub struct PenState {
    pub pts: Vec<create::PenPoint>,
}

#[derive(Default)]
pub struct CanvasState {
    /// The document changed since the last paint (bumps the tile revision).
    pub dirty: bool,
    /// Tile cache and per-tile textures.
    pub tiles: tiles::TileView,
    /// GPU canvas renderer state (preference, availability, caches).
    pub gpu: gpu::GpuView,
    /// Motion mode: the evaluated page and the playback clock.
    pub motion: crate::motion::playback::Playback,
    pub inter: Interaction,
    pub guides: Vec<snap::Guide>,
    pub measures: Vec<snap::Measure>,
    pub pen: Option<PenState>,
    pub text: Option<TextSession>,
    /// Point editing of a Vector node.
    pub vedit: Option<vector::VectorEdit>,
    /// The shape builder session.
    pub builder: Option<builder::Builder>,
    pub pointer_page: Option<DV>,
    pub cursor: Option<egui::CursorIcon>,
    pub last_render_ms: f32,
}

impl CanvasState {
    /// Drops all in-flight state without touching any document (used when the
    /// document itself is being replaced).
    pub fn reset_interaction(&mut self) {
        self.inter = Interaction::None;
        self.guides.clear();
        self.measures.clear();
        self.pen = None;
        self.text = None;
        self.vedit = None;
        self.builder = None;
        // the page or the whole document may be different now
        self.tiles.invalidate_all();
    }
}

impl App {
    /// Cancels the running interaction, restoring the document.
    pub fn cancel_interaction(&mut self) {
        let inter = std::mem::take(&mut self.canvas.inter);
        match inter {
            Interaction::Move(m) => m.tx.cancel(&mut self.doc),
            Interaction::Resize(r) => r.tx.cancel(&mut self.doc),
            Interaction::Rotate(r) => r.tx.cancel(&mut self.doc),
            Interaction::Draw(d) => d.tx.cancel(&mut self.doc),
            Interaction::Connect(c) => {
                c.tx.cancel(&mut self.doc);
                self.board.target = None;
            }
            Interaction::Table(d) => d.tx.cancel(&mut self.doc),
            Interaction::VectorDrag => {
                if let Some(v) = self.canvas.vedit.as_mut() {
                    v.marquee = None;
                    if let Some(d) = v.drag.take() {
                        d.tx.cancel(&mut self.doc);
                    }
                }
                vector::reload(self);
            }
            Interaction::Builder => {
                if let Some(b) = self.canvas.builder.as_mut() {
                    b.brush = None;
                }
            }
            _ => {}
        }
        self.canvas.guides.clear();
        self.canvas.measures.clear();
        self.canvas.pen = None;
        if let Some(t) = self.canvas.text.take() {
            t.tx.cancel(&mut self.doc);
        }
        self.changed();
        self.sanitize();
    }

    /// Switches tool, finishing any pen path or text edit in progress.
    pub fn set_tool(&mut self, tool: tools::Tool) {
        interact::finish_pen(self);
        interact::finish_text(self, true);
        self.cancel_interaction();
        builder::finish(self);
        self.tool = tool;
        if tool.is_drawing() {
            // the toolbar button and tray show the drawing tool last used
            self.board.brush = tool;
        }
        if tool == tools::Tool::ShapeBuilder && !builder::start(self) {
            self.tool = tools::Tool::Move;
        }
    }
}

pub fn show(app: &mut App, ui: &mut Ui, rect: ERect) {
    app.canvas_rect = rect;
    if !app.first_fit_done {
        app.first_fit_done = true;
        match app.startup_zoom.take() {
            Some(z) => app.zoom_on_content(z),
            None if app.is_deck() => {
                if let Some(id) = app.active_slide() {
                    app.select_slide(id);
                } else {
                    app.fit_all();
                }
            }
            None => app.fit_all(),
        }
    }
    if app.is_board() {
        // the dotted board under the (transparent) page tiles
        crate::board::canvas::background(app, ui, rect);
    }
    tiles::paint(app, ui, rect);
    let resp = ui.interact(rect, ui.id().with("canvas"), Sense::click_and_drag());
    // While the colour picker waits for an eyedropper click the canvas must not react.
    let eyedropping = app.props.picker.as_ref().is_some_and(|p| p.eyedrop);
    // An old version shown from the version history is read-only.
    let read_only = app.read_only();
    if !eyedropping && !read_only && !proto::process(app, ui, rect, &resp) {
        interact::process(app, ui, rect, &resp);
    }
    crate::comments::pins::process(app, rect, &resp);
    grids::paint(app, ui, rect);
    draw::overlays(app, ui, rect);
    crate::collab::draw::overlays(app, ui, rect);
    crate::motion::inspector::paint_overlay(app, ui, rect);
    vector::toolbar(app, ui, rect);
    rulers::overlay(app, ui, rect);
    proto::overlays(app, ui, rect);
    crate::panels::inspect::measure_overlay(app, ui, rect);
    crate::aiimg::ui::canvas_overlay(app, ui, rect);
    crate::dev::annot::overlay(app, ui, rect);
    if resp.hovered() || !matches!(app.canvas.inter, Interaction::None) {
        if let Some(c) = app.canvas.cursor {
            ui.ctx().set_cursor_icon(c);
        }
    }
    if proto::active(app) && (app.proto.hot.is_some() || app.proto.connect.is_some()) {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    // Comment pins and popovers sit above everything painted on the canvas.
    crate::comments::pins::overlay(app, ui, rect);
}
