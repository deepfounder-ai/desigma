//! File → Export code…: turns the selected frame into a small project on disk.
//!
//! The dialog picks the target (HTML + CSS, React, Tailwind, SwiftUI, Compose),
//! whether React uses Tailwind classes, and whether to write `tokens.css`. The
//! files themselves come from `desigma_export::code_export`, so the CLI and the
//! MCP server write exactly what the dialog writes.

use crate::app::App;
use crate::theme;
use crate::widgets;
use desigma_doc::NodeId;
use desigma_export::code_export::{export_project, project_name, CodeOptions, CodeTarget};
use egui::{Align2, Color32, Context, Id, Key, Modifiers, Order, Pos2, Rect, Sense, Ui, Vec2};

/// The open dialog's state.
#[derive(Clone, Debug)]
pub struct CodeExportDialog {
    /// The layer being exported.
    pub node: NodeId,
    pub target: CodeTarget,
    pub react_tailwind: bool,
    pub tokens: bool,
    /// Last error, shown inside the dialog.
    pub error: Option<String>,
}

impl CodeExportDialog {
    pub fn new(node: NodeId) -> Self {
        Self { node, target: CodeTarget::Html, react_tailwind: false, tokens: true, error: None }
    }

    fn options(&self) -> CodeOptions {
        CodeOptions { target: self.target, react_tailwind: self.react_tailwind, tokens: self.tokens, embed_fonts: true, asset_dir: "assets".into() }
    }
}

impl App {
    /// File → Export code…: opens the dialog for the first selected layer.
    pub fn export_code_dialog(&mut self) {
        let Some(node) = self.selection.first().copied() else {
            self.show_toast("Select a frame to export", true);
            return;
        };
        if !exportable(&self.doc, node) {
            self.show_toast("Export code works on frames, sections, components and pages", true);
            return;
        }
        self.code_export = Some(CodeExportDialog::new(node));
    }

    /// Asks for a folder and writes the project there.
    pub fn run_code_export(&mut self, d: &CodeExportDialog) {
        let Some(dir) = rfd::FileDialog::new().set_title("Export code to folder").pick_folder() else { return };
        self.write_code_export(d, &dir);
    }

    /// Writes the project for `d` under `dir/<project name>`.
    pub fn write_code_export(&mut self, d: &CodeExportDialog, dir: &std::path::Path) {
        let doc = self.export_doc().into_owned();
        let project = match export_project(&doc, d.node, &d.options()) {
            Ok(p) => p,
            Err(e) => {
                self.show_toast(e, true);
                return;
            }
        };
        let root = dir.join(project_name(&doc, d.node));
        match project.write_to(&root) {
            Ok(files) => {
                self.code_export = None;
                self.show_toast(format!("Exported {} files to {}", files.len(), root.display()), false);
            }
            Err(e) => self.show_toast(e, true),
        }
    }
}

/// `--export-code FILE --out DIR`: writes the project without opening a window.
pub fn run_cli(file: &std::path::Path, code: Option<&str>, out: Option<&std::path::Path>, select: Option<&str>, react_tailwind: bool, no_tokens: bool) -> Result<String, String> {
    let doc = crate::io::load_path(file)?;
    let target = match code {
        Some(c) => CodeTarget::parse(c).ok_or_else(|| format!("unknown code target '{c}'"))?,
        None => CodeTarget::Html,
    };
    let node = match select {
        Some(name) if !name.eq_ignore_ascii_case("none") => doc
            .nodes
            .values()
            .find(|n| n.name.eq_ignore_ascii_case(name) && exportable(&doc, n.id))
            .map(|n| n.id)
            .ok_or_else(|| format!("no frame named '{name}'"))?,
        _ => doc
            .pages
            .iter()
            .find_map(|p| {
                doc.get(*p)
                    .and_then(|p| p.children.iter().find_map(|c| doc.get(*c).filter(|n| exportable(&doc, n.id)).map(|n| n.id)))
            })
            .ok_or("the file has no frame to export")?,
    };
    let out = out.ok_or("--export-code needs --out DIR")?;
    let opts = CodeOptions { target, react_tailwind, tokens: !no_tokens, embed_fonts: true, asset_dir: "assets".into() };
    let project = export_project(&doc, node, &opts)?;
    let root = out.join(project_name(&doc, node));
    let files = project.write_to(&root)?;
    Ok(format!("wrote {} files to {}", files.len(), root.display()))
}

