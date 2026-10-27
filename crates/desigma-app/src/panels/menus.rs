//! Application menus and the shared action dispatcher (menus + shortcuts).

use crate::app::{App, MenuKind};
use crate::canvas::textsession::{text_command, TextCmd};
use crate::canvas::tools::{Tool, GROUPS};
use crate::icons::{self, Icon};
use crate::ops::Align;
use crate::richtext::ListKind;
use crate::theme;
use crate::widgets::menu::{popup_menu, popup_tree, preset_path, MenuItem, MenuResult, TreeResult};
use crate::widgets::{self, menu_panel};
use desigma_doc::find::MatchCriteria;
use desigma_doc::NodeKind;
use egui::{Align2, Context, Id, Key, Modifiers, Pos2, Rect, Sense, Vec2};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    New,
    /// File -> New board.
    NewBoard,
    NewDeck,
    Open,
    Save,
    SaveAs,
    Export,
    /// File -> Export code…: a whole frame as an HTML/CSS, React, Tailwind, SwiftUI or Compose project.
    ExportCode,
    ExportSlidePdf,
    ExportDeckPdf,
    Undo,
    Redo,
    SelectAll,
    SelectInverse,
    SelectChildren,
    SelectParent,
    /// Select matching layers (same type and visual properties).
    SelectMatching,
    /// Select all with the same fill / stroke / effect / font / instance.
    SelectSame(MatchCriteria),
    /// Opens the Find and replace panel.
    Find,
    Delete,
    Duplicate,
    Group,
    Ungroup,
    FrameSelection,
    AutoLayout,
    FitAll,
    FitSelection,
    Zoom100,
    ZoomIn,
    ZoomOut,
    ZoomTo(f32),
    BringForward,
    SendBackward,
    BringToFront,
    SendToBack,
    ToggleSidebar,
    CreateComponent,
    Detach,
    CombineVariants,
    GoToMain,
    ResetInstance,
    Copy,
    Paste,
    TogglePixelGrid,
    ToggleRulers,
    ToggleLayoutGrids,
    ToggleSnapPixel,
    TextBold,
    TextItalic,
    TextUnderline,
    TextSizeUp,
    TextSizeDown,
    TextBulletList,
    TextNumberedList,
    Align(Align),
    Distribute,
    Flatten,
    /// Ctrl+Alt+O: turn the stroke into filled geometry.
    OutlineStroke,
    /// Wrap the selection in a boolean group.
    Boolean(desigma_doc::BooleanOp),
    /// Enter vertex editing of the selected shape.
    EditVertices,
    /// Join the two selected vertices.
    VectorJoin,
    /// Cut the path at the selected vertices or point.
    VectorSplit,
    /// Reduce the nodes of the selected vectors.
    VectorSimplify,
    OpenPalette,
    Plugins,
    About,
    /// Home screen (file browser).
    BackToFiles,
    SaveToLibrary,
    /// File -> Share…: start or manage a live session.
    Share,
    /// File -> Join session….
    JoinSession,
    /// File -> Show version history.
    ToggleVersionHistory,
    /// Ctrl+Alt+S: save a named version.
    SaveVersion,
    /// Preferences -> Your name.
    YourName,
    /// Preferences -> Use GPU rendering.
    ToggleGpu,
    /// Preferences -> Check for updates automatically.
    ToggleUpdateCheck,
    /// Preferences -> Allow MCP agents to control this editor.
    ToggleAgentBridge,
    /// Preferences -> Network proxy…
    NetworkProxy,
    /// Help -> Check for updates.
    CheckForUpdates,
    Shortcuts,
    AboutDesigma,
    OpenGithub,
    /// Dev mode: marks the selected frames and sections Ready for dev.
    MarkReadyForDev,
    MarkCompleted,
    ClearDevStatus,
    /// Switches to a tool (also from the command palette).
    Tool(Tool),
}

