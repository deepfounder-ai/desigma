//! Structure of imported documents.

use desigma_doc::*;
use desigma_svg::{import_svg, import_svg_named, import_svg_named_with, looks_like_svg, svg_to_nodes, ImportOptions};

fn import(svg: &str) -> (Document, NodeId) {
    let mut doc = Document::new("t");
    let page = doc.add_page("Page 1");
    let id = import_svg(svg, &mut doc, page, Vec2::new(100.0, 50.0)).expect("import");
    (doc, id)
}

fn kids(doc: &Document, id: NodeId) -> Vec<&Node> {
    doc.get(id).unwrap().children.iter().map(|c| doc.get(*c).unwrap()).collect()
}

fn dump(doc: &Document, id: NodeId) -> String {
    let mut s = String::new();
    doc.walk(id, |n, depth| {
        let k = match &n.kind {
            NodeKind::Frame(_) => "Frame",
            NodeKind::Group => "Group",
            NodeKind::Rectangle => "Rectangle",
            NodeKind::Ellipse(_) => "Ellipse",
            NodeKind::Vector { .. } => "Vector",
            NodeKind::Text(_) => "Text",
            _ => "Other",
        };
        s.push_str(&format!(
            "{}{k}{} \"{}\" at ({:.1},{:.1}) {:.1}x{:.1}\n",
            "  ".repeat(depth),
            if n.is_mask { " (mask)" } else { "" },
            n.name,
            n.transform.tx,
            n.transform.ty,
            n.size.x,
            n.size.y
        ));
    });
    s
}