fn exportable(doc: &desigma_doc::Document, node: NodeId) -> bool {
    doc.get(node).is_some_and(|n| {
        use desigma_doc::NodeKind::*;
        matches!(n.kind, Frame(_) | Section | Component(_) | ComponentSet(_) | Page { .. } | Group)
    })
}

const W: f32 = 460.0;
const PAD: f32 = 20.0;
const BTN_H: f32 = 32.0;
const ROW_H: f32 = 26.0;

/// Draws the dialog when it is open. Call once per frame.
pub fn show(app: &mut App, ctx: &Context) {
    let Some(mut d) = app.code_export.take() else { return };
    let screen = ctx.content_rect();
    let (enter, esc) = ctx.input_mut(|i| (i.consume_key(Modifiers::NONE, Key::Enter), i.consume_key(Modifiers::NONE, Key::Escape)));
    let text_w = W - PAD * 2.0;
    let react = d.target == CodeTarget::React;
    let mut h = PAD + 24.0 + 12.0;
    h += 20.0 + 8.0; // "Export as" label
    h += ROW_H * CodeTarget::ALL.len() as f32 + 8.0;
    if react {
        h += ROW_H;
    }
    h += ROW_H; // tokens
    if d.error.is_some() {
        h += 34.0;
    }
    h += BTN_H + PAD;
    let rect = Rect::from_center_size(screen.center() - Vec2::new(0.0, 24.0), Vec2::new(W, h));

    let submit = enter;
    let mut cancel = esc;
    let mut export = false;
    egui::Area::new(Id::new("code-export")).order(Order::Foreground).fixed_pos(screen.min).interactable(true).show(ctx, |ui| {
        let scrim = ui.allocate_rect(screen, Sense::click_and_drag());
        let p = ui.painter().clone();
        p.rect_filled(screen, 0.0, Color32::from_black_alpha(140));
        if scrim.clicked() && !rect.contains(scrim.interact_pointer_pos().unwrap_or(Pos2::ZERO)) {
            cancel = true;
        }
        widgets::menu_panel(&p, rect, theme::MENU_RADIUS);
        let x0 = rect.left() + PAD;
        let mut y = rect.top() + PAD;
        p.text(Pos2::new(x0, y + 12.0), Align2::LEFT_CENTER, "Export code", theme::font_large_strong(), theme::TEXT);
        y += 24.0 + 12.0;
        p.text(Pos2::new(x0, y + 10.0), Align2::LEFT_CENTER, "Export as", theme::font_small_medium(), theme::TEXT_SECONDARY);
        y += 20.0 + 8.0;

        for (i, target) in CodeTarget::ALL.into_iter().enumerate() {
            let r = Rect::from_min_size(Pos2::new(x0, y), Vec2::new(text_w, ROW_H));
            let resp = ui.interact(r, Id::new(("code-target", i)), Sense::click());
            let on = d.target == target;
            if on {
                p.rect_filled(r.shrink2(Vec2::new(2.0, 1.0)), theme::cr(theme::RADIUS_MEDIUM), theme::BG_HOVER);
            }
            let cx = Pos2::new(r.left() + 10.0, r.center().y);
            p.circle_stroke(cx, 6.0, egui::Stroke::new(1.5, if on { theme::BG_BRAND } else { theme::TEXT_TERTIARY }));
            if on {
                p.circle_filled(cx, 3.5, theme::BG_BRAND);
            }
            p.text(Pos2::new(r.left() + 26.0, r.center().y), Align2::LEFT_CENTER, target.label(), theme::font_body(), theme::TEXT);
            let files = match target {
                CodeTarget::Html => "index.html + styles.css",
                CodeTarget::React => "App.jsx + styles.css",
                CodeTarget::Tailwind => "index.html (Tailwind CDN)",
                CodeTarget::SwiftUI => "View.swift",
                CodeTarget::Compose => "Screen.kt",
            };
            p.text(Pos2::new(r.right() - 10.0, r.center().y), Align2::RIGHT_CENTER, files, theme::font_small(), theme::TEXT_TERTIARY);
            widgets::record(ui, &format!("code-target-{i}"), r);
            if resp.clicked() {
                d.target = target;
            }
            y += ROW_H;
        }
        y += 8.0;

        if react {
            toggle(ui, &p, x0, &mut y, text_w, "Style with Tailwind classes", "code-react-tailwind", &mut d.react_tailwind);
        }
        toggle(ui, &p, x0, &mut y, text_w, "Write tokens.css with the design variables", "code-tokens", &mut d.tokens);

        if let Some(e) = &d.error {
            let g = p.layout(e.clone(), theme::font_small(), theme::DANGER, text_w);
            p.galley(Pos2::new(x0, y), g, theme::DANGER);
        }

        let ok_w = widgets::text_width(&p, "Export…", &theme::font_body_strong()).max(48.0) + 32.0;
        let ok_r = Rect::from_min_size(Pos2::new(rect.right() - PAD - ok_w, rect.bottom() - PAD - BTN_H), Vec2::new(ok_w, BTN_H));
        let resp = ui.interact(ok_r, Id::new("code-export-ok"), Sense::click());
        let bg = if resp.hovered() || resp.is_pointer_button_down_on() { theme::BG_BRAND_HOVER } else { theme::BG_BRAND };
        p.rect_filled(ok_r, theme::cr(theme::RADIUS_MEDIUM), bg);
        p.text(ok_r.center(), Align2::CENTER_CENTER, "Export…", theme::font_body_strong(), Color32::WHITE);
        widgets::record(ui, "code-export-ok", ok_r);
        if resp.clicked() || submit {
            export = true;
        }
        let cw = widgets::text_width(&p, "Cancel", &theme::font_body()) + 32.0;
        let cr = Rect::from_min_size(Pos2::new(ok_r.left() - 8.0 - cw, ok_r.top()), Vec2::new(cw, BTN_H));
        let cresp = ui.interact(cr, Id::new("code-export-cancel"), Sense::click());
        widgets::button_bg(&p, cr, &cresp, false, theme::cr(theme::RADIUS_MEDIUM));
        p.text(cr.center(), Align2::CENTER_CENTER, "Cancel", theme::font_body(), theme::TEXT);
        widgets::record(ui, "code-export-cancel", cr);
        if cresp.clicked() {
            cancel = true;
        }
    });

    if cancel {
        return;
    }
    if export {
        app.run_code_export(&d);
        // the file dialog may have been cancelled: keep the settings around
        if app.code_export.is_none() {
            app.code_export = Some(d);
        }
        return;
    }
    app.code_export = Some(d);
}