pub fn run(app: &mut App, ctx: &Context, a: Action) {
    match a {
        Action::New => app.new_file_here(),
        Action::NewBoard => app.new_board_here(),
        Action::NewDeck => app.new_deck_here(),
        Action::Open => app.open_dialog(),
        Action::Save => app.save(false),
        Action::SaveAs => app.save(true),
        Action::Export => app.export_selection(),
        Action::ExportCode => app.export_code_dialog(),
        Action::ExportSlidePdf => app.export_deck_pdf(false),
        Action::ExportDeckPdf => app.export_deck_pdf(true),
        Action::Undo => {
            app.undo();
        }
        Action::Redo => {
            app.redo();
        }
        Action::SelectAll => app.select_all_layers(),
        Action::SelectInverse => app.select_inverse(),
        Action::SelectChildren => app.select_children(),
        Action::SelectParent => app.select_parent(),
        Action::SelectMatching => app.select_matching(MatchCriteria::Properties),
        Action::SelectSame(c) => app.select_matching(c),
        Action::Find => app.open_find(),
        Action::Delete => app.delete_selection(),
        Action::Duplicate => app.duplicate_selection(),
        Action::Group => app.group_selection(false),
        Action::Ungroup => app.ungroup_selection(),
        Action::FrameSelection => app.group_selection(true),
        Action::AutoLayout => app.toggle_auto_layout(),
        Action::FitAll => app.fit_all(),
        Action::FitSelection => app.fit_selection(),
        Action::Zoom100 => app.zoom_to(1.0),
        Action::ZoomIn => app.zoom_by(1.25),
        Action::ZoomOut => app.zoom_by(0.8),
        Action::ZoomTo(z) => app.zoom_to(z),
        Action::BringForward => app.reorder(true, false),
        Action::SendBackward => app.reorder(false, false),
        Action::BringToFront => app.reorder(true, true),
        Action::SendToBack => app.reorder(false, true),
        Action::ToggleSidebar => app.ui.left_collapsed = !app.ui.left_collapsed,
        Action::CreateComponent => app.create_component(),
        Action::Detach => app.detach_selection(),
        Action::CombineVariants => app.combine_selection_as_variants(),
        Action::GoToMain => {
            if let Some(i) = app.selected_instances().first().copied() {
                app.go_to_main(i);
            }
        }
        Action::ResetInstance => app.reset_selection(),
        Action::Copy => {
            if let Some(s) = app.copy_selection() {
                ctx.copy_text(s);
            }
        }
        Action::Paste => app.paste(None),
        Action::TogglePixelGrid => app.ui.pixel_grid = !app.ui.pixel_grid,
        Action::ToggleRulers => app.ui.rulers = !app.ui.rulers,
        Action::ToggleLayoutGrids => app.ui.hide_layout_grids = !app.ui.hide_layout_grids,
        Action::ToggleSnapPixel => app.ui.no_pixel_snap = !app.ui.no_pixel_snap,
        Action::TextBold => text_command(app, TextCmd::Bold),
        Action::TextItalic => text_command(app, TextCmd::Italic),
        Action::TextUnderline => text_command(app, TextCmd::Underline),
        Action::TextSizeUp => text_command(app, TextCmd::SizeUp),
        Action::TextSizeDown => text_command(app, TextCmd::SizeDown),
        Action::TextBulletList => text_command(app, TextCmd::List(ListKind::Bullet)),
        Action::TextNumberedList => text_command(app, TextCmd::List(ListKind::Numbered)),
        Action::Align(a) => app.align(a),
        Action::Distribute => {
            app.distribute_selection();
        }
        Action::Flatten => app.flatten_selection(),
        Action::OutlineStroke => app.outline_stroke_selection(),
        Action::Boolean(op) => app.boolean_selection(op),
        Action::EditVertices => {
            if let Some(id) = app.selection.first().copied().filter(|_| app.selection.len() == 1) {
                crate::canvas::vector::enter(app, id);
            }
        }
        Action::VectorJoin => {
            if !crate::canvas::vector::join_selected(app) {
                app.show_toast("Select two vertices to join", false);
            }
        }
        Action::VectorSplit => {
            if !crate::canvas::vector::split_selected(app) {
                app.show_toast("Select a vertex or click a segment to split the path", false);
            }
        }
        Action::VectorSimplify => app.simplify_selection(),
        Action::OpenPalette => {
            app.ui.palette_query.clear();
            app.ui.palette_sel = 0;
            app.ui.menu = Some(MenuKind::Palette);
            app.ui.menu_anchor = None;
        }
        Action::About => app.show_toast("desigma, a Figma-compatible design editor", false),
        Action::Plugins => app.open_plugins(),
        Action::BackToFiles => app.go_home(),
        Action::SaveToLibrary => app.begin_save_to_library(),
        Action::Share => app.open_share(),
        Action::JoinSession => app.open_join(None),
        Action::ToggleVersionHistory => app.toggle_version_history(),
        Action::SaveVersion => app.begin_save_version(),
        Action::YourName => app.ask_author_name(),
        Action::ToggleGpu => app.toggle_gpu(),
        Action::ToggleUpdateCheck => app.toggle_update_check(),
        Action::ToggleAgentBridge => app.toggle_agent_bridge(ctx),
        Action::NetworkProxy => app.open_proxy_dialog(),
        Action::CheckForUpdates => app.check_for_updates(ctx, true),
        Action::Shortcuts => app.show_shortcuts(),
        Action::AboutDesigma => app.show_about(),
        Action::OpenGithub => app.confirm_open_github(),
        Action::MarkReadyForDev => {
            let sel = app.top_selection();
            app.dev_mark_ready(&sel);
        }
        Action::MarkCompleted => {
            let sel = app.top_selection();
            app.dev_mark_completed(&sel);
        }
        Action::ClearDevStatus => {
            let sel = app.top_selection();
            app.dev_clear_status(&sel);
        }
        Action::Tool(t) => pick_tool(app, t),
    }
    ctx.request_repaint();
}

/// Selects `t` and remembers it as its toolbar group's current pick.
pub fn pick_tool(app: &mut App, t: Tool) {
    if let Some((g, i)) = GROUPS.iter().enumerate().find_map(|(g, tools)| tools.iter().position(|x| *x == t).map(|i| (g, i))) {
        app.group_pick[g] = i;
    }
    app.set_tool(t);
}

