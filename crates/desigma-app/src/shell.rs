//! Window shell: lays out the panels, routes shortcuts and drives screenshots.

use crate::app::{App, MenuKind};
use crate::canvas::tools::Tool;
use crate::canvas::{self, interact};
use crate::cli::Options;
use crate::demo;
use crate::icons::Icon;
use crate::panels::menus::{self, Action};
use crate::panels::{self, left, rail, right, toolbar};
use crate::theme;
use crate::widgets;
use egui::{Align2, Context, Event, Key, Pos2, Rect, Vec2};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

/// Set once an `--screenshot` run has written its PNG.
pub static SHOT_OK: AtomicBool = AtomicBool::new(false);
/// Why a `--screenshot` run produced no file, for the exit message.
pub static SHOT_ERR: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

/// Frames to wait for a requested screenshot before asking again; a request made while the
/// window is still being resized or not yet presented can be dropped (#35).
const SHOT_RETRY_FRAMES: u32 = 30;
/// Frames after which a screenshot run gives up.
const SHOT_GIVE_UP_FRAMES: u32 = 600;

fn shot_error(msg: String) {
    if let Ok(mut e) = SHOT_ERR.lock() {
        *e = msg;
    }
}

struct Shot {
    path: PathBuf,
    size: (u32, u32),
    /// Device pixel ratio: the PNG is `size * scale` pixels, laid out as `size` points.
    scale: f32,
    frames: u32,
    /// Frame of the latest screenshot request, and how many were sent.
    requested_at: Option<u32>,
    requests: u32,
}

pub struct Shell {
    app: App,
    shot: Option<Shot>,
    title: String,
    /// Listens for links and files handed over by a second launch; `None` when another editor owns it.
    _instance: Option<crate::instance::Instance>,
}

/// A `projects` folder next to the library folder instead of inside it: a hand-made project
/// put one level too high, which the home screen would otherwise ignore without a word (#35).
pub(crate) fn stray_projects_dir(library: &std::path::Path) -> Option<PathBuf> {
    let stray = library.parent()?.join("projects");
    (stray.is_dir() && stray != library.join("projects")).then_some(stray)
}

impl Shell {
    pub fn new(cc: &eframe::CreationContext<'_>, mut opts: Options) -> Self {
        // A real launch always has a library; screenshot runs only touch it for the home screen.
        if opts.library.is_none() && (opts.screenshot.is_none() || opts.home) {
            opts.library = Some(desigma_library::Library::default_root());
        }
        opts.home = opts.wants_home();
        let screenshot = opts.screenshot.is_some();
        // "Open in new window" children leave update checks to the first window
        let launch = !screenshot && opts.open_id.is_none();
        let (gpu_flag, bridge_flag, library) = (opts.gpu, opts.bridge, opts.library.clone());
        // Saved UI language, else the system one on an interactive launch (screenshots stay English
        // unless a language was saved).
        crate::i18n::set(crate::i18n::startup(library.as_deref(), !screenshot));
        let mut shell = Self::build(&cc.egui_ctx, opts);
        if let Some(rs) = &cc.wgpu_render_state {
            if let Some(max) = canvas::gpu::init(rs) {
                shell.app.canvas.gpu.available = true;
                shell.app.canvas.gpu.set_max_texture(max);
            }
        }
        // GPU canvas on by default; screenshots stay on the deterministic CPU path unless --gpu
        let saved = library.as_deref().and_then(|l| crate::comments::settings::load_flag(l, "use_gpu"));
        shell.app.canvas.gpu.pref = if screenshot { gpu_flag } else { gpu_flag || saved.unwrap_or(true) };
        shell.app.updater.start(library.as_deref(), launch);
        if let Some(stray) = library.as_deref().and_then(stray_projects_dir) {
            let msg = format!(
                "Projects in {} are outside the library and are not shown; move them into {}",
                stray.display(),
                library.as_deref().map(|l| l.join("projects")).unwrap_or_default().display()
            );
            eprintln!("desigma: {msg}");
            if !screenshot {
                shell.app.show_toast(msg, true);
            }
        }
        // Links and files from a second launch (and, on macOS, from Finder and the browser)
        crate::instance::set_context(&cc.egui_ctx);
        #[cfg(target_os = "macos")]
        crate::macos::register();
        if launch {
            match crate::instance::Instance::start(&crate::instance::default_dir(), crate::instance::push) {
                Ok(i) => shell._instance = i,
                Err(e) => eprintln!("desigma: cannot listen for links: {e}"),
            }
        }
        // --bridge turns the agent bridge on for this launch only; the Preferences toggle saves it
        let saved_bridge = library.as_deref().and_then(|l| crate::comments::settings::load_flag(l, crate::bridge::FLAG)) == Some(true);
        if launch && (bridge_flag || saved_bridge) {
            if let Err(e) = shell.app.start_bridge(&cc.egui_ctx) {
                shell.app.show_toast(e, true);
            }
        }
        shell
    }