#[test]
fn root_is_a_clipping_frame_sized_to_the_svg_and_placed_at() {
    let (doc, id) =
        import(r##"<svg xmlns="http://www.w3.org/2000/svg" width="48" height="32" viewBox="0 0 48 32"><rect width="10" height="10" fill="#f00"/></svg>"##);
    let root = doc.get(id).unwrap();
    assert_eq!(root.name, "svg");
    assert_eq!((root.size.x, root.size.y), (48.0, 32.0));
    assert_eq!((root.transform.tx, root.transform.ty), (100.0, 50.0));
    assert!(root.fills.is_empty());
    assert!(matches!(&root.kind, NodeKind::Frame(f) if f.clips_content));
    assert_eq!(doc.get(root.parent.unwrap()).unwrap().children, vec![id]);
}

#[test]
fn viewbox_scaling_is_baked_into_the_content() {
    let (doc, id) = import(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="48" height="48" viewBox="0 0 24 24"><rect x="2" y="4" width="6" height="8" fill="#00f"/></svg>"##,
    );
    let r = kids(&doc, id)[0];
    assert!(matches!(r.kind, NodeKind::Rectangle));
    // the viewBox scale (2x) lives in the rectangle's transform
    assert_eq!((r.transform.a, r.transform.d), (2.0, 2.0));
    assert_eq!((r.transform.tx, r.transform.ty), (4.0, 8.0));
    assert_eq!((r.size.x, r.size.y), (6.0, 8.0));
}

#[test]
fn shapes_become_rectangles_and_ellipses_with_radius() {
    let (doc, id) = import(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100">
          <rect id="card" x="10" y="20" width="40" height="30" rx="6" fill="#ff0000" stroke="#000" stroke-width="2"/>
          <circle cx="70" cy="30" r="10" fill="#00ff00"/>
          <ellipse cx="30" cy="80" rx="20" ry="8" fill="#0000ff"/>
          <rect x="60" y="60" width="20" height="20" rx="4" ry="2" fill="#123456"/>
          <path d="M0 0 L10 0 L10 10 Z" fill="#333"/>
        </svg>"##,
    );
    let k = kids(&doc, id);
    assert_eq!(k.len(), 5, "{}", dump(&doc, id));
    assert!(matches!(k[0].kind, NodeKind::Rectangle));
    assert_eq!(k[0].name, "card");
    assert_eq!(k[0].corner_radii, [6.0; 4]);
    assert_eq!((k[0].transform.tx, k[0].transform.ty, k[0].size.x, k[0].size.y), (10.0, 20.0, 40.0, 30.0));
    assert_eq!(k[0].stroke.weight, 2.0);
    assert_eq!(k[0].stroke.align, StrokeAlign::Center);
    assert!(matches!(k[1].kind, NodeKind::Ellipse(_)));
    assert_eq!((k[1].transform.tx, k[1].transform.ty, k[1].size.x), (60.0, 20.0, 20.0));
    assert!(matches!(k[2].kind, NodeKind::Ellipse(_)));
    assert_eq!((k[2].size.x, k[2].size.y), (40.0, 16.0));
    // elliptical corners can't be a Rectangle
    assert!(matches!(k[3].kind, NodeKind::Vector { .. }));
    let NodeKind::Vector { paths, .. } = &k[4].kind else { panic!("vector expected") };
    assert!(matches!(paths[0].cmds.last(), Some(PathCmd::Close)));
}

#[test]
fn skewed_rect_stays_a_vector_but_rotation_keeps_the_rectangle() {
    let (doc, id) = import(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100">
          <rect width="20" height="10" transform="rotate(30)" fill="red"/>
          <rect width="20" height="10" transform="skewX(20)" fill="red"/>
        </svg>"##,
    );
    let k = kids(&doc, id);
    assert!(matches!(k[0].kind, NodeKind::Rectangle));
    assert!((k[0].transform.rotation_rad() - 30f32.to_radians()).abs() < 1e-4);
    assert!(matches!(k[1].kind, NodeKind::Vector { .. }));
}

#[test]
fn stroke_properties_map_over() {
    let (doc, id) = import(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M5 12h14" stroke-dasharray="3 2" stroke-miterlimit="7"/><path d="M12 5l7 7-7 7" stroke-linecap="square" stroke-linejoin="bevel"/></svg>"##,
    );
    let k = kids(&doc, id);
    let a = &k[0];
    assert!(a.fills.is_empty());
    assert_eq!(a.stroke.weight, 2.0);
    assert_eq!((a.stroke.cap, a.stroke.join), (StrokeCap::Round, StrokeJoin::Round));
    assert_eq!(a.stroke.dashes, vec![3.0, 2.0]);
    assert_eq!(a.stroke.miter_limit, 7.0);
    assert!(matches!(a.stroke.paints[0].kind, PaintKind::Solid(c) if c.r == 0.0 && c.a == 1.0));
    let b = &k[1];
    assert_eq!((b.stroke.cap, b.stroke.join), (StrokeCap::Square, StrokeJoin::Bevel));
    let NodeKind::Vector { paths, .. } = &b.kind else { panic!() };
    assert!(paths[0].cmds.iter().any(|c| matches!(c, PathCmd::LineTo(_))));
}

#[test]
fn curves_and_even_odd_survive() {
    let (doc, id) = import(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="40"><path fill-rule="evenodd" fill="#000" d="M0 0 C10 0 10 10 0 10 Q-5 5 0 0 Z M2 2 L4 2 L4 4 Z"/></svg>"##,
    );
    let n = kids(&doc, id)[0];
    let NodeKind::Vector { paths, .. } = &n.kind else { panic!() };
    assert_eq!(paths[0].fill_rule, FillRule::EvenOdd);
    assert!(paths[0].cmds.iter().any(|c| matches!(c, PathCmd::CubicTo(..))));
    assert!(paths[0].cmds.iter().any(|c| matches!(c, PathCmd::QuadTo(..))));
    assert_eq!(paths[0].cmds.iter().filter(|c| matches!(c, PathCmd::MoveTo(_))).count(), 2);
}

#[test]
fn groups_keep_transform_and_opacity_and_single_wrappers_fold() {
    let (doc, id) = import(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100">
          <g id="pair" transform="translate(10 20)" opacity="0.5"><rect width="5" height="5" fill="red"/><rect x="10" width="5" height="5" fill="blue"/></g>
          <g transform="translate(30 40)"><rect id="solo" width="5" height="5" fill="red"/></g>
        </svg>"##,
    );
    let k = kids(&doc, id);
    assert_eq!(k.len(), 2, "{}", dump(&doc, id));
    assert!(matches!(k[0].kind, NodeKind::Group));
    assert_eq!(k[0].name, "pair");
    assert_eq!(k[0].opacity, 0.5);
    assert_eq!((k[0].transform.tx, k[0].transform.ty), (10.0, 20.0));
    assert_eq!(k[0].children.len(), 2);
    assert!(matches!(k[1].kind, NodeKind::Rectangle), "single child folded: {}", dump(&doc, id));
    assert_eq!((k[1].transform.tx, k[1].transform.ty), (30.0, 40.0));
}

#[test]
fn gradients_become_node_normalized_handles() {
    let (doc, id) = import(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100">
          <defs>
            <linearGradient id="l" x1="0" y1="0" x2="1" y2="0"><stop offset="0" stop-color="#ff0000"/><stop offset="1" stop-color="#0000ff" stop-opacity="0.5"/></linearGradient>
            <radialGradient id="r" cx="0.5" cy="0.5" r="0.5"><stop offset="0" stop-color="#ffffff"/><stop offset="1" stop-color="#000000"/></radialGradient>
            <linearGradient id="u" gradientUnits="userSpaceOnUse" x1="50" y1="10" x2="50" y2="60"><stop offset="0" stop-color="#00ff00"/><stop offset="1" stop-color="#000000"/></linearGradient>
          </defs>
          <rect x="10" y="10" width="40" height="20" fill="url(#l)"/>
          <rect x="10" y="40" width="40" height="40" fill="url(#r)"/>
          <rect x="60" y="10" width="30" height="50" fill="url(#u)" fill-opacity="0.8"/>
        </svg>"##,
    );
    let k = kids(&doc, id);
    let near = |a: Vec2, x: f32, y: f32| (a.x - x).abs() < 1e-4 && (a.y - y).abs() < 1e-4;
    let PaintKind::LinearGradient { handles, stops } = &k[0].fills[0].kind else { panic!() };
    assert!(near(handles[0], 0.0, 0.0) && near(handles[1], 1.0, 0.0) && near(handles[2], 0.0, 1.0), "{handles:?}");
    assert_eq!(stops.len(), 2);
    assert!((stops[1].color.a - 0.5).abs() < 1e-4);
    let PaintKind::RadialGradient { handles, .. } = &k[1].fills[0].kind else { panic!() };
    assert!(near(handles[0], 0.5, 0.5) && near(handles[1], 1.0, 0.5) && near(handles[2], 0.5, 1.0), "{handles:?}");
    // userSpaceOnUse: x = 50 is the middle of the rect at x 60..90? no, left of it
    let PaintKind::LinearGradient { handles, .. } = &k[2].fills[0].kind else { panic!() };
    assert!(near(handles[0], -10.0 / 30.0, 0.0) && near(handles[1], -10.0 / 30.0, 1.0), "{handles:?}");
    assert!((k[2].fills[0].opacity - 0.8).abs() < 1e-4);
}

#[test]
fn embedded_images_become_image_fills() {
    let mut png = std::io::Cursor::new(Vec::new());
    image::RgbaImage::from_pixel(4, 2, image::Rgba([255, 0, 0, 255])).write_to(&mut png, image::ImageFormat::Png).unwrap();
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode(png.into_inner());
    let svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="40" height="20"><image x="5" y="5" width="20" height="10" xlink:href="data:image/png;base64,{b64}"/></svg>"#
    );
    let (doc, id) = import(&svg);
    let n = kids(&doc, id)[0];
    assert!(matches!(n.kind, NodeKind::Rectangle));
    let PaintKind::Image { image_ref, .. } = &n.fills[0].kind else { panic!() };
    assert!(doc.images.contains_key(image_ref));
    let bb = (n.transform.a * n.size.x, n.transform.d * n.size.y);
    assert_eq!(bb, (20.0, 10.0));
    assert_eq!((n.transform.tx, n.transform.ty), (5.0, 5.0));
}

