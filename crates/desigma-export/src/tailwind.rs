//! Tailwind CSS output: markup with utility classes.

use crate::codegen::*;
use crate::css::paint_css;
use crate::util::{css_color, num, num2};
use desigma_doc::*;

/// HTML with Tailwind utility classes for `node` and its descendants. Values on the Tailwind
/// scale use tokens (`p-4`, `rounded-lg`), everything else exact arbitrary values (`w-[312px]`);
/// bound variables become CSS variable references (`bg-[var(--color-primary)]`). Instances
/// are written as `<Component prop="value" />`.
pub fn tailwind_for(doc: &Document, node: NodeId) -> String {
    let mut o = Out::new("  ");
    markup(doc, node, Parent::root(), 0, &mut o, false, false);
    o.finish()
}

/// The same markup for a standalone HTML page: component instances become
/// `div`s (an unknown tag would be dropped by the browser, taking the instance's
/// resolved children with it) and asset paths are relative to the page.
pub fn tailwind_html(doc: &Document, node: NodeId) -> String {
    let mut o = Out::new("  ");
    markup(doc, node, Parent::root(), 0, &mut o, false, true);
    o.finish()
}

/// Same markup for use inside JSX (`className`, escaped text).
pub(crate) fn jsx_markup(doc: &Document, node: NodeId, depth: usize, o: &mut Out) {
    markup(doc, node, Parent::root(), depth, o, true, false);
}

const SPACING: [(f32, &str); 35] = [
    (0.0, "0"),
    (1.0, "px"),
    (2.0, "0.5"),
    (4.0, "1"),
    (6.0, "1.5"),
    (8.0, "2"),
    (10.0, "2.5"),
    (12.0, "3"),
    (14.0, "3.5"),
    (16.0, "4"),
    (20.0, "5"),
    (24.0, "6"),
    (28.0, "7"),
    (32.0, "8"),
    (36.0, "9"),
    (40.0, "10"),
    (44.0, "11"),
    (48.0, "12"),
    (52.0, "13"),
    (56.0, "14"),
    (60.0, "15"),
    (64.0, "16"),
    (72.0, "18"),
    (80.0, "20"),
    (96.0, "24"),
    (112.0, "28"),
    (128.0, "32"),
    (144.0, "36"),
    (160.0, "40"),
    (176.0, "44"),
    (192.0, "48"),
    (208.0, "52"),
    (224.0, "56"),
    (240.0, "60"),
    (256.0, "64"),
];

fn space_token(px: f32) -> Option<&'static str> {
    SPACING.iter().find(|(v, _)| (*v - px).abs() < 1e-3).map(|(_, t)| *t)
}

/// A length token as a CSS value: number tokens are stored unitless, so they
/// need `calc(var(--x) * 1px)` to be a valid length.
fn length_var(v: &Variable) -> String {
    if v.kind == VariableKind::Number {
        format!("calc({}*1px)", css_var(v))
    } else {
        css_var(v)
    }
}

/// `prefix-token` or `prefix-[Npx]` / `prefix-[var(--x)]`.
fn spaced(prefix: &str, px: f32, var: Option<&Variable>) -> String {
    if let Some(v) = var {
        return format!("{prefix}-[{}]", length_var(v));
    }
    match space_token(px) {
        Some(t) => format!("{prefix}-{t}"),
        None => format!("{prefix}-[{}px]", num(px)),
    }
}

fn hex8(c: &Color, opacity: f32) -> String {
    let a = (c.a * opacity).clamp(0.0, 1.0);
    let base = format!("#{}", c.to_hex());
    if (a - 1.0).abs() < 0.002 {
        base
    } else {
        format!("{base}{:02X}", (a * 255.0).round() as u8)
    }
}

fn color_class(doc: &Document, n: &Node, key: &str, prefix: &str, p: &Paint, text: bool) -> String {
    if let Some(v) = bound(doc, n, key) {
        return if text { format!("{prefix}-[color:{}]", css_var(v)) } else { format!("{prefix}-[{}]", css_var(v)) };
    }
    match &p.kind {
        PaintKind::Solid(c) => format!("{prefix}-[{}]", hex8(c, p.opacity)),
        _ => format!("{prefix}-[{}]", paint_css(p, Vec2::new(1.0, 1.0)).replace(", ", ",").replace(' ', "_")),
    }
}

