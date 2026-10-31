//! XML pre-scan: usvg turns `<rect>`, `<circle>` and `<ellipse>` into plain
//! paths. To rebuild Rectangle / Ellipse nodes we find those elements first,
//! make sure each carries an `id` (usvg keeps ids on its paths) and remember
//! what shape the id stood for.

use std::collections::HashMap;

/// Prefix of ids we inject; never shown as layer names.
pub const GENERATED_ID: &str = "__dsg";
/// Injected on an unnamed `<g>` with several element children so usvg keeps it and the
/// converter does not fold it: in hand-written SVG such a group is an artboard or a
/// section, not a wrapper. Starts with [`GENERATED_ID`], so it is never shown as a name.
pub const KEPT_GROUP_ID: &str = "__dsgg";

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
    /// `rx`, `ry` as written (already defaulted from each other), `None` when
    /// the value uses units we do not evaluate (then the rect stays a vector).
    Rect {
        rx: Option<f32>,
        ry: Option<f32>,
    },
    Ellipse,
}

/// A `<text>` element as written in the source.
///
/// `usvg` only keeps text it can shape, and it is built with the bundled Inter
/// face alone — so any run outside its coverage (CJK, Cyrillic, emoji …) lays
/// out zero glyphs and the element disappears without a trace. Keeping the raw
/// attributes lets the converter rebuild an editable Text node in that case.
#[derive(Clone, Debug, PartialEq)]
pub struct RawText {
    pub x: f32,
    pub y: f32,
    pub font_size: f32,
    pub font_weight: u16,
    pub font_family: String,
    /// First solid color found in `fill`, already `#rrggbb`.
    pub fill: Option<String>,
    /// `start` / `middle` / `end`.
    pub anchor: String,
    pub content: String,
}

pub struct Scanned {
    /// The markup to feed to usvg (ids injected).
    pub svg: String,
    pub shapes: HashMap<String, Shape>,
    /// Text elements, in document order (see [`RawText`]).
    pub texts: Vec<RawText>,
}

pub fn prescan(svg: &str) -> Scanned {
    let Ok(doc) = roxmltree::Document::parse_with_options(svg, roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() }) else {
        // usvg will report the parse error.
        return Scanned { svg: svg.to_string(), shapes: HashMap::new(), texts: Vec::new() };
    };
    let mut shapes = HashMap::new();
    let mut texts: Vec<RawText> = Vec::new();
    let mut inserts: Vec<(usize, String)> = Vec::new();
    let mut counter = 0usize;
    for node in doc.descendants().filter(|n| n.is_element()) {
        let shape = match node.tag_name().name() {
            "rect" => {
                let rx = len_attr(&node, "rx");
                let ry = len_attr(&node, "ry");
                // An unparsable value poisons both radii.
                let bad = matches!(rx, Some(None)) || matches!(ry, Some(None));
                let (rx, ry) = if bad {
                    (None, None)
                } else {
                    let (rx, ry) = (rx.flatten(), ry.flatten());
                    (Some(rx.or(ry).unwrap_or(0.0)), Some(ry.or(rx).unwrap_or(0.0)))
                };
                Shape::Rect { rx, ry }
            }
            "circle" | "ellipse" => Shape::Ellipse,
            "g" => {
                let unnamed = node.attribute("id").is_none_or(str::is_empty) && node.attribute("data-name").is_none();
                if unnamed && node.children().filter(|c| c.is_element()).count() >= 2 {
                    let id = format!("{KEPT_GROUP_ID}{counter}");
                    counter += 1;
                    let start = node.range().start;
                    inserts.push((start + 2, format!(" id=\"{id}\"")));
                }
                continue;
            }
            "text" => {
                let content = node.text().unwrap_or("").to_string();
                if !content.trim().is_empty() {
                    texts.push(RawText {
                        x: num_attr(&node, "x").unwrap_or(0.0),
                        y: num_attr(&node, "y").unwrap_or(0.0),
                        font_size: num_attr(&node, "font-size").unwrap_or(16.0),
                        font_weight: num_attr(&node, "font-weight").unwrap_or(400.0).round() as u16,
                        font_family: node.attribute("font-family").unwrap_or("Inter").split(',').next().unwrap_or("Inter").trim().trim_matches('\'').to_string(),
                        fill: node.attribute("fill").map(|v| v.trim().to_string()).filter(|v| !v.is_empty() && v != "none"),
                        anchor: node.attribute("text-anchor").unwrap_or("start").to_string(),
                        content,
                    });
                }
                continue;
            }
            _ => continue,
        };
        let id = match node.attribute("id") {
            Some(id) if !id.is_empty() => id.to_string(),
            _ => {
                let id = format!("{GENERATED_ID}{counter}");
                counter += 1;
                // Insert right after the tag name.
                let start = node.range().start;
                let rest = &svg[start + 1..];
                let name_len = rest.find(|c: char| c.is_whitespace() || c == '/' || c == '>').unwrap_or(rest.len());
                inserts.push((start + 1 + name_len, format!(" id=\"{id}\"")));
                id
            }
        };
        shapes.insert(id, shape);
    }
    if inserts.is_empty() {
        return Scanned { svg: svg.to_string(), shapes, texts };
    }
    inserts.sort_by_key(|(at, _)| std::cmp::Reverse(*at));
    let mut out = svg.to_string();
    for (at, text) in inserts {
        out.insert_str(at, &text);
    }
    Scanned { svg: out, shapes, texts }
}

fn num_attr(node: &roxmltree::Node, name: &str) -> Option<f32> {
    let v = node.attribute(name)?.trim();
    let num = v.strip_suffix("px").unwrap_or(v).trim();
    num.parse::<f32>().ok().filter(|n| n.is_finite())
}

/// `Some(Some(v))` parsed, `Some(None)` present but not a plain length, `None` absent.
fn len_attr(node: &roxmltree::Node, name: &str) -> Option<Option<f32>> {
    let v = node.attribute(name)?.trim();
    let num = v.strip_suffix("px").unwrap_or(v).trim();
    Some(num.parse::<f32>().ok().filter(|n| n.is_finite() && *n >= 0.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn injects_ids_and_records_shapes() {
        let s = prescan(
            r#"<svg xmlns="http://www.w3.org/2000/svg"><rect id="a" width="4" height="4" ry="2"/><circle r="3"/><ellipse id="e" rx="1" ry="2"/><rect width="1" height="1" rx="50%"/></svg>"#,
        );
        assert_eq!(s.shapes["a"], Shape::Rect { rx: Some(2.0), ry: Some(2.0) });
        assert_eq!(s.shapes["e"], Shape::Ellipse);
        assert_eq!(s.shapes.len(), 4);
        assert!(s.svg.contains("<circle id=\"__dsg0\" r=\"3\"/>"));
        assert!(s.svg.contains("<rect id=\"__dsg1\" width=\"1\""));
        assert!(s.shapes.values().any(|v| *v == Shape::Rect { rx: None, ry: None }));
    }
}
