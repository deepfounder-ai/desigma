//! React (JSX) output with inline styles or Tailwind classes.

use crate::codegen::*;
use crate::css::css_for;
use crate::tailwind::{instance_attrs, jsx_markup};
use crate::util::num;
use desigma_doc::*;

/// Options of [`react_with`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReactOptions {
    /// Use Tailwind `className`s instead of inline `style` objects.
    pub tailwind: bool,
}

/// A React component for `node` using inline styles.
pub fn react_for(doc: &Document, node: NodeId) -> String {
    react_with(doc, node, &ReactOptions::default())
}

/// A React component for `node`: `export function Name() { return (...) }` with one JSX element
/// per layer. Instances become `<Component prop="value" />`; bound variables are written as
/// `var(--token)` references.
pub fn react_with(doc: &Document, node: NodeId, opts: &ReactOptions) -> String {
    let Some(n) = doc.get(node) else { return String::new() };
    let mut o = Out::new("  ");
    o.line(0, format!("export function {}() {{", view_name(doc, n)));
    o.line(1, "return (");
    if opts.tailwind {
        jsx_markup(doc, node, 2, &mut o);
    } else {
        element(doc, node, Parent::root(), 2, &mut o);
    }
    o.line(1, ");");
    o.line(0, "}");
    o.finish()
}

fn camel_prop(p: &str) -> String {
    let mut out = String::new();
    let mut up = false;
    let p = if let Some(r) = p.strip_prefix('-') {
        up = true;
        r
    } else {
        p
    };
    for c in p.chars() {
        if c == '-' {
            up = true;
        } else if up {
            out.extend(c.to_uppercase());
            up = false;
        } else {
            out.push(c);
        }
    }
    out
}

fn js_value(v: &str) -> String {
    let t = v.trim();
    if let Some(px) = t.strip_suffix("px") {
        if px.parse::<f32>().is_ok() {
            return num(px.parse().unwrap_or(0.0));
        }
    }
    if t.parse::<f32>().is_ok() {
        return t.to_string();
    }
    format!("'{}'", t.replace('\\', "\\\\").replace('\'', "\\'"))
}

/// `key: value` pairs of a node's CSS (with variable references) as JS object entries.
fn style_entries(doc: &Document, id: NodeId, root: bool) -> Vec<String> {
    css_for(doc, id)
        .lines()
        .filter_map(|l| {
            let (p, v) = l.trim_end_matches(';').split_once(": ")?;
            // the root is placed by whoever renders the component
            if root && matches!(p, "position" | "left" | "top") {
                return None;
            }
            Some(format!("{}: {},", camel_prop(p), js_value(v)))
        })
        .collect()
}

fn escape_text(s: &str) -> String {
    let mut o = String::new();
    for ch in s.chars() {
        match ch {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '{' => o.push_str("&#123;"),
            '}' => o.push_str("&#125;"),
            '\n' => o.push_str("<br />"),
            c => o.push(c),
        }
    }
    o
}

fn open_tag(o: &mut Out, depth: usize, tag: &str, extra: &str, style: &[String], close: &str) {
    if style.is_empty() {
        o.line(depth, format!("<{tag}{extra}{close}"));
        return;
    }
    o.line(depth, format!("<{tag}{extra}"));
    o.line(depth + 1, "style={{");
    for s in style {
        o.line(depth + 2, s);
    }
    o.line(depth + 1, "}}");
    o.line(depth, close);
}

