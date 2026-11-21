//! Vue 3 single-file component output.

use crate::codegen::*;
use crate::css::css_for;
use desigma_doc::*;

/// A Vue 3 single-file component for `node` and its descendants: a `<template>` with one element
/// per layer, an optional `<script setup>` listing the components the template uses, and a
/// `<style scoped>` block holding each layer's CSS. Instances become component tags with their
/// changed properties, and bound variables stay `var(--token)` references in the style block.
pub fn vue_for(doc: &Document, node: NodeId) -> String {
    if doc.get(node).is_none() {
        return String::new();
    }
    let mut s = Sfc::default();
    let mut body = String::new();
    s.element(doc, node, 1, &mut body);
    let mut out = String::from("<template>\n");
    out.push_str(&body);
    out.push_str("</template>\n");

    if !s.components.is_empty() {
        out.push_str("\n<script setup>\n");
        for c in &s.components {
            out.push_str(&format!("// import {c} from './{c}.vue'\n"));
        }
        out.push_str("</script>\n");
    }

    out.push_str("\n<style scoped>\n");
    if s.uses_tokens {
        out.push_str("/* var(--token) references come from your design tokens. */\n");
    }
    for (class, decls) in &s.rules {
        out.push_str(&format!(".{class} {{\n"));
        for d in decls {
            out.push_str(&format!("  {d}\n"));
        }
        out.push_str("}\n");
    }
    out.push_str("</style>\n");
    out
}

#[derive(Default)]
struct Sfc {
    /// CSS rules in document order: class name and its declarations.
    rules: Vec<(String, Vec<String>)>,
    /// Component tags the template uses, in first-use order.
    components: Vec<String>,
    /// Whether any declaration references a design token.
    uses_tokens: bool,
}

/// `&`, `<`, `>` and `{{` escaped for a Vue template.
fn escape_text(s: &str) -> String {
    let mut o = String::new();
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '{' => o.push_str("&#123;"),
            '}' => o.push_str("&#125;"),
            c => o.push(c),
        }
    }
    o
}

/// `&`, `<`, `"` escaped for an attribute value.
fn escape_attr(s: &str) -> String {
    let mut o = String::new();
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '"' => o.push_str("&quot;"),
            c => o.push(c),
        }
    }
    o
}

impl Sfc {
    /// Registers a layer's CSS under a unique class name and returns that name.
    fn class(&mut self, doc: &Document, n: &Node, root: bool) -> String {
        let base = kebab(&n.name);
        let mut name = base.clone();
        let mut i = 2;
        while self.rules.iter().any(|(c, _)| *c == name) {
            name = format!("{base}-{i}");
            i += 1;
        }
        let decls: Vec<String> = css_for(doc, n.id)
            .lines()
            .filter(|l| !l.trim().is_empty())
            .filter(|l| {
                // the root is placed by whoever renders the component
                !(root && (l.starts_with("position: ") || l.starts_with("left: ") || l.starts_with("top: ")))
            })
            .map(|l| l.trim().to_string())
            .collect();
        if decls.iter().any(|d| d.contains("var(--")) {
            self.uses_tokens = true;
        }
        self.rules.push((name.clone(), decls));
        name
    }

    fn props(&mut self, doc: &Document, n: &Node) -> String {
        let comp = view_name(doc, n);
        if !self.components.contains(&comp) {
            self.components.push(comp.clone());
        }
        let mut out = String::new();
        for (name, lit) in instance_args(doc, n) {
            out.push_str(&match lit {
                PropLit::Bool(b) => format!(" :{name}=\"{b}\""),
                PropLit::Text(t) => format!(" {name}=\"{}\"", escape_attr(&t)),
                PropLit::Instance(v) => format!(" :{name}=\"{v}\""),
            });
        }
        out
    }

    fn element(&mut self, doc: &Document, id: NodeId, depth: usize, out: &mut String) {
        let Some(n) = doc.get(id) else { return };
        if !n.visible {
            return;
        }
        let pad = "  ".repeat(depth);
        let class = self.class(doc, n, depth == 1);
        if matches!(n.kind, NodeKind::Instance { .. }) {
            let props = self.props(doc, n);
            out.push_str(&format!("{pad}<{} class=\"{class}\"{props} />\n", view_name(doc, n)));
            return;
        }
        if is_vector_asset(doc, id) {
            out.push_str(&format!("{pad}<img class=\"{class}\" src=\"/assets/{}.svg\" alt=\"{}\" />\n", kebab(&n.name), escape_attr(&n.name)));
            return;
        }
        if let Some(t) = text_of(n) {
            out.push_str(&format!("{pad}<p class=\"{class}\">{}</p>\n", escape_text(&t.content)));
            return;
        }
        let tag = if matches!(n.kind, NodeKind::Section) { "section" } else { "div" };
        let kids = visible_children(doc, n);
        if kids.is_empty() {
            out.push_str(&format!("{pad}<{tag} class=\"{class}\" />\n"));
            return;
        }
        out.push_str(&format!("{pad}<{tag} class=\"{class}\">\n"));
        for k in kids {
            self.element(doc, k.id, depth + 1, out);
        }
        out.push_str(&format!("{pad}</{tag}>\n"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::fixture;

    #[test]
    fn golden_card() {
        let (d, id) = fixture::card();
        let actual = vue_for(&d, id);
        assert!(actual.starts_with("<template>\n  <div class=\"card\">\n"), "root tag: {actual}");
        assert!(actual.contains("<p class=\"title\">Hello world</p>"), "{actual}");
        assert!(actual.contains("<Button class=\"button\" label=\"Save\" :disabled=\"true\" />"), "{actual}");
        assert!(actual.contains("<style scoped>"), "{actual}");
        assert!(actual.contains(".card {"), "{actual}");
        assert!(actual.contains("display: flex;"), "{actual}");
        // the root is placed by the parent, so it carries no absolute position
        assert!(!actual.contains(".card {\n  position: absolute"), "{actual}");
        assert!(actual.contains("var(--color-primary)"), "{actual}");
        assert!(actual.ends_with("</style>\n"), "{actual}");
    }

    #[test]
    fn script_setup_lists_components() {
        let (d, id) = fixture::card();
        let actual = vue_for(&d, id);
        assert!(actual.contains("<script setup>\n// import Button from './Button.vue'\n</script>"), "{actual}");
    }

    #[test]
    fn text_is_escaped() {
        let mut d = Document::new("t");
        let page = d.add_page("p");
        let id = d.alloc_id();
        let mut label = Node::text(id, Rect::new(0.0, 0.0, 100.0, 20.0), "a < b & {{ c }}", TextStyle::default());
        label.name = "Label".into();
        let label = d.insert(page, label).unwrap();
        let out = vue_for(&d, label);
        assert!(out.contains("a &lt; b &amp; &#123;&#123; c &#125;&#125;"), "{out}");
    }
}