#[test]
fn external_hrefs_are_never_read() {
    let (doc, id) = import(
        r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="10" height="10"><image width="10" height="10" xlink:href="Cargo.toml"/></svg>"#,
    );
    assert!(kids(&doc, id).is_empty());
    assert!(doc.images.is_empty());
}

#[test]
fn rect_clip_path_becomes_a_clipping_frame_and_others_become_masks() {
    let (doc, id) = import(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100">
          <defs>
            <clipPath id="box"><rect x="10" y="10" width="30" height="30" rx="4"/></clipPath>
            <clipPath id="tri"><path d="M50 10 L90 10 L70 40 Z"/></clipPath>
          </defs>
          <g clip-path="url(#box)"><rect width="100" height="100" fill="red"/><circle cx="30" cy="30" r="30" fill="blue"/></g>
          <g clip-path="url(#tri)"><rect width="100" height="100" fill="green"/><circle cx="60" cy="20" r="30" fill="blue"/></g>
        </svg>"##,
    );
    let k = kids(&doc, id);
    assert_eq!(k.len(), 2, "{}", dump(&doc, id));
    let NodeKind::Frame(f) = &k[0].kind else { panic!("{}", dump(&doc, id)) };
    assert!(f.clips_content);
    assert_eq!((k[0].transform.tx, k[0].transform.ty, k[0].size.x, k[0].size.y), (10.0, 10.0, 30.0, 30.0));
    assert_eq!(k[0].corner_radii, [4.0; 4]);
    // children are re-based into the frame
    let inner = kids(&doc, k[0].id);
    assert_eq!((inner[0].transform.tx, inner[0].transform.ty), (-10.0, -10.0));

    assert!(matches!(k[1].kind, NodeKind::Group));
    let inner = kids(&doc, k[1].id);
    assert!(inner[0].is_mask, "{}", dump(&doc, id));
    assert_eq!(inner.len(), 3);
}

