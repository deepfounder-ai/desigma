//! Flutter (Dart) output.

use crate::codegen::*;
use crate::util::{num, num2};
use desigma_doc::*;

/// A Flutter `StatelessWidget` for `node` and its descendants.
///
/// Auto layout becomes `Row`/`Column` (`mainAxisAlignment`, `crossAxisAlignment` and the
/// `spacing` argument, which needs Flutter 3.27 or newer), absolute children become
/// `Positioned` inside a `Stack`, text becomes `Text` with a `TextStyle`, instances call the
/// component by name, and a container's fill, stroke, corner radii, effects and padding become
/// a `BoxDecoration` plus `EdgeInsets`. Bound variables become `Theme.of(context).colorScheme.*`
/// colour references and `DesignTokens.*` numbers.
pub fn flutter_for(doc: &Document, node: NodeId) -> String {
    let Some(n) = doc.get(node) else { return String::new() };
    let name = view_name(doc, n);
    let body = statement(&widget(doc, node, Parent::root()), 2);
    format!(
        "class {name} extends StatelessWidget {{
  const {name}({{super.key}});

  @override
  Widget build(BuildContext context) {{
{body}
  }}
}}
"
    )
}

/// Indents the continuation lines of `s` by `n` levels, leaving the first line alone so it can
/// follow a `prefix: ` already written by the caller.
fn hang(s: &str, n: usize) -> String {
    let pad = "  ".repeat(n);
    let mut out = String::new();
    for (i, l) in s.lines().enumerate() {
        if i > 0 {
            out.push('\n');
            out.push_str(&pad);
        }
        out.push_str(l);
    }
    out
}

/// `return <expr>;` at `base` levels of indentation.
fn statement(expr: &str, base: usize) -> String {
    let pad = "  ".repeat(base);
    let mut out = String::new();
    for (i, l) in expr.lines().enumerate() {
        if i == 0 {
            out.push_str(&format!("{pad}return {l}"));
        } else {
            out.push_str(&format!("\n{pad}{l}"));
        }
    }
    out.push(';');
    out
}

/// `Head(pos..., name: value, ...)`: one line while it stays short, one argument per line
/// otherwise.
fn call(head: &str, pos: &[String], args: &[(String, String)]) -> String {
    let total = pos.len() + args.len();
    if total == 0 {
        return format!("{head}()");
    }
    let mut parts: Vec<String> = pos.to_vec();
    parts.extend(args.iter().map(|(k, v)| format!("{k}: {v}")));
    let single_line = parts.iter().all(|p| !p.contains('\n')) && parts.iter().map(|p| p.len()).sum::<usize>() + head.len() + 2 * total < 80;
    if single_line {
        return format!("{head}({})", parts.join(", "));
    }
    let mut s = format!("{head}(\n");
    for p in pos {
        s.push_str(&format!("  {},\n", hang(p, 1)));
    }
    for (k, v) in args {
        s.push_str(&format!("  {k}: {},\n", hang(v, 1)));
    }
    s.push(')');
    s
}

/// A Dart string literal.
fn dart_str(s: &str) -> String {
    let mut o = String::from("'");
    for c in s.chars() {
        match c {
            '\'' => o.push_str("\\'"),
            '\\' => o.push_str("\\\\"),
            '$' => o.push_str("\\$"),
            '\n' => o.push_str("\\n"),
            c => o.push(c),
        }
    }
    o.push('\'');
    o
}

fn dp(doc: &Document, n: &Node, key: &str, v: f32) -> String {
    match bound(doc, n, key) {
        Some(var) => format!("DesignTokens.{}", token_camel(var, false)),
        None => num(v),
    }
}

