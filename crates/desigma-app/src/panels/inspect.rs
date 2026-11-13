//! Dev mode inspect panel (right side) and the canvas measurement overlay.

use super::{clipped, scrollbar, wheel_scroll};
use crate::app::App;
use crate::dev::color::ColorFormat;
use crate::theme;
use crate::util::world_corners;
use crate::widgets::{self, elide, fmt_num, text_width};
use desigma_doc::*;
use egui::{Align2, Color32, Id, Pos2, Rect, Sense, Stroke, Ui, Vec2};

pub(crate) const PAD: f32 = 16.0;
pub(crate) const ROW_H: f32 = 24.0;
const MEASURE: Color32 = theme::MEASURE;

fn c32(c: &Color, opacity: f32) -> Color32 {
    let f = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    Color32::from_rgba_unmultiplied(f(c.r), f(c.g), f(c.b), f(c.a * opacity))
}

// ------------------------------------------------------------------ data

/// One colour used by a layer.
#[derive(Clone, Debug, PartialEq)]
pub struct ColorEntry {
    pub color: Color,
    /// Paint opacity times colour alpha, 0..1.
    pub opacity: f32,
    /// "Fill", "Stroke", "Text", "Gradient stop".
    pub role: &'static str,
    pub style: Option<String>,
}

impl ColorEntry {
    pub fn hex(&self) -> String {
        ColorFormat::Hex.value(self.color, self.opacity)
    }
}

fn push_paints(out: &mut Vec<ColorEntry>, paints: &[Paint], role: &'static str, style: Option<&String>) {
    for p in paints.iter().filter(|p| p.visible) {
        let mut add =
            |c: &Color, role: &'static str| out.push(ColorEntry { color: *c, opacity: (c.a * p.opacity).clamp(0.0, 1.0), role, style: style.cloned() });
        match &p.kind {
            PaintKind::Solid(c) => add(c, role),
            PaintKind::LinearGradient { stops, .. }
            | PaintKind::RadialGradient { stops, .. }
            | PaintKind::AngularGradient { stops, .. }
            | PaintKind::DiamondGradient { stops, .. } => {
                for s in stops {
                    add(&s.color, "Gradient stop");
                }
            }
            PaintKind::Image { .. } => {}
        }
    }
}

/// Distinct colours of the layer and its descendants (fills, strokes, text).
pub fn collect_colors(doc: &Document, root: NodeId) -> Vec<ColorEntry> {
    let mut all = Vec::new();
    doc.walk(root, |n, _| {
        let n = resolve_node(doc, n);
        let fill_style = n.style_refs.get("fill").map(|k| doc.style_name(k).to_string());
        let stroke_style = n.style_refs.get("stroke").map(|k| doc.style_name(k).to_string());
        match &n.kind {
            NodeKind::Text(t) => {
                let text_style = n.style_refs.get("text").map(|k| doc.style_name(k).to_string());
                push_paints(&mut all, &t.style.fills, "Text", text_style.as_ref());
                for r in &t.runs {
                    push_paints(&mut all, &r.style.fills, "Text", None);
                }
            }
            _ => push_paints(&mut all, &n.fills, "Fill", fill_style.as_ref()),
        }
        if n.stroke.weight > 0.0 || n.stroke.side_weights.is_some() {
            push_paints(&mut all, &n.stroke.paints, "Stroke", stroke_style.as_ref());
        }
    });
    let mut out: Vec<ColorEntry> = Vec::new();
    for e in all {
        let dup = out.iter().any(|o| o.color.to_hex() == e.color.to_hex() && (o.opacity - e.opacity).abs() < 0.000001);
        if !dup {
            out.push(e);
        }
        if out.len() >= 24 {
            break;
        }
    }
    out
}

pub fn weight_name(w: u16) -> &'static str {
    match w {
        0..=149 => "Thin",
        150..=249 => "ExtraLight",
        250..=349 => "Light",
        350..=449 => "Regular",
        450..=549 => "Medium",
        550..=649 => "SemiBold",
        650..=749 => "Bold",
        750..=849 => "ExtraBold",
        _ => "Black",
    }
}