    pub fn build(ctx: &Context, opts: Options) -> Self {
        theme::install_fonts(ctx);
        theme::apply(ctx);
        // `--motion` on the plain demo opens the animated onboarding instead
        let motion_demo = opts.motion && opts.fixture.is_none() && opts.file.is_none() && opts.open_id.is_none() && !opts.home;
        let mut app = App::new(match opts.fixture.as_deref() {
            Some("no-selection") => demo::reference_document(),
            Some("variables") => demo::variables_document(),
            Some(crate::showcase::FIXTURE) => crate::showcase::document(),
            Some(crate::showcase::MOTION_FIXTURE) => crate::showcase::motion_document(),
            None if motion_demo => crate::showcase::motion_document(),
            Some(crate::protovars::FIXTURE) => crate::protovars::document(),
            Some("deck") => crate::decks::demo::document(),
            Some("board") => crate::board::demo::document(),
            Some("board-empty") => crate::board::create::new_document("Untitled"),
            Some(f) if crate::vecfixture::FIXTURES.contains(&f) => crate::vecfixture::document(),
            _ => demo::demo_document(),
        });
        if !matches!(opts.fixture.as_deref(), Some("variables" | "deck" | crate::showcase::FIXTURE | crate::showcase::MOTION_FIXTURE)) && !motion_demo {
            app.doc.name = "Untitled".into();
        }
        if opts.fixture.as_deref() == Some("deck") {
            if let Some(id) = app.deck_slides().first().copied() {
                app.select_slide(id);
            }
        }
        if opts.fixture.as_deref() == Some("comments") {
            crate::comments::fixture::apply(&mut app);
        }
        if let Some(f) = opts.fixture.as_deref().filter(|f| crate::vecfixture::FIXTURES.contains(f)) {
            crate::vecfixture::apply(&mut app, f);
        }
        if let Some(root) = opts.library.clone() {
            app.attach_library(root);
        }
        if let Some(f) = &opts.file {
            app.open_path(f.clone());
        }
        if let Some(link) = opts.join.clone() {
            app.open_join(Some(link));
        }
        if let Some(id) = &opts.open_id {
            app.open_library_file(id);
        } else if opts.home {
            app.go_home();
        }
        if let Some(name) = opts.select.as_ref().filter(|n| n.as_str() != "none") {
            // a top-level layer (screen) wins over same-named layers deeper in the tree
            let named =
                |top: bool| app.doc.nodes.values().find(|n| &n.name == name && (!top || n.parent.is_some_and(|p| app.doc.pages.contains(&p)))).map(|n| n.id);
            if let Some(id) = named(true).or_else(|| named(false)) {
                if let Some(pg) = app.doc.page_of(id) {
                    app.page = pg;
                }
                app.selection = vec![id];
            }
        }
        if let Some(spec) = &opts.menu {
            let (w, h) = opts.size.unwrap_or((theme::DEFAULT_WINDOW.0 as u32, theme::DEFAULT_WINDOW.1 as u32));
            let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(w as f32, h as f32));
            if !menus::preset_menu(ctx, &mut app, spec, screen) {
                eprintln!("desigma: unknown menu '{spec}'");
            }
        }
        if let Some(l) = &opts.dev {
            app.ui.dev_mode = true;
            app.ui.right_tab = Some(crate::app::RightTab::Design);
            app.dev.lang = match l.as_str() {
                "tailwind" => crate::dev::Language::Tailwind,
                "swiftui" => crate::dev::Language::SwiftUi,
                "compose" => crate::dev::Language::Compose,
                "react" => crate::dev::Language::React,
                _ => crate::dev::Language::Css,
            };
            if opts.dev_demo {
                crate::dev::preset::apply(&mut app);
            }
        }
        if opts.motion {
            crate::motion::timeline::open_demo(&mut app);
        }
        app.ui.rulers = opts.rulers;
        if opts.grid {
            app.ui.pixel_grid = true;
        }
        app.startup_zoom = opts.zoom.map(|z| z / 100.0);
        if let Some(kind) = &opts.layout_grid {
            let target = app.selection.first().copied();
            if let Some(id) = target {
                let k = ["grid", "columns", "rows"].iter().position(|n| n == kind).unwrap_or(1);
                // screenshot aid: a red 10% grid, columns with the usual 5 x 20px gutter
                let grid = crate::panels::layoutguide::default_grid(k, desigma_doc::Color::rgba(255, 0, 0, 26));
                app.edit_nodes(&[id], |n| {
                    if let Some(f) = n.kind.frame_props_mut() {
                        f.grids.push(grid.clone());
                    }
                });
            }
        }
        if let Some(name) = &opts.edit_text {
            let found = app.doc.nodes.values().find(|n| &n.name == name && matches!(n.kind, desigma_doc::NodeKind::Text(_))).map(|n| n.id);
            if let Some(id) = found {
                if let Some(pg) = app.doc.page_of(id) {
                    app.page = pg;
                }
                canvas::interact::begin_text_edit(&mut app, id);
                if let (Some((a, b)), Some(sess)) = (opts.select_text, app.canvas.text.as_mut()) {
                    if let Some(desigma_doc::NodeKind::Text(t)) = app.doc.get(id).map(|n| &n.kind) {
                        let byte = |c: usize| t.content.char_indices().nth(c).map(|(i, _)| i).unwrap_or(t.content.len());
                        sess.anchor = byte(a);
                        sess.caret = byte(b);
                    }
                }
            } else {
                eprintln!("desigma: no text layer named '{name}'");
            }
        }
        if opts.tab.as_deref() == Some("prototype") {
            app.ui.right_tab = Some(crate::app::RightTab::Prototype);
        }
        if opts.present {
            app.start_present();
            if let Some(st) = &mut app.proto.present {
                st.start_fullscreen = opts.fullscreen;
            }
            if opts.panel.as_deref() == Some("present-variables") {
                if opts.fixture.as_deref() == Some(crate::protovars::FIXTURE) {
                    crate::protovars::prime_present(&mut app);
                } else if let Some(st) = app.proto.present.as_mut() {
                    st.show_vars = true;
                }
            }
        }
        if let Some(q) = &opts.find {
            app.find.query = q.clone();
            app.open_find();
            app.find.focus = false;
            if let Some(r) = &opts.replace_with {
                app.find.replacement = r.clone();
                app.find.show_replace = true;
            }
        }
        match opts.panel.as_deref() {
            Some("interaction") => {
                app.ui.right_tab = Some(crate::app::RightTab::Prototype);
                crate::protovars::open_editor(&mut app);
            }
            Some("present-variables") => {}
            Some("libraries") => app.open_libraries(),
            Some(panel @ ("plugins" | "plugin-install")) => {
                app.open_plugins();
                app.plugins.install_form = panel == "plugin-install";
                app.plugins.fresh = app.plugins.install_form;
            }
            Some("version-history") => crate::history::preset::apply(&mut app, opts.library.clone()),
            Some(p @ ("motion-inspector" | "motion-easing" | "motion-library" | "motion-clip")) => crate::motion::inspector::preset(&mut app, p),
            Some(p @ ("dev-annotate" | "dev-compare")) => crate::dev::preset::panel(&mut app, p),
            Some("share") => app.open_share(),
            Some("collab") => crate::collab::preview(&mut app),
            Some(p @ ("updater" | "updater-banner")) => crate::updater::preset(&mut app, p),
            Some(panel @ ("variables" | "variables-floating")) => {
                app.vars.full = panel == "variables";
                app.open_variables();
            }
            Some(spec) => {
                let (w, h) = opts.size.unwrap_or((theme::DEFAULT_WINDOW.0 as u32, theme::DEFAULT_WINDOW.1 as u32));
                panels::properties::preset_popup(&mut app, spec, Vec2::new(w as f32, h as f32));
            }
            None => {}
        }
        let shot = opts.screenshot.map(|path| Shot {
            path,
            size: opts.size.unwrap_or((theme::DEFAULT_WINDOW.0 as u32, theme::DEFAULT_WINDOW.1 as u32)),
            scale: opts.scale.unwrap_or(1.0),
            frames: 0,
            requested_at: None,
            requests: 0,
        });
        Self { app, shot, title: String::new(), _instance: None }
    }

    fn drive_screenshot(&mut self, ctx: &Context) {
        let Some(shot) = &mut self.shot else { return };
        shot.frames += 1;
        ctx.request_repaint();
        if shot.frames == 1 {
            // `scale` physical pixels per egui point (1 by default, so the PNG matches the requested size).
            ctx.set_pixels_per_point(shot.scale);
            let npp = ctx.input(|i| i.viewport().native_pixels_per_point).unwrap_or(1.0).max(0.1);
            let phys = Vec2::new(shot.size.0 as f32 * shot.scale, shot.size.1 as f32 * shot.scale);
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(phys / npp));
        }
        let events = ctx.input(|i| i.events.clone());
        for ev in events {
            if let Event::Screenshot { image, .. } = ev {
                let (w, h) = (image.width() as u32, image.height() as u32);
                let raw: Vec<u8> = image.pixels.iter().flat_map(|c| c.to_array()).collect();
                if let Some(buf) = image::RgbaImage::from_raw(w, h, raw) {
                    let want = ((shot.size.0 as f32 * shot.scale).round() as u32, (shot.size.1 as f32 * shot.scale).round() as u32);
                    let buf = if (w, h) != want { image::imageops::resize(&buf, want.0, want.1, image::imageops::FilterType::Lanczos3) } else { buf };
                    if let Some(dir) = shot.path.parent() {
                        let _ = std::fs::create_dir_all(dir);
                    }
                    match buf.save(&shot.path) {
                        Ok(()) => SHOT_OK.store(true, Ordering::SeqCst),
                        Err(e) => shot_error(format!("cannot write {}: {e}", shot.path.display())),
                    }
                } else {
                    shot_error(format!("the captured frame ({w}x{h}) had no pixel data"));
                }
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                return;
            }
        }
        let sized = {
            let r = ctx.content_rect();
            (r.width() - shot.size.0 as f32).abs() < 1.5 && (r.height() - shot.size.1 as f32).abs() < 1.5
        };
        let ready = (shot.frames >= 12 && sized) || shot.frames >= 60;
        // A request can be dropped (a resize in flight, a frame never presented): ask again
        // until a frame arrives instead of waiting on a single request.
        let due = match shot.requested_at {
            None => ready,
            Some(at) => shot.frames >= at + SHOT_RETRY_FRAMES,
        };
        if due {
            shot.requested_at = Some(shot.frames);
            shot.requests += 1;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        if shot.frames > SHOT_GIVE_UP_FRAMES {
            let r = ctx.content_rect();
            shot_error(format!(
                "no frame arrived after {} request(s) in {} frames (window {:.0}x{:.0}, wanted {}x{})",
                shot.requests,
                shot.frames,
                r.width(),
                r.height(),
                shot.size.0,
                shot.size.1
            ));
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn handle_keys(&mut self, ctx: &Context) {
        if self.app.plugins.open
            || self.app.dialog.is_some()
            || self.app.history.save_modal.is_some()
            || self.app.dev.compare.is_some()
            || self.app.updater.dialog
        {
            return;
        }
        let typing = ctx.egui_wants_keyboard_input();
        if self.app.vars.open {
            // The Variables view handles its own keys (and undo / redo).
            return;
        }
        // Open menus own the keyboard (arrows, Enter, Escape, typing in the palette).
        if self.app.ui.menu.is_some() || self.app.ui.ctx_menu.is_some() || self.app.vars.open {
            return;
        }
        let editing_text = self.app.canvas.text.is_some();
        if typing && !editing_text {
            return;
        }
        let events = ctx.input(|i| i.events.clone());
        for ev in events {
            match ev {
                Event::Copy if !editing_text && crate::motion::timeline::copy_keys(&mut self.app, ctx) => {}
                Event::Paste(_) if !editing_text && crate::motion::timeline::paste_keys(&mut self.app, ctx) => {}
                Event::Copy if !editing_text => {
                    if let Some(s) = self.app.copy_selection() {
                        ctx.copy_text(s);
                    }
                }
                Event::Cut if !editing_text => {
                    if let Some(s) = self.app.cut_selection() {
                        ctx.copy_text(s);
                    }
                }
                Event::Paste(s) if !editing_text => self.app.paste(Some(&s)),
                Event::Key { key, pressed: true, modifiers, .. } => {
                    if editing_text {
                        continue;
                    }
                    self.key(ctx, key, modifiers);
                }
                _ => {}
            }
        }
    }

    fn key(&mut self, ctx: &Context, key: Key, m: egui::Modifiers) {
        let app = &mut self.app;
        let cmd = m.command;
        if app.motion.enabled && crate::motion::timeline::key(app, key, m) {
            return;
        }
        if cmd {
            let act = match (key, m.shift, m.alt) {
                (Key::E, false, _) => Some(Action::Flatten),
                (Key::O, false, true) => Some(Action::OutlineStroke),
                (Key::J, false, false) => Some(Action::VectorJoin),
                (Key::J, true, false) => Some(Action::VectorSplit),
                (Key::Z, false, _) => Some(Action::Undo),
                (Key::Z, true, _) | (Key::Y, false, _) => Some(Action::Redo),
                (Key::O, false, _) => Some(Action::Open),
                (Key::S, false, true) => Some(Action::SaveVersion),
                (Key::S, false, _) => Some(Action::Save),
                (Key::S, true, _) => Some(Action::SaveAs),
                (Key::D, false, _) => Some(Action::Duplicate),
                (Key::G, false, false) => Some(Action::Group),
                (Key::G, true, _) => Some(Action::Ungroup),
                (Key::G, false, true) => Some(Action::FrameSelection),
                (Key::K, false, true) => Some(Action::CreateComponent),
                (Key::B, false, true) => Some(Action::Detach),
                (Key::K, true, false) => Some(Action::CombineVariants),
                (Key::A, true, false) => Some(Action::SelectInverse),
                (Key::A, false, true) => Some(Action::SelectMatching),
                (Key::A, false, _) => Some(Action::SelectAll),
                (Key::F, false, false) => Some(Action::Find),
                (Key::Quote, false, _) => Some(Action::TogglePixelGrid),
                (Key::Quote, true, _) => Some(Action::ToggleSnapPixel),
                (Key::Num4, true, _) => Some(Action::ToggleLayoutGrids),
                (Key::B, false, false) => Some(Action::TextBold),
                (Key::I, false, false) => Some(Action::TextItalic),
                (Key::U, false, false) => Some(Action::TextUnderline),
                (Key::Period, true, false) => Some(Action::TextSizeUp),
                (Key::Comma, true, false) => Some(Action::TextSizeDown),
                (Key::Num8, true, false) => Some(Action::TextBulletList),
                (Key::Num7, true, false) => Some(Action::TextNumberedList),
                (Key::CloseBracket, false, _) => Some(Action::BringForward),
                (Key::OpenBracket, false, _) => Some(Action::SendBackward),
                (Key::CloseBracket, true, _) => Some(Action::BringToFront),
                (Key::OpenBracket, true, _) => Some(Action::SendToBack),
                (Key::Equals | Key::Plus, _, _) => Some(Action::ZoomIn),
                (Key::Minus, _, _) => Some(Action::ZoomOut),
                (Key::Backslash, _, _) => Some(Action::ToggleSidebar),
                (Key::K, false, false) => Some(Action::OpenPalette),
                (Key::Enter, false, true) => {
                    app.start_present();
                    None
                }
                _ => None,
            };
            if let Some(a) = act {
                menus::run(app, ctx, a);
            }
            return;
        }
        if app.canvas.vedit.is_some() && canvas::vector::key(app, key, m) {
            return;
        }
        if app.canvas.builder.is_some() && matches!(key, Key::Escape | Key::Enter) {
            // a brush stroke in progress is dropped first; otherwise the result is applied
            if matches!(app.canvas.inter, canvas::Interaction::Builder) {
                canvas::builder::cancel_brush(app);
            } else {
                canvas::builder::finish(app);
                app.tool = Tool::Move;
            }
            return;
        }
        match key {
            // Shift+Backspace picks the eraser on a board
            Key::Backspace if m.shift && app.is_board() => app.set_tool(Tool::Eraser),
            Key::Delete | Key::Backspace => menus::run(app, ctx, Action::Delete),
            // an open comment draft or thread takes the Escape itself (egui drops text focus on Escape,
            // so the shortcut handler cannot tell that the key was meant for the text box)
            Key::Escape if app.comments.captures_escape() => {}
            Key::Escape => {
                if !matches!(app.canvas.inter, canvas::Interaction::None) {
                    app.cancel_interaction();
                } else if app.canvas.pen.is_some() {
                    interact::finish_pen(app);
                    app.tool = Tool::Move;
                } else if app.ui.menu.is_some() {
                    app.ui.menu = None;
                } else if app.proto.editing.is_some() {
                    app.proto.editing = None;
                } else if app.find.open {
                    app.close_find();
                } else if app.layers.filter_open {
                    app.close_layer_filter();
                } else if app.tool != Tool::Move {
                    app.tool = Tool::Move;
                } else {
                    app.clear_selection();
                }
            }
            Key::Enter => {
                if app.canvas.pen.is_some() {
                    interact::finish_pen(app);
                    app.tool = Tool::Move;
                } else if m.shift {
                    menus::run(app, ctx, Action::SelectParent);
                } else if let Some(n) = app.selection.first().filter(|_| app.selection.len() == 1).and_then(|s| app.doc.get(*s)) {
                    // A single text or vector layer is entered for editing; anything else selects its children.
                    let id = n.id;
                    if matches!(n.kind, desigma_doc::NodeKind::Text(_))
                        || matches!(n.kind, desigma_doc::NodeKind::Sticky(_) | desigma_doc::NodeKind::ShapeWithText(_) | desigma_doc::NodeKind::CodeBlock(_))
                    {
                        interact::begin_text_edit(app, id);
                    } else if matches!(n.kind, desigma_doc::NodeKind::Table(_)) {
                        crate::board::table::tab_selected(app, true);
                    } else if matches!(n.kind, desigma_doc::NodeKind::BoardSection(_)) {
                        crate::board::section::begin_rename(app, id);
                    } else if matches!(n.kind, desigma_doc::NodeKind::Connector(_)) {
                        crate::board::create::edit_label(app, id);
                    } else if canvas::vector::is_vector(app, id) {
                        // vectors and basic shapes (which become vectors on the first edit)
                        canvas::vector::enter(app, id);
                    } else {
                        menus::run(app, ctx, Action::SelectChildren);
                    }
                } else {
                    menus::run(app, ctx, Action::SelectChildren);
                }
            }
            // board: Tab starts the next sticky note to the right of the selected one
            Key::Tab if app.is_board() => {
                if m.shift {
                    crate::board::table::tab_selected(app, false);
                } else if !crate::board::create::tab_next(app) {
                    crate::board::table::tab_selected(app, true);
                }
            }
            Key::Backslash => menus::run(app, ctx, Action::SelectParent),
            Key::ArrowLeft | Key::ArrowRight | Key::ArrowUp | Key::ArrowDown => {
                if !app.selection.is_empty() {
                    let step = if m.shift { theme::NUDGE_BIG } else { theme::NUDGE };
                    let (dx, dy) = match key {
                        Key::ArrowLeft => (-step, 0.0),
                        Key::ArrowRight => (step, 0.0),
                        Key::ArrowUp => (0.0, -step),
                        _ => (0.0, step),
                    };
                    app.nudge(dx, dy);
                }
            }
            Key::Num1 if m.shift => menus::run(app, ctx, Action::FitAll),
            Key::Num2 if m.shift => menus::run(app, ctx, Action::FitSelection),
            Key::Num0 if m.shift => menus::run(app, ctx, Action::Zoom100),
            Key::A if m.shift => menus::run(app, ctx, Action::AutoLayout),
            Key::R if m.shift => menus::run(app, ctx, Action::ToggleRulers),
            Key::Quote if m.shift => menus::run(app, ctx, Action::TogglePixelGrid),
            k => {
                if !m.alt {
                    let tool = if app.is_board() { Tool::from_key_board(k, m.shift) } else { Tool::from_key(k, m.shift) };
                    if let Some(t) = tool {
                        app.set_tool(t);
                    }
                }
            }
        }
    }

    /// Right-click on the canvas selects the layer under the pointer and opens the selection menu.
    fn context_menu(&mut self, ui: &egui::Ui, canvas_rect: Rect) {
        let (clicked, pos) = ui.ctx().input(|i| (i.pointer.secondary_clicked(), i.pointer.interact_pos()));
        let Some(pos) = pos.filter(|p| clicked && canvas_rect.contains(*p) && !self.app.vars.open) else { return };
        let app = &mut self.app;
        if !matches!(app.canvas.inter, canvas::Interaction::None) || app.canvas.text.is_some() {
            return;
        }
        let page = app.view.to_page(app.canvas_rect.min, pos);
        match interact::hit_under(app, page, false) {
            Some(hit) if !app.is_selected(hit) => app.select_only(hit),
            Some(_) => {}
            None => app.clear_selection(),
        }
        app.ui.ctx_menu = Some(pos);
    }

    fn toast(&mut self, ui: &egui::Ui, screen: Rect) {
        let Some(t) = &self.app.toast else { return };
        if self.app.now > t.until {
            self.app.toast = None;
            return;
        }
        let p = ui.painter();
        let font = theme::font_body();
        let text = crate::i18n::tr_text(&t.text);
        let w = widgets::text_width(p, &text, &font) + 48.0;
        let r = Rect::from_center_size(
            Pos2::new(screen.center().x, screen.bottom() - theme::TOAST_BOTTOM - theme::TOAST_HEIGHT / 2.0),
            Vec2::new(w, theme::TOAST_HEIGHT),
        );
        widgets::elevation_shadow(p, r, theme::TOAST_RADIUS);
        p.rect_filled(r, theme::cr(theme::TOAST_RADIUS), theme::BG_MENU);
        p.circle_filled(Pos2::new(r.left() + 16.0, r.center().y), 4.0, if t.error { theme::DANGER } else { theme::BG_BRAND });
        p.text(Pos2::new(r.left() + 28.0, r.center().y), Align2::LEFT_CENTER, text, font, theme::TEXT);
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
    }
}

impl eframe::App for Shell {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.17, 0.17, 0.17, 1.0]
    }

    /// Tab belongs to the text session (it nests list items): take it out of the input before egui
    /// uses it to move keyboard focus between widgets.
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        if self.app.plugins.open {
            return;
        }
        let Some(sess) = self.app.canvas.text.as_mut() else { return };
        if ctx.memory(|m| m.focused()).is_some() {
            return;
        }
        raw.events.retain(|e| match e {
            egui::Event::Key { key: egui::Key::Tab, pressed, modifiers, .. } if !modifiers.command => {
                if *pressed {
                    sess.tabs.push(modifiers.shift);
                }
                false
            }
            _ => true,
        });
    }

    /// Runs even while the window is minimized or occluded (eframe then skips `ui`), so agent
    /// calls keep being answered; the bridge server requests a repaint for each one.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.app.now = ctx.input(|i| i.time);
        self.app.poll_external_change(ctx);
        if self.app.ext_watch.is_some() {
            // Keep polling even while nothing else asks for frames.
            ctx.request_repaint_after(std::time::Duration::from_millis(1000));
        }
        self.app.poll_bridge();
        self.handle_opens(ctx);
        crate::collab::pump(&mut self.app, ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.app.poll_fonts();
        if self.app.fonts.is_loading() {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
        }
        self.draw(ui);
    }
}