fn color(doc: &Document, n: &Node, key: &str, c: &Color, opacity: f32) -> String {
    if let Some(v) = bound(doc, n, key) {
        return format!("Theme.of(context).colorScheme.{}", token_camel(v, true));
    }
    let a = ((c.a * opacity).clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("Color(0x{a:02X}{})", c.to_hex())
}

fn align(v: Vec2) -> String {
    format!("Alignment({}, {})", num2(v.x * 2.0 - 1.0), num2(v.y * 2.0 - 1.0))
}

fn gradient(doc: &Document, n: &Node, p: &Paint) -> Option<String> {
    let colors = |stops: &[GradientStop]| -> String { stops.iter().map(|s| color(doc, n, "", &s.color, p.opacity)).collect::<Vec<_>>().join(", ") };
    Some(match &p.kind {
        PaintKind::Solid(_) => return None,
        PaintKind::LinearGradient { handles, stops } => call(
            "LinearGradient",
            &[],
            &[("colors".into(), format!("[{}]", colors(stops))), ("begin".into(), align(handles[0])), ("end".into(), align(handles[1]))],
        ),
        PaintKind::RadialGradient { handles, stops } | PaintKind::DiamondGradient { handles, stops } => call(
            "RadialGradient",
            &[],
            &[("colors".into(), format!("[{}]", colors(stops))), ("center".into(), align(handles[0])), ("radius".into(), num(handles[1].x - handles[0].x))],
        ),
        PaintKind::AngularGradient { handles, stops } => {
            call("SweepGradient", &[], &[("colors".into(), format!("[{}]", colors(stops))), ("center".into(), align(handles[0]))])
        }
        PaintKind::Image { .. } => return None,
    })
}

fn border_radius(doc: &Document, n: &Node) -> Option<String> {
    if matches!(n.kind, NodeKind::Ellipse(_)) {
        return Some("BorderRadius.all(Radius.elliptical(1000, 1000))".into());
    }
    let keys = ["topLeftRadius", "topRightRadius", "bottomRightRadius", "bottomLeftRadius"];
    let r = n.corner_radii;
    if r.iter().all(|v| *v == 0.0) && !keys.iter().any(|k| bound(doc, n, k).is_some()) {
        return None;
    }
    let uniform = r.iter().all(|v| *v == r[0]) && keys.iter().all(|k| bound(doc, n, k).map(|v| &v.id) == bound(doc, n, keys[0]).map(|v| &v.id));
    if uniform {
        return Some(format!("BorderRadius.circular({})", dp(doc, n, keys[0], r[0])));
    }
    let v = |i: usize| format!("Radius.circular({})", dp(doc, n, keys[i], r[i]));
    Some(format!("BorderRadius.only(topLeft: {}, topRight: {}, bottomRight: {}, bottomLeft: {})", v(0), v(1), v(2), v(3)))
}

fn decoration(doc: &Document, n: &Node) -> Option<String> {
    let mut args: Vec<(String, String)> = Vec::new();
    let radius = border_radius(doc, n);
    if let Some((i, p)) = n.fills.iter().enumerate().rev().find(|(_, p)| p.visible && p.opacity > 0.0) {
        match (gradient(doc, n, p), &p.kind) {
            (Some(g), _) => args.push(("gradient".into(), g)),
            (None, PaintKind::Solid(c)) => args.push(("color".into(), color(doc, n, &format!("fills/{i}"), c, p.opacity))),
            _ => {}
        }
    }
    if let Some(r) = &radius {
        args.push(("borderRadius".into(), r.clone()));
    }
    if let Some(p) = n.stroke.paints.iter().find(|p| p.visible && p.opacity > 0.0).filter(|_| n.stroke.weight > 0.0) {
        if let PaintKind::Solid(c) = &p.kind {
            let b = call(
                "Border",
                &[],
                &[
                    (
                        "top".into(),
                        format!("BorderSide(color: {}, width: {})", color(doc, n, "strokes/0", c, p.opacity), dp(doc, n, "strokeWeight", n.stroke.weight)),
                    ),
                    ("right".into(), "BorderSide.none".into()),
                    ("bottom".into(), "BorderSide.none".into()),
                    ("left".into(), "BorderSide.none".into()),
                ],
            );
            args.push(("border".into(), b));
        }
    }
    let shadows = drop_shadows(n);
    if !shadows.is_empty() {
        let list: Vec<String> = shadows
            .iter()
            .map(|(c, off, radius, spread)| {
                let col = color(doc, n, "", c, 1.0);
                call(
                    "BoxShadow",
                    &[],
                    &[
                        ("color".into(), col),
                        ("offset".into(), call("Offset", &[num(off.x), num(off.y)], &[])),
                        ("blurRadius".into(), num(*radius)),
                        ("spreadRadius".into(), num(*spread)),
                    ],
                )
            })
            .collect();
        args.push(("boxShadow".into(), list_arg(&list)));
    }
    if args.is_empty() {
        return None;
    }
    Some(call("BoxDecoration", &[], &args))
}

/// `[\n  a,\n  b,\n]`
fn list_arg(items: &[String]) -> String {
    if items.is_empty() {
        return "[]".into();
    }
    let mut s = String::from("[\n");
    for it in items {
        s.push_str(&format!("  {},\n", hang(it, 1)));
    }
    s.push(']');
    s
}

fn padding_edge(doc: &Document, n: &Node, al: &AutoLayout) -> Option<String> {
    let keys = ["paddingTop", "paddingRight", "paddingBottom", "paddingLeft"];
    let p = al.padding;
    let any = |i: usize| p[i] > 0.0 || bound(doc, n, keys[i]).is_some();
    if !(0..4).any(any) {
        return None;
    }
    let v = |i: usize| dp(doc, n, keys[i], p[i]);
    let same = |a: usize, b: usize| p[a] == p[b] && bound(doc, n, keys[a]).map(|x| &x.id) == bound(doc, n, keys[b]).map(|x| &x.id);
    if same(0, 1) && same(1, 2) && same(2, 3) {
        return Some(call("EdgeInsets.all", &[v(0)], &[]));
    }
    if same(1, 3) && same(0, 2) {
        return Some(call("EdgeInsets.symmetric", &[], &[("horizontal".into(), v(1)), ("vertical".into(), v(0))]));
    }
    Some(call("EdgeInsets.only", &[], &[("left".into(), v(3)), ("top".into(), v(0)), ("right".into(), v(1)), ("bottom".into(), v(2))]))
}

fn font_weight(w: u16) -> &'static str {
    match w {
        0..=149 => "FontWeight.w100",
        150..=249 => "FontWeight.w200",
        250..=349 => "FontWeight.w300",
        350..=449 => "FontWeight.w400",
        450..=549 => "FontWeight.w500",
        550..=649 => "FontWeight.w600",
        650..=749 => "FontWeight.w700",
        750..=849 => "FontWeight.w800",
        _ => "FontWeight.w900",
    }
}