/// Label of a tool in the toolbar dropdowns.
pub fn tool_menu_label(t: Tool) -> &'static str {
    match t {
        Tool::Image => "Image/video…",
        Tool::Comment => "Comment",
        t => t.label(),
    }
}

// ---------------------------------------------------------------- menu tree
/// A menu entry with the action it runs (leaves) or its children (submenus).
struct Node {
    item: MenuItem,
    act: Option<Action>,
    kids: Vec<Node>,
}

fn leaf(label: &str, shortcut: &str, act: Action) -> Node {
    Node { item: MenuItem::new(label).shortcut(shortcut), act: Some(act), kids: vec![] }
}

fn submenu(label: &str, kids: Vec<Node>) -> Node {
    Node { item: MenuItem::new(label), act: None, kids }
}

impl Node {
    fn off(mut self, disabled: bool) -> Self {
        self.item.disabled = disabled;
        self
    }
    fn sep(mut self) -> Self {
        self.item.separator_after = true;
        self
    }
    fn on(mut self, checked: bool) -> Self {
        self.item = self.item.checked(checked);
        self
    }
    fn icon(mut self, i: Icon) -> Self {
        self.item = self.item.icon(i);
        self
    }
}

fn to_items(nodes: &[Node]) -> Vec<MenuItem> {
    nodes
        .iter()
        .map(|n| {
            let mut it = n.item.clone();
            it.children = to_items(&n.kids);
            it
        })
        .collect()
}

fn resolve(nodes: &[Node], path: &[usize]) -> Option<Action> {
    let (first, rest) = path.split_first()?;
    let n = nodes.get(*first)?;
    if rest.is_empty() {
        n.act
    } else {
        resolve(&n.kids, rest)
    }
}

fn find_path(nodes: &[Node], labels: &[&str]) -> Vec<usize> {
    let mut out = vec![];
    let mut cur = nodes;
    for l in labels {
        let Some(i) = cur.iter().position(|n| n.item.label.eq_ignore_ascii_case(l)) else { break };
        out.push(i);
        cur = &cur[i].kids;
    }
    out
}

/// A disabled placeholder (a Figma feature we do not implement) that keeps the menu's geometry.
fn stub_sub(label: &str) -> Node {
    submenu(label, vec![leaf("Not available", "", Action::About).off(true)]).off(true)
}

/// "Select all with same ..." entries (the Edit menu's submenu; the palette lists them flat).
fn same_nodes(enabled: bool) -> Vec<Node> {
    [
        ("Select all with same fill", MatchCriteria::Fill),
        ("Select all with same stroke", MatchCriteria::Stroke),
        ("Select all with same effect", MatchCriteria::Effect),
        ("Select all with same font", MatchCriteria::Font),
        ("Select all with same instance", MatchCriteria::Instance),
    ]
    .into_iter()
    .map(|(label, c)| leaf(label, "", Action::SelectSame(c)).off(!enabled))
    .collect()
}