#[test]
fn luminance_masks_turn_brightness_into_alpha() {
    let (doc, id) = import(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100">
          <mask id="m"><rect width="100" height="100" fill="#000"/><rect width="50" height="100" fill="#fff"/></mask>
          <rect width="100" height="100" fill="red" mask="url(#m)"/>
        </svg>"##,
    );
    let g = kids(&doc, id)[0];
    let ms = kids(&doc, g.id);
    assert!(ms[0].is_mask);
    let m = kids(&doc, ms[0].id);
    let alpha = |n: &Node| match &n.fills[0].kind {
        PaintKind::Solid(c) => c.a,
        _ => panic!(),
    };
    assert!(alpha(m[0]) < 0.01 && alpha(m[1]) > 0.99);
}

#[test]
fn filters_map_to_effects() {
    let (doc, id) = import(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100">
          <filter id="s" x="-50%" y="-50%" width="200%" height="200%"><feDropShadow dx="2" dy="4" stdDeviation="3" flood-color="#112233" flood-opacity="0.4"/></filter>
          <filter id="b"><feGaussianBlur stdDeviation="2.5"/></filter>
          <rect x="10" y="10" width="30" height="30" fill="red" filter="url(#s)"/>
          <circle cx="70" cy="70" r="10" fill="blue" filter="url(#b)"/>
        </svg>"##,
    );
    let k = kids(&doc, id);
    assert!(matches!(k[0].kind, NodeKind::Rectangle), "{}", dump(&doc, id));
    let Effect::DropShadow { color, offset, radius, .. } = &k[0].effects[0] else { panic!("{:?}", k[0].effects) };
    assert_eq!((offset.x, offset.y, *radius), (2.0, 4.0, 6.0));
    assert!((color.a - 0.4).abs() < 1e-3 && (color.b - 0x33 as f32 / 255.0).abs() < 1e-3);
    assert!(matches!(k[1].effects[0], Effect::LayerBlur { radius, .. } if radius == 5.0));
}

