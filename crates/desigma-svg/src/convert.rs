//! usvg tree -> desigma nodes.

use crate::effects::effects_of;
use crate::paint::{self, affine, Frame};
use crate::scan::{RawText, Shape, GENERATED_ID, KEPT_GROUP_ID};
use crate::SvgNodes;
use desigma_doc::*;
use std::collections::{BTreeMap, HashMap};
use usvg::tiny_skia_path::PathSegment;

/// What a chain of folded-away ancestors contributes to the next real node.
#[derive(Clone, Debug)]
pub(crate) struct Attrs {
    /// Transform from the nearest real parent's space.
    pub pre: Affine,
    pub opacity: f32,
    pub blend: Option<BlendMode>,
    pub effects: Vec<Effect>,
}

impl Attrs {
    pub fn with_pre(pre: Affine) -> Self {
        Self { pre, opacity: 1.0, blend: None, effects: Vec::new() }
    }

    fn plain(&self) -> bool {
        self.opacity >= 1.0 && self.blend.is_none() && self.effects.is_empty()
    }

    /// `self` is the outer chain, the arguments the next group down. `None`
    /// when both carry appearance (two groups' effects can't share a node).
    fn compose(&self, local: Affine, opacity: f32, blend: Option<BlendMode>, effects: Vec<Effect>) -> Option<Attrs> {
        let inner_plain = opacity >= 1.0 && blend.is_none() && effects.is_empty();
        if !self.plain() && !inner_plain {
            return None;
        }
        let mut all = self.effects.clone();
        all.extend(effects);
        Some(Attrs { pre: self.pre * local, opacity: self.opacity * opacity, blend: self.blend.or(blend), effects: all })
    }
}

pub(crate) struct Conv<'a> {
    pub alloc: &'a mut dyn FnMut() -> NodeId,
    pub nodes: Vec<Node>,
    pub images: BTreeMap<String, Vec<u8>>,
    shapes: HashMap<String, Shape>,
    /// `<text>` elements usvg could not shape (no glyph coverage); emitted verbatim.
    texts: Vec<RawText>,
    /// Converting a `clipPath`: only fill geometry matters, painted black.
    clip_mode: bool,
    /// Converting a luminance `<mask>`: paints become their luminance as alpha.
    luma: bool,
}

pub(crate) fn convert_tree(tree: &usvg::Tree, name: &str, shapes: HashMap<String, Shape>, texts: Vec<RawText>, alloc: &mut dyn FnMut() -> NodeId) -> SvgNodes {
    let mut c = Conv { alloc, nodes: Vec::new(), images: BTreeMap::new(), shapes, texts, clip_mode: false, luma: false };
    let size = tree.size();
    let mut root = c.new_node(NodeKind::Frame(FrameProps { clips_content: true, ..Default::default() }));
    root.name = if name.is_empty() { "svg".into() } else { name.to_string() };
    root.size = Vec2::new(size.width(), size.height());
    root.fills = Vec::new();
    c.nodes.push(root);
    let g = tree.root();
    let kids_only =
        g.clip_path().is_none() && g.mask().is_none() && g.filters().is_empty() && g.opacity().get() >= 1.0 && g.blend_mode() == usvg::BlendMode::Normal;
    if kids_only {
        let a = Attrs::with_pre(affine(g.transform()));
        c.convert_list(g.children(), 0, &a);
    } else {
        c.convert_group(g, 0, Attrs::with_pre(Affine::IDENTITY));
    }
    c.emit_unshaped_texts();
    SvgNodes { nodes: c.nodes, images: c.images }
}

pub(crate) fn layer_name(id: &str) -> Option<String> {
    (!id.is_empty() && !id.starts_with(GENERATED_ID)).then(|| id.to_string())
}

fn blend_of(b: usvg::BlendMode) -> Option<BlendMode> {
    use usvg::BlendMode as U;
    Some(match b {
        U::Normal => return None,
        U::Multiply => BlendMode::Multiply,
        U::Screen => BlendMode::Screen,
        U::Overlay => BlendMode::Overlay,
        U::Darken => BlendMode::Darken,
        U::Lighten => BlendMode::Lighten,
        U::ColorDodge => BlendMode::ColorDodge,
        U::ColorBurn => BlendMode::ColorBurn,
        U::HardLight => BlendMode::HardLight,
        U::SoftLight => BlendMode::SoftLight,
        U::Difference => BlendMode::Difference,
        U::Exclusion => BlendMode::Exclusion,
        U::Hue => BlendMode::Hue,
        U::Saturation => BlendMode::Saturation,
        U::Color => BlendMode::Color,
        U::Luminosity => BlendMode::Luminosity,
    })
}