/// Label/value pairs describing a text style.
pub fn typography_rows(t: &TextProps) -> Vec<(&'static str, String)> {
    let s = &t.style;
    let mut rows = vec![
        ("Font", s.font_family.clone()),
        ("Style", format!("{}{}", weight_name(s.font_weight), if s.italic { " Italic" } else { "" })),
        ("Size", format!("{}px", fmt_num(s.font_size))),
        (
            "Line height",
            match s.line_height {
                LineHeight::Auto => "Auto".to_string(),
                LineHeight::Pixels(p) => format!("{}px", fmt_num(p)),
                LineHeight::Percent(p) => format!("{}%", fmt_num(p)),
            },
        ),
        ("Letter spacing", format!("{}px", fmt_num(s.letter_spacing))),
        (
            "Align",
            match s.align_h {
                TextAlignH::Left => "Left",
                TextAlignH::Center => "Center",
                TextAlignH::Right => "Right",
                TextAlignH::Justified => "Justified",
            }
            .to_string(),
        ),
    ];
    if s.paragraph_spacing != 0.0 {
        rows.push(("Paragraph spacing", format!("{}px", fmt_num(s.paragraph_spacing))));
    }
    if s.decoration != TextDecoration::None {
        rows.push(("Decoration", format!("{:?}", s.decoration)));
    }
    if s.case != TextCase::Original {
        rows.push(("Case", format!("{:?}", s.case)));
    }
    if !t.runs.is_empty() {
        rows.push(("Style runs", t.runs.len().to_string()));
    }
    rows
}

/// A downloadable asset of a layer.
#[derive(Clone, Debug, PartialEq)]
pub enum Asset {
    /// Raw embedded image, by `Document::images` key.
    Image(String),
    Svg,
    Png2x,
}

/// Assets offered for download: embedded images of the subtree, then the layer
/// itself as SVG and PNG.
pub fn collect_assets(doc: &Document, root: NodeId) -> Vec<Asset> {
    let mut refs: Vec<String> = Vec::new();
    doc.walk(root, |n, _| {
        let mut visit = |paints: &[Paint]| {
            for p in paints {
                if let PaintKind::Image { image_ref, .. } = &p.kind {
                    if doc.images.contains_key(image_ref) && !refs.contains(image_ref) {
                        refs.push(image_ref.clone());
                    }
                }
            }
        };
        visit(&n.fills);
        visit(&n.stroke.paints);
    });
    let mut out: Vec<Asset> = refs.into_iter().map(Asset::Image).collect();
    out.push(Asset::Svg);
    out.push(Asset::Png2x);
    out
}

// ------------------------------------------------------------------ drawing

pub(crate) struct Cur {
    pub x0: f32,
    pub oy: f32,
    pub y: f32,
    pub w: f32,
}

impl Cur {
    pub fn rect(&self, x: f32, dy: f32, w: f32, h: f32) -> Rect {
        Rect::from_min_size(Pos2::new(self.x0 + x, self.oy + self.y + dy), Vec2::new(w, h))
    }
}

pub(crate) fn copy(app: &mut App, ui: &Ui, text: String, what: &str) {
    ui.ctx().copy_text(text);
    app.show_toast(format!("Copied {what}"), false);
}

pub(crate) fn small_button(ui: &mut Ui, rect: Rect, key: impl widgets::Key, label: &str) -> bool {
    widgets::record(ui, &format!("btn-{key:?}"), rect);
    let r = ui.interact(rect, ui.id().with(("insp-btn", key)), Sense::click());
    ui.painter().rect_filled(rect, theme::cr(theme::RADIUS_MEDIUM), if r.hovered() { theme::BG_TERTIARY } else { theme::BG_SECONDARY });
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, label, theme::font_body(), theme::TEXT);
    r.clicked()
}