fn text_style(doc: &Document, n: &Node, t: &TextProps) -> String {
    let s = &t.style;
    let mut args: Vec<(String, String)> = Vec::new();
    if let Some(p) = first_paint(&s.fills) {
        if let PaintKind::Solid(c) = &p.kind {
            args.push(("color".into(), color(doc, n, "fills/0", c, p.opacity)));
        }
    }
    args.push((
        "fontSize".into(),
        match bound(doc, n, "fontSize") {
            Some(v) => format!("DesignTokens.{}", token_camel(v, false)),
            None => num(s.font_size),
        },
    ));
    if s.font_weight != 400 {
        args.push(("fontWeight".into(), font_weight(s.font_weight).into()));
    }
    args.push(("fontFamily".into(), dart_str(&s.font_family)));
    match s.line_height {
        LineHeight::Auto => {}
        LineHeight::Percent(p) => args.push(("height".into(), num2(p / 100.0))),
        LineHeight::Pixels(p) if s.font_size > 0.0 => args.push(("height".into(), num2(p / s.font_size))),
        LineHeight::Pixels(_) => {}
    }
    if s.italic {
        args.push(("fontStyle".into(), "FontStyle.italic".into()));
    }
    if s.letter_spacing != 0.0 {
        let v = match bound(doc, n, "letterSpacing") {
            Some(var) => format!("DesignTokens.{}", token_camel(var, false)),
            None => num(s.letter_spacing),
        };
        args.push(("letterSpacing".into(), v));
    }
    match s.decoration {
        TextDecoration::None => {}
        TextDecoration::Underline => args.push(("decoration".into(), "TextDecoration.underline".into())),
        TextDecoration::Strikethrough => args.push(("decoration".into(), "TextDecoration.lineThrough".into())),
    }
    call("TextStyle", &[], &args)
}

fn text_widget(doc: &Document, n: &Node, t: &TextProps) -> String {
    let mut args: Vec<(String, String)> = vec![("style".into(), text_style(doc, n, t))];
    match t.style.align_h {
        TextAlignH::Left => {}
        TextAlignH::Center => args.push(("textAlign".into(), "TextAlign.center".into())),
        TextAlignH::Right => args.push(("textAlign".into(), "TextAlign.right".into())),
        TextAlignH::Justified => args.push(("textAlign".into(), "TextAlign.justify".into())),
    }
    call("Text", &[dart_str(&t.content)], &args)
}

/// `width`/`height` arguments for a fixed axis, `double.infinity` for a filled one.
fn dims(doc: &Document, n: &Node, sz: &Sizes) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let axis = |name: &str, d: Dim| match d {
        Dim::Fixed(v) => Some((name.to_string(), dp(doc, n, name, v))),
        Dim::Fill => Some((name.to_string(), "double.infinity".into())),
        Dim::Hug => None,
    };
    out.extend(axis("width", sz.w));
    out.extend(axis("height", sz.h));
    out
}