fn radius_class(r: f32, size: Vec2) -> String {
    if r >= size.x.min(size.y) / 2.0 && r > 0.0 && size.x.min(size.y) > 0.0 && r >= 9999.0 {
        return "rounded-full".into();
    }
    match r as i32 {
        0 => "rounded-none".into(),
        2 => "rounded-sm".into(),
        4 => "rounded".into(),
        6 => "rounded-md".into(),
        8 => "rounded-lg".into(),
        12 => "rounded-xl".into(),
        16 => "rounded-2xl".into(),
        24 => "rounded-3xl".into(),
        _ => format!("rounded-[{}px]", num(r)),
    }
}

fn corner_classes(doc: &Document, n: &Node) -> Vec<String> {
    let keys = ["topLeftRadius", "topRightRadius", "bottomRightRadius", "bottomLeftRadius"];
    let r = n.corner_radii;
    let any_bound = keys.iter().any(|k| bound(doc, n, k).is_some());
    if !any_bound {
        if r.iter().all(|v| *v == 0.0) {
            return vec![];
        }
        if r.iter().all(|v| *v == r[0]) {
            return vec![radius_class(r[0], n.size)];
        }
    }
    if any_bound && keys.iter().all(|k| bound(doc, n, k).map(|v| v.id.as_str()) == bound(doc, n, keys[0]).map(|v| v.id.as_str())) {
        return vec![format!("rounded-[{}]", length_var(bound(doc, n, keys[0]).unwrap()))];
    }
    ["tl", "tr", "br", "bl"]
        .iter()
        .enumerate()
        .filter(|(i, _)| r[*i] > 0.0 || bound(doc, n, keys[*i]).is_some())
        .map(|(i, side)| match bound(doc, n, keys[i]) {
            Some(v) => format!("rounded-{side}-[{}]", length_var(v)),
            None => format!("rounded-{side}-[{}px]", num(r[i])),
        })
        .collect()
}

fn opacity_class(o: f32) -> Option<String> {
    if o >= 0.999 {
        return None;
    }
    let pct = (o * 100.0).round();
    if [0.0, 5.0, 10.0, 20.0, 25.0, 30.0, 40.0, 50.0, 60.0, 70.0, 75.0, 80.0, 90.0, 95.0].contains(&pct) && ((o * 100.0) - pct).abs() < 0.01 {
        Some(format!("opacity-{}", pct as i32))
    } else {
        Some(format!("opacity-[{}]", num2(o.max(0.0))))
    }
}

fn shadow_value(n: &Node) -> Option<String> {
    let mut parts = Vec::new();
    for e in &n.effects {
        match e {
            Effect::DropShadow { color, offset, radius, spread, visible: true, .. } => {
                parts.push(format!(
                    "{}px_{}px_{}px_{}px_{}",
                    num(offset.x),
                    num(offset.y),
                    num(*radius),
                    num(*spread),
                    css_color(color, 1.0).replace(", ", ",")
                ));
            }
            Effect::InnerShadow { color, offset, radius, spread, visible: true } => {
                parts.push(format!(
                    "inset_{}px_{}px_{}px_{}px_{}",
                    num(offset.x),
                    num(offset.y),
                    num(*radius),
                    num(*spread),
                    css_color(color, 1.0).replace(", ", ",")
                ));
            }
            _ => {}
        }
    }
    (!parts.is_empty()).then(|| format!("shadow-[{}]", parts.join(",")))
}

fn size_class(prefix: &str, d: Dim, var: Option<&Variable>) -> Option<String> {
    match d {
        Dim::Fixed(px) => Some(spaced(prefix, px, var)),
        Dim::Hug | Dim::Fill => None,
    }
}