impl Shell {
    /// Links and files handed over by a second launch or the OS (see `instance.rs`): a link
    /// opens Join session prefilled, a file opens like the CLI path argument, and the window
    /// comes to the front.
    fn handle_opens(&mut self, ctx: &Context) {
        let opens = crate::instance::take();
        if opens.is_empty() {
            return;
        }
        for open in opens {
            match open {
                crate::instance::Open::Link(link) => self.app.request_open(crate::instance::Open::Link(link)),
                crate::instance::Open::File(path) => self.app.request_open(crate::instance::Open::File(path)),
            }
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        // when the OS refuses to steal focus this flashes the taskbar / bounces the Dock instead
        ctx.send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(egui::UserAttentionType::Informational));
        ctx.request_repaint();
    }

    /// Files dropped on the window: `.svg` files are placed at the drop point,
    /// anything else (the first file) is opened as a document.
    fn handle_drops(&mut self, ctx: &Context) {
        let (files, pos) = ctx.input(|i| (i.raw.dropped_files.clone(), i.pointer.hover_pos().or(i.pointer.latest_pos())));
        let is_svg = |f: &egui::DroppedFile| {
            let name = f.path.as_ref().and_then(|p| p.to_str()).unwrap_or(&f.name);
            name.rsplit('.').next().is_some_and(|e| e.eq_ignore_ascii_case("svg"))
        };
        if let Some(path) = files.iter().filter(|f| !is_svg(f)).find_map(|f| f.path.clone()) {
            self.app.request_open(crate::instance::Open::File(path));
            return;
        }
        for (nth, f) in files.iter().filter(|f| is_svg(f)).enumerate() {
            let text = match (&f.path, &f.bytes) {
                (Some(p), _) => std::fs::read_to_string(p).map_err(|e| format!("Cannot read {}: {e}", p.display())),
                (None, Some(b)) => String::from_utf8(b.to_vec()).map_err(|_| "The SVG is not valid UTF-8".to_string()),
                (None, None) => continue,
            };
            let stem = f
                .path
                .as_deref()
                .and_then(|p| p.file_stem())
                .and_then(|s| s.to_str())
                .or_else(|| f.name.strip_suffix(".svg"))
                .filter(|s| !s.is_empty())
                .unwrap_or("svg")
                .to_string();
            match text {
                Ok(t) => crate::clipboard::drop_svg(&mut self.app, &t, &stem, pos, nth),
                Err(e) => self.app.show_toast(e, true),
            }
        }
    }

