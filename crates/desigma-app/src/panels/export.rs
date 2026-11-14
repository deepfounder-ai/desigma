//! Inspector "Export" section: per-layer export rows (scale + format), a
//! preview thumbnail, the export button and a "Copy as" context menu.

use crate::app::App;
use crate::icons::{self, Icon};
use crate::theme;
use crate::widgets::field;
use crate::widgets::menu::{dropdown, popup_menu, MenuItem, MenuResult};
use crate::widgets::{self, elide};
use desigma_doc::{Document, ExportConstraint, ExportFileFormat, ExportPreset, NodeId, Rect as DRect};
use desigma_export::{ExportFormat, ExportScale, ExportSetting};
use egui::{Align2, Id, Pos2, Rect, Sense, TextureHandle, TextureOptions, Ui, Vec2};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

const ROW_GAP: f32 = 8.0;
const PREVIEW_H: f32 = 120.0;
// scale | suffix | format | remove, filling the 184px content width.
const COL_GAP: f32 = 4.0;
const SCALE_W: f32 = 56.0;
const SUFFIX_W: f32 = 36.0;
const FORMAT_W: f32 = 56.0;
const THUMB_CHECK_SECONDS: f64 = 0.3;

/// What "Copy as" can put on the clipboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CopyKind {
    Png,
    Svg,
    Css,
}

pub const COPY_ITEMS: [(CopyKind, &str); 3] = [(CopyKind::Png, "Copy as PNG"), (CopyKind::Svg, "Copy as SVG"), (CopyKind::Css, "Copy as CSS")];

#[derive(Clone)]
struct Thumb {
    tex: TextureHandle,
    node: NodeId,
    fingerprint: u64,
    checked: f64,
}

/// The inspector's scale parsing ("2x", "512w", "512h") as the stored constraint.
pub fn constraint_of(s: ExportScale) -> ExportConstraint {
    match s {
        ExportScale::Factor(f) => ExportConstraint::Scale(f),
        ExportScale::Width(w) => ExportConstraint::Width(w as f32),
        ExportScale::Height(h) => ExportConstraint::Height(h as f32),
    }
}

/// Inverse of [`constraint_of`].
pub fn scale_of(c: ExportConstraint) -> ExportScale {
    match c {
        ExportConstraint::Scale(f) => ExportScale::Factor(f),
        ExportConstraint::Width(w) => ExportScale::Width(w.round().max(1.0) as u32),
        ExportConstraint::Height(h) => ExportScale::Height(h.round().max(1.0) as u32),
    }
}

fn format_of(f: ExportFileFormat) -> Option<ExportFormat> {
    match f {
        ExportFileFormat::Png => Some(ExportFormat::Png),
        ExportFileFormat::Jpg => Some(ExportFormat::Jpg),
        ExportFileFormat::Svg => Some(ExportFormat::Svg),
        ExportFileFormat::Pdf => None,
    }
}

fn file_format_of(f: ExportFormat) -> ExportFileFormat {
    match f {
        ExportFormat::Png => ExportFileFormat::Png,
        ExportFormat::Jpg => ExportFileFormat::Jpg,
        ExportFormat::Svg => ExportFileFormat::Svg,
    }
}

fn format_label(f: ExportFileFormat) -> &'static str {
    match format_of(f) {
        Some(x) => x.label(),
        None => "PDF",
    }
}

/// Renderable form of a stored preset; `None` for formats that cannot be written (PDF).
pub fn setting_of(p: &ExportPreset) -> Option<ExportSetting> {
    Some(ExportSetting { scale: scale_of(p.constraint), format: format_of(p.format)? })
}

/// Output file name: layer name + the preset's suffix (or the `@2x` style default).
pub fn preset_file_name(p: &ExportPreset, layer: &str) -> String {
    let s = setting_of(p).unwrap_or_default();
    let name = s.file_name(layer);
    if p.suffix.is_empty() {
        return name;
    }
    let ext = s.format.extension();
    format!("{}{}.{ext}", desigma_export::sanitize_file_name(layer), p.suffix)
}

/// Rows every selected node has (in the first node's order). With one node: all its rows.
pub fn shared_rows(doc: &Document, sel: &[NodeId]) -> Vec<ExportPreset> {
    let Some(first) = sel.first().and_then(|id| doc.get(*id)) else { return vec![] };
    first.export_settings.iter().filter(|p| sel[1..].iter().all(|id| doc.get(*id).is_some_and(|n| n.export_settings.contains(p)))).cloned().collect()
}