fn element(doc: &Document, id: NodeId, parent: Parent, depth: usize, o: &mut Out) {
    let Some(n) = doc.get(id) else { return };
    if !n.visible {
        return;
    }
    let style = style_entries(doc, id, parent.root);
    if matches!(n.kind, NodeKind::Instance { .. }) {
        open_tag(o, depth, &view_name(doc, n), &instance_attrs(doc, n), &style, "/>");
        return;
    }
    if is_vector_asset(doc, id) {
        let extra = format!(
            " src=\"/assets/{}.svg\" alt=\"{}\" width={{{}}} height={{{}}}",
            kebab(&n.name),
            n.name.replace('"', "&quot;"),
            num(n.size.x),
            num(n.size.y)
        );
        let style: Vec<String> = style.into_iter().filter(|s| !s.starts_with("width:") && !s.starts_with("height:")).collect();
        open_tag(o, depth, "img", &extra, &style, "/>");
        return;
    }
    if let Some(t) = text_of(n) {
        open_tag(o, depth, "p", "", &style, ">");
        o.line(depth + 1, escape_text(&t.content));
        o.line(depth, "</p>");
        return;
    }
    let tag = if matches!(n.kind, NodeKind::Section) { "section" } else { "div" };
    let kids = visible_children(doc, n);
    if kids.is_empty() {
        open_tag(o, depth, tag, "", &style, "/>");
        return;
    }
    open_tag(o, depth, tag, "", &style, ">");
    let me = Parent::of(n);
    for k in kids {
        element(doc, k.id, me, depth + 1, o);
    }
    o.line(depth, format!("</{tag}>"));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::fixture;

    #[test]
    fn golden_inline_card() {
        let (d, id) = fixture::card();
        let expected = r####"export function Card() {
  return (
    <div
      style={{
        display: 'flex',
        width: 312,
        height: 200,
        padding: 16,
        flexDirection: 'column',
        alignItems: 'flex-start',
        gap: 8,
        background: '#FFF',
        border: '1px solid #E5E5E5',
        borderRadius: 8,
        overflow: 'hidden',
        boxShadow: '0px 4px 8px 0px rgba(0, 0, 0, 0.25)',
      }}
    >
      <p
        style={{
          color: '#111',
          fontFamily: 'Inter',
          fontSize: 20,
          fontStyle: 'normal',
          fontWeight: 600,
          lineHeight: 'normal',
        }}
      >
        Hello world
      </p>
      <p
        style={{
          alignSelf: 'stretch',
          color: 'var(--color-text-muted)',
          fontFamily: 'Inter',
          fontSize: 14,
          fontStyle: 'normal',
          fontWeight: 400,
          lineHeight: 20,
          textAlign: 'center',
        }}
      >
        Body copy
      </p>
      <div
        style={{
          display: 'flex',
          alignSelf: 'stretch',
          padding: '8px 12px',
          justifyContent: 'center',
          alignItems: 'center',
          gap: 'calc(var(--spacing-md) * 1px)',
          background: 'var(--color-primary)',
          borderRadius: 4,
        }}
      >
        <Button label="Save" disabled
          style={{
            width: 80,
            height: 32,
            flexShrink: 0,
          }}
        />
        <div
          style={{
            width: 24,
            height: 24,
            flexShrink: 0,
            background: '#F00',
            borderRadius: '50%',
          }}
        />
      </div>
    </div>
  );
}"####;
        assert_eq!(react_for(&d, id), expected);
    }

    #[test]
    fn golden_tailwind_card() {
        let (d, id) = fixture::card();
        let expected = r####"export function Card() {
  return (
    <div className="flex flex-col items-start gap-2 p-4 w-[312px] h-[200px] bg-[#FFFFFF] border border-[#E5E5E5] rounded-lg overflow-hidden shadow-[0px_4px_8px_0px_rgba(0,0,0,0.25)]">
      <p className="font-['Inter'] text-xl font-semibold text-[#111111]">Hello world</p>
      <p className="self-stretch font-['Inter'] text-sm font-normal leading-5 text-center text-[color:var(--color-text-muted)]">Body copy</p>
      <div className="flex justify-center items-center gap-[calc(var(--spacing-md)*1px)] px-3 py-2 self-stretch bg-[var(--color-primary)] rounded">
        <Button label="Save" disabled className="w-20 h-8 shrink-0" />
        <div className="w-6 h-6 shrink-0 bg-[#FF0000] rounded-full" />
      </div>
    </div>
  );
}"####;
        assert_eq!(react_with(&d, id, &ReactOptions { tailwind: true }), expected);
    }
}