fn pad_classes(doc: &Document, n: &Node, al: &AutoLayout) -> Vec<String> {
    let keys = ["paddingTop", "paddingRight", "paddingBottom", "paddingLeft"];
    let var = |i: usize| bound(doc, n, keys[i]);
    let p = al.padding;
    if p.iter().all(|v| *v == 0.0) && keys.iter().all(|k| bound(doc, n, k).is_none()) {
        return vec![];
    }
    let same = |a: usize, b: usize| p[a] == p[b] && var(a).map(|v| &v.id) == var(b).map(|v| &v.id);
    if same(0, 1) && same(1, 2) && same(2, 3) {
        return vec![spaced("p", p[0], var(0))];
    }
    let mut out = Vec::new();
    if same(1, 3) {
        out.push(spaced("px", p[1], var(1)));
    } else {
        out.push(spaced("pr", p[1], var(1)));
        out.push(spaced("pl", p[3], var(3)));
    }
    if same(0, 2) {
        out.push(spaced("py", p[0], var(0)));
    } else {
        out.push(spaced("pt", p[0], var(0)));
        out.push(spaced("pb", p[2], var(2)));
    }
    // order: horizontal then vertical, zero sides dropped
    out.retain(|c| !c.ends_with("-0"));
    out
}

fn text_classes(doc: &Document, n: &Node, t: &TextProps, c: &mut Vec<String>) {
    let s = &t.style;
    c.push(format!("font-['{}']", s.font_family.replace(' ', "_")));
    let fs = match s.font_size as i32 {
        12 if s.font_size == 12.0 => Some("text-xs"),
        14 if s.font_size == 14.0 => Some("text-sm"),
        16 if s.font_size == 16.0 => Some("text-base"),
        18 if s.font_size == 18.0 => Some("text-lg"),
        20 if s.font_size == 20.0 => Some("text-xl"),
        24 if s.font_size == 24.0 => Some("text-2xl"),
        30 if s.font_size == 30.0 => Some("text-3xl"),
        36 if s.font_size == 36.0 => Some("text-4xl"),
        48 if s.font_size == 48.0 => Some("text-5xl"),
        60 if s.font_size == 60.0 => Some("text-6xl"),
        72 if s.font_size == 72.0 => Some("text-7xl"),
        _ => None,
    };
    match (bound(doc, n, "fontSize"), fs) {
        (Some(v), _) => c.push(format!("text-[length:{}]", length_var(v))),
        (None, Some(t)) => c.push(t.into()),
        (None, None) => c.push(format!("text-[{}px]", num(s.font_size))),
    }
    let w = match s.font_weight {
        100 => "font-thin",
        200 => "font-extralight",
        300 => "font-light",
        400 => "font-normal",
        500 => "font-medium",
        600 => "font-semibold",
        700 => "font-bold",
        800 => "font-extrabold",
        900 => "font-black",
        other => return c.push(format!("font-[{other}]")),
    };
    match bound(doc, n, "fontWeight") {
        Some(v) => c.push(format!("font-[{}]", css_var(v))),
        None => c.push(w.into()),
    }
    if s.italic {
        c.push("italic".into());
    }
    match (bound(doc, n, "lineHeight"), s.line_height) {
        (Some(v), _) => c.push(format!("leading-[{}]", length_var(v))),
        (None, LineHeight::Auto) => {}
        (None, LineHeight::Pixels(p)) => c.push(spaced("leading", p, None)),
        (None, LineHeight::Percent(p)) => c.push(format!("leading-[{}]", num2(p / 100.0))),
    }
    match (bound(doc, n, "letterSpacing"), s.letter_spacing) {
        (Some(v), _) => c.push(format!("tracking-[{}]", length_var(v))),
        (None, l) if l != 0.0 => c.push(format!("tracking-[{}px]", num(l))),
        _ => {}
    }
    match s.align_h {
        TextAlignH::Left => {}
        TextAlignH::Center => c.push("text-center".into()),
        TextAlignH::Right => c.push("text-right".into()),
        TextAlignH::Justified => c.push("text-justify".into()),
    }
    match s.decoration {
        TextDecoration::None => {}
        TextDecoration::Underline => c.push("underline".into()),
        TextDecoration::Strikethrough => c.push("line-through".into()),
    }
    match s.case {
        TextCase::Original => {}
        TextCase::Upper => c.push("uppercase".into()),
        TextCase::Lower => c.push("lowercase".into()),
        TextCase::Title => c.push("capitalize".into()),
        TextCase::SmallCaps => c.push("[font-variant:small-caps]".into()),
    }
    if let Some(p) = first_paint(&s.fills) {
        c.push(color_class(doc, n, "fills/0", "text", p, true));
    }
}