/// The logo menu: sections separated by lines, every submenu wired to real commands.
fn main_nodes(app: &App) -> Vec<Node> {
    let has_sel = !app.selection.is_empty();
    let has_text = app.selection.iter().any(|id| app.doc.get(*id).is_some_and(|n| matches!(n.kind, NodeKind::Text(_))));
    let has_inst = !app.selected_instances().is_empty();
    let multi = app.selection.len() > 1;
    let mut file = vec![
        leaf("New design file", "", Action::New),
        leaf("New board", "", Action::NewBoard),
        leaf("New deck", "", Action::NewDeck),
        leaf("Open…", "Ctrl+O", Action::Open),
        leaf("Save", "Ctrl+S", Action::Save),
        leaf("Save as…", "Ctrl+Shift+S", Action::SaveAs),
        leaf("Save to library…", "", Action::SaveToLibrary).off(app.lib.is_none() || app.lib_file.is_some()).sep(),
        leaf("Share…", "", Action::Share).off(app.collab.is_guest()),
        leaf("Join session…", "", Action::JoinSession).sep(),
        leaf("Show version history", "", Action::ToggleVersionHistory).on(app.history.open),
        leaf("Save to version history…", "Ctrl+Alt+S", Action::SaveVersion).off(!app.has_history()).sep(),
        leaf("Export selection as PNG…", "", Action::Export).off(!has_sel),
        leaf("Export code…", "", Action::ExportCode).off(!has_sel),
    ];
    if app.is_deck() {
        file.push(leaf("Export current slide as PDF…", "", Action::ExportSlidePdf).off(app.deck_pdf_slides(false).is_err()));
        file.push(leaf("Export all slides as PDF…", "", Action::ExportDeckPdf).off(app.deck_pdf_slides(true).is_err()));
    }
    let edit = vec![
        leaf("Undo", "Ctrl+Z", Action::Undo).off(!app.hist.can_undo()),
        leaf("Redo", "Ctrl+Shift+Z", Action::Redo).off(!app.hist.can_redo()).sep(),
        leaf("Copy", "Ctrl+C", Action::Copy).off(!has_sel),
        leaf("Paste", "Ctrl+V", Action::Paste).off(app.clip.is_none()),
        leaf("Duplicate", "Ctrl+D", Action::Duplicate).off(!has_sel),
        leaf("Delete", "Del", Action::Delete).off(!has_sel).sep(),
        leaf("Select all", "Ctrl+A", Action::SelectAll),
        leaf("Select inverse", "Ctrl+Shift+A", Action::SelectInverse),
        leaf("Select children", "Enter", Action::SelectChildren).off(!has_sel),
        leaf("Select parent", "Shift+Enter", Action::SelectParent).off(!has_sel),
        leaf("Select matching layers", "Ctrl+Alt+A", Action::SelectMatching).off(!has_sel),
        submenu("Select all with same", same_nodes(has_sel)).off(!has_sel).sep(),
        leaf("Find and replace…", "Ctrl+F", Action::Find),
    ];
    let view = vec![
        leaf("Zoom in", "Ctrl+=", Action::ZoomIn),
        leaf("Zoom out", "Ctrl+-", Action::ZoomOut),
        leaf("Zoom to fit", "Shift+1", Action::FitAll),
        leaf("Zoom to selection", "Shift+2", Action::FitSelection).off(!has_sel),
        leaf("Zoom to 100%", "Shift+0", Action::Zoom100).sep(),
        leaf("Pixel grid", "Shift+'", Action::TogglePixelGrid).on(app.ui.pixel_grid),
        leaf("Snap to pixel grid", "Ctrl+Shift+'", Action::ToggleSnapPixel).on(!app.ui.no_pixel_snap),
        leaf("Layout guides", "Ctrl+Shift+4", Action::ToggleLayoutGrids).on(!app.ui.hide_layout_grids),
        leaf("Rulers", "Shift+R", Action::ToggleRulers).on(app.ui.rulers),
        leaf("Show/Hide UI", "Ctrl+\\", Action::ToggleSidebar).on(!app.ui.left_collapsed),
    ];
    let object = vec![
        leaf("Group selection", "Ctrl+G", Action::Group).off(!has_sel),
        leaf("Ungroup", "Ctrl+Shift+G", Action::Ungroup).off(!has_sel),
        leaf("Frame selection", "Ctrl+Alt+G", Action::FrameSelection).off(!has_sel),
        leaf("Add auto layout", "Shift+A", Action::AutoLayout).off(!has_sel).sep(),
        leaf("Create component", "Ctrl+Alt+K", Action::CreateComponent).off(!has_sel),
        leaf("Detach instance", "Ctrl+Alt+B", Action::Detach).off(!has_inst).sep(),
        leaf("Bring to front", "Ctrl+Shift+]", Action::BringToFront).off(!has_sel),
        leaf("Bring forward", "Ctrl+]", Action::BringForward).off(!has_sel),
        leaf("Send backward", "Ctrl+[", Action::SendBackward).off(!has_sel),
        leaf("Send to back", "Ctrl+Shift+[", Action::SendToBack).off(!has_sel),
    ];
    let text = vec![
        leaf("Bold", "Ctrl+B", Action::TextBold).off(!has_text),
        leaf("Italic", "Ctrl+I", Action::TextItalic).off(!has_text),
        leaf("Underline", "Ctrl+U", Action::TextUnderline).off(!has_text).sep(),
        leaf("Increase font size", "Ctrl+Shift+>", Action::TextSizeUp).off(!has_text),
        leaf("Decrease font size", "Ctrl+Shift+<", Action::TextSizeDown).off(!has_text).sep(),
        leaf("Bulleted list", "Ctrl+Shift+8", Action::TextBulletList).off(!has_text),
        leaf("Numbered list", "Ctrl+Shift+7", Action::TextNumberedList).off(!has_text),
    ];
    let arrange = vec![submenu(
        "Align and distribute",
        vec![
            leaf("Align left", "Alt+A", Action::Align(Align::Left)).off(!has_sel),
            leaf("Align horizontal centers", "Alt+H", Action::Align(Align::HCenter)).off(!has_sel),
            leaf("Align right", "Alt+D", Action::Align(Align::Right)).off(!has_sel).sep(),
            leaf("Align top", "Alt+W", Action::Align(Align::Top)).off(!has_sel),
            leaf("Align vertical centers", "Alt+V", Action::Align(Align::VCenter)).off(!has_sel),
            leaf("Align bottom", "Alt+S", Action::Align(Align::Bottom)).off(!has_sel).sep(),
            leaf("Distribute spacing", "", Action::Distribute).off(!multi),
        ],
    )];
    let editing = app.canvas.vedit.is_some();
    let has_vec = app.selection.iter().any(|id| crate::canvas::vector::is_vector(app, *id));
    let vector = vec![
        leaf("Outline stroke", "Ctrl+Alt+O", Action::OutlineStroke).off(!has_sel),
        leaf("Flatten", "Ctrl+E", Action::Flatten).off(!has_sel).sep(),
        submenu(
            "Boolean groups",
            vec![
                leaf("Union selection", "", Action::Boolean(desigma_doc::BooleanOp::Union)).off(!multi),
                leaf("Subtract selection", "", Action::Boolean(desigma_doc::BooleanOp::Subtract)).off(!multi),
                leaf("Intersect selection", "", Action::Boolean(desigma_doc::BooleanOp::Intersect)).off(!multi),
                leaf("Exclude selection", "", Action::Boolean(desigma_doc::BooleanOp::Exclude)).off(!multi),
            ],
        ),
        leaf("Shape builder", "Shift+M", Action::Tool(Tool::ShapeBuilder)).off(!multi).sep(),
        leaf("Edit vertices", "Enter", Action::EditVertices).off(app.selection.len() != 1 || !has_vec || editing),
        leaf("Simplify path", "", Action::VectorSimplify).off(!has_vec),
        leaf("Join", "Ctrl+J", Action::VectorJoin).off(!editing),
        leaf("Split", "Ctrl+Shift+J", Action::VectorSplit).off(!editing),
    ];
    let help = vec![
        leaf("Command palette", "Ctrl+K", Action::OpenPalette),
        leaf("Keyboard shortcuts", "", Action::Shortcuts).sep(),
        leaf("About desigma", "", Action::AboutDesigma),
        leaf(&format!("Version {}", crate::VERSION), "", Action::About).off(true),
        leaf("Check for updates…", "", Action::CheckForUpdates),
        leaf("desigma on GitHub", "", Action::OpenGithub),
    ];
    vec![
        leaf("Back to files", "", Action::BackToFiles).sep(),
        leaf("Actions…", "Ctrl+K", Action::OpenPalette).icon(Icon::Search).sep(),
        submenu("File", file),
        submenu("Edit", edit),
        submenu("View", view),
        submenu("Object", object),
        submenu("Text", text),
        submenu("Arrange", arrange),
        submenu("Vector", vector).sep(),
        leaf("Plugins", "", Action::Plugins),
        stub_sub("Widgets"),
        submenu(
            "Preferences",
            vec![
                leaf("Your name…", "", Action::YourName),
                leaf("Use GPU rendering", "", Action::ToggleGpu).on(app.canvas.gpu.active()).off(!app.canvas.gpu.available),
                // builds from source never check, so the toggle is off and greyed there
                leaf("Check for updates automatically", "", Action::ToggleUpdateCheck)
                    .on(crate::RELEASE_BUILD && !app.updater.no_auto_check)
                    .off(!crate::RELEASE_BUILD),
                leaf("Allow MCP agents to control this editor", "", Action::ToggleAgentBridge).on(app.bridge.bridge.is_some()),
                leaf("Network proxy…", "", Action::NetworkProxy),
            ],
        ),
        leaf("Libraries", "", Action::About).off(true).sep(),
        submenu("Help", help),
    ]
}