pub(crate) fn section_title(ui: &Ui, c: &mut Cur, label: &str) {
    widgets::hline(ui.painter(), c.x0, c.x0 + c.w, c.oy + c.y, theme::BORDER);
    c.y += 1.0;
    let r = c.rect(PAD, 0.0, c.w - 2.0 * PAD, theme::SECTION_TITLE_HEIGHT);
    ui.painter().text(Pos2::new(r.left(), r.center().y), Align2::LEFT_CENTER, crate::i18n::tr(label), theme::font_body_strong(), theme::TEXT);
    c.y += theme::SECTION_TITLE_HEIGHT - 8.0;
}

pub(crate) fn kv(ui: &mut Ui, app: &mut App, c: &mut Cur, label: &str, value: &str) {
    let r = c.rect(PAD, 0.0, c.w - 2.0 * PAD, ROW_H);
    let resp = ui.interact(r, ui.id().with(("insp-kv", label)), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(r.expand2(Vec2::new(8.0, 0.0)), theme::cr(theme::RADIUS_MEDIUM), theme::BG_SECONDARY);
    }
    ui.painter().text(Pos2::new(r.left(), r.center().y), Align2::LEFT_CENTER, label, theme::font_body(), theme::TEXT_SECONDARY);
    let val = elide(ui.painter(), value, &theme::font_body(), r.width() - 100.0);
    ui.painter().text(Pos2::new(r.left() + 100.0, r.center().y), Align2::LEFT_CENTER, val, theme::font_body(), theme::TEXT);
    if resp.clicked() {
        copy(app, ui, value.to_string(), label);
    }
    c.y += ROW_H;
}

fn ext_for(bytes: &[u8]) -> &'static str {
    match image::guess_format(bytes) {
        Ok(image::ImageFormat::Jpeg) => "jpg",
        Ok(image::ImageFormat::WebP) => "webp",
        Ok(image::ImageFormat::Gif) => "gif",
        _ => "png",
    }
}

pub(crate) fn download(app: &mut App, id: NodeId, asset: &Asset) {
    let name = app.doc.get(id).map(|n| desigma_export::sanitize_file_name(&n.name)).unwrap_or_else(|| "asset".into());
    let (file, bytes, label, ext): (String, Vec<u8>, &str, &str) = match asset {
        Asset::Image(r) => {
            let Some(data) = app.doc.images.get(r) else { return };
            let ext = ext_for(data);
            (format!("{name}.{ext}"), data.clone(), "Image", ext)
        }
        Asset::Svg => {
            (format!("{name}.svg"), desigma_export::export_svg(&app.export_doc(), id, &desigma_export::SvgOptions::default()).into_bytes(), "SVG", "svg")
        }
        Asset::Png2x => (format!("{name}@2x.png"), desigma_export::export_png(&app.export_doc(), id, 2.0), "PNG", "png"),
    };
    if bytes.is_empty() {
        app.show_toast("Nothing to download", true);
        return;
    }
    let Some(path) = rfd::FileDialog::new().add_filter(label, &[ext]).set_file_name(file).save_file() else { return };
    match std::fs::write(&path, bytes) {
        Ok(()) => app.show_toast(format!("Saved {}", path.display()), false),
        Err(e) => app.show_toast(format!("Download failed: {e}"), true),
    }
}

fn kind_label(k: &NodeKind) -> &'static str {
    k.default_name()
}

/// Right-panel content in Dev mode.
pub fn show(app: &mut App, ui: &mut Ui, region: Rect) {
    let state_id = Id::new("inspect-scroll");
    let (mut scroll, content_h) = ui.ctx().data(|d| d.get_temp::<(f32, f32)>(state_id)).unwrap_or((0.0, 0.0));
    wheel_scroll(ui, region, &mut scroll, content_h);
    let sel = app.selection.clone();
    let end = clipped(ui, region, |ui| {
        let mut c = Cur { x0: region.left() - theme::BORDER_WIDTH, oy: region.top() - scroll, y: 0.0, w: region.width() + theme::BORDER_WIDTH };
        let Some(&id) = sel.first() else {
            ui.painter().text(
                Pos2::new(region.center().x, region.top() + 56.0),
                Align2::CENTER_CENTER,
                "Select a layer to inspect",
                theme::font_body(),
                theme::TEXT_TERTIARY,
            );
            return 80.0;
        };
        body(app, ui, &mut c, id, sel.len());
        c.y + 24.0
    });
    ui.ctx().data_mut(|d| d.insert_temp(state_id, (scroll, end)));
    scrollbar(ui, region, scroll, end);
}