/// True when the linear part of `t` keeps right angles (rotation + scale).
fn no_skew(t: &Affine) -> bool {
    let dot = t.a * t.c + t.b * t.d;
    let scale = ((t.a * t.a + t.b * t.b) * (t.c * t.c + t.d * t.d)).sqrt();
    dot.abs() <= 1e-3 * scale.max(1e-9)
}

fn fnv(bytes: &[u8]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// A rectangle/circle `clipPath` that can become a clipping Frame.
struct RectClip {
    /// Frame space -> clipped group's space.
    local: Affine,
    size: Vec2,
    radius: f32,
}

impl Conv<'_> {
    pub fn new_node(&mut self, kind: NodeKind) -> Node {
        Node::new((self.alloc)(), kind)
    }

    /// Appends `node` as the last child of `nodes[parent]`.
    pub fn push(&mut self, mut node: Node, parent: usize) -> usize {
        node.parent = Some(self.nodes[parent].id);
        let id = node.id;
        self.nodes[parent].children.push(id);
        self.nodes.push(node);
        self.nodes.len() - 1
    }

    /// Converts sibling nodes, merging runs of paths with identical geometry.
    pub fn convert_list(&mut self, kids: &[usvg::Node], parent: usize, a: &Attrs) {
        for it in items(kids) {
            self.convert_item(&it, parent, a.clone());
        }
    }

    fn convert_item(&mut self, it: &Item, parent: usize, a: Attrs) {
        match it {
            Item::One(n) => self.convert_node(n, parent, a),
            Item::Stack(paths) => self.convert_path(paths, parent, &a),
        }
    }

    pub fn convert_node(&mut self, n: &usvg::Node, parent: usize, a: Attrs) {
        match n {
            usvg::Node::Group(g) => self.convert_group(g, parent, a),
            usvg::Node::Path(p) => self.convert_path(&[p], parent, &a),
            usvg::Node::Image(i) => self.convert_image(i, parent, &a),
            usvg::Node::Text(t) => crate::text::convert_text(self, t, parent, a),
        }
    }

    /// A Group node carrying only `a` (used when two groups' appearance can't merge).
    pub fn group_from(&mut self, a: &Attrs, parent: usize) -> usize {
        let mut node = self.new_node(NodeKind::Group);
        self.apply_attrs(&mut node, a);
        self.push(node, parent)
    }

    pub fn apply_attrs(&self, node: &mut Node, a: &Attrs) {
        node.transform = a.pre;
        node.opacity = a.opacity;
        if let Some(b) = a.blend {
            node.blend = b;
        }
        node.effects = a.effects.clone();
    }

    pub fn convert_group(&mut self, g: &usvg::Group, parent: usize, a: Attrs) {
        let kids = g.children();
        if kids.is_empty() {
            return;
        }
        let local = affine(g.transform());
        let effects = effects_of(g.filters());
        let blend = blend_of(g.blend_mode());
        let opacity = g.opacity().get();
        let (clip, mask) = (g.clip_path(), g.mask());
        let merged = a.compose(local, opacity, blend, effects.clone());

        // Fold appearance-free wrappers into a single child.
        if let (Some(m), None, None) = (&merged, clip, mask) {
            let its = items(kids);
            if let [only] = &its[..] {
                return self.convert_item(only, parent, m.clone());
            }
            // An unnamed, untransformed, plain group (what `export_svg` writes per
            // layer wrapper) adds nothing: its children join the parent.
            if m.plain() && layer_name(g.id()).is_none() && !g.id().starts_with(KEPT_GROUP_ID) && local == Affine::IDENTITY {
                return self.convert_list(kids, parent, &Attrs::with_pre(m.pre));
            }
        }

        let (parent, a) = match merged {
            Some(m) => (parent, m),
            None => {
                let p = self.group_from(&a, parent);
                (p, Attrs::with_pre(Affine::IDENTITY).compose(local, opacity, blend, effects).expect("identity chain is plain"))
            }
        };

        let frame = clip.and_then(|c| self.rect_clip(c));
        // A clip that repeats the parent frame's own clip is redundant.
        if let (Some(f), None) = (&frame, mask) {
            let p = &self.nodes[parent];
            let same = matches!(&p.kind, NodeKind::Frame(fp) if fp.clips_content)
                && a.plain()
                && a.pre == Affine::IDENTITY
                && f.local == Affine::IDENTITY
                && (p.size.x - f.size.x).abs() < 1e-3
                && (p.size.y - f.size.y).abs() < 1e-3
                && (p.corner_radii[0] - f.radius).abs() < 1e-3;
            if same {
                return self.convert_list(kids, parent, &Attrs::with_pre(Affine::IDENTITY));
            }
        }
        let mut node = match &frame {
            Some(f) => {
                let mut n = self.new_node(NodeKind::Frame(FrameProps { clips_content: true, ..Default::default() }));
                n.size = f.size;
                n.corner_radii = [f.radius; 4];
                n.fills = Vec::new();
                n
            }
            None => self.new_node(NodeKind::Group),
        };
        self.apply_attrs(&mut node, &a);
        if let Some(f) = &frame {
            node.transform = a.pre * f.local;
        }
        if let Some(name) = layer_name(g.id()) {
            node.name = name;
        }
        // Effects see the clipped/masked result in SVG order (filter first), so
        // with a clip or mask they live on an inner wrapper of the content.
        let wrap = !a.effects.is_empty() && (clip.is_some() || mask.is_some());
        if wrap {
            node.effects.clear();
        }
        let cid = self.push(node, parent);
        let base = frame.as_ref().and_then(|f| f.local.invert()).unwrap_or(Affine::IDENTITY);

        if let (Some(c), None) = (clip, &frame) {
            self.add_clip_mask(c, cid, base);
        }
        if let Some(m) = mask {
            self.add_mask(m, cid, base);
        }
        let content = if wrap {
            let mut w = self.new_node(NodeKind::Group);
            w.name = "Filter".into();
            w.effects = a.effects.clone();
            self.push(w, cid)
        } else {
            cid
        };
        self.convert_list(kids, content, &Attrs::with_pre(base));
    }

    // ---- clip paths and masks ----------------------------------------------

    fn rect_clip(&self, c: &usvg::ClipPath) -> Option<RectClip> {
        if c.clip_path().is_some() {
            return None;
        }
        // Optionally one level of transform-only groups, then a single path.
        let mut g = c.root();
        let mut ts = affine(c.transform()) * affine(g.transform());
        let path = loop {
            match g.children() {
                [usvg::Node::Path(p)] => break p,
                [usvg::Node::Group(inner)] if inner.clip_path().is_none() && inner.mask().is_none() => {
                    g = inner;
                    ts = ts * affine(g.transform());
                }
                _ => return None,
            }
        };
        if !path.is_visible() || !no_skew(&ts) {
            return None;
        }
        let bb = path.bounding_box();
        let (w, h) = (bb.width(), bb.height());
        if !(w > 0.0 && h > 0.0) {
            return None;
        }
        let radius = match *self.shapes.get(path.id())? {
            Shape::Rect { rx: Some(rx), ry: Some(ry) } if (rx - ry).abs() < 1e-3 => rx.min(w / 2.0).min(h / 2.0),
            Shape::Ellipse if (w - h).abs() < 1e-3 * w.max(h) => w / 2.0,
            _ => return None,
        };
        let local = ts * Affine::translate(bb.x(), bb.y());
        local.invert()?;
        Some(RectClip { local, size: Vec2::new(w, h), radius })
    }

    fn add_clip_mask(&mut self, c: &usvg::ClipPath, container: usize, base: Affine) {
        let mut m = self.new_node(NodeKind::Group);
        m.name = "Clip".into();
        m.is_mask = true;
        m.transform = base * affine(c.transform());
        let mid = self.push(m, container);
        let saved = std::mem::replace(&mut self.clip_mode, true);
        self.convert_group_children(c.root(), mid);
        self.clip_mode = saved;
    }

    fn add_mask(&mut self, mask: &usvg::Mask, container: usize, base: Affine) {
        let mut m = self.new_node(NodeKind::Group);
        m.name = "Mask".into();
        m.is_mask = true;
        m.transform = base;
        let mid = self.push(m, container);
        let saved_luma = std::mem::replace(&mut self.luma, mask.kind() == usvg::MaskType::Luminance);
        self.convert_group_children(mask.root(), mid);
        self.luma = saved_luma;
    }

    /// Converts the children of a clip/mask root group under `parent`.
    fn convert_group_children(&mut self, g: &usvg::Group, parent: usize) {
        let a = Attrs::with_pre(affine(g.transform()));
        self.convert_list(g.children(), parent, &a);
    }

    // ---- paths ----------------------------------------------------------------

    fn paint_of(&mut self, p: &usvg::Paint, opacity: f32, fr: &Frame) -> Option<Paint> {
        let kind = match p {
            usvg::Paint::Color(c) => PaintKind::Solid(paint::color(*c, 1.0)),
            usvg::Paint::LinearGradient(g) => paint::linear(g, fr),
            usvg::Paint::RadialGradient(g) => paint::radial(g, fr),
            usvg::Paint::Pattern(pt) => match self.pattern_image_fill(pt, fr) {
                Some(k) => k,
                None => {
                    let (c, a) = paint::pattern_color(pt.root())?;
                    return Some(self.finish_paint(PaintKind::Solid(paint::color(c, 1.0)), opacity * a));
                }
            },
        };
        Some(self.finish_paint(kind, opacity))
    }

    fn finish_paint(&self, kind: PaintKind, opacity: f32) -> Paint {
        let p = Paint { kind, opacity, visible: true, blend: BlendMode::Normal };
        if self.luma {
            luma_paint(p)
        } else {
            p
        }
    }

    /// A pattern that tiles exactly one image over the whole shape is an image fill.
    fn pattern_image_fill(&mut self, pt: &usvg::Pattern, fr: &Frame) -> Option<PaintKind> {
        let pi = paint::pattern_image(pt)?;
        let t = pt.transform();
        if (t.sx - 1.0).abs() > 1e-3 || (t.sy - 1.0).abs() > 1e-3 || t.kx.abs() > 1e-3 || t.ky.abs() > 1e-3 || t.tx.abs() > 0.5 || t.ty.abs() > 0.5 {
            return None;
        }
        let tile = pt.rect();
        let near = |a: f32, b: f32| (a - b).abs() <= 0.5 + 0.01 * b.abs();
        if !(near(tile.x(), fr.origin.0) && near(tile.y(), fr.origin.1) && near(tile.width(), fr.size.0) && near(tile.height(), fr.size.1)) {
            return None;
        }
        let ts = pi.transform;
        if ts.kx.abs() > 1e-3 || ts.ky.abs() > 1e-3 {
            return None;
        }
        let key = self.store_image(pi.image.kind())?;
        let size = pi.image.size();
        let (iw, ih) = (size.width() * ts.sx.abs(), size.height() * ts.sy.abs());
        let covers = iw >= tile.width() - 0.5 && ih >= tile.height() - 0.5;
        Some(PaintKind::Image { image_ref: key, scale_mode: if covers { ImageScaleMode::Fill } else { ImageScaleMode::Fit } })
    }

    fn store_image(&mut self, kind: &usvg::ImageKind) -> Option<String> {
        let bytes: Vec<u8> = match kind {
            usvg::ImageKind::JPEG(d) | usvg::ImageKind::PNG(d) => d.to_vec(),
            // The renderer decodes PNG/JPEG only.
            usvg::ImageKind::GIF(d) | usvg::ImageKind::WEBP(d) => {
                let img = image::load_from_memory(d).ok()?;
                let mut out = std::io::Cursor::new(Vec::new());
                img.write_to(&mut out, image::ImageFormat::Png).ok()?;
                out.into_inner()
            }
            usvg::ImageKind::SVG(_) => return None,
        };
        let key = format!("svg-{:016x}", fnv(&bytes));
        self.images.insert(key.clone(), bytes);
        Some(key)
    }

    fn stroke_of(&mut self, s: &usvg::Stroke, fr: &Frame) -> Stroke {
        let paints = self.paint_of(s.paint(), s.opacity().get(), fr).into_iter().collect();
        Stroke {
            paints,
            weight: s.width().get(),
            side_weights: None,
            align: StrokeAlign::Center,
            cap: match s.linecap() {
                usvg::LineCap::Butt => StrokeCap::None,
                usvg::LineCap::Round => StrokeCap::Round,
                usvg::LineCap::Square => StrokeCap::Square,
            },
            join: match s.linejoin() {
                usvg::LineJoin::Miter | usvg::LineJoin::MiterClip => StrokeJoin::Miter,
                usvg::LineJoin::Round => StrokeJoin::Round,
                usvg::LineJoin::Bevel => StrokeJoin::Bevel,
            },
            miter_limit: s.miterlimit().get(),
            dashes: s.dasharray().map(|d| d.to_vec()).unwrap_or_default(),
        }
    }

    /// One node for a stack of paths with identical geometry (fills first, then
    /// strokes), the way `export_svg` writes a node's paints as separate elements.
    fn convert_path(&mut self, stack: &[&usvg::Path], parent: usize, a: &Attrs) {
        let p = stack[0];
        if !p.is_visible() {
            return;
        }
        let bb = p.bounding_box();
        let fr = Frame::new(bb.x(), bb.y(), bb.width(), bb.height());
        let rule = match stack.iter().find_map(|q| q.fill()).map(|f| f.rule()) {
            Some(usvg::FillRule::EvenOdd) => FillRule::EvenOdd,
            _ => FillRule::NonZero,
        };

        let kind = match self.shapes.get(p.id()).copied() {
            Some(Shape::Rect { rx: Some(rx), ry: Some(ry) }) if (rx - ry).abs() < 1e-3 && no_skew(&a.pre) && bb.width() > 0.0 && bb.height() > 0.0 => {
                Some((NodeKind::Rectangle, rx.min(bb.width() / 2.0).min(bb.height() / 2.0)))
            }
            Some(Shape::Ellipse) if no_skew(&a.pre) && bb.width() > 0.0 && bb.height() > 0.0 => Some((NodeKind::Ellipse(ArcData::default()), 0.0)),
            _ => None,
        };
        let (kind, radius) = kind.unwrap_or_else(|| {
            let cmds = path_cmds(p.data(), fr.origin);
            (NodeKind::Vector { paths: vec![Path { cmds, fill_rule: rule }], network: None }, 0.0)
        });

        let mut node = self.new_node(kind);
        self.apply_attrs(&mut node, a);
        node.transform = a.pre * Affine::translate(fr.origin.0, fr.origin.1);
        node.size = Vec2::new(fr.size.0, fr.size.1);
        node.corner_radii = [radius; 4];
        if let Some(name) = stack.iter().find_map(|q| layer_name(q.id())) {
            node.name = name;
        }
        if self.clip_mode {
            node.fills = vec![Paint::solid(Color::BLACK)];
        } else {
            for q in stack {
                if let Some(f) = q.fill() {
                    node.fills.extend(self.paint_of(f.paint(), f.opacity().get(), &fr));
                }
                if let Some(s) = q.stroke() {
                    if node.stroke.paints.is_empty() {
                        node.stroke = self.stroke_of(s, &fr);
                    } else {
                        node.stroke.paints.extend(self.paint_of(s.paint(), s.opacity().get(), &fr));
                    }
                }
            }
        }
        self.push(node, parent);
    }

    fn convert_image(&mut self, img: &usvg::Image, parent: usize, a: &Attrs) {
        if !img.is_visible() {
            return;
        }
        let Some(key) = self.store_image(img.kind()) else { return };
        let size = img.size();
        let mut node = self.new_node(NodeKind::Rectangle);
        self.apply_attrs(&mut node, a);
        node.size = Vec2::new(size.width(), size.height());
        node.name = layer_name(img.id()).unwrap_or_else(|| "Image".into());
        node.fills =
            vec![Paint { kind: PaintKind::Image { image_ref: key, scale_mode: ImageScaleMode::Fill }, opacity: 1.0, visible: true, blend: BlendMode::Normal }];
        self.push(node, parent);
    }

    /// Paints for text fills, in the text node's box.
    pub fn text_paints(&mut self, fill: Option<&usvg::Fill>, fr: &Frame) -> Vec<Paint> {
        match fill {
            Some(f) => self.paint_of(f.paint(), f.opacity().get(), fr).into_iter().collect(),
            None => Vec::new(),
        }
    }
}