fn tool_nodes(app: &App, g: usize) -> Vec<Node> {
    GROUPS[g]
        .iter()
        .map(|t| {
            leaf(tool_menu_label(*t), t.shortcut(), Action::Tool(*t))
                .on(app.group_pick[g].min(GROUPS[g].len() - 1) == GROUPS[g].iter().position(|x| x == t).unwrap_or(0))
                .icon(super::toolbar::tool_icon(*t))
        })
        .collect()
}

/// Id under which a menu keeps its highlight state.
pub fn menu_id(kind: MenuKind) -> Id {
    Id::new(("app-menu", kind))
}

/// Opens a menu from a CLI spec like `main`, `main/File`, `shape`, `palette` (for screenshots).
pub fn preset_menu(ctx: &Context, app: &mut App, spec: &str, screen: Rect) -> bool {
    let (name, sub) = spec.split_once('/').map_or((spec, ""), |(a, b)| (a, b));
    let labels: Vec<&str> = sub.split('/').filter(|s| !s.is_empty()).collect();
    match name {
        "main" => {
            app.ui.menu = Some(MenuKind::Main);
            app.ui.menu_anchor =
                Some(Rect::from_min_size(Pos2::new(theme::RAIL_MENU_BUTTON_X, theme::RAIL_MENU_BUTTON_Y), Vec2::splat(theme::RAIL_MENU_BUTTON_SIZE)));
            let path = find_path(&main_nodes(app), &labels);
            if !path.is_empty() {
                preset_path(ctx, menu_id(MenuKind::Main), &path);
            }
        }
        "shape" | "move" | "frame" | "pen" | "text" => {
            let g = match name {
                "move" => 0,
                "frame" => 1,
                "shape" => 2,
                "pen" => 3,
                _ => 4,
            };
            app.ui.menu = Some(MenuKind::ToolGroup(g));
            app.ui.menu_anchor = Some(super::toolbar::chevron_rect(screen, g));
        }
        "palette" => run(app, ctx, Action::OpenPalette),
        _ => return false,
    }
    true
}