fn body(app: &mut App, ui: &mut Ui, c: &mut Cur, id: NodeId, count: usize) {
    let Some(n) = app.doc.get(id).cloned() else { return };

    // Header.
    c.y += 12.0;
    let hdr = c.rect(PAD, 0.0, c.w - 2.0 * PAD, 24.0);
    let name = elide(ui.painter(), &n.name, &theme::font_large_strong(), hdr.width());
    ui.painter().text(Pos2::new(hdr.left(), hdr.center().y), Align2::LEFT_CENTER, name, theme::font_large_strong(), theme::TEXT);
    c.y += 24.0;
    let sub = if count > 1 { format!("{} · {} layers selected, showing the first", kind_label(&n.kind), count) } else { kind_label(&n.kind).to_string() };
    let sub_r = c.rect(PAD, 0.0, c.w - 2.0 * PAD, 18.0);
    let sub = elide(ui.painter(), &sub, &theme::font_body(), sub_r.width());
    ui.painter().text(Pos2::new(sub_r.left(), sub_r.center().y), Align2::LEFT_CENTER, sub, theme::font_body(), theme::TEXT_SECONDARY);
    c.y += 26.0;

    crate::dev::sections::status_header(app, ui, c, id);
    crate::dev::sections::component_section(app, ui, c, id);
    crate::dev::sections::annotations_section(app, ui, c, id);
    crate::dev::sections::links_section(app, ui, c, id);

    // Layout.
    section_title(ui, c, "Layout");
    kv(ui, app, c, "Width", &format!("{}px", fmt_num(n.size.x)));
    kv(ui, app, c, "Height", &format!("{}px", fmt_num(n.size.y)));
    kv(ui, app, c, "X", &format!("{}px", fmt_num(n.transform.tx)));
    kv(ui, app, c, "Y", &format!("{}px", fmt_num(n.transform.ty)));
    let rot = n.transform.rotation_rad().to_degrees();
    if rot.abs() > 0.01 {
        kv(ui, app, c, "Rotation", &format!("{}°", fmt_num(rot)));
    }
    c.y += 4.0;
    crate::dev::sections::box_model_section(ui, c, &app.doc, id);

    // Code.
    crate::dev::sections::code_section(app, ui, c, id);
    crate::dev::sections::variables_section(app, ui, c, id);

    // Colors.
    let colors = collect_colors(&app.doc, id);
    if !colors.is_empty() {
        section_title(ui, c, "Colors");
        let dd = c.rect(c.w - PAD - 80.0, -24.0, 80.0, 24.0);
        widgets::record(ui, "dev-color-format", dd);
        let options: Vec<String> = ColorFormat::ALL.iter().map(|f| f.label().to_string()).collect();
        if let Some(i) = widgets::menu::dropdown(ui, dd, "dev-color-format", app.dev.color_format.label(), &options) {
            app.dev.color_format = ColorFormat::ALL[i];
        }
        for (i, e) in colors.iter().enumerate() {
            let r = c.rect(PAD, 0.0, c.w - 2.0 * PAD, 44.0);
            widgets::record(ui, &format!("dev-copy-color-{i}"), r);
            let resp = ui.interact(r, ui.id().with(("insp-color", i)), Sense::click());
            if resp.hovered() {
                ui.painter().rect_filled(r.expand2(Vec2::new(8.0, 0.0)), theme::cr(theme::RADIUS_MEDIUM), theme::BG_SECONDARY);
            }
            let sw = Rect::from_min_size(Pos2::new(r.left(), r.center().y - 8.0), Vec2::splat(16.0));
            widgets::swatch(ui.painter(), sw, c32(&e.color.with_alpha(1.0), e.opacity));
            let value = if app.dev.color_format == ColorFormat::Hex { e.hex() } else { app.dev.color_format.value(e.color, e.opacity) };
            let label = elide(ui.painter(), &value, &theme::font_body(), r.width() - 24.0);
            ui.painter().text(Pos2::new(sw.right() + 8.0, r.top() + 12.0), Align2::LEFT_CENTER, label, theme::font_body(), theme::TEXT);
            let detail = match &e.style {
                Some(s) => format!("{} · {}", e.role, s),
                None => e.role.to_string(),
            };
            let detail = elide(ui.painter(), &detail, &theme::font_small(), r.width() - 64.0);
            ui.painter().text(Pos2::new(sw.right() + 8.0, r.top() + 31.0), Align2::LEFT_CENTER, detail, theme::font_small(), theme::TEXT_SECONDARY);
            if resp.hovered() {
                ui.painter().text(Pos2::new(r.right(), r.top() + 31.0), Align2::RIGHT_CENTER, "Copy", theme::font_small(), theme::TEXT_SECONDARY);
            }
            if resp.clicked() {
                copy(app, ui, value.clone(), "color");
            }
            resp.on_hover_text(format!("Click to copy {value}"));
            c.y += 44.0;
        }
        c.y += 8.0;
    }

    // Typography.
    if let NodeKind::Text(t) = &n.kind {
        section_title(ui, c, "Typography");
        for (label, value) in typography_rows(t) {
            kv(ui, app, c, label, &value);
        }
        c.y += 8.0;
    }

    crate::dev::sections::assets_section(app, ui, c, id);
}