fn main_axis(a: LayoutAlign) -> &'static str {
    match a {
        LayoutAlign::Center => "MainAxisAlignment.center",
        LayoutAlign::End => "MainAxisAlignment.end",
        LayoutAlign::SpaceBetween => "MainAxisAlignment.spaceBetween",
        _ => "MainAxisAlignment.start",
    }
}

fn cross_axis(a: LayoutAlign) -> &'static str {
    match a {
        LayoutAlign::Center => "CrossAxisAlignment.center",
        LayoutAlign::End => "CrossAxisAlignment.end",
        _ => "CrossAxisAlignment.start",
    }
}

fn flow(doc: &Document, n: &Node, al: &AutoLayout, kids: &[&Node]) -> String {
    let horizontal = al.direction == LayoutDirection::Horizontal;
    let mut args: Vec<(String, String)> =
        vec![("mainAxisAlignment".into(), main_axis(al.primary_align).into()), ("crossAxisAlignment".into(), cross_axis(al.counter_align).into())];
    if al.item_spacing > 0.0 || bound(doc, n, "itemSpacing").is_some() {
        args.push(("spacing".into(), dp(doc, n, "itemSpacing", al.item_spacing)));
    }
    let me = Parent::of(n);
    let items: Vec<String> = kids
        .iter()
        .map(|k| {
            let csz = sizes(k, me);
            let w = widget(doc, k.id, me);
            let grows = horizontal && csz.w == Dim::Fill;
            let grows = grows || (!horizontal && csz.h == Dim::Fill);
            if grows {
                call("Expanded", &[], &[("child".into(), w)])
            } else {
                w
            }
        })
        .collect();
    args.push(("children".into(), list_arg(&items)));
    call(if horizontal { "Row" } else { "Column" }, &[], &args)
}

fn stack(doc: &Document, n: &Node, kids: &[&Node]) -> String {
    let me = Parent::of(n);
    let items: Vec<String> = kids
        .iter()
        .map(|k| {
            let csz = sizes(k, me);
            let w = widget(doc, k.id, me);
            match offset(k, me, &csz) {
                Some((x, y)) if x != 0.0 || y != 0.0 => call("Positioned", &[], &[("left".into(), num(x)), ("top".into(), num(y)), ("child".into(), w)]),
                _ => w,
            }
        })
        .collect();
    call("Stack", &[], &[("clipBehavior".into(), "Clip.none".into()), ("children".into(), list_arg(&items))])
}