fn file_items(app: &App) -> Vec<(MenuItem, Option<Action>)> {
    let has_sel = !app.selection.is_empty();
    let mut items = vec![
        (MenuItem::new("New design file"), Some(Action::New)),
        (MenuItem::new("New board"), Some(Action::NewBoard)),
        (MenuItem::new("New deck"), Some(Action::NewDeck)),
        (MenuItem::new("Open…").shortcut("Ctrl+O"), Some(Action::Open)),
        (MenuItem::new("Save").shortcut("Ctrl+S"), Some(Action::Save)),
        (MenuItem::new("Save as…").shortcut("Ctrl+Shift+S"), Some(Action::SaveAs)),
        (MenuItem::new("Save to library…").disabled(app.lib.is_none() || app.lib_file.is_some()).sep(), Some(Action::SaveToLibrary)),
        (MenuItem::new("Share…").disabled(app.collab.is_guest()), Some(Action::Share)),
        (MenuItem::new("Join session…").sep(), Some(Action::JoinSession)),
        (MenuItem::new("Show version history").checked(app.history.open), Some(Action::ToggleVersionHistory)),
        (MenuItem::new("Save to version history…").shortcut("Ctrl+Alt+S").disabled(!app.has_history()).sep(), Some(Action::SaveVersion)),
        (MenuItem::new("Undo").shortcut("Ctrl+Z").disabled(!app.hist.can_undo()), Some(Action::Undo)),
        (MenuItem::new("Redo").shortcut("Ctrl+Shift+Z").disabled(!app.hist.can_redo()).sep(), Some(Action::Redo)),
        (MenuItem::new("Select all").shortcut("Ctrl+A"), Some(Action::SelectAll)),
        (MenuItem::new("Group selection").shortcut("Ctrl+G").disabled(!has_sel), Some(Action::Group)),
        (MenuItem::new("Frame selection").shortcut("Ctrl+Alt+G").disabled(!has_sel), Some(Action::FrameSelection)),
        (MenuItem::new("Add auto layout").shortcut("Shift+A").disabled(!has_sel).sep(), Some(Action::AutoLayout)),
        (MenuItem::new("Create component").shortcut("Ctrl+Alt+K").disabled(!has_sel), Some(Action::CreateComponent)),
        (MenuItem::new("Detach instance").shortcut("Ctrl+Alt+B").disabled(!has_sel).sep(), Some(Action::Detach)),
        (MenuItem::new("Export selection as PNG…").disabled(!has_sel), Some(Action::Export)),
        (MenuItem::new("Export code…").disabled(!has_sel), Some(Action::ExportCode)),
    ];
    if app.is_deck() {
        items.push((MenuItem::new("Export current slide as PDF…").disabled(app.deck_pdf_slides(false).is_err()), Some(Action::ExportSlidePdf)));
        items.push((MenuItem::new("Export all slides as PDF…").disabled(app.deck_pdf_slides(true).is_err()), Some(Action::ExportDeckPdf)));
    }
    items
}

fn zoom_items() -> Vec<(MenuItem, Option<Action>)> {
    vec![
        (MenuItem::new("Zoom in").shortcut("Ctrl+="), Some(Action::ZoomIn)),
        (MenuItem::new("Zoom out").shortcut("Ctrl+-").sep(), Some(Action::ZoomOut)),
        (MenuItem::new("Zoom to fit").shortcut("Shift+1"), Some(Action::FitAll)),
        (MenuItem::new("Zoom to selection").shortcut("Shift+2").sep(), Some(Action::FitSelection)),
        (MenuItem::new("Zoom to 50%"), Some(Action::ZoomTo(0.5))),
        (MenuItem::new("Zoom to 100%").shortcut("Shift+0"), Some(Action::Zoom100)),
        (MenuItem::new("Zoom to 200%"), Some(Action::ZoomTo(2.0))),
    ]
}

/// Shows the currently open menu (if any) and executes picked entries.
pub fn show(app: &mut App, ctx: &Context) {
    let Some(kind) = app.ui.menu else { return };
    let anchor = app.ui.menu_anchor;
    let id = menu_id(kind);
    let screen = ctx.content_rect();
    match kind {
        MenuKind::Palette => palette(app, ctx),
        MenuKind::ToolGroup(g) => {
            let nodes = tool_nodes(app, g);
            let items = to_items(&nodes);
            let h = theme::MENU_PADDING * 2.0 + items.len() as f32 * theme::MENU_ITEM_HEIGHT;
            let pos = super::toolbar::tool_menu_pos(screen, g, h);
            match popup_tree(ctx, id, pos, theme::TOOL_MENU_WIDTH, &items, anchor) {
                TreeResult::Picked(path) => {
                    app.ui.menu = None;
                    if let Some(a) = resolve(&nodes, &path) {
                        run(app, ctx, a);
                    }
                }
                TreeResult::Closed => app.ui.menu = None,
                TreeResult::Open => {}
            }
        }
        MenuKind::Main => {
            let nodes = main_nodes(app);
            let items = to_items(&nodes);
            match popup_tree(ctx, id, Pos2::new(theme::MAIN_MENU_X, theme::MAIN_MENU_Y), theme::MAIN_MENU_WIDTH, &items, anchor) {
                TreeResult::Picked(path) => {
                    app.ui.menu = None;
                    if let Some(a) = resolve(&nodes, &path) {
                        run(app, ctx, a);
                    }
                }
                TreeResult::Closed => app.ui.menu = None,
                TreeResult::Open => {}
            }
        }
        other => {
            let entries = match other {
                MenuKind::Zoom => zoom_items(),
                _ => file_items(app),
            };
            let items: Vec<MenuItem> = entries.iter().map(|e| e.0.clone()).collect();
            let a = anchor.unwrap_or(Rect::NOTHING);
            let pos = match other {
                MenuKind::FileMenu => a.right_top() + Vec2::new(4.0, 0.0),
                _ => a.left_bottom() + Vec2::new(0.0, 4.0),
            };
            match popup_menu(ctx, id, pos, theme::MENU_WIDTH, &items, anchor) {
                MenuResult::Picked(i) => {
                    app.ui.menu = None;
                    if let Some(act) = entries[i].1 {
                        run(app, ctx, act);
                    }
                }
                MenuResult::Closed => app.ui.menu = None,
                MenuResult::Open => {}
            }
        }
    }
}