#[test]
fn text_becomes_an_editable_text_node() {
    let (doc, id) = import(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100">
          <text x="20" y="40" font-family="Inter" font-size="24" font-weight="700" fill="#336699">Hello</text>
          <text x="100" y="80" text-anchor="middle" font-size="12" font-style="italic" fill="#000">Centered</text>
        </svg>"##,
    );
    let k = kids(&doc, id);
    assert_eq!(k.len(), 2, "{}", dump(&doc, id));
    let NodeKind::Text(t) = &k[0].kind else { panic!("{}", dump(&doc, id)) };
    assert_eq!(t.content, "Hello");
    assert_eq!((t.style.font_size, t.style.font_weight, t.style.align_h), (24.0, 700, TextAlignH::Left));
    assert!(matches!(&t.style.fills[0].kind, PaintKind::Solid(c) if (c.r - 0.2).abs() < 0.01));
    // the baseline sits at y = 40
    let geom = desigma_render::text_geometry(t, Vec2::ZERO);
    assert!((k[0].transform.ty + geom.lines[0].baseline - 40.0).abs() < 0.01);
    assert_eq!(k[0].transform.tx, 20.0);
    let NodeKind::Text(t2) = &k[1].kind else { panic!() };
    assert_eq!(t2.style.align_h, TextAlignH::Center);
    assert!(t2.style.italic);
    // centered on x = 100
    assert!((k[1].transform.tx + k[1].size.x / 2.0 - 100.0).abs() < 0.01);
}

#[test]
fn multiline_tspans_merge_into_one_node() {
    let (doc, id) = import(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100"><text font-size="16" fill="#000"><tspan x="10" y="30">one</tspan><tspan x="10" y="50">two</tspan><tspan x="10" y="70">three</tspan></text></svg>"##,
    );
    let k = kids(&doc, id);
    assert_eq!(k.len(), 1, "{}", dump(&doc, id));
    let NodeKind::Text(t) = &k[0].kind else { panic!() };
    assert_eq!(t.content, "one\ntwo\nthree");
    assert_eq!(t.style.line_height, LineHeight::Pixels(20.0));
}

#[test]
fn svg_to_nodes_is_a_self_contained_subtree() {
    let mut next = 100u64;
    let out = svg_to_nodes(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><g opacity=".5"><rect width="4" height="4"/><rect x="5" width="4" height="4"/></g></svg>"##,
        "icon",
        &mut || {
            next += 1;
            NodeId(next)
        },
    )
    .unwrap();
    assert_eq!(out.nodes.len(), 4);
    assert_eq!(out.root().name, "icon");
    assert!(out.root().parent.is_none());
    for n in &out.nodes[1..] {
        let p = out.nodes.iter().find(|p| p.id == n.parent.unwrap()).unwrap();
        assert!(p.children.contains(&n.id));
    }
    let ids: std::collections::BTreeSet<_> = out.nodes.iter().map(|n| n.id).collect();
    assert_eq!(ids.len(), 4);
}

#[test]
fn errors_and_sniffing() {
    let mut doc = Document::new("t");
    let page = doc.add_page("p");
    assert!(import_svg("not svg", &mut doc, page, Vec2::ZERO).is_err());
    assert!(import_svg("<svg xmlns=\"http://www.w3.org/2000/svg\"", &mut doc, page, Vec2::ZERO).is_err());
    assert!(looks_like_svg("  <svg width=\"1\"/>"));
    assert!(looks_like_svg("<?xml version=\"1.0\"?>\n<!-- c --><svg/>"));
    assert!(!looks_like_svg("<html></html>"));
    assert!(!looks_like_svg("hello <svg>"));
    let id = import_svg_named("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"5\" height=\"5\"/>", "logo", &mut doc, page, Vec2::ZERO).unwrap();
    assert_eq!(doc.get(id).unwrap().name, "logo");
}

#[test]
fn unnamed_multi_child_groups_stay_groups() {
    // Hand-written artboards: each <g> holds a background and its content (#34).
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="900" height="300">
      <g><rect width="400" height="300" fill="#eee"/><rect x="20" y="20" width="50" height="50" fill="#f00"/></g>
      <g><rect x="500" width="400" height="300" fill="#ddd"/><rect x="520" y="20" width="50" height="50" fill="#00f"/></g>
    </svg>"##;
    let (doc, root) = import(svg);
    let top = kids(&doc, root);
    assert_eq!(top.len(), 2, "{}", dump(&doc, root));
    assert!(top.iter().all(|g| matches!(g.kind, NodeKind::Group) && g.children.len() == 2 && g.name == "Group"), "{}", dump(&doc, root));
    // A single-child wrapper still folds into its child, and desigma's own export wrappers
    // (data-name) still join their parent.
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100">
      <g><rect width="10" height="10" fill="#f00"/></g>
      <g data-name="Wrapper"><rect x="20" width="10" height="10" fill="#0f0"/><rect x="40" width="10" height="10" fill="#00f"/></g>
    </svg>"##;
    let (doc, root) = import(svg);
    assert!(kids(&doc, root).iter().all(|n| matches!(n.kind, NodeKind::Rectangle)), "{}", dump(&doc, root));
}

