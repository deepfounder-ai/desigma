//! `<text>` -> Text nodes.
//!
//! usvg resolves a text element into chunks (runs placed at an absolute
//! position, with `text-anchor`) and, once the bundled font is loaded, into
//! positioned glyphs. The glyph origins give each line's baseline; the chunk
//! spans give the style. Chunks that look like the lines of one paragraph
//! (same x, even spacing) merge into one multi-line Text node.

use crate::convert::{Attrs, Conv};
use crate::paint::Frame;
use desigma_doc::*;
use usvg::{FontFamily, FontStyle, TextAnchor, TextFlow, TextSpan, WritingMode};

struct Line<'a> {
    text: String,
    x: f32,
    baseline: f32,
    anchor: TextAnchor,
    anchor_x: Option<f32>,
    /// Style spans as byte ranges of `text`.
    spans: Vec<(usize, usize, &'a TextSpan)>,
}

pub(crate) fn convert_text(c: &mut Conv, t: &usvg::Text, parent: usize, a: Attrs) {
    if let Some(lines) = collect(t).filter(|lines| !lines.is_empty()) {
        emit(c, lines, parent, a);
        return;
    }
    let flat = t.flattened();
    // No glyphs at all: the font family is not in usvg's database (it only
    // carries the bundled Inter face), so nothing was laid out and the outlines
    // are empty. Before this branch such text vanished from the document with no
    // node and no pixels — a CJK-only SVG imported as a blank file. usvg has
    // already resolved inheritance, CSS and every ancestor transform into the
    // chunks, so they alone are enough to rebuild an editable Text node.
    if flat.children().is_empty() {
        let lines = unshaped_lines(t);
        if !lines.is_empty() {
            emit(c, lines, parent, a);
            return;
        }
    }
    // Otherwise text on a path, rotated glyphs, vertical text: keep the look as
    // outlines.
    c.convert_group(flat, parent, a);
}

/// Lines of a `<text>` whose glyphs usvg could not place, from the chunks alone.
///
/// The mirrors of [`collect`]'s checks are gone on purpose: there is no glyph
/// list to walk, only the resolved `x`/`y`/`anchor` and the style spans.
fn unshaped_lines(t: &usvg::Text) -> Vec<Line<'_>> {
    let mut lines = Vec::new();
    for ch in t.chunks() {
        // Trailing blanks (e.g. indentation before `</text>`) are not content.
        let text = ch.text().trim_end();
        if text.trim().is_empty() {
            continue;
        }
        let spans = ch.spans().iter().filter(|s| s.is_visible() && s.start() < text.len()).map(|s| (s.start(), s.end().min(text.len()), s)).collect();
        lines.push(Line { text: text.to_string(), x: ch.x().unwrap_or(0.0), baseline: ch.y().unwrap_or(0.0), anchor: ch.anchor(), anchor_x: ch.x(), spans });
    }
    lines
}

fn collect(t: &usvg::Text) -> Option<Vec<Line<'_>>> {
    if t.writing_mode() != WritingMode::LeftToRight || t.rotate().iter().any(|r| r.abs() > 1e-6) {
        return None;
    }
    if t.chunks().iter().any(|ch| !matches!(ch.text_flow(), TextFlow::Linear)) {
        return None;
    }
    // (x, baseline y, text bytes) of every laid out glyph, in order.
    let glyphs: Vec<(f32, f32, usize)> = t
        .layouted()
        .iter()
        .flat_map(|s| s.positioned_glyphs.iter())
        .map(|g| {
            let ts = g.transform();
            (ts.tx, ts.ty, g.text.len())
        })
        .collect();
    let mut lines = Vec::new();
    let mut at = 0usize;
    for ch in t.chunks() {
        let want = ch.text().len();
        if want == 0 {
            continue;
        }
        let first = *glyphs.get(at)?;
        let mut got = 0usize;
        while got < want {
            let g = glyphs.get(at)?;
            got += g.2;
            at += 1;
        }
        if got != want {
            return None;
        }
        // Indentation between tspans (xml:space="preserve") is not text.
        if ch.text().trim().is_empty() {
            continue;
        }
        // Trailing blanks (e.g. indentation before `</text>`) are not content.
        let text = ch.text().trim_end();
        let spans = ch.spans().iter().filter(|s| s.is_visible() && s.start() < text.len()).map(|s| (s.start(), s.end().min(text.len()), s)).collect();
        lines.push(Line { text: text.to_string(), x: first.0, baseline: first.1, anchor: ch.anchor(), anchor_x: ch.x(), spans });
    }
    (at == glyphs.len()).then_some(lines)
}

fn family_of(f: &FontFamily) -> String {
    match f {
        FontFamily::Named(n) => n.clone(),
        FontFamily::SansSerif => "Inter".into(),
        FontFamily::Serif => "Times New Roman".into(),
        FontFamily::Monospace => "Courier New".into(),
        FontFamily::Cursive | FontFamily::Fantasy => "Inter".into(),
    }
}