// ------------------------------------------------------------------ measurements

pub(crate) fn page_aabb(doc: &Document, id: NodeId) -> Option<Rect> {
    doc.get(id)?;
    let pts = world_corners(doc, id);
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for p in pts {
        x0 = x0.min(p.x);
        y0 = y0.min(p.y);
        x1 = x1.max(p.x);
        y1 = y1.max(p.y);
    }
    Some(Rect::from_min_max(Pos2::new(x0, y0), Pos2::new(x1, y1)))
}

/// One measured gap: a segment in page space and its length.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Gap {
    pub from: (f32, f32),
    pub to: (f32, f32),
    pub value: f32,
}

fn axis_gaps(a: (f32, f32), b: (f32, f32)) -> Vec<(f32, f32)> {
    // Disjoint ranges: the single space between them. Overlapping: the two edge offsets.
    if b.0 >= a.1 {
        vec![(a.1, b.0)]
    } else if b.1 <= a.0 {
        vec![(b.1, a.0)]
    } else {
        let mut v = Vec::new();
        if (a.0 - b.0).abs() > 0.5 {
            v.push((a.0.min(b.0), a.0.max(b.0)));
        }
        if (a.1 - b.1).abs() > 0.5 {
            v.push((a.1.min(b.1), a.1.max(b.1)));
        }
        v
    }
}

fn overlap_mid(a: (f32, f32), b: (f32, f32)) -> Option<f32> {
    let (lo, hi) = (a.0.max(b.0), a.1.min(b.1));
    (hi > lo).then_some((lo + hi) / 2.0)
}

/// Distances from `a` (selection) to `b` (hovered layer), Figma style: the
/// space between facing edges, or the edge offsets when one contains / overlaps
/// the other.
pub fn measure(a: Rect, b: Rect) -> Vec<Gap> {
    let (ax, ay) = ((a.left(), a.right()), (a.top(), a.bottom()));
    let (bx, by) = ((b.left(), b.right()), (b.top(), b.bottom()));
    let mut out = Vec::new();
    // y position of horizontal gap lines: middle of the vertical overlap, else B's nearest edge
    let y_at = overlap_mid(ay, by).unwrap_or(if by.0 >= ay.1 { by.0 } else { by.1 });
    let x_at = overlap_mid(ax, bx).unwrap_or(if bx.0 >= ax.1 { bx.0 } else { bx.1 });
    for (x0, x1) in axis_gaps(ax, bx) {
        out.push(Gap { from: (x0, y_at), to: (x1, y_at), value: x1 - x0 });
    }
    for (y0, y1) in axis_gaps(ay, by) {
        out.push(Gap { from: (x_at, y0), to: (x_at, y1), value: y1 - y0 });
    }
    out
}