fn path_cmds(p: &usvg::tiny_skia_path::Path, origin: (f32, f32)) -> Vec<PathCmd> {
    let v = |pt: usvg::tiny_skia_path::Point| Vec2::new(pt.x - origin.0, pt.y - origin.1);
    p.segments()
        .map(|s| match s {
            PathSegment::MoveTo(a) => PathCmd::MoveTo(v(a)),
            PathSegment::LineTo(a) => PathCmd::LineTo(v(a)),
            PathSegment::QuadTo(a, b) => PathCmd::QuadTo(v(a), v(b)),
            PathSegment::CubicTo(a, b, c) => PathCmd::CubicTo(v(a), v(b), v(c)),
            PathSegment::Close => PathCmd::Close,
        })
        .collect()
}

/// A luminance mask reads brightness; desigma masks read alpha. Painting the
/// same shape black with alpha = luminance x alpha is equivalent.
fn luma_paint(mut p: Paint) -> Paint {
    let conv = |c: Color| Color { r: 0.0, g: 0.0, b: 0.0, a: paint::luminance(&c) * c.a };
    match &mut p.kind {
        PaintKind::Solid(c) => *c = conv(*c),
        PaintKind::LinearGradient { stops, .. }
        | PaintKind::RadialGradient { stops, .. }
        | PaintKind::AngularGradient { stops, .. }
        | PaintKind::DiamondGradient { stops, .. } => {
            for s in stops {
                s.color = conv(s.color);
            }
        }
        PaintKind::Image { .. } => {}
    }
    p
}