fn toggle(ui: &mut Ui, p: &egui::Painter, x0: f32, y: &mut f32, w: f32, label: &str, id: &str, on: &mut bool) {
    let r = Rect::from_min_size(Pos2::new(x0, *y), Vec2::new(w, ROW_H));
    let resp = ui.interact(r, Id::new(id), Sense::click());
    p.text(Pos2::new(r.left(), r.center().y), Align2::LEFT_CENTER, label, theme::font_body(), theme::TEXT_SECONDARY);
    let tr = Rect::from_center_size(Pos2::new(r.right() - 14.0, r.center().y), Vec2::new(28.0, 16.0));
    p.rect_filled(tr, theme::cr(8), if *on { theme::BG_BRAND } else { theme::BG_TERTIARY });
    p.circle_filled(Pos2::new(if *on { tr.right() - 8.0 } else { tr.left() + 8.0 }, tr.center().y), 6.0, theme::TEXT);
    widgets::record(ui, id, r);
    if resp.clicked() {
        *on = !*on;
    }
    *y += ROW_H;
}

#[cfg(test)]
mod tests {
    use super::*;
    use desigma_doc::{Document, Node, Rect as DRect};

    fn frame_doc() -> (Document, NodeId) {
        let mut d = Document::new("t");
        let page = d.add_page("p");
        let id = d.alloc_id();
        let mut n = Node::frame(id, DRect::new(0.0, 0.0, 100.0, 100.0));
        n.name = "Card".into();
        let id = d.insert(page, n).unwrap();
        (d, id)
    }

    #[test]
    fn only_containers_are_exportable() {
        let (mut d, frame) = frame_doc();
        assert!(exportable(&d, frame));
        let page = d.pages[0];
        assert!(exportable(&d, page));
        let id = d.alloc_id();
        let rect = d.insert(frame, Node::rectangle(id, DRect::new(0.0, 0.0, 10.0, 10.0))).unwrap();
        assert!(!exportable(&d, rect));
    }

    #[test]
    fn dialog_defaults_to_html_with_tokens() {
        let (_, frame) = frame_doc();
        let d = CodeExportDialog::new(frame);
        assert_eq!(d.target, CodeTarget::Html);
        assert!(d.tokens);
        assert!(!d.react_tailwind);
        assert_eq!(d.options().target, CodeTarget::Html);
    }
}