/// One edit of the shared rows.
#[derive(Clone, Debug, PartialEq)]
pub enum RowEdit {
    Add(ExportPreset),
    Replace(ExportPreset, ExportPreset),
    Remove(ExportPreset),
}

impl App {
    /// Applies a row edit to every node in `sel` as one undo step.
    pub fn edit_export_rows(&mut self, sel: &[NodeId], edit: &RowEdit) {
        self.edit_nodes(sel, |n| match edit {
            RowEdit::Add(p) => n.export_settings.push(p.clone()),
            RowEdit::Replace(old, new) => {
                if let Some(slot) = n.export_settings.iter_mut().find(|p| *p == old) {
                    *slot = new.clone();
                }
            }
            RowEdit::Remove(old) => {
                if let Some(i) = n.export_settings.iter().position(|p| p == old) {
                    n.export_settings.remove(i);
                }
            }
        });
    }
}

fn fingerprint(doc: &Document, root: NodeId) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    doc.walk(root, |n, _| format!("{n:?}").hash(&mut h));
    for (k, v) in &doc.images {
        k.hash(&mut h);
        v.len().hash(&mut h);
    }
    h.finish()
}

fn thumbnail(app: &App, ui: &Ui, node: NodeId) -> Option<Thumb> {
    let key = Id::new("export-thumb");
    let ctx = ui.ctx();
    let cached = ctx.data(|d| d.get_temp::<Thumb>(key));
    if let Some(t) = &cached {
        if t.node == node && app.now - t.checked < THUMB_CHECK_SECONDS {
            return cached;
        }
    }
    let fp = fingerprint(&app.doc, node);
    if let Some(mut t) = cached {
        if t.node == node && t.fingerprint == fp {
            t.checked = app.now;
            ctx.data_mut(|d| d.insert_temp(key, t.clone()));
            return Some(t);
        }
    }
    let b: DRect = desigma_export::render_bounds(&app.export_doc(), node);
    if b.w <= 0.0 || b.h <= 0.0 {
        return None;
    }
    // Render at 2x the preview box so it stays sharp on hi-dpi screens.
    let scale = (theme::SECTION_CONTENT_W * 2.0 / b.w).min(PREVIEW_H * 2.0 / b.h).clamp(0.01, 2.0);
    let pm = desigma_export::render_export(&app.export_doc(), node, scale)?;
    let img = egui::ColorImage::from_rgba_premultiplied([pm.width() as usize, pm.height() as usize], pm.data());
    let tex = ctx.load_texture("export-preview", img, TextureOptions::LINEAR);
    let t = Thumb { tex, node, fingerprint: fp, checked: app.now };
    ctx.data_mut(|d| d.insert_temp(key, t.clone()));
    Some(t)
}