// ---------------------------------------------------------------- command palette
struct Command {
    label: String,
    shortcut: String,
    act: Action,
    disabled: bool,
}

fn collect(nodes: &[Node], out: &mut Vec<Command>) {
    for n in nodes {
        match n.act {
            Some(act) if !matches!(act, Action::OpenPalette) => {
                out.push(Command { label: n.item.label.clone(), shortcut: n.item.shortcut.clone(), act, disabled: n.item.disabled })
            }
            _ => collect(&n.kids, out),
        }
    }
}

/// Every command with its shortcut: the whole main menu plus all tools.
fn commands(app: &App) -> Vec<Command> {
    let mut out = Vec::new();
    collect(&main_nodes(app), &mut out);
    out.retain(|c| c.label != "Libraries" && c.label != "Not available" && !c.label.starts_with("Version "));
    for g in GROUPS {
        for t in *g {
            out.push(Command {
                label: tool_menu_label(*t).trim_end_matches('…').into(),
                shortcut: t.shortcut().into(),
                act: Action::Tool(*t),
                disabled: false,
            });
        }
    }
    out.push(Command {
        label: "Combine as variants".into(),
        shortcut: "Ctrl+Shift+K".into(),
        act: Action::CombineVariants,
        disabled: app.selection.is_empty(),
    });
    out
}

const PALETTE_ROWS: usize = 8;
const PALETTE_SEARCH_H: f32 = 32.0;

fn matches_query(label: &str, q: &str) -> bool {
    let l = label.to_lowercase();
    q.split_whitespace().all(|w| l.contains(w))
}