/// Red distance lines between the selection and the hovered layer: always in Dev mode,
/// and while Alt (Option) is held when designing. Call after the regular canvas overlays.
pub fn measure_overlay(app: &App, ui: &Ui, rect: Rect) {
    let alt = ui.input(|i| i.modifiers.alt) && matches!(app.canvas.inter, crate::canvas::Interaction::None);
    if !app.ui.dev_mode && !alt {
        return;
    }
    let Some(hover) = app.hover.or(app.hover_layer) else { return };
    if app.selection.is_empty() || app.selection.contains(&hover) {
        return;
    }
    let mut sel_box: Option<Rect> = None;
    for id in &app.selection {
        if let Some(r) = page_aabb(&app.doc, *id) {
            sel_box = Some(sel_box.map_or(r, |s| s.union(r)));
        }
    }
    let (Some(a), Some(b)) = (sel_box, page_aabb(&app.doc, hover)) else { return };
    let p = ui.painter().with_clip_rect(rect);
    let to_s = |x: f32, y: f32| app.view.to_screen(rect.min, desigma_doc::Vec2::new(x, y));
    // hovered layer outline
    p.rect_stroke(Rect::from_two_pos(to_s(b.left(), b.top()), to_s(b.right(), b.bottom())), 0.0, Stroke::new(1.0, MEASURE), egui::StrokeKind::Outside);
    let stroke = Stroke::new(1.0, MEASURE);
    for g in measure(a, b) {
        let (s0, s1) = (to_s(g.from.0, g.from.1), to_s(g.to.0, g.to.1));
        p.line_segment([s0, s1], stroke);
        let horizontal = (s0.y - s1.y).abs() < 0.5;
        for e in [s0, s1] {
            let (d0, d1) = if horizontal { (Vec2::new(0.0, -4.0), Vec2::new(0.0, 4.0)) } else { (Vec2::new(-4.0, 0.0), Vec2::new(4.0, 0.0)) };
            p.line_segment([e + d0, e + d1], stroke);
        }
        let mid = Pos2::new((s0.x + s1.x) / 2.0, (s0.y + s1.y) / 2.0);
        let label = fmt_num((g.value * 10.0).round() / 10.0);
        let font = theme::font_small_medium();
        let w = text_width(&p, &label, &font) + 10.0;
        let pill = Rect::from_center_size(mid + if horizontal { Vec2::new(0.0, -14.0) } else { Vec2::new(18.0 + w / 2.0, 0.0) }, Vec2::new(w, 16.0));
        p.rect_filled(pill, theme::cr(theme::RADIUS_SMALL), MEASURE);
        p.text(pill.center(), Align2::CENTER_CENTER, label, font, Color32::WHITE);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::from_min_size(Pos2::new(x, y), Vec2::new(w, h))
    }

    #[test]
    fn disjoint_boxes_measure_the_space_between() {
        let g = measure(r(0.0, 0.0, 100.0, 50.0), r(120.0, 10.0, 30.0, 20.0));
        assert_eq!(g.len(), 3, "one horizontal gap, two vertical offsets");
        let h = g.iter().find(|g| (g.from.1 - g.to.1).abs() < 1e-3).unwrap();
        assert_eq!((h.from.0, h.to.0, h.value), (100.0, 120.0, 20.0));
        // vertical ranges overlap on 10..30 so the line sits in the middle at y = 20
        assert_eq!(h.from.1, 20.0);
        // vertical axis: B's 10..30 is inside A's 0..50, offsets are 10 (top) and 20 (bottom)
        let v: Vec<f32> = g.iter().filter(|g| (g.from.0 - g.to.0).abs() < 1e-3).map(|g| g.value).collect();
        assert_eq!(v, vec![10.0, 20.0]);
    }

    #[test]
    fn contained_box_gets_four_offsets() {
        let g = measure(r(0.0, 0.0, 100.0, 100.0), r(10.0, 20.0, 60.0, 50.0));
        let mut vals: Vec<f32> = g.iter().map(|g| g.value).collect();
        vals.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert_eq!(vals, vec![10.0, 20.0, 30.0, 30.0]);
    }

    #[test]
    fn colors_are_deduped_and_include_text_and_strokes() {
        let mut d = Document::new("t");
        let page = d.add_page("p");
        let f = d.alloc_id();
        let mut fr = Node::frame(f, desigma_doc::Rect::new(0.0, 0.0, 100.0, 100.0));
        fr.stroke.paints = vec![Paint::solid(Color::rgb(1, 2, 3))];
        fr.stroke.weight = 1.0;
        let f = d.insert(page, fr).unwrap();
        let t = d.alloc_id();
        d.insert(f, Node::text(t, desigma_doc::Rect::new(0.0, 0.0, 10.0, 10.0), "x", TextStyle::default())).unwrap();
        let t2 = d.alloc_id();
        d.insert(f, Node::text(t2, desigma_doc::Rect::new(0.0, 0.0, 10.0, 10.0), "y", TextStyle::default())).unwrap();
        let cols = collect_colors(&d, f);
        let roles: Vec<(&str, String)> = cols.iter().map(|c| (c.role, c.hex())).collect();
        assert_eq!(roles, vec![("Fill", "#FFFFFF".into()), ("Stroke", "#010203".into()), ("Text", "#000000".into())]);
    }

    #[test]
    fn collected_colors_keep_distinct_alpha_and_multiply_paint_opacity_once() {
        let mut d = Document::new("t");
        let page = d.add_page("p");
        let id = d.alloc_id();
        let mut n = Node::rectangle(id, desigma_doc::Rect::new(0.0, 0.0, 10.0, 10.0));
        let mut paint = Paint::solid(Color::rgb(255, 0, 0).with_alpha(0.5));
        paint.opacity = 0.5;
        let mut other = paint.clone();
        other.opacity = 0.502;
        n.fills = vec![paint.clone(), paint, other];
        d.insert(page, n).unwrap();
        let colors = collect_colors(&d, id);
        assert_eq!(colors.len(), 2, "deduplicate repeated paints, but preserve distinct opacity values");
        assert_eq!(colors[0].opacity, 0.25);
        assert_eq!(colors[0].hex(), "#FF000040");
        assert_eq!(ColorFormat::Rgb.value(colors[0].color, colors[0].opacity), "rgb(255 0 0 / 0.25)");
    }

    #[test]
    fn collected_colors_resolve_inherited_modes_without_changing_stored_paints() {
        let mut d = Document::new("t");
        let page = d.add_page("p");
        let frame = d.alloc_id();
        d.insert(page, Node::frame(frame, desigma_doc::Rect::new(0.0, 0.0, 100.0, 100.0))).unwrap();
        let id = d.alloc_id();
        let mut n = Node::rectangle(id, desigma_doc::Rect::new(0.0, 0.0, 10.0, 10.0));
        n.fills = vec![Paint::solid(Color::rgb(255, 0, 0))];
        n.variables.bindings.insert("fills/0".into(), "background".into());
        d.insert(frame, n).unwrap();
        d.variable_collections.insert(
            "theme".into(),
            VariableCollection {
                id: "theme".into(),
                name: "Theme".into(),
                modes: vec![VariableMode { id: "light".into(), name: "Light".into() }, VariableMode { id: "dark".into(), name: "Dark".into() }],
                default_mode: "light".into(),
                variables: vec!["background".into()],
                key: None,
            },
        );
        d.variables.insert(
            "background".into(),
            Variable {
                id: "background".into(),
                name: "Background".into(),
                collection: "theme".into(),
                kind: VariableKind::Color,
                values: [("light".into(), VariableValue::Color(Color::WHITE)), ("dark".into(), VariableValue::Color(Color::BLACK))].into(),
                description: String::new(),
                scopes: vec![],
                hidden_from_publishing: false,
                key: None,
            },
        );
        assert_eq!(collect_colors(&d, id)[0].hex(), "#FFFFFF");
        d.get_mut(frame).unwrap().variables.modes.insert("theme".into(), "dark".into());
        let stored = d.clone();
        assert_eq!(collect_colors(&d, id)[0].hex(), "#000000");
        assert_eq!(d.get(id), stored.get(id));
        assert_eq!(d.get(frame), stored.get(frame));
        assert_eq!(d.get(id).unwrap().fills, vec![Paint::solid(Color::rgb(255, 0, 0))]);
    }

    #[test]
    fn inspect_panel_and_overlay_draw_for_a_text_selection() {
        let mut d = Document::new("t");
        let page = d.add_page("p");
        let f = d.alloc_id();
        let f = d.insert(page, Node::frame(f, desigma_doc::Rect::new(0.0, 0.0, 200.0, 100.0))).unwrap();
        let t = d.alloc_id();
        d.insert(f, Node::text(t, desigma_doc::Rect::new(10.0, 10.0, 80.0, 20.0), "Hello", TextStyle::default())).unwrap();
        let r = d.alloc_id();
        let r = d.insert(f, Node::rectangle(r, desigma_doc::Rect::new(120.0, 40.0, 40.0, 40.0))).unwrap();
        let mut app = App::new(d);
        app.ui.dev_mode = true;
        app.selection = vec![t];
        app.hover = Some(r);
        let ctx = egui::Context::default();
        theme::install_fonts(&ctx);
        for i in 0..3 {
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 800.0))),
                time: Some(i as f64 * 0.1),
                ..Default::default()
            };
            let _ = ctx.run_ui(raw, |ui| {
                show(&mut app, ui, Rect::from_min_size(Pos2::new(660.0, 100.0), Vec2::new(239.0, 600.0)));
                measure_overlay(&app, ui, Rect::from_min_size(Pos2::ZERO, Vec2::new(650.0, 700.0)));
            });
        }
        // text spans x 10..90, the rectangle starts at x = 120: a 30px gap
        let g = measure(page_aabb(&app.doc, t).unwrap(), page_aabb(&app.doc, r).unwrap());
        assert!(g.iter().any(|g| (g.value - 30.0).abs() < 1e-3));
    }

    #[test]
    fn typography_and_assets() {
        let t = TextProps {
            content: "a".into(),
            style: TextStyle { font_weight: 600, italic: true, font_size: 14.0, line_height: LineHeight::Percent(120.0), ..Default::default() },
            runs: vec![],
            auto_resize: TextAutoResize::None,
            max_lines: None,
            ..Default::default()
        };
        let rows = typography_rows(&t);
        assert_eq!(rows[1], ("Style", "SemiBold Italic".to_string()));
        assert_eq!(rows[3], ("Line height", "120%".to_string()));
        let mut d = Document::new("t");
        let page = d.add_page("p");
        d.images.insert("abc".into(), vec![1, 2, 3]);
        let i = d.alloc_id();
        let mut n = Node::rectangle(i, desigma_doc::Rect::new(0.0, 0.0, 5.0, 5.0));
        n.fills = vec![Paint {
            kind: PaintKind::Image { image_ref: "abc".into(), scale_mode: ImageScaleMode::Fill },
            opacity: 1.0,
            visible: true,
            blend: BlendMode::Normal,
        }];
        d.insert(page, n).unwrap();
        assert_eq!(collect_assets(&d, i), vec![Asset::Image("abc".into()), Asset::Svg, Asset::Png2x]);
    }
}