/// Draws the Export section starting at absolute `(x0, top)` and returns the
/// height it used. `x0` is the panel's outer left edge.
pub fn section(app: &mut App, ui: &mut Ui, x0: f32, top: f32, sel: &[NodeId]) -> f32 {
    let Some(&first) = sel.first() else { return 0.0 };
    let name = app.doc.get(first).map(|n| n.name.clone()).unwrap_or_default();
    let rows = shared_rows(&app.doc, sel);
    let any_rows = sel.iter().any(|id| app.doc.get(*id).is_some_and(|n| !n.export_settings.is_empty()));
    let mut edit: Option<RowEdit> = None;
    let at = |x: f32, dy: f32, w: f32, h: f32| Rect::from_min_size(Pos2::new(x0 + x, top + dy), Vec2::new(w, h));
    let mut y = 0.0;

    // Title + add button.
    let title = at(0.0, 0.0, theme::RIGHT_PANEL_WIDTH - 1.0, theme::SECTION_TITLE_HEIGHT);
    ui.painter().text(
        Pos2::new(title.left() + theme::SECTION_CONTENT_X, title.center().y),
        Align2::LEFT_CENTER,
        "Export",
        theme::font_body_strong(),
        theme::TEXT,
    );
    // Created before the button so the button sits on top of it.
    let title_resp = ui.interact(title, ui.id().with("export-title"), Sense::click());
    let plus = at(theme::SECTION_TRAILING_X, 8.0, theme::PANEL_ACTION_BUTTON, theme::PANEL_ACTION_BUTTON);
    if widgets::icon_button(ui, plus, "export-add", Icon::Plus, false, "Add export setting").clicked() {
        edit = Some(RowEdit::Add(rows.last().cloned().unwrap_or_default()));
    }
    y += theme::SECTION_TITLE_HEIGHT;

    // Rows (the ones every selected layer has).
    for (i, old) in rows.iter().enumerate() {
        let mut row = old.clone();
        let mut changed = false;
        let scale_r = at(theme::SECTION_CONTENT_X, y, SCALE_W, theme::FIELD_HEIGHT);
        let suffix_r = at(theme::SECTION_CONTENT_X + SCALE_W + COL_GAP, y, SUFFIX_W, theme::FIELD_HEIGHT);
        let fmt_r = at(theme::SECTION_CONTENT_X + SCALE_W + SUFFIX_W + COL_GAP * 2.0, y, FORMAT_W, theme::FIELD_HEIGHT);
        let minus =
            at(theme::SECTION_CONTENT_X + theme::SECTION_CONTENT_W - theme::PANEL_ACTION_BUTTON, y, theme::PANEL_ACTION_BUTTON, theme::PANEL_ACTION_BUTTON);

        // Scale: editable text ("2x", "512w") with a presets menu behind the chevron.
        let text_r = Rect::from_min_max(scale_r.min, Pos2::new(scale_r.right() - 20.0, scale_r.bottom()));
        let chev_r = Rect::from_min_max(Pos2::new(scale_r.right() - 20.0, scale_r.top()), scale_r.max);
        let fid = ui.id().with(("export-scale", first, i));
        field::field_bg(ui, scale_r, fid);
        if let Some(s) = field::edit_text(ui, text_r, fid, &scale_of(row.constraint).to_string(), theme::font_body(), theme::TEXT, 4.0) {
            if let Some(sc) = ExportScale::parse(&s) {
                row.constraint = constraint_of(sc);
                changed = true;
            }
        }
        let chev = ui.interact(chev_r, ui.id().with(("export-scale-chev", first, i)), Sense::click());
        icons::paint(ui.painter(), Icon::ChevronDown, chev_r.center(), 12.0, if chev.hovered() { theme::ICON } else { theme::ICON_SECONDARY });
        let menu_id = ui.id().with(("export-scale-menu", first, i));
        let open = ui.data(|d| d.get_temp::<bool>(menu_id)).unwrap_or(false);
        if chev.clicked() {
            ui.data_mut(|d| d.insert_temp(menu_id, !open));
        }
        if open {
            let cur = scale_of(row.constraint);
            let items: Vec<MenuItem> = ExportScale::PRESETS.iter().map(|s| MenuItem::new(s.to_string()).checked(*s == cur)).collect();
            match popup_menu(ui.ctx(), menu_id.with("popup"), scale_r.left_bottom() + Vec2::new(0.0, 2.0), scale_r.width().max(64.0), &items, Some(scale_r)) {
                MenuResult::Picked(p) => {
                    row.constraint = constraint_of(ExportScale::PRESETS[p]);
                    changed = true;
                    ui.data_mut(|d| d.insert_temp(menu_id, false));
                }
                MenuResult::Closed => {
                    ui.data_mut(|d| d.insert_temp(menu_id, false));
                }
                MenuResult::Open => {}
            }
        }

        // Suffix: empty means the automatic "@2x" style name.
        let sid = ui.id().with(("export-suffix", first, i));
        field::field_bg(ui, suffix_r, sid);
        if let Some(s) = field::edit_text(ui, suffix_r, sid, &row.suffix, theme::font_body(), theme::TEXT, 6.0) {
            row.suffix = s.trim().to_string();
            changed = true;
        }
        if row.suffix.is_empty() && !ui.memory(|m| m.has_focus(sid)) {
            ui.painter().text(Pos2::new(suffix_r.left() + 6.0, suffix_r.center().y), Align2::LEFT_CENTER, "Suffix", theme::font_body(), theme::TEXT_TERTIARY);
        }

        let options: Vec<String> = ExportFormat::ALL.iter().map(|f| f.label().to_string()).collect();
        if let Some(p) = dropdown(ui, fmt_r, ("export-format", first, i), format_label(row.format), &options) {
            row.format = file_format_of(ExportFormat::ALL[p]);
            changed = true;
        }
        if widgets::icon_button(ui, minus, ("export-remove", i), Icon::Minus, false, "Remove export setting").clicked() {
            edit = Some(RowEdit::Remove(old.clone()));
        } else if changed && row != *old {
            edit = Some(RowEdit::Replace(old.clone(), row));
        }
        y += theme::FIELD_HEIGHT + ROW_GAP;
    }
    if let Some(e) = edit.take() {
        app.edit_export_rows(sel, &e);
    }
    if rows.is_empty() && any_rows {
        // Layers disagree and share no row: say so, as Figma does.
        ui.painter().text(
            Pos2::new(x0 + theme::SECTION_CONTENT_X, top + y + theme::FIELD_HEIGHT / 2.0),
            Align2::LEFT_CENTER,
            "Mixed",
            theme::font_body(),
            theme::TEXT_TERTIARY,
        );
        y += theme::FIELD_HEIGHT + ROW_GAP;
    }
    if !any_rows {
        context_menu(app, ui, &title_resp, first);
        return y + 8.0 + anim_block(app, ui, x0, top + y + 8.0, first);
    }

    // Preview.
    let prev = at(theme::SECTION_CONTENT_X, y, theme::SECTION_CONTENT_W, PREVIEW_H);
    ui.painter().rect_filled(prev, theme::cr(theme::RADIUS_MEDIUM), theme::BG_SECONDARY);
    if let Some(t) = thumbnail(app, ui, first) {
        // The texture holds up to 2x the logical size; never show it larger than that.
        let sz = t.tex.size_vec2();
        let k = ((prev.width() - 16.0) / sz.x).min((prev.height() - 16.0) / sz.y).min(0.5);
        let draw = Rect::from_center_size(prev.center(), sz * k);
        ui.painter().image(t.tex.id(), draw, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), egui::Color32::WHITE);
    }
    let prev_resp = ui.interact(prev, ui.id().with("export-preview-hit"), Sense::click());
    y += PREVIEW_H + ROW_GAP;

    // Export button.
    let btn = at(theme::SECTION_CONTENT_X, y, theme::SECTION_CONTENT_W, theme::FIELD_HEIGHT);
    let resp = ui.interact(btn, ui.id().with("export-btn"), Sense::click());
    ui.painter().rect_filled(btn, theme::cr(theme::RADIUS_MEDIUM), if resp.hovered() { theme::BG_TERTIARY } else { theme::BG_SECONDARY });
    icons::paint(ui.painter(), Icon::Export, Pos2::new(btn.left() + 16.0, btn.center().y), 14.0, theme::ICON_SECONDARY);
    let label = if sel.len() > 1 { format!("Export {} layers", sel.len()) } else { format!("Export {name}") };
    let label = elide(ui.painter(), &label, &theme::font_body(), btn.width() - 44.0);
    ui.painter().text(Pos2::new(btn.left() + 32.0, btn.center().y), Align2::LEFT_CENTER, label, theme::font_body(), theme::TEXT);
    if resp.clicked() {
        run_export(app, sel);
    }
    context_menu(app, ui, &resp, first);
    context_menu(app, ui, &prev_resp, first);
    context_menu(app, ui, &title_resp, first);
    let y = y + theme::FIELD_HEIGHT + 12.0;
    y + anim_block(app, ui, x0, top + y, first)
}