fn classes(doc: &Document, n: &Node, parent: Parent) -> Vec<String> {
    let mut c: Vec<String> = Vec::new();
    let sz = sizes(n, parent);
    let own_al = n.kind.frame_props().and_then(|f| f.auto_layout);
    let has_children = !visible_children(doc, n).is_empty();
    let abs_children = visible_children(doc, n).iter().any(|k| own_al.is_none() || k.layout_child.absolute);

    if let Some((x, y)) = offset(n, parent, &sz) {
        c.push("absolute".into());
        c.push(format!("left-[{}px]", num(x)));
        c.push(format!("top-[{}px]", num(y)));
    } else if has_children && abs_children && n.kind.is_container() {
        c.push("relative".into());
    }
    let rot = rotation_deg(n);
    if rot != 0.0 {
        c.push(format!("rotate-[{}deg]", num2(rot)));
    }
    if let Some(al) = own_al {
        c.push("flex".into());
        if al.direction == LayoutDirection::Vertical {
            c.push("flex-col".into());
        }
        if al.wrap {
            c.push("flex-wrap".into());
        }
        match al.primary_align {
            LayoutAlign::Center => c.push("justify-center".into()),
            LayoutAlign::End => c.push("justify-end".into()),
            LayoutAlign::SpaceBetween => c.push("justify-between".into()),
            _ => {}
        }
        match al.counter_align {
            LayoutAlign::Center => c.push("items-center".into()),
            LayoutAlign::End => c.push("items-end".into()),
            LayoutAlign::Baseline => c.push("items-baseline".into()),
            _ => c.push("items-start".into()),
        }
        if al.item_spacing > 0.0 && al.primary_align != LayoutAlign::SpaceBetween {
            c.push(spaced("gap", al.item_spacing, bound(doc, n, "itemSpacing")));
        }
        c.extend(pad_classes(doc, n, &al));
    }
    // size
    let main_fill = |is_w: bool| sz.in_flow && (is_w == sz.row_parent);
    for (is_w, d) in [(true, sz.w), (false, sz.h)] {
        match d {
            Dim::Fill if main_fill(is_w) => {
                if !c.contains(&"flex-1".to_string()) {
                    c.push("flex-1".into());
                }
            }
            Dim::Fill => c.push("self-stretch".into()),
            _ => {}
        }
    }
    if let Some(w) = size_class("w", sz.w, bound(doc, n, "width")) {
        c.push(w);
    }
    if let Some(h) = size_class("h", sz.h, bound(doc, n, "height")) {
        c.push(h);
    }
    if sz.in_flow && !c.contains(&"flex-1".to_string()) && (matches!(sz.w, Dim::Fixed(_)) || matches!(sz.h, Dim::Fixed(_))) {
        c.push("shrink-0".into());
    }
    // fills
    if let Some(t) = text_of(n) {
        text_classes(doc, n, t, &mut c);
    } else if !matches!(n.kind, NodeKind::Line) {
        let visible: Vec<(usize, &Paint)> = n.fills.iter().enumerate().filter(|(_, p)| p.visible && p.opacity > 0.0).collect();
        if let Some((i, p)) = visible.last() {
            c.push(color_class(doc, n, &format!("fills/{i}"), "bg", p, false));
        }
    }
    // stroke
    if let Some(p) = n.stroke.paints.iter().find(|p| p.visible && p.opacity > 0.0).filter(|_| n.stroke.weight > 0.0 && text_of(n).is_none()) {
        let w = n.stroke.weight;
        let side = if matches!(n.kind, NodeKind::Line) { "border-t" } else { "border" };
        c.push(match w as i32 {
            1 if w == 1.0 => side.to_string(),
            2 | 4 | 8 if w.fract() == 0.0 => format!("{side}-{}", w as i32),
            _ => format!("{side}-[{}px]", num(w)),
        });
        if n.stroke.dashes.iter().any(|d| *d > 0.0) {
            c.push("border-dashed".into());
        }
        c.push(color_class(doc, n, "strokes/0", "border", p, false));
        if n.stroke.align != StrokeAlign::Inside {
            c.push(format!("outline-offset-[{}px]", if n.stroke.align == StrokeAlign::Center { num(-w / 2.0) } else { "0".into() }));
        }
    }
    // shape
    match &n.kind {
        NodeKind::Ellipse(a) if (a.end - a.start).abs() >= std::f32::consts::TAU - 1e-4 && a.inner_radius <= 0.0 => c.push("rounded-full".into()),
        NodeKind::Rectangle | NodeKind::Frame(_) | NodeKind::Component(_) | NodeKind::ComponentSet(_) | NodeKind::Instance { .. } => {
            c.extend(corner_classes(doc, n))
        }
        _ => {}
    }
    if n.kind.frame_props().map(|f| f.clips_content).unwrap_or(false) && has_children {
        c.push("overflow-hidden".into());
    }
    // effects
    if let Some(s) = shadow_value(n) {
        c.push(s);
    }
    for e in &n.effects {
        match e {
            Effect::LayerBlur { radius, visible: true } if *radius > 0.0 => c.push(format!("blur-[{}px]", num(radius / 2.0))),
            Effect::BackgroundBlur { radius, visible: true } if *radius > 0.0 => c.push(format!("backdrop-blur-[{}px]", num(radius / 2.0))),
            _ => {}
        }
    }
    match bound(doc, n, "opacity") {
        Some(v) => c.push(format!("opacity-[calc({}/100)]", css_var(v))),
        None => c.extend(opacity_class(n.opacity)),
    }
    c
}