fn widget(doc: &Document, id: NodeId, parent: Parent) -> String {
    let Some(n) = doc.get(id) else { return "const SizedBox.shrink()".into() };
    if !n.visible {
        return "const SizedBox.shrink()".into();
    }
    let sz = sizes(n, parent);
    let rot = rotation_deg(n);
    let own_al = n.kind.frame_props().and_then(|f| f.auto_layout);
    let kids = visible_children(doc, n);

    // Leaf kinds first: they own the whole widget.
    let leaf = if matches!(n.kind, NodeKind::Instance { .. }) {
        let args: Vec<(String, String)> = instance_args(doc, n)
            .into_iter()
            .map(|(name, lit)| match lit {
                PropLit::Bool(b) => (name, b.to_string()),
                PropLit::Text(t) => (name, dart_str(&t)),
                PropLit::Instance(v) => (name, call(&v, &[], &[])),
            })
            .collect();
        Some(call(&view_name(doc, n), &[], &args))
    } else if is_vector_asset(doc, id) {
        Some(call("Image.asset", &[dart_str(&format!("assets/{}.svg", snake(&n.name)))], &[("width".into(), num(n.size.x)), ("height".into(), num(n.size.y))]))
    } else {
        text_of(n).map(|t| text_widget(doc, n, t))
    };

    let mut args: Vec<(String, String)> = dims(doc, n, &sz);
    let mut inner = leaf;
    if inner.is_none() {
        if let Some(deco) = decoration(doc, n) {
            args.push(("decoration".into(), deco));
        }
        if let Some(al) = own_al.filter(|_| !kids.is_empty()) {
            if let Some(pad) = padding_edge(doc, n, &al) {
                args.push(("padding".into(), pad));
            }
            inner = Some(flow(doc, n, &al, &kids));
        } else if !kids.is_empty() {
            if let Some(al) = own_al {
                if let Some(pad) = padding_edge(doc, n, &al) {
                    args.push(("padding".into(), pad));
                }
            }
            inner = Some(stack(doc, n, &kids));
        }
    }
    if let Some(c) = inner {
        args.push(("child".into(), c));
    }

    let has_dressing = args.iter().any(|(k, _)| k != "child");
    let size_only = args.iter().all(|(k, _)| k == "width" || k == "height" || k == "child");
    let mut w = if !has_dressing {
        // Nothing to say about this layer beyond its content.
        match args.pop() {
            Some((_, child)) => child,
            None => "const SizedBox.shrink()".to_string(),
        }
    } else if size_only {
        call("SizedBox", &[], &args)
    } else {
        call("Container", &[], &args)
    };

    // Presentation wrappers, innermost first.
    let clips = n.kind.frame_props().map(|f| f.clips_content).unwrap_or(false);
    if clips {
        if let Some(r) = border_radius(doc, n) {
            w = call("ClipRRect", &[], &[("borderRadius".into(), r), ("child".into(), w)]);
        }
    }
    if n.opacity < 0.999 {
        let v = match bound(doc, n, "opacity") {
            Some(var) => format!("DesignTokens.{} / 100", token_camel(var, false)),
            None => num2((n.opacity * 100.0).round() / 100.0),
        };
        w = call("Opacity", &[], &[("opacity".into(), v), ("child".into(), w)]);
    }
    if rot != 0.0 {
        w = call("Transform.rotate", &[], &[("angle".into(), format!("{} * math.pi / 180", num(rot))), ("child".into(), w)]);
    }
    w
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::fixture;

    #[test]
    fn golden_card() {
        let (d, id) = fixture::card();
        let actual = flutter_for(&d, id);
        // A widget tree, not a snippet: it has to be a class with a build method.
        assert!(
            actual.starts_with(
                "class Card extends StatelessWidget {\n  const Card({super.key});\n\n  @override\n  Widget build(BuildContext context) {\n    return "
            ),
            "{actual}"
        );
        // auto layout becomes Row/Column with the axis arguments
        assert!(actual.contains("child: Column("), "{actual}");
        assert!(actual.contains("mainAxisAlignment: MainAxisAlignment.start"), "{actual}");
        assert!(actual.contains("crossAxisAlignment: CrossAxisAlignment.start"), "{actual}");
        assert!(actual.contains("spacing: 8,"), "{actual}");
        assert!(actual.contains("child: Row("), "{actual}");
        // bound variables become theme and token references
        assert!(actual.contains("Theme.of(context).colorScheme.textMuted"), "{actual}");
        assert!(actual.contains("Theme.of(context).colorScheme.primary"), "{actual}");
        assert!(actual.contains("spacing: DesignTokens.spacingMd"), "{actual}");
        // fill on the cross axis stretches
        assert!(actual.contains("width: double.infinity,"), "{actual}");
        // instance becomes a component call with its changed properties, sized by its layer
        assert!(actual.contains("Button(label: 'Save', disabled: true)"), "{actual}");
        assert!(actual.contains("width: 80,"), "{actual}");
        // a layer that only has content is not wrapped in a pointless container
        assert!(!actual.contains("Container(\n        child:"), "{actual}");
        // ellipse corner radius, text style and padding
        assert!(actual.contains("Radius.elliptical(1000, 1000)"), "{actual}");
        assert!(actual.contains("fontWeight: FontWeight.w600"), "{actual}");
        assert!(actual.contains("padding: EdgeInsets.all(16)"), "{actual}");
        assert!(actual.ends_with("}\n"), "{actual}");
    }

    #[test]
    fn absolute_children_are_positioned() {
        let mut d = Document::new("t");
        let page = d.add_page("p");
        let id = d.alloc_id();
        let mut card = Node::frame(id, Rect::new(0.0, 0.0, 200.0, 100.0));
        card.name = "Card".into();
        let card = d.insert(page, card).unwrap();
        let dot_id = d.alloc_id();
        let mut dot = Node::rectangle(dot_id, Rect::new(12.0, 8.0, 4.0, 4.0));
        dot.name = "Dot".into();
        d.insert(card, dot).unwrap();
        let out = flutter_for(&d, card);
        assert!(out.contains("Stack("), "{out}");
        assert!(out.contains("left: 12,"), "{out}");
        assert!(out.contains("top: 8,"), "{out}");
    }
}