fn palette(app: &mut App, ctx: &Context) {
    let cmds = commands(app);
    let q = app.ui.palette_query.to_lowercase();
    let list: Vec<usize> = (0..cmds.len()).filter(|i| matches_query(&cmds[*i].label, &q)).collect();
    let n = list.len();
    let (down, up, enter, esc) = ctx.input_mut(|i| {
        let m = Modifiers::NONE;
        (i.consume_key(m, Key::ArrowDown), i.consume_key(m, Key::ArrowUp), i.consume_key(m, Key::Enter), i.consume_key(m, Key::Escape))
    });
    if n > 0 {
        if down {
            app.ui.palette_sel = (app.ui.palette_sel + 1) % n;
        }
        if up {
            app.ui.palette_sel = (app.ui.palette_sel + n - 1) % n;
        }
    }
    app.ui.palette_sel = app.ui.palette_sel.min(n.saturating_sub(1));
    let first = (app.ui.palette_sel + 1).saturating_sub(PALETTE_ROWS);
    let shown = n.saturating_sub(first).min(PALETTE_ROWS);
    let screen = ctx.content_rect();
    let list_h = if n == 0 { theme::MENU_ITEM_HEIGHT } else { shown as f32 * theme::MENU_ITEM_HEIGHT };
    let h = theme::MENU_PADDING + PALETTE_SEARCH_H + widgets::menu::SEP_H + list_h + theme::MENU_PADDING;
    let rect =
        Rect::from_min_size(Pos2::new(screen.center().x - theme::PALETTE_WIDTH / 2.0, screen.top() + theme::PALETTE_TOP), Vec2::new(theme::PALETTE_WIDTH, h));
    let font = theme::font_body();
    let mut pick: Option<usize> = None;
    let mut hover: Option<usize> = None;
    let moved = ctx.input(|i| i.pointer.delta() != Vec2::ZERO);
    let ptr = ctx.input(|i| i.pointer.hover_pos());
    egui::Area::new(Id::new("palette")).order(egui::Order::Foreground).fixed_pos(rect.min).interactable(true).show(ctx, |ui| {
        ui.allocate_exact_size(rect.size(), Sense::hover());
        let p = ui.painter().clone();
        menu_panel(&p, rect, theme::MENU_RADIUS);
        let srow = Rect::from_min_size(Pos2::new(rect.left(), rect.top() + theme::MENU_PADDING), Vec2::new(rect.width(), PALETTE_SEARCH_H));
        icons::paint(
            &p,
            Icon::Search,
            Pos2::new(srow.left() + theme::MENU_ICON_X_PLAIN + theme::MENU_ICON_BOX / 2.0, srow.center().y),
            theme::MENU_ICON_SIZE,
            theme::ICON_SECONDARY,
        );
        let field = Rect::from_min_max(
            Pos2::new(srow.left() + theme::MENU_ICON_X_PLAIN + theme::MENU_ICON_BOX + theme::MENU_ICON_LABEL_GAP, srow.top()),
            Pos2::new(srow.right() - theme::MENU_SHORTCUT_PAD, srow.bottom()),
        );
        let tid = Id::new("palette-input");
        let mut text = app.ui.palette_query.clone();
        let te = egui::TextEdit::singleline(&mut text)
            .id(tid)
            .font(font.clone())
            .text_color(theme::TEXT)
            .frame(egui::Frame::NONE)
            .margin(egui::Margin::ZERO)
            .vertical_align(egui::Align::Center)
            .hint_text(egui::RichText::new("Search actions").font(font.clone()).color(theme::TEXT_TERTIARY))
            .desired_width(field.width());
        ui.put(field, te);
        ui.memory_mut(|m| m.request_focus(tid));
        if text != app.ui.palette_query {
            app.ui.palette_query = text;
            app.ui.palette_sel = 0;
        }
        widgets::hline(&p, rect.left(), rect.right(), srow.bottom() + theme::MENU_SEP_MARGIN, theme::BORDER_MENU);
        let top = srow.bottom() + widgets::menu::SEP_H;
        if n == 0 {
            p.text(
                Pos2::new(rect.left() + theme::MENU_LABEL_X_PLAIN, top + theme::MENU_ITEM_HEIGHT / 2.0),
                Align2::LEFT_CENTER,
                "No matching actions",
                font.clone(),
                theme::TEXT_DISABLED,
            );
        }
        for r in 0..shown {
            let li = first + r;
            let c = &cmds[list[li]];
            let row = Rect::from_min_size(Pos2::new(rect.left(), top + r as f32 * theme::MENU_ITEM_HEIGHT), Vec2::new(rect.width(), theme::MENU_ITEM_HEIGHT));
            let resp = ui.interact(row, Id::new(("palette-row", li)), if c.disabled { Sense::hover() } else { Sense::click() });
            let lit = app.ui.palette_sel == li && !c.disabled;
            if ptr.is_some_and(|p| row.contains(p)) {
                hover = Some(li);
            }
            if resp.clicked() && !c.disabled {
                pick = Some(list[li]);
            }
            if lit {
                p.rect_filled(row.shrink2(Vec2::new(theme::MENU_PILL_INSET, 0.0)), theme::cr(theme::MENU_PILL_RADIUS), theme::BG_MENU_SELECTED);
            }
            let col = if c.disabled { theme::TEXT_DISABLED } else { theme::TEXT };
            p.text(Pos2::new(row.left() + theme::MENU_LABEL_X_PLAIN, row.center().y), Align2::LEFT_CENTER, &c.label, font.clone(), col);
            if !c.shortcut.is_empty() {
                let sc = if c.disabled {
                    theme::TEXT_DISABLED
                } else if lit {
                    theme::MENU_SHORTCUT_HIGHLIGHT
                } else {
                    theme::TEXT_SECONDARY
                };
                p.text(Pos2::new(row.right() - theme::MENU_SHORTCUT_PAD, row.center().y), Align2::RIGHT_CENTER, &c.shortcut, font.clone(), sc);
            }
        }
    });
    if moved {
        if let Some(h) = hover {
            app.ui.palette_sel = h;
        }
    }
    if enter && n > 0 {
        let c = &cmds[list[app.ui.palette_sel]];
        if !c.disabled {
            pick = Some(list[app.ui.palette_sel]);
        }
    }
    let outside = ctx.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !rect.contains(p)));
    if esc || outside {
        app.ui.menu = None;
        return;
    }
    if let Some(ci) = pick {
        let act = cmds[ci].act;
        app.ui.menu = None;
        run(app, ctx, act);
    }
}

/// Palette rows (label, shortcut) matching `query`, for tests.
#[cfg(test)]
pub fn palette_entries(app: &App, query: &str) -> Vec<(String, String)> {
    let q = query.to_lowercase();
    commands(app).into_iter().filter(|c| matches_query(&c.label, &q)).map(|c| (c.label, c.shortcut)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_matching_is_word_based() {
        assert!(matches_query("Zoom to fit", "zoom fit"));
        assert!(matches_query("Zoom to fit", ""));
        assert!(!matches_query("Zoom to fit", "pen"));
    }

    #[test]
    fn help_shows_the_running_version_but_it_is_not_a_command() {
        fn find<'a>(nodes: &'a [Node], label: &str) -> Option<&'a Node> {
            nodes.iter().find_map(|n| if n.item.label == label { Some(n) } else { find(&n.kids, label) })
        }
        let app = App::new(desigma_doc::Document::new("t"));
        let nodes = main_nodes(&app);
        let help = find(&nodes, "Help").expect("a Help menu");
        let row = find(&help.kids, &format!("Version {}", crate::VERSION)).expect("a version row in Help");
        assert!(row.item.disabled, "the version is shown, not clickable");
        assert!(find(&help.kids, "Check for updates…").is_some_and(|n| !n.item.disabled));
        assert!(!commands(&app).iter().any(|c| c.label.starts_with("Version ")));
    }
}