fn escape_text(s: &str, jsx: bool) -> String {
    let mut o = String::new();
    for ch in s.chars() {
        match ch {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '{' if jsx => o.push_str("&#123;"),
            '}' if jsx => o.push_str("&#125;"),
            '\n' => o.push_str("<br />"),
            c => o.push(c),
        }
    }
    o
}

/// ` name="value"` attributes of an instance's component properties.
pub(crate) fn instance_attrs(doc: &Document, n: &Node) -> String {
    let mut o = String::new();
    for (name, lit) in instance_args(doc, n) {
        match lit {
            PropLit::Bool(true) => o.push_str(&format!(" {name}")),
            PropLit::Bool(false) => o.push_str(&format!(" {name}={{false}}")),
            PropLit::Text(t) => o.push_str(&format!(" {name}=\"{}\"", t.replace('"', "&quot;"))),
            PropLit::Instance(v) => o.push_str(&format!(" {name}={{<{v} />}}")),
        }
    }
    o
}

fn markup(doc: &Document, id: NodeId, parent: Parent, depth: usize, o: &mut Out, jsx: bool, html: bool) {
    let Some(n) = doc.get(id) else { return };
    if !n.visible {
        return;
    }
    let attr = if jsx { "className" } else { "class" };
    let cls = classes(doc, n, parent).join(" ");
    let class_attr = if cls.is_empty() { String::new() } else { format!(" {attr}=\"{cls}\"") };

    // In a standalone page an unknown tag would be dropped by the browser, taking
    // the instance's resolved children with it, so it becomes a `div` there.
    if matches!(n.kind, NodeKind::Instance { .. }) && !html {
        o.line(depth, format!("<{}{}{class_attr} />", view_name(doc, n), instance_attrs(doc, n)));
        return;
    }
    if is_vector_asset(doc, id) {
        let src = if html { format!("assets/{}.svg", kind_slug(&n.name)) } else { format!("/assets/{}.svg", kind_slug(&n.name)) };
        let size = if jsx {
            format!(" width={{{}}} height={{{}}}", num(n.size.x), num(n.size.y))
        } else {
            format!(" width=\"{}\" height=\"{}\"", num(n.size.x), num(n.size.y))
        };
        // The SVG paints itself, so only the classes that place it belong on the
        // <img>: a `border`, `bg-*` or `shadow-*` copied from the vector would
        // draw a box around the icon.
        let keep = |c: &str| {
            !c.starts_with("w-")
                && !c.starts_with("h-")
                && (c.starts_with("absolute")
                    || c.starts_with("relative")
                    || c.starts_with("left-")
                    || c.starts_with("top-")
                    || c.starts_with("right-")
                    || c.starts_with("bottom-")
                    || c.starts_with("shrink")
                    || c.starts_with("flex-")
                    || c.starts_with("self-")
                    || c.starts_with("grow")
                    || c.starts_with("order-")
                    || c.starts_with("rotate-")
                    || c.starts_with("z-"))
        };
        let no_size: String = cls.split(' ').filter(|c| keep(c)).collect::<Vec<_>>().join(" ");
        let class_attr = if no_size.is_empty() { String::new() } else { format!(" {attr}=\"{no_size}\"") };
        o.line(depth, format!("<img src=\"{src}\" alt=\"{}\"{size}{class_attr} />", n.name.replace('"', "&quot;")));
        return;
    }
    if let Some(t) = text_of(n) {
        o.line(depth, format!("<p{class_attr}>{}</p>", escape_text(&t.content, jsx)));
        return;
    }
    let tag = if matches!(n.kind, NodeKind::Section) { "section" } else { "div" };
    let kids = visible_children(doc, n);
    if kids.is_empty() {
        if jsx {
            o.line(depth, format!("<{tag}{class_attr} />"));
        } else {
            o.line(depth, format!("<{tag}{class_attr}></{tag}>"));
        }
        return;
    }
    o.line(depth, format!("<{tag}{class_attr}>"));
    let me = Parent::of(n);
    for k in kids {
        markup(doc, k.id, me, depth + 1, o, jsx, html);
    }
    o.line(depth, format!("</{tag}>"));
}