const ANIM_FORMAT_W: f32 = 62.0;
const ANIM_FPS_W: f32 = 62.0;
const ANIM_SCALE_W: f32 = 52.0;

/// "Animation" block under the export rows, shown when the layer's page has Motion keyframes:
/// format / frame rate / scale pickers and the export button (a progress bar with Cancel while an
/// export runs). Returns the height used, 0 when the page is not animated.
fn anim_block(app: &mut App, ui: &mut Ui, x0: f32, top: f32, node: NodeId) -> f32 {
    use crate::motion::playback::export::{format_label, fps_label, scale_label, FPS_CHOICES, SCALE_CHOICES};
    use desigma_export::anim::AnimFormat;
    if !app.has_motion_export(node) {
        return 0.0;
    }
    let at = |x: f32, dy: f32, w: f32, h: f32| Rect::from_min_size(Pos2::new(x0 + x, top + dy), Vec2::new(w, h));
    let mut y = 0.0;
    ui.painter().text(
        Pos2::new(x0 + theme::SECTION_CONTENT_X, top + theme::SECTION_TITLE_HEIGHT / 2.0),
        Align2::LEFT_CENTER,
        "Animation",
        theme::font_body_strong(),
        theme::TEXT,
    );
    y += theme::SECTION_TITLE_HEIGHT;
    let running = app.canvas.motion.export.running();
    let cur = app.canvas.motion.export.opts.clone();
    let x = theme::SECTION_CONTENT_X;
    let fmt_r = at(x, y, ANIM_FORMAT_W, theme::FIELD_HEIGHT);
    let fps_r = at(x + ANIM_FORMAT_W + COL_GAP, y, ANIM_FPS_W, theme::FIELD_HEIGHT);
    let scale_r = at(x + ANIM_FORMAT_W + ANIM_FPS_W + COL_GAP * 2.0, y, ANIM_SCALE_W, theme::FIELD_HEIGHT);
    let formats: Vec<String> = AnimFormat::ALL.iter().map(|f| format_label(*f).to_string()).collect();
    let fpses: Vec<String> = FPS_CHOICES.iter().map(|f| fps_label(*f)).collect();
    let scales: Vec<String> = SCALE_CHOICES.iter().map(|s| scale_label(*s)).collect();
    let opts = &mut app.canvas.motion.export.opts;
    if let Some(i) = dropdown(ui, fmt_r, ("anim-format", node), format_label(cur.format), &formats) {
        opts.format = AnimFormat::ALL[i];
    }
    if let Some(i) = dropdown(ui, fps_r, ("anim-fps", node), &fps_label(cur.fps), &fpses) {
        opts.fps = FPS_CHOICES[i];
    }
    if let Some(i) = dropdown(ui, scale_r, ("anim-scale", node), &scale_label(cur.scale), &scales) {
        opts.scale = SCALE_CHOICES[i];
    }
    y += theme::FIELD_HEIGHT + ROW_GAP;

    let btn = at(x, y, theme::SECTION_CONTENT_W, theme::FIELD_HEIGHT);
    if running {
        let p = app.canvas.motion.export.progress();
        ui.painter().rect_filled(btn, theme::cr(theme::RADIUS_MEDIUM), theme::BG_SECONDARY);
        let fill = Rect::from_min_size(btn.min, Vec2::new(btn.width() * p, btn.height()));
        ui.painter().rect_filled(fill, theme::cr(theme::RADIUS_MEDIUM), theme::BG_TERTIARY);
        ui.painter().text(
            Pos2::new(btn.left() + 12.0, btn.center().y),
            Align2::LEFT_CENTER,
            format!("Exporting {}%", (p * 100.0).round() as u32),
            theme::font_body(),
            theme::TEXT,
        );
        let cancel = Rect::from_center_size(Pos2::new(btn.right() - 14.0, btn.center().y), Vec2::splat(theme::PANEL_ACTION_BUTTON));
        if widgets::icon_button(ui, cancel, "anim-export-cancel", Icon::Close, false, "Cancel export").clicked() {
            app.canvas.motion.export.cancel();
        }
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
    } else {
        let resp = ui.interact(btn, ui.id().with("anim-export-btn"), Sense::click());
        ui.painter().rect_filled(btn, theme::cr(theme::RADIUS_MEDIUM), if resp.hovered() { theme::BG_TERTIARY } else { theme::BG_SECONDARY });
        icons::paint(ui.painter(), Icon::Export, Pos2::new(btn.left() + 16.0, btn.center().y), 14.0, theme::ICON_SECONDARY);
        let tl = app.doc.page_of(node).map(|p| app.doc.timeline(p)).unwrap_or_default();
        let label = format!("Export {} - {} fps", format_label(cur.format), cur.effective_fps(&tl));
        ui.painter().text(Pos2::new(btn.left() + 32.0, btn.center().y), Align2::LEFT_CENTER, label, theme::font_body(), theme::TEXT);
        if resp.clicked() {
            app.export_animation_dialog(node);
        }
    }
    y + theme::FIELD_HEIGHT + 12.0
}