    fn sync_title(&mut self, ctx: &Context) {
        let title = if self.app.home.is_some() { theme::APP_TITLE.to_string() } else { self.app.title() };
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
    }

    /// One frame of the home screen (file browser): files dropped on the window are imported.
    fn home_frame(&mut self, ui: &mut egui::Ui, ctx: &Context) {
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect());
        if !dropped.is_empty() && self.app.dialog.is_none() {
            let target = self.app.home_target().or_else(|| self.app.home.as_ref().and_then(|h| h.snap.projects.first().map(|p| (p.id.clone(), None))));
            if let Some((p, f)) = target {
                self.app.import_into_library(&dropped, &p, f.as_deref());
            }
        }
        crate::home::show(&mut self.app, ui);
        crate::home::dialog::show(&mut self.app, ctx);
        crate::collab::ui::join_dialog(&mut self.app, ctx);
        crate::unsaved::dialog(&mut self.app, ctx);
        crate::extwatch::dialog(&mut self.app, ctx);
        let full = ui.max_rect();
        self.toast(ui, full);
        self.sync_title(ctx);
    }

    /// One full UI pass over the window.
    pub fn draw(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.app.now = ctx.input(|i| i.time);
        self.app.poll_plugins();
        self.app.poll_updater(&ctx);
        self.app.poll_install(&ctx);
        self.drive_screenshot(&ctx);
        if ctx.input(|i| i.viewport().close_requested()) {
            crate::decks::ui::flush_notes(&mut self.app);
            self.app.save_close_version();
            self.app.flush_autosave();
            // "Restart to update": everything is saved, so the new version can start now
            crate::updater::install::run_stashed();
        }
        self.app.scrub_sync(&ctx);
        if self.app.home.is_some() {
            self.home_frame(ui, &ctx);
            crate::plugins::ui::show(&mut self.app, &ctx);
            crate::updater::ui::show(&mut self.app, &ctx);
            return;
        }
        self.app.autosave_tick(&ctx);
        self.app.history_tick();
        self.app.dev_tick(&ctx);
        self.app.motion_tick(&ctx);
        if self.app.proto.present.is_some() {
            crate::present::show(&mut self.app, ui);
            return;
        }
        self.handle_keys(&ctx);
        if !self.app.plugins.open {
            self.handle_drops(&ctx);
        }

        let full = ui.max_rect();
        ui.painter().rect_filled(full, 0.0, theme::BG);
        let rail_w = theme::RAIL_WIDTH + theme::BORDER_WIDTH;
        let panel_w = theme::LEFT_PANEL_WIDTH + theme::BORDER_WIDTH;
        let rail_rect = Rect::from_min_max(full.min, Pos2::new(full.left() + rail_w, full.bottom()));
        let left_rect = Rect::from_min_max(Pos2::new(rail_rect.right(), full.top()), Pos2::new(rail_rect.right() + panel_w, full.bottom()));
        // a board has no right panel: its contextual toolbar floats over the canvas instead
        let board = self.app.is_board();
        let right_w = if board { 0.0 } else { theme::RIGHT_PANEL_WIDTH + theme::BORDER_WIDTH };
        let right_rect = Rect::from_min_max(Pos2::new(full.right() - right_w, full.top()), full.max);
        let canvas_left = if self.app.ui.left_collapsed { rail_rect.right() } else { left_rect.right() };
        let deck = self.app.is_deck();
        let notes_h = if deck { 132.0_f32.min(full.height() * 0.28) } else { 0.0 };
        let canvas_rect = Rect::from_min_max(Pos2::new(canvas_left, full.top()), Pos2::new(right_rect.left(), full.bottom() - notes_h));
        let notes_rect = Rect::from_min_max(Pos2::new(canvas_left, canvas_rect.bottom()), Pos2::new(right_rect.left(), full.bottom()));
        // Motion mode: the timeline docks under the canvas and the toolbar floats above it.
        let motion_on = self.app.motion.enabled && !board && !deck;
        let (canvas_rect, timeline_rect) = if motion_on {
            let (c, t) = crate::motion::timeline::split(&ctx, canvas_rect);
            (c, Some(t))
        } else {
            (canvas_rect, None)
        };

        let full_vars = self.app.vars.open && self.app.vars.full;
        if full_vars {
            // The local-variables full view replaces the left panel, canvas and right panel.
            rail::show(&mut self.app, ui, rail_rect);
            let content = Rect::from_min_max(Pos2::new(rail_rect.right(), full.top()), full.max);
            crate::variables::view::show(&mut self.app, ui, content);
        } else {
            canvas::show(&mut self.app, ui, canvas_rect);
            if let Some(tr) = timeline_rect {
                crate::motion::timeline::show(&mut self.app, ui, tr);
            }
            if deck {
                crate::decks::ui::notes(&mut self.app, ui, notes_rect);
            }
            rail::show(&mut self.app, ui, rail_rect);
            if self.app.ui.left_collapsed {
                let b = Rect::from_min_size(Pos2::new(canvas_rect.left() + 8.0, canvas_rect.top() + 8.0), Vec2::splat(theme::PANEL_ACTION_BUTTON));
                ui.painter().rect_filled(b, theme::cr(theme::RADIUS_MEDIUM), theme::BG);
                if widgets::icon_button(ui, b, "show-sidebar", Icon::Sidebar, false, "Show sidebar").clicked() {
                    self.app.ui.left_collapsed = false;
                }
            } else if deck {
                let mode = Rect::from_min_size(left_rect.min, Vec2::new(left_rect.width(), 36.0));
                crate::decks::ui::mode_bar(&mut self.app, ui, mode);
                let body = Rect::from_min_max(Pos2::new(left_rect.left(), mode.bottom()), left_rect.max);
                if self.app.decks.design_mode {
                    left::show(&mut self.app, ui, body);
                } else {
                    crate::decks::ui::sidebar(&mut self.app, ui, body);
                }
            } else {
                left::show(&mut self.app, ui, left_rect);
            }
            if board {
                crate::board::toolbar::show(&mut self.app, ui, canvas_rect_for_toolbar(full));
                crate::board::bar::show(&mut self.app, ui, canvas_rect);
                crate::board::section::rename_ui(&mut self.app, ui, canvas_rect);
                crate::board::canvas::zoom_pill(&mut self.app, ui, canvas_rect);
                crate::board::session::show(&mut self.app, ui, canvas_rect);
                crate::collab::ui::board_strip(&mut self.app, ui, canvas_rect);
            } else {
                right::show(&mut self.app, ui, right_rect);
                let tb = if deck {
                    canvas_rect
                } else if timeline_rect.is_some() {
                    Rect::from_min_max(full.min, Pos2::new(full.right(), canvas_rect.bottom()))
                } else {
                    canvas_rect_for_toolbar(full)
                };
                toolbar::show(&mut self.app, ui, tb);
            }
            crate::history::banner::show(&mut self.app, ui, canvas_rect);
            crate::collab::ui::banner(&mut self.app, ui, canvas_rect);
            self.context_menu(ui, canvas_rect);
            menus::show(&mut self.app, &ctx);
            panels::context::show(&mut self.app, &ctx);
            // the floating panel (or only the inspector's binding picker when closed)
            crate::variables::view::show(&mut self.app, ui, full);
        }
        crate::home::dialog::show(&mut self.app, &ctx);
        crate::history::modal::show(&mut self.app, &ctx);
        crate::dev::compare::show(&mut self.app, &ctx);
        crate::libraries::show(&mut self.app, &ctx);
        crate::plugins::ui::show(&mut self.app, &ctx);
        crate::updater::ui::show(&mut self.app, &ctx);
        crate::bridge::ui::show(&mut self.app, &ctx);
        crate::collab::ui::share_popover(&mut self.app, &ctx);
        crate::collab::ui::requests(&mut self.app, &ctx);
        crate::collab::ui::join_dialog(&mut self.app, &ctx);
        crate::unsaved::dialog(&mut self.app, &ctx);
        crate::extwatch::dialog(&mut self.app, &ctx);
        self.toast(ui, full);

        self.sync_title(&ctx);
        if self.app.ui.menu == Some(MenuKind::Main) && self.app.ui.menu_anchor.is_none() {
            self.app.ui.menu = None;
        }
    }
}

/// The toolbar is centered on the window (not the canvas).
fn canvas_rect_for_toolbar(full: Rect) -> Rect {
    full
}

#[cfg(test)]
mod tests;