fn kind_slug(name: &str) -> String {
    crate::codegen::kebab(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::fixture;

    #[test]
    fn golden_card() {
        let (d, id) = fixture::card();
        let expected = r####"<div class="flex flex-col items-start gap-2 p-4 w-[312px] h-[200px] bg-[#FFFFFF] border border-[#E5E5E5] rounded-lg overflow-hidden shadow-[0px_4px_8px_0px_rgba(0,0,0,0.25)]">
  <p class="font-['Inter'] text-xl font-semibold text-[#111111]">Hello world</p>
  <p class="self-stretch font-['Inter'] text-sm font-normal leading-5 text-center text-[color:var(--color-text-muted)]">Body copy</p>
  <div class="flex justify-center items-center gap-[calc(var(--spacing-md)*1px)] px-3 py-2 self-stretch bg-[var(--color-primary)] rounded">
    <Button label="Save" disabled class="w-20 h-8 shrink-0" />
    <div class="w-6 h-6 shrink-0 bg-[#FF0000] rounded-full"></div>
  </div>
</div>"####;
        assert_eq!(tailwind_for(&d, id), expected);
    }

    #[test]
    fn html_page_writes_instances_as_divs_and_relative_assets() {
        let (d, id) = fixture::card();
        let out = tailwind_html(&d, id);
        assert!(!out.contains("<Button"), "an unknown tag would be dropped: {out}");
        assert!(out.contains("w-20 h-8 shrink-0"), "the instance keeps its classes: {out}");
        let (mut d, card) = fixture::card();
        let v = d.alloc_id();
        let mut icon = Node::new(v, NodeKind::Vector { paths: vec![], network: None });
        icon.name = "Chevron Down".into();
        icon.size = Vec2::new(16.0, 16.0);
        d.insert(card, icon).unwrap();
        assert!(tailwind_html(&d, card).contains("src=\"assets/chevron-down.svg\""), "asset paths are relative to the page");
        assert!(tailwind_for(&d, card).contains("src=\"/assets/chevron-down.svg\""), "Dev Mode keeps the served path");
    }

    #[test]
    fn exact_values_when_off_scale() {
        let (mut d, id) = fixture::card();
        d.get_mut(id).unwrap().size = Vec2::new(313.0, 200.0);
        d.get_mut(id).unwrap().corner_radii = [10.0, 10.0, 0.0, 0.0];
        d.get_mut(id).unwrap().opacity = 0.37;
        let out = tailwind_for(&d, id);
        assert!(out.contains("w-[313px]"), "{out}");
        assert!(out.contains("rounded-tl-[10px] rounded-tr-[10px]"), "{out}");
        assert!(out.contains("opacity-[0.37]"), "{out}");
    }
}