/// Opens/draws the "Copy as" menu for right clicks on `anchor`.
fn context_menu(app: &mut App, ui: &mut Ui, anchor: &egui::Response, node: NodeId) {
    let id = Id::new("export-ctx-menu");
    if anchor.secondary_clicked() {
        if let Some(p) = anchor.interact_pointer_pos() {
            ui.data_mut(|d| d.insert_temp(id, p));
        }
    }
    let Some(pos) = ui.data(|d| d.get_temp::<Pos2>(id)) else { return };
    // Only the response that opened it draws it (all three share one position).
    if !anchor.rect.contains(pos) {
        return;
    }
    let items: Vec<MenuItem> = COPY_ITEMS.iter().map(|(_, l)| MenuItem::new(*l)).collect();
    match popup_menu(ui.ctx(), id.with("popup"), pos, 150.0, &items, None) {
        MenuResult::Picked(i) => {
            ui.data_mut(|d| d.remove_temp::<Pos2>(id));
            copy_as(app, node, COPY_ITEMS[i].0);
        }
        MenuResult::Closed => {
            ui.data_mut(|d| d.remove_temp::<Pos2>(id));
        }
        MenuResult::Open => {}
    }
}

/// Puts `node` on the clipboard as PNG (2x), SVG text or CSS text.
pub fn copy_as(app: &mut App, node: NodeId, kind: CopyKind) {
    let result: Result<&str, String> = (|| {
        let mut cb = arboard::Clipboard::new().map_err(|e| e.to_string())?;
        match kind {
            CopyKind::Png => {
                let pm = desigma_export::render_export(&app.export_doc(), node, 2.0).ok_or("Nothing to copy")?;
                let (w, h) = (pm.width() as usize, pm.height() as usize);
                let bytes = desigma_export::rgba_bytes(&pm);
                cb.set_image(arboard::ImageData { width: w, height: h, bytes: bytes.into() }).map_err(|e| e.to_string())?;
                Ok("Copied as PNG")
            }
            CopyKind::Svg => {
                let svg = desigma_export::export_svg(&app.export_doc(), node, &desigma_export::SvgOptions::default());
                cb.set_text(svg).map_err(|e| e.to_string())?;
                Ok("Copied as SVG")
            }
            CopyKind::Css => {
                cb.set_text(desigma_export::css_for(&app.export_doc(), node)).map_err(|e| e.to_string())?;
                Ok("Copied as CSS")
            }
        }
    })();
    match result {
        Ok(msg) => app.show_toast(msg, false),
        Err(e) => app.show_toast(format!("Copy failed: {e}"), true),
    }
}