fn png_file(path: &std::path::Path) {
    image::RgbaImage::from_pixel(4, 2, image::Rgba([0, 128, 255, 255])).save(path).unwrap();
}

fn import_file(dir: &std::path::Path, svg: &str) -> Document {
    let mut doc = Document::new("t");
    let page = doc.add_page("Page 1");
    import_svg_named_with(svg, "f", &mut doc, page, Vec2::ZERO, &ImportOptions { base_dir: Some(dir.to_path_buf()) }).unwrap();
    doc
}

#[test]
fn images_next_to_the_svg_file_load_and_others_do_not() {
    let base = std::env::temp_dir().join(format!("desigma-svg-href-{}", std::process::id()));
    let dir = base.join("design");
    std::fs::create_dir_all(dir.join("assets")).unwrap();
    png_file(&dir.join("assets/hero image.png"));
    png_file(&base.join("outside.png"));
    let svg = |href: &str| {
        format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="40" height="20"><image width="20" height="10" xlink:href="{href}"/></svg>"#
        )
    };
    let image_count = |doc: &Document| doc.images.len();

    let doc = import_file(&dir, &svg("assets/hero%20image.png"));
    assert_eq!(image_count(&doc), 1, "relative image next to the SVG is embedded");
    let doc = import_file(&dir, &svg("./assets/hero image.png"));
    assert_eq!(image_count(&doc), 1);

    // Outside the SVG's folder, absolute paths and URLs are never read.
    let outside = base.join("outside.png");
    for href in ["../outside.png", outside.to_str().unwrap(), "file:///etc/hostname", "https://example.com/a.png"] {
        let doc = import_file(&dir, &svg(href));
        assert_eq!(image_count(&doc), 0, "{href}");
    }
    // Without a base folder (paste, MCP text), no file is read at all.
    let (doc, _) = import(&svg("assets/hero%20image.png"));
    assert_eq!(image_count(&doc), 0);
    let _ = std::fs::remove_dir_all(&base);
}

/// Every Text node under `id`, in document order.
fn texts(doc: &Document, id: NodeId) -> Vec<&Node> {
    let mut ids = Vec::new();
    doc.walk(id, |n, _| {
        if matches!(n.kind, NodeKind::Text(_)) {
            ids.push(n.id);
        }
    });
    ids.iter().filter_map(|i| doc.get(*i)).collect()
}

/// Content of every Text node under `id`, in document order.
fn text_contents(doc: &Document, id: NodeId) -> Vec<String> {
    let mut out = Vec::new();
    doc.walk(id, |n, _| {
        if let NodeKind::Text(t) = &n.kind {
            out.push(t.content.clone());
        }
    });
    out
}

#[test]
fn text_outside_the_bundled_font_keeps_its_content() {
    // usvg only knows the bundled Inter face, so a font family it cannot find lays
    // out zero glyphs and the element used to vanish (no node, no pixels). The
    // resolved chunks are kept so an editable Text node still comes out.
    let (doc, id) = import(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100">
          <text x="20" y="40" font-family="PingFang SC" font-size="12" font-weight="500" fill="#64748b">用户名</text>
        </svg>"##,
    );
    let k = kids(&doc, id);
    assert_eq!(k.len(), 1, "{}", dump(&doc, id));
    let NodeKind::Text(t) = &k[0].kind else { panic!("{}", dump(&doc, id)) };
    assert_eq!(t.content, "用户名");
    assert_eq!((t.style.font_size, t.style.font_weight), (12.0, 500));
    assert_eq!(t.style.font_family, "PingFang SC");
    assert_eq!(k[0].transform.tx, 20.0);
    // Baseline `y=40`: the box sits above it, not on it.
    assert!(k[0].transform.ty < 40.0 && k[0].transform.ty > 25.0, "{}", dump(&doc, id));
}