/// A sibling to convert: any node, or a stack of path elements that draw the
/// same outline and so form one desigma node.
enum Item<'a> {
    One(&'a usvg::Node),
    Stack(Vec<&'a usvg::Path>),
}

fn same_stroke(a: &usvg::Stroke, b: &usvg::Stroke) -> bool {
    a.width().get() == b.width().get()
        && a.linecap() == b.linecap()
        && a.linejoin() == b.linejoin()
        && a.miterlimit().get() == b.miterlimit().get()
        && a.dasharray() == b.dasharray()
}

/// Whether `q` can join the stack started by `first`: same outline, and
/// paints in fill-then-stroke order with one stroke style.
fn joins(first: &usvg::Path, stack: &[&usvg::Path], q: &usvg::Path) -> bool {
    if !q.is_visible() || (!q.id().is_empty() && !q.id().starts_with(GENERATED_ID) && q.id() != first.id()) {
        return false;
    }
    if path_cmds(q.data(), (0.0, 0.0)) != path_cmds(first.data(), (0.0, 0.0)) {
        return false;
    }
    let stroked = stack.iter().find_map(|p| p.stroke());
    match (q.fill().is_some(), q.stroke()) {
        // another fill: only before any stroke
        (true, None) => stroked.is_none(),
        // another stroke paint with the same style
        (false, Some(s)) => stroked.is_none_or(|t| same_stroke(t, s)),
        _ => false,
    }
}

fn items(kids: &[usvg::Node]) -> Vec<Item<'_>> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < kids.len() {
        if let usvg::Node::Path(p) = &kids[i] {
            let mut stack: Vec<&usvg::Path> = vec![p];
            while let Some(usvg::Node::Path(q)) = kids.get(i + stack.len()) {
                if !joins(p, &stack, q) {
                    break;
                }
                stack.push(q);
            }
            i += stack.len();
            out.push(Item::Stack(stack));
        } else {
            out.push(Item::One(&kids[i]));
            i += 1;
        }
    }
    out
}

impl Conv<'_> {
    /// Emit a Text node for every `<text>` whose glyphs usvg could not shape.
    ///
    /// usvg is built with the bundled Inter face only, so any script outside its
    /// coverage lays out zero glyphs and `convert_text` falls back to flattened
    /// outlines — which are empty. Before this pass such text vanished from the
    /// document (no node, no pixels). Matching by content leaves text usvg did
    /// shape untouched, so nothing is emitted twice.
    fn emit_unshaped_texts(&mut self) {
        let mut shaped: HashMap<String, usize> = HashMap::new();
        for n in &self.nodes {
            if let NodeKind::Text(t) = &n.kind {
                *shaped.entry(t.content.trim().to_string()).or_default() += 1;
            }
        }
        let raws = std::mem::take(&mut self.texts);
        for raw in raws {
            let key = raw.content.trim().to_string();
            match shaped.get_mut(&key) {
                Some(count) if *count > 0 => *count -= 1,
                _ => crate::text::emit_raw(self, &raw, 0),
            }
        }
    }
}