/// Writes every selected layer with its own export rows. One file asks for a
/// path; several ask for a folder and use `name@2x.png` style names.
pub fn run_export(app: &mut App, ids: &[NodeId]) {
    let mut jobs: Vec<(String, NodeId, ExportSetting)> = Vec::new();
    let mut seen: HashMap<String, u32> = HashMap::new();
    let mut skipped_pdf = false;
    for id in ids {
        let Some(n) = app.doc.get(*id) else { continue };
        for p in &n.export_settings {
            let Some(s) = setting_of(p) else {
                skipped_pdf = true;
                continue;
            };
            let mut file = preset_file_name(p, &n.name);
            let count = seen.entry(file.clone()).or_insert(0);
            *count += 1;
            if *count > 1 {
                let stem = file.rsplit_once('.').map(|(a, _)| a.to_string()).unwrap_or_else(|| file.clone());
                file = format!("{stem} {count}.{}", s.format.extension());
            }
            jobs.push((file, *id, s.clone()));
        }
    }
    if jobs.is_empty() {
        app.show_toast(if skipped_pdf { "PDF export is not supported" } else { "Nothing to export" }, true);
        return;
    }
    if jobs.len() == 1 {
        let (file, id, s) = jobs.into_iter().next().expect("len == 1");
        let label = s.format.label();
        let ext = s.format.extension();
        let file_name = file.clone();
        app.pick_file_async(
            move || rfd::FileDialog::new().add_filter(label, &[ext]).set_file_name(file_name).save_file(),
            move |app, path| {
                let bytes = s.render(&app.doc, id);
                if bytes.is_empty() {
                    app.show_toast("Nothing to export", true);
                    return;
                }
                match std::fs::write(&path, bytes) {
                    Ok(()) => app.show_toast(format!("Exported {}", path.display()), false),
                    Err(e) => app.show_toast(format!("Export failed: {e}"), true),
                }
            },
        );
    } else {
        app.pick_file_async(
            || rfd::FileDialog::new().set_title("Choose export folder").pick_folder(),
            move |app, dir| {
                let mut written = Vec::new();
                for (file, id, s) in &jobs {
                    let bytes = s.render(&app.doc, *id);
                    if bytes.is_empty() {
                        continue;
                    }
                    let path = dir.join(file);
                    match std::fs::write(&path, bytes) {
                        Ok(()) => written.push(path),
                        Err(e) => {
                            app.show_toast(format!("Export failed: {e}"), true);
                            return;
                        }
                    }
                }
                match written.as_slice() {
                    [one] => app.show_toast(format!("Exported {}", one.display()), false),
                    many => app.show_toast(format!("Exported {} files", many.len()), false),
                }
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use desigma_doc::{Node, Rect as R};

    fn frame(ctx: &egui::Context, t: f64, events: Vec<egui::Event>, f: impl FnMut(&mut Ui)) {
        let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(400.0, 800.0))), time: Some(t), events, ..Default::default() };
        let _ = ctx.run_ui(raw, f);
    }

    #[test]
    fn plus_adds_a_row_and_shows_preview_and_button() {
        let mut d = Document::new("t");
        let page = d.add_page("p");
        let id = d.alloc_id();
        d.insert(page, Node::rectangle(id, R::new(0.0, 0.0, 40.0, 30.0))).unwrap();
        let mut app = App::new(d);
        app.selection = vec![id];
        let sel = vec![id];
        let ctx = egui::Context::default();
        theme::install_fonts(&ctx);
        let mut h0 = 0.0;
        frame(&ctx, 0.0, vec![], |ui| h0 = section(&mut app, ui, 0.0, 0.0, &sel));
        assert!(h0 < 60.0, "only the title without rows: {h0}");
        // click the "+" at the right end of the title row
        let p = Pos2::new(theme::SECTION_TRAILING_X + 12.0, 20.0);
        let ev = |pressed| egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        frame(&ctx, 0.1, vec![egui::Event::PointerMoved(p)], |ui| {
            section(&mut app, ui, 0.0, 0.0, &sel);
        });
        frame(&ctx, 0.2, vec![ev(true)], |ui| {
            section(&mut app, ui, 0.0, 0.0, &sel);
        });
        frame(&ctx, 0.3, vec![ev(false)], |ui| {
            section(&mut app, ui, 0.0, 0.0, &sel);
        });
        assert_eq!(app.doc.get(id).unwrap().export_settings, vec![ExportPreset::default()]);
        assert!(app.hist.can_undo(), "adding a row is an undo step");
        let mut h1 = 0.0;
        frame(&ctx, 0.4, vec![], |ui| h1 = section(&mut app, ui, 0.0, 0.0, &sel));
        // title + one row + preview + button
        assert!(h1 > h0 + PREVIEW_H, "{h1} vs {h0}");
        // the preview texture was created
        assert!(ctx.data(|d| d.get_temp::<Thumb>(Id::new("export-thumb"))).is_some());
    }

    fn two_rects() -> (App, NodeId, NodeId) {
        let mut d = Document::new("t");
        let page = d.add_page("p");
        let a = d.alloc_id();
        d.insert(page, Node::rectangle(a, R::new(0.0, 0.0, 40.0, 30.0))).unwrap();
        let b = d.alloc_id();
        d.insert(page, Node::rectangle(b, R::new(50.0, 0.0, 40.0, 30.0))).unwrap();
        (App::new(d), a, b)
    }

    #[test]
    fn format_labels_fit_their_dropdown() {
        let ctx = egui::Context::default();
        theme::install_fonts(&ctx);
        frame(&ctx, 0.0, vec![], |ui| {
            for l in ["PNG", "JPG", "SVG", "PDF"] {
                let w = widgets::text_width(ui.painter(), l, &theme::font_menu());
                assert!(w <= FORMAT_W - 28.0, "{l} is {w}px wide");
            }
            for l in ["512w", "1.5x", "2x"] {
                let w = widgets::text_width(ui.painter(), l, &theme::font_body());
                assert!(w <= SCALE_W - 20.0 - 8.0, "{l} is {w}px wide");
            }
        });
    }

    #[test]
    fn scale_strings_round_trip_through_constraints() {
        for s in ["2x", "0.75x", "512w", "64h"] {
            let sc = ExportScale::parse(s).unwrap();
            assert_eq!(scale_of(constraint_of(sc)), sc, "{s}");
        }
        assert_eq!(constraint_of(ExportScale::parse("512w").unwrap()), ExportConstraint::Width(512.0));
        assert_eq!(constraint_of(ExportScale::parse("3x").unwrap()), ExportConstraint::Scale(3.0));
        let p = ExportPreset { format: ExportFileFormat::Jpg, constraint: ExportConstraint::Height(64.0), suffix: String::new() };
        assert_eq!(preset_file_name(&p, "Logo"), "Logo@64h.jpg");
        let p = ExportPreset { suffix: "-dark".into(), ..p };
        assert_eq!(preset_file_name(&p, "Logo"), "Logo-dark.jpg");
        assert!(setting_of(&ExportPreset { format: ExportFileFormat::Pdf, ..Default::default() }).is_none());
    }

    #[test]
    fn rows_add_edit_remove_are_undoable_and_survive_save_and_load() {
        let (mut app, a, _) = two_rects();
        let one = ExportPreset::default();
        app.edit_export_rows(&[a], &RowEdit::Add(one.clone()));
        let two = ExportPreset { format: ExportFileFormat::Svg, constraint: ExportConstraint::Width(512.0), suffix: "@512".into() };
        app.edit_export_rows(&[a], &RowEdit::Add(two.clone()));
        let edited = ExportPreset { constraint: ExportConstraint::Scale(2.0), ..one.clone() };
        app.edit_export_rows(&[a], &RowEdit::Replace(one.clone(), edited.clone()));
        assert_eq!(app.doc.get(a).unwrap().export_settings, vec![edited.clone(), two.clone()]);

        let mut path = std::env::temp_dir();
        path.push(format!("desigma_export_rows_{}", std::process::id()));
        let p = crate::io::save_path(&app.doc, &path).unwrap();
        let back = crate::io::load_path(&p).unwrap();
        std::fs::remove_file(p).ok();
        assert_eq!(back.get(a).unwrap().export_settings, vec![edited.clone(), two.clone()]);

        app.edit_export_rows(&[a], &RowEdit::Remove(edited));
        assert_eq!(app.doc.get(a).unwrap().export_settings, vec![two]);
        app.undo();
        assert_eq!(app.doc.get(a).unwrap().export_settings.len(), 2);
        app.undo();
        assert_eq!(app.doc.get(a).unwrap().export_settings[0], one, "the edit undoes in one step");
    }

    #[test]
    fn multi_selection_shows_and_edits_shared_rows_only() {
        let (mut app, a, b) = two_rects();
        let shared = ExportPreset::default();
        let only_a = ExportPreset { format: ExportFileFormat::Jpg, ..Default::default() };
        app.edit_export_rows(&[a, b], &RowEdit::Add(shared.clone()));
        app.edit_export_rows(&[a], &RowEdit::Add(only_a.clone()));
        assert_eq!(shared_rows(&app.doc, &[a, b]), vec![shared.clone()]);
        assert_eq!(shared_rows(&app.doc, &[a]), vec![shared.clone(), only_a.clone()]);

        let new = ExportPreset { constraint: ExportConstraint::Scale(4.0), ..shared.clone() };
        app.edit_export_rows(&[a, b], &RowEdit::Replace(shared.clone(), new.clone()));
        assert_eq!(app.doc.get(a).unwrap().export_settings, vec![new.clone(), only_a]);
        assert_eq!(app.doc.get(b).unwrap().export_settings, vec![new.clone()]);
        app.edit_export_rows(&[a, b], &RowEdit::Remove(new));
        assert_eq!(app.doc.get(b).unwrap().export_settings, vec![]);
        assert_eq!(app.doc.get(a).unwrap().export_settings.len(), 1, "the non-shared row is kept");
    }

    #[test]
    fn fingerprint_changes_with_the_subtree() {
        let mut d = Document::new("t");
        let page = d.add_page("p");
        let id = d.alloc_id();
        d.insert(page, Node::rectangle(id, R::new(0.0, 0.0, 10.0, 10.0))).unwrap();
        let a = fingerprint(&d, id);
        assert_eq!(a, fingerprint(&d, id));
        d.get_mut(id).unwrap().opacity = 0.5;
        assert_ne!(a, fingerprint(&d, id));
    }
}