/// Style of a span with a placeholder fill (real fills need the node box).
fn style_of(s: &TextSpan, align: TextAlignH) -> TextStyle {
    let fills = match s.fill().and_then(|f| crate::paint::paint_color(f.paint(), f.opacity().get())) {
        Some((col, a)) => vec![Paint { kind: PaintKind::Solid(crate::paint::color(col, 1.0)), opacity: a, visible: true, blend: BlendMode::Normal }],
        None => Vec::new(),
    };
    let dec = s.decoration();
    TextStyle {
        font_family: s.font().families().first().map(family_of).unwrap_or_else(|| "Inter".into()),
        font_weight: s.font().weight(),
        italic: s.font().style() != FontStyle::Normal,
        font_size: s.font_size().get(),
        line_height: LineHeight::Auto,
        letter_spacing: s.letter_spacing(),
        paragraph_spacing: 0.0,
        align_h: align,
        align_v: TextAlignV::Top,
        case: TextCase::Original,
        decoration: if dec.underline().is_some() {
            TextDecoration::Underline
        } else if dec.line_through().is_some() {
            TextDecoration::Strikethrough
        } else {
            TextDecoration::None
        },
        fills,
        features: Vec::new(),
    }
}

/// Lines that read as one paragraph: same anchor and x, constant pitch.
fn pitch_of(lines: &[Line]) -> Option<f32> {
    let [first, second, ..] = lines else { return None };
    let pitch = second.baseline - first.baseline;
    if pitch <= 0.0 {
        return None;
    }
    let same_x = |l: &Line| match first.anchor {
        TextAnchor::Start => (l.x - first.x).abs() < 0.5,
        _ => matches!((l.anchor_x, first.anchor_x), (Some(a), Some(b)) if (a - b).abs() < 0.5),
    };
    let ok = lines.windows(2).all(|w| ((w[1].baseline - w[0].baseline) - pitch).abs() < 0.5) && lines.iter().all(|l| l.anchor == first.anchor && same_x(l));
    ok.then_some(pitch)
}

fn emit(c: &mut Conv, lines: Vec<Line>, parent: usize, a: Attrs) {
    let groups: Vec<(Vec<Line>, Option<f32>)> = match pitch_of(&lines) {
        Some(p) => vec![(lines, Some(p))],
        None => lines.into_iter().map(|l| (vec![l], None)).collect(),
    };
    let many = groups.len() > 1;
    let (parent, a) = if many {
        let container = c.group_from(&a, parent);
        (container, Attrs::with_pre(Affine::IDENTITY))
    } else {
        (parent, a)
    };
    for (lines, pitch) in groups {
        emit_node(c, &lines, pitch, parent, &a);
    }
}

fn emit_node(c: &mut Conv, lines: &[Line], pitch: Option<f32>, parent: usize, a: &Attrs) {
    let first = &lines[0];
    let align = match first.anchor {
        TextAnchor::Start => TextAlignH::Left,
        TextAnchor::Middle if first.anchor_x.is_some() => TextAlignH::Center,
        TextAnchor::End if first.anchor_x.is_some() => TextAlignH::Right,
        _ => TextAlignH::Left,
    };

    // Content and style runs.
    let mut content = String::new();
    let mut base: Option<(TextStyle, &TextSpan)> = None;
    let mut runs: Vec<(TextRun, &TextSpan)> = Vec::new();
    for (i, l) in lines.iter().enumerate() {
        if i > 0 {
            content.push('\n');
        }
        let off = content.len();
        content.push_str(&l.text);
        for (s, e, sp) in &l.spans {
            let st = style_of(sp, align);
            match &base {
                None => base = Some((st, sp)),
                Some((b, _)) if *b == st => {}
                Some(_) => runs.push((TextRun { start: off + s, end: off + e, style: st }, sp)),
            }
        }
    }
    let Some((mut style, base_span)) = base else { return };
    if let Some(p) = pitch {
        style.line_height = LineHeight::Pixels(p);
    }
    for (r, _) in &mut runs {
        r.style.line_height = style.line_height;
    }
    let mut props = TextProps {
        content: content.clone(),
        style,
        runs: runs.iter().map(|(r, _)| r.clone()).collect(),
        auto_resize: TextAutoResize::WidthAndHeight,
        max_lines: None,
        ..Default::default()
    };
    let geom = desigma_render::text_geometry(&props, Vec2::ZERO);
    let (w, h) = (geom.width, geom.height);
    let first_baseline = geom.lines.first().map(|l| l.baseline).unwrap_or(h * 0.8);
    let x = match (align, first.anchor_x) {
        (TextAlignH::Center, Some(ax)) => ax - w / 2.0,
        (TextAlignH::Right, Some(ax)) => ax - w,
        _ => first.x,
    };
    let y = first.baseline - first_baseline;

    // Real fills, now that the box is known.
    let fr = Frame::new(x, y, w, h);
    props.style.fills = c.text_paints(base_span.fill(), &fr);
    for (i, (_, sp)) in runs.iter().enumerate() {
        props.runs[i].style.fills = c.text_paints(sp.fill(), &fr);
    }

    let mut node = c.new_node(NodeKind::Text(props));
    c.apply_attrs(&mut node, a);
    node.transform = a.pre * Affine::translate(x, y);
    node.size = Vec2::new(w, h);
    let label: String = content.lines().next().unwrap_or("").chars().take(40).collect();
    node.name = label.trim().to_string();
    c.push(node, parent);
}
