//! Helpers shared by the code generators (Tailwind, SwiftUI, Compose, React).

use crate::util::num;
use desigma_doc::*;

/// Indenting line writer.
pub(crate) struct Out {
    unit: &'static str,
    lines: Vec<String>,
}

impl Out {
    pub fn new(unit: &'static str) -> Self {
        Self { unit, lines: Vec::new() }
    }

    pub fn line(&mut self, depth: usize, s: impl AsRef<str>) {
        self.lines.push(format!("{}{}", self.unit.repeat(depth), s.as_ref()));
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    pub fn finish(self) -> String {
        self.lines.join("\n")
    }
}

/// Splits an identifier-ish string into words (non-alphanumerics and lower->upper boundaries).
fn words(s: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut prev_lower = false;
    for c in s.chars() {
        if c.is_alphanumeric() {
            if c.is_uppercase() && prev_lower && !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            prev_lower = c.is_lowercase() || c.is_ascii_digit();
            cur.push(c);
        } else {
            prev_lower = false;
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn cap(w: &str) -> String {
    let mut c = w.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

pub fn pascal(s: &str) -> String {
    let p: String = words(s).iter().map(|w| cap(w)).collect();
    match p.chars().next() {
        None => "Layer".into(),
        Some(c) if c.is_ascii_digit() => format!("V{p}"),
        _ => p,
    }
}

pub fn camel(s: &str) -> String {
    let p = pascal(s);
    let mut c = p.chars();
    match c.next() {
        Some(f) => f.to_lowercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

pub fn kebab(s: &str) -> String {
    let w: Vec<String> = words(s).iter().map(|w| w.to_lowercase()).collect();
    if w.is_empty() {
        "layer".into()
    } else {
        w.join("-")
    }
}

pub fn snake(s: &str) -> String {
    kebab(s).replace('-', "_")
}

/// Name of the view/component a node becomes: the component for instances (the set for
/// variants), else the node's own name.
pub fn view_name(doc: &Document, n: &Node) -> String {
    if let Some(c) = instance_component(n).and_then(|c| doc.get(c)) {
        let set = c.parent.and_then(|p| doc.get(p)).filter(|p| matches!(p.kind, NodeKind::ComponentSet(_)));
        return pascal(&set.unwrap_or(c).name);
    }
    pascal(&n.name)
}

pub fn bound<'a>(doc: &'a Document, n: &Node, key: &str) -> Option<&'a Variable> {
    doc.variables.get(n.variables.bindings.get(key)?)
}

fn segments(v: &Variable) -> Vec<String> {
    let s: Vec<String> = v.name.split('/').map(kebab).filter(|s| !s.is_empty()).collect();
    if s.is_empty() {
        vec![kebab(&v.id)]
    } else {
        s
    }
}

/// `var(--color-primary)` for a variable named `Color/Primary`.
pub fn css_var(v: &Variable) -> String {
    let mut s = segments(v);
    if s.len() == 1 && v.kind == VariableKind::Color {
        s.insert(0, "color".into());
    }
    format!("var(--{})", s.join("-"))
}

/// camelCase token name, dropping a leading `color` segment (`Color/Brand/Primary` -> `brandPrimary`).
pub fn token_camel(v: &Variable, drop_color: bool) -> String {
    let mut s = segments(v);
    if drop_color && s.len() > 1 && (s[0] == "color" || s[0] == "colors") {
        s.remove(0);
    }
    camel(&s.join(" "))
}

pub fn first_paint(paints: &[Paint]) -> Option<&Paint> {
    paints.iter().rev().find(|p| p.visible && p.opacity > 0.0)
}

pub fn visible_children<'a>(doc: &'a Document, n: &Node) -> Vec<&'a Node> {
    n.children.iter().filter_map(|c| doc.get(*c)).filter(|c| c.visible).collect()
}

pub fn is_vector_kind(k: &NodeKind) -> bool {
    matches!(k, NodeKind::Vector { .. } | NodeKind::BooleanOp(_) | NodeKind::Star { .. } | NodeKind::Polygon { .. })
}

/// Whether a layer exports as an icon/vector asset: a vector shape, or a small container holding
/// only vector shapes.
pub fn is_vector_asset(doc: &Document, node: NodeId) -> bool {
    let Some(n) = doc.get(node) else { return false };
    if is_vector_kind(&n.kind) {
        return true;
    }
    if !matches!(n.kind, NodeKind::Frame(_) | NodeKind::Group | NodeKind::Component(_) | NodeKind::Instance { .. }) || n.children.is_empty() {
        return false;
    }
    if n.size.x > 128.0 || n.size.y > 128.0 {
        return false;
    }
    let mut all = true;
    let mut leaves = 0;
    for c in n.children.iter().filter_map(|c| doc.get(*c)).filter(|c| c.visible) {
        if is_vector_kind(&c.kind) || matches!(c.kind, NodeKind::Line | NodeKind::Rectangle | NodeKind::Ellipse(_)) {
            leaves += 1;
        } else if !is_vector_asset(doc, c.id) {
            all = false;
        }
    }
    all && leaves > 0
}

/// Vector-asset layers inside `root` (outermost only), in document order.
pub fn vector_assets(doc: &Document, root: NodeId) -> Vec<NodeId> {
    fn rec(doc: &Document, id: NodeId, out: &mut Vec<NodeId>) {
        let Some(n) = doc.get(id) else { return };
        if !n.visible {
            return;
        }
        if is_vector_asset(doc, id) {
            out.push(id);
            return;
        }
        for c in &n.children {
            rec(doc, *c, out);
        }
    }
    let mut out = Vec::new();
    rec(doc, root, &mut out);
    out
}

/// A literal value of an instance's component property.
pub(crate) enum PropLit {
    Bool(bool),
    Text(String),
    Instance(String),
}

/// Component properties of an instance worth passing as arguments: values that differ from the
/// default, plus variant selections. Names are camelCased without Figma's `#id` suffix.
pub(crate) fn instance_args(doc: &Document, n: &Node) -> Vec<(String, PropLit)> {
    let Some(comp) = instance_component(n) else { return Vec::new() };
    let mut out = Vec::new();
    for def in prop_defs(doc, comp) {
        let value = prop_value(doc, n.id, &def);
        let name = camel(def.name.split('#').next().unwrap_or(&def.name));
        let lit = match (&def.kind, &value) {
            (PropKind::Variant { .. }, PropValue::Variant(v)) => PropLit::Text(v.clone()),
            _ if value == def.default => continue,
            (_, PropValue::Bool(b)) => PropLit::Bool(*b),
            (_, PropValue::Text(t)) => PropLit::Text(t.clone()),
            (_, PropValue::Variant(v)) => PropLit::Text(v.clone()),
            (_, PropValue::Instance(c)) => PropLit::Instance(doc.get(*c).map(|c| pascal(&c.name)).unwrap_or_else(|| "Icon".into())),
        };
        out.push((name, lit));
    }
    out
}

/// Sizing of one axis.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Dim {
    Fixed(f32),
    Hug,
    /// Fill the parent (flex grow on the main axis, stretch on the cross axis).
    Fill,
}

/// The auto-layout parent context of a node.
#[derive(Clone, Copy)]
pub(crate) struct Parent {
    pub al: Option<AutoLayout>,
    pub root: bool,
}

impl Parent {
    pub fn root() -> Self {
        Self { al: None, root: true }
    }
    pub fn of(n: &Node) -> Self {
        Self { al: n.kind.frame_props().and_then(|f| f.auto_layout), root: false }
    }
}

pub(crate) struct Sizes {
    pub w: Dim,
    pub h: Dim,
    /// Placed by flow of an auto-layout parent.
    pub in_flow: bool,
    /// Whether the parent lays out in a row (main axis horizontal).
    pub row_parent: bool,
}

pub(crate) fn sizes(n: &Node, parent: Parent) -> Sizes {
    let in_flow = parent.al.is_some() && !n.layout_child.absolute && !parent.root;
    let own_al = n.kind.frame_props().and_then(|f| f.auto_layout);
    let row_parent = parent.al.map(|a| a.direction == LayoutDirection::Horizontal).unwrap_or(true);
    let hug_w = own_al.map(|a| if a.direction == LayoutDirection::Horizontal { a.primary_sizing } else { a.counter_sizing } == Sizing::Hug).unwrap_or(false);
    let hug_h = own_al.map(|a| if a.direction == LayoutDirection::Horizontal { a.counter_sizing } else { a.primary_sizing } == Sizing::Hug).unwrap_or(false);
    let text_w = matches!(&n.kind, NodeKind::Text(t) if t.auto_resize == TextAutoResize::WidthAndHeight);
    let text_h = matches!(&n.kind, NodeKind::Text(t) if matches!(t.auto_resize, TextAutoResize::WidthAndHeight | TextAutoResize::Height));
    let grows = in_flow && n.layout_child.grow > 0.0;
    let fill_w = in_flow && (n.layout_child.horizontal == Sizing::Fill || (grows && row_parent));
    let fill_h = in_flow && (n.layout_child.vertical == Sizing::Fill || (grows && !row_parent));
    let w = if fill_w {
        Dim::Fill
    } else if hug_w || text_w {
        Dim::Hug
    } else {
        Dim::Fixed(n.size.x)
    };
    let h = if fill_h {
        Dim::Fill
    } else if hug_h || text_h {
        Dim::Hug
    } else if matches!(n.kind, NodeKind::Line) {
        Dim::Fixed(0.0)
    } else {
        Dim::Fixed(n.size.y)
    };
    Sizes { w, h, in_flow, row_parent }
}

/// Position offset for children that are not placed by auto layout.
pub(crate) fn offset(n: &Node, parent: Parent, sz: &Sizes) -> Option<(f32, f32)> {
    if parent.root || sz.in_flow {
        return None;
    }
    Some((n.transform.tx, n.transform.ty))
}

pub(crate) fn rotation_deg(n: &Node) -> f32 {
    let r = n.transform.rotation_rad().to_degrees();
    if r.abs() > 0.01 {
        r
    } else {
        0.0
    }
}

pub fn text_of(n: &Node) -> Option<&TextProps> {
    match &n.kind {
        NodeKind::Text(t) => Some(t),
        _ => None,
    }
}

/// Resolved `"` / `\` / newline escaping for Swift and Kotlin string literals.
pub fn quote(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '$' => o.push_str("\\$"),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

/// Visible effects of a given sort.
pub(crate) fn drop_shadows(n: &Node) -> Vec<(&Color, &Vec2, f32, f32)> {
    n.effects
        .iter()
        .filter_map(|e| match e {
            Effect::DropShadow { color, offset, radius, spread, visible: true, .. } => Some((color, offset, *radius, *spread)),
            _ => None,
        })
        .collect()
}

// ---------------------------------------------------------------- CSS variable references

fn set_decl(lines: &mut Vec<String>, prop: &str, value: String) {
    let pre = format!("{prop}: ");
    if let Some(l) = lines.iter_mut().find(|l| l.starts_with(&pre)) {
        *l = format!("{prop}: {value};");
    } else {
        lines.push(format!("{prop}: {value};"));
    }
}

fn has_decl(lines: &[String], prop: &str) -> bool {
    let pre = format!("{prop}: ");
    lines.iter().any(|l| l.starts_with(&pre))
}

/// Replaces literal values in CSS declarations by `var(--token)` references for properties
/// bound to variables.
pub(crate) fn apply_css_vars(doc: &Document, n: &Node, lines: &mut Vec<String>) {
    if n.variables.bindings.is_empty() {
        return;
    }
    let b = |key: &str| bound(doc, n, key).map(css_var);
    // A number token (`Space/Screen` = 20) is stored unitless, so a length
    // property needs `calc(var(--space-screen) * 1px)`: a bare `var()` there is
    // invalid CSS and the declaration is dropped.
    let len = |key: &str| bound(doc, n, key).map(|v| if v.kind == VariableKind::Number { format!("calc({} * 1px)", css_var(v)) } else { css_var(v) });
    let is_text = matches!(n.kind, NodeKind::Text(_));
    let visible_fills = if is_text {
        text_of(n).map(|t| t.style.fills.iter().filter(|p| p.visible).count()).unwrap_or(0)
    } else {
        n.fills.iter().filter(|p| p.visible).count()
    };
    if let Some(v) = b("fills/0") {
        if is_text {
            if has_decl(lines, "color") {
                set_decl(lines, "color", v);
            }
        } else if visible_fills == 1 && has_decl(lines, "background") {
            set_decl(lines, "background", v);
        }
    }
    if let Some(v) = b("strokes/0") {
        for prop in ["border", "outline"] {
            let pre = format!("{prop}: ");
            if let Some(l) = lines.iter_mut().find(|l| l.starts_with(&pre)) {
                let body = l[pre.len()..].trim_end_matches(';');
                let mut parts = body.splitn(3, ' ');
                let (w, style) = (parts.next().unwrap_or(""), parts.next().unwrap_or("solid"));
                *l = format!("{prop}: {w} {style} {v};");
            }
        }
        for prop in ["border-color", "-webkit-text-stroke-color"] {
            if has_decl(lines, prop) {
                set_decl(lines, prop, v.clone());
            }
        }
    }
    for (key, prop) in [
        ("width", "width"),
        ("height", "height"),
        ("fontSize", "font-size"),
        ("fontWeight", "font-weight"),
        ("lineHeight", "line-height"),
        ("letterSpacing", "letter-spacing"),
    ] {
        // `font-weight` is a unitless number even when the token is one.
        let value = if prop == "font-weight" { b(key) } else { len(key) };
        if let Some(v) = value {
            if has_decl(lines, prop) {
                set_decl(lines, prop, v);
            }
        }
    }
    if let Some(v) = b("opacity") {
        set_decl(lines, "opacity", format!("calc({} / 100)", v.trim()));
    }
    if let Some(v) = len("itemSpacing") {
        if has_decl(lines, "gap") {
            set_decl(lines, "gap", v);
        }
    }
    // padding: expand to a 4-value shorthand when any side is bound
    let sides = ["paddingTop", "paddingRight", "paddingBottom", "paddingLeft"];
    if sides.iter().any(|k| bound(doc, n, k).is_some()) {
        if let Some(al) = n.kind.frame_props().and_then(|f| f.auto_layout) {
            let vals: Vec<String> = sides.iter().enumerate().map(|(i, k)| len(k).unwrap_or_else(|| format!("{}px", num(al.padding[i])))).collect();
            lines.retain(|l| !l.starts_with("padding: "));
            let value = css_box(&vals);
            // keep declaration order: padding follows the size declarations
            let at = lines
                .iter()
                .position(|l| l.starts_with("flex-direction: ") || l.starts_with("justify-content: ") || l.starts_with("align-items: "))
                .unwrap_or(lines.len());
            lines.insert(at, format!("padding: {value};"));
        }
    }
    let corners = ["topLeftRadius", "topRightRadius", "bottomRightRadius", "bottomLeftRadius"];
    if corners.iter().any(|k| bound(doc, n, k).is_some()) {
        let vals: Vec<String> = corners.iter().enumerate().map(|(i, k)| len(k).unwrap_or_else(|| format!("{}px", num(n.corner_radii[i])))).collect();
        set_decl(lines, "border-radius", css_box(&vals));
    }
}

fn css_box(v: &[String]) -> String {
    if v.iter().all(|x| *x == v[0]) {
        v[0].clone()
    } else if v[0] == v[2] && v[1] == v[3] {
        format!("{} {}", v[0], v[1])
    } else if v[1] == v[3] {
        format!("{} {} {}", v[0], v[1], v[2])
    } else {
        v.join(" ")
    }
}

/// Fixture used by the generators' golden tests: an auto-layout card with text, an instance and
/// bound variables.
#[cfg(test)]
pub(crate) mod fixture {
    use super::*;

    pub fn card() -> (Document, NodeId) {
        let mut d = Document::new("t");
        let page = d.add_page("p");
        d.variable_collections.insert(
            "c1".into(),
            VariableCollection {
                id: "c1".into(),
                name: "Tokens".into(),
                modes: vec![VariableMode { id: "light".into(), name: "Light".into() }, VariableMode { id: "dark".into(), name: "Dark".into() }],
                default_mode: "light".into(),
                variables: vec!["primary".into(), "muted".into(), "md".into()],
                key: None,
            },
        );
        let var = |id: &str, name: &str, kind, values: Vec<(&str, VariableValue)>| Variable {
            id: id.into(),
            name: name.into(),
            collection: "c1".into(),
            kind,
            values: values.into_iter().map(|(k, v)| (k.to_string(), v)).collect(),
            description: String::new(),
            scopes: vec![],
            hidden_from_publishing: false,
            key: None,
        };
        d.variables.insert(
            "primary".into(),
            var(
                "primary",
                "Color/Primary",
                VariableKind::Color,
                vec![("light", VariableValue::Color(Color::rgb(0x0D, 0x99, 0xFF))), ("dark", VariableValue::Color(Color::rgb(0x4D, 0xB8, 0xFF)))],
            ),
        );
        d.variables.insert(
            "muted".into(),
            var(
                "muted",
                "Color/Text/Muted",
                VariableKind::Color,
                vec![("light", VariableValue::Color(Color::rgb(0x66, 0x66, 0x66))), ("dark", VariableValue::Color(Color::rgb(0xAA, 0xAA, 0xAA)))],
            ),
        );
        d.variables.insert(
            "md".into(),
            var("md", "Spacing/Md", VariableKind::Number, vec![("light", VariableValue::Number(12.0)), ("dark", VariableValue::Number(12.0))]),
        );

        let comp_id = d.alloc_id();
        let mut comp = Node::new(comp_id, NodeKind::Component(FrameProps::default()));
        comp.name = "Button".into();
        comp.size = Vec2::new(80.0, 32.0);
        comp.component.props = vec![
            ComponentPropDef { name: "Label".into(), kind: PropKind::Text, default: PropValue::Text("Button".into()) },
            ComponentPropDef { name: "Disabled".into(), kind: PropKind::Boolean, default: PropValue::Bool(false) },
        ];
        let lib_page = d.add_page("Components");
        let comp_id = d.insert(lib_page, comp).unwrap();

        let card_id = d.alloc_id();
        let mut card = Node::frame(card_id, Rect::new(0.0, 0.0, 312.0, 200.0));
        card.name = "Card".into();
        card.corner_radii = [8.0; 4];
        card.stroke.paints = vec![Paint::solid(Color::rgb(0xE5, 0xE5, 0xE5))];
        card.stroke.weight = 1.0;
        card.effects = vec![Effect::DropShadow {
            color: Color::rgba(0, 0, 0, 64),
            offset: Vec2::new(0.0, 4.0),
            radius: 8.0,
            spread: 0.0,
            visible: true,
            show_behind: false,
        }];
        if let NodeKind::Frame(f) = &mut card.kind {
            f.clips_content = true;
            f.auto_layout = Some(AutoLayout {
                direction: LayoutDirection::Vertical,
                item_spacing: 8.0,
                padding: [16.0; 4],
                counter_sizing: Sizing::Fixed,
                primary_sizing: Sizing::Fixed,
                ..Default::default()
            });
        }
        let card_id = d.insert(page, card).unwrap();

        let title_id = d.alloc_id();
        let mut title = Node::text(
            title_id,
            Rect::new(0.0, 0.0, 120.0, 24.0),
            "Hello world",
            TextStyle {
                font_family: "Inter".into(),
                font_weight: 600,
                font_size: 20.0,
                fills: vec![Paint::solid(Color::rgb(0x11, 0x11, 0x11))],
                ..Default::default()
            },
        );
        title.name = "Title".into();
        d.insert(card_id, title).unwrap();

        let body_id = d.alloc_id();
        let mut body = Node::text(
            body_id,
            Rect::new(0.0, 0.0, 280.0, 20.0),
            "Body copy",
            TextStyle {
                font_family: "Inter".into(),
                font_size: 14.0,
                line_height: LineHeight::Pixels(20.0),
                align_h: TextAlignH::Center,
                fills: vec![Paint::solid(Color::rgb(0x66, 0x66, 0x66))],
                ..Default::default()
            },
        );
        body.name = "Body".into();
        body.layout_child.horizontal = Sizing::Fill;
        body.variables.bindings.insert("fills/0".into(), "muted".into());
        if let NodeKind::Text(t) = &mut body.kind {
            t.auto_resize = TextAutoResize::Height;
        }
        d.insert(card_id, body).unwrap();

        let row_id = d.alloc_id();
        let mut row = Node::frame(row_id, Rect::new(0.0, 0.0, 280.0, 48.0));
        row.name = "Actions".into();
        row.fills = vec![Paint::solid(Color::rgb(0x0D, 0x99, 0xFF))];
        row.corner_radii = [4.0; 4];
        row.layout_child.horizontal = Sizing::Fill;
        row.variables.bindings.insert("fills/0".into(), "primary".into());
        row.variables.bindings.insert("itemSpacing".into(), "md".into());
        if let NodeKind::Frame(f) = &mut row.kind {
            f.auto_layout = Some(AutoLayout {
                direction: LayoutDirection::Horizontal,
                item_spacing: 12.0,
                padding: [8.0, 12.0, 8.0, 12.0],
                primary_align: LayoutAlign::Center,
                counter_align: LayoutAlign::Center,
                primary_sizing: Sizing::Fixed,
                counter_sizing: Sizing::Hug,
                ..Default::default()
            });
        }
        let row_id = d.insert(card_id, row).unwrap();

        let inst_id = d.alloc_id();
        let mut inst = Node::new(inst_id, NodeKind::Instance { frame: FrameProps::default(), component: Some(comp_id), component_key: None });
        inst.name = "Button".into();
        inst.size = Vec2::new(80.0, 32.0);
        inst.component.values.insert("Label".into(), PropValue::Text("Save".into()));
        inst.component.values.insert("Disabled".into(), PropValue::Bool(true));
        d.insert(row_id, inst).unwrap();

        let dot_id = d.alloc_id();
        let mut dot = Node::ellipse(dot_id, Rect::new(0.0, 0.0, 24.0, 24.0));
        dot.fills = vec![Paint::solid(Color::rgb(0xFF, 0x00, 0x00))];
        dot.name = "Dot".into();
        d.insert(row_id, dot).unwrap();
        (d, card_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifier_helpers() {
        assert_eq!(pascal("Primary button/large"), "PrimaryButtonLarge");
        assert_eq!(camel("Color/Brand Primary"), "colorBrandPrimary");
        assert_eq!(kebab("primaryDark Blue"), "primary-dark-blue");
        assert_eq!(pascal("123 go"), "V123Go");
        assert_eq!(snake("Chevron Down"), "chevron_down");
    }

    #[test]
    fn css_uses_variable_references() {
        let (d, card) = fixture::card();
        let row = d.get(card).unwrap().children[2];
        let css = crate::css_for(&d, row);
        assert!(css.contains("background: var(--color-primary);"), "{css}");
        assert!(css.contains("gap: calc(var(--spacing-md) * 1px);"), "{css}");
        let body = d.get(card).unwrap().children[1];
        assert!(crate::css_for(&d, body).contains("color: var(--color-text-muted);"));
        // unbound properties keep their literal values
        assert!(crate::css_for(&d, card).contains("background: #FFF;"));
    }

    #[test]
    fn icons_become_asset_references_in_every_language() {
        let (mut d, card) = fixture::card();
        let id = d.alloc_id();
        let mut v = Node::new(id, NodeKind::Vector { paths: vec![], network: None });
        v.name = "Chevron Down".into();
        v.size = Vec2::new(16.0, 16.0);
        d.insert(card, v).unwrap();
        assert!(crate::tailwind_for(&d, card).contains(r#"<img src="/assets/chevron-down.svg" alt="Chevron Down" width="16" height="16""#));
        assert!(crate::swiftui_for(&d, card).contains(
            "Image(\"chevron-down\")
        .resizable()"
        ));
        assert!(crate::compose_for(&d, card).contains("painter = painterResource(R.drawable.chevron_down)"));
        assert!(crate::react_for(&d, card).contains("<img"));
        assert_eq!(vector_assets(&d, card), vec![id]);
    }

    #[test]
    fn variable_names() {
        let (d, _) = fixture::card();
        let v = &d.variables["primary"];
        assert_eq!(css_var(v), "var(--color-primary)");
        assert_eq!(token_camel(&d.variables["muted"], true), "textMuted");
        assert_eq!(css_var(&d.variables["md"]), "var(--spacing-md)");
    }
}