#[test]
fn shaped_text_is_not_duplicated_by_the_chunk_fallback() {
    let (doc, id) = import(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100">
          <text x="20" y="40" font-family="Inter" font-size="16" fill="#000">Hello</text>
          <text x="20" y="80" font-family="PingFang SC" font-size="12" fill="#000">你好</text>
        </svg>"##,
    );
    assert_eq!(text_contents(&doc, id), vec!["Hello", "你好"], "{}", dump(&doc, id));
}

#[test]
fn unshaped_text_in_a_translated_group_keeps_its_place_and_tspans() {
    // What Figma and Illustrator write: text wrapped in translated groups, one
    // <tspan> per style run, no x/y on the spans.
    let (doc, id) = import(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100">
          <g transform="translate(30 10)">
            <text x="5" y="40" font-family="PingFang SC" font-size="12"><tspan>用户</tspan><tspan font-weight="700">名</tspan></text>
          </g>
        </svg>"##,
    );
    let t = texts(&doc, id);
    assert_eq!(t.len(), 1, "{}", dump(&doc, id));
    let NodeKind::Text(text) = &t[0].kind else { unreachable!() };
    assert_eq!(text.content, "用户名");
    assert_eq!(text.style.font_family, "PingFang SC");
    // 30 from the group, 5 from the element.
    assert_eq!(t[0].transform.tx, 35.0, "{}", dump(&doc, id));
}

#[test]
fn unshaped_text_follows_the_viewbox_scale() {
    let (doc, id) = import(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="48" height="48" viewBox="0 0 24 24">
          <rect width="4" height="4" fill="#00f"/>
          <text x="6" y="12" font-family="PingFang SC" font-size="4">汉字</text>
        </svg>"##,
    );
    let t = texts(&doc, id);
    assert_eq!(t.len(), 1, "{}", dump(&doc, id));
    // The viewBox scales user units by 2; the text box has to scale with it, like
    // the rect next to it (the scale may live on the text or on a group around it).
    let w = doc.world_transform(t[0].id);
    assert_eq!((w.a, w.d), (2.0, 2.0), "{}", dump(&doc, id));
    // `import` places the frame at (100, 50); the user-space x=6 becomes 2×6.
    assert!((w.tx - 100.0 - 12.0).abs() < 1e-3, "{}", dump(&doc, id));
    assert!((w.ty - 50.0 - 2.0 * t[0].transform.ty).abs() < 1e-3, "{}", dump(&doc, id));
}

#[test]
fn text_kept_as_outlines_is_not_emitted_twice() {
    // Vertical text with a font usvg does have: collect() refuses it, the glyphs
    // flatten to real outlines — so no Text node may be added on top.
    let (doc, id) = import(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100">
          <text x="20" y="40" font-family="Inter" font-size="16" writing-mode="vertical-rl">Hello</text>
        </svg>"##,
    );
    assert!(texts(&doc, id).is_empty(), "{}", dump(&doc, id));
    assert!(kids(&doc, id).iter().any(|n| matches!(n.kind, NodeKind::Vector { .. })), "{}", dump(&doc, id));
}

#[test]
fn unshaped_text_inherits_fill_and_font_from_its_group() {
    let (doc, id) = import(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100">
          <g fill="#ff0000" font-size="20"><text x="10" y="30" font-family="PingFang SC">汉字</text></g>
        </svg>"##,
    );
    let t = texts(&doc, id);
    assert_eq!(t.len(), 1, "{}", dump(&doc, id));
    let NodeKind::Text(text) = &t[0].kind else { unreachable!() };
    assert_eq!(text.style.font_size, 20.0, "{}", dump(&doc, id));
    assert_eq!(text.style.fills.len(), 1, "{}", dump(&doc, id));
    assert!(matches!(text.style.fills[0].kind, PaintKind::Solid(c) if c == Color::from_hex("#ff0000").unwrap()), "{}", dump(&doc, id));
}
