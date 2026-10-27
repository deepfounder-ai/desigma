//! Code export: a frame (or page) becomes a small project on disk.
//!
//! Dev Mode's code panel gives you one snippet for one layer. This turns a whole
//! selection into files you can open in an editor and run:
//!
//! * **HTML + CSS** — a standalone `index.html` plus `styles.css`, with vector
//!   layers written as SVG files next to it.
//! * **React** — `App.jsx` (or `App.tsx`), a `styles.css` when the component
//!   uses inline styles, and the same SVG assets.
//! * **Tailwind** — `index.html` with Tailwind classes, ready for the CDN build.
//! * **SwiftUI / Compose** — `View.swift` / `Screen.kt` with the token
//!   placeholders the Dev Mode panel prints.
//!
//! Every project also gets a `tokens.css` (`:root` custom properties) when the
//! document has variables, and an `assets/` folder with the SVGs and images the
//! layers reference. Nothing here talks to the network or the UI: it takes a
//! `Document` and returns a list of files, so the app, the CLI and the MCP
//! server all share it.

use crate::codegen::{kebab, snake, text_of, vector_assets, visible_children};
use crate::util::num;
use crate::css::css_for;
use crate::util::css_color;
use crate::SvgOptions;
use desigma_doc::*;
use std::collections::BTreeMap;

/// A file of an exported project: a path relative to the project root and its contents.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct File {
    pub path: String,
    pub contents: Vec<u8>,
}

impl File {
    fn text(path: impl Into<String>, contents: impl Into<String>) -> Self {
        Self { path: path.into(), contents: contents.into().into_bytes() }
    }
}

/// A whole exported project.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Project {
    /// Files in write order (directories are implied by the paths).
    pub files: Vec<File>,
}

impl Project {
    pub fn file(&self, path: &str) -> Option<&File> {
        self.files.iter().find(|f| f.path == path)
    }

    pub fn text(&self, path: &str) -> Option<&str> {
        self.file(path).and_then(|f| std::str::from_utf8(&f.contents).ok())
    }

    /// Writes every file under `dir`, creating directories.
    ///
    /// Refuses to overwrite, so an export never clobbers a project you edited.
    /// Every target path is checked first: a half-written project is worse than
    /// none, and the files already there are never touched.
    pub fn write_to(&self, dir: &std::path::Path) -> Result<Vec<std::path::PathBuf>, String> {
        for f in &self.files {
            let path = dir.join(&f.path);
            if std::fs::symlink_metadata(&path).is_ok() {
                return Err(format!("{} already exists: choose a new directory, or move that file away", path.display()));
            }
        }
        let mut written = Vec::new();
        for f in &self.files {
            let path = dir.join(&f.path);
            let result = (|| -> std::io::Result<()> {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&path)?;
                std::io::Write::write_all(&mut file, &f.contents)
            })();
            if let Err(e) = result {
                // Roll back what this call created, so a failed export leaves nothing.
                for done in &written {
                    let _ = std::fs::remove_file(done);
                }
                return Err(format!("Cannot write {}: {e}", path.display()));
            }
            written.push(path);
        }
        Ok(written)
    }
}

/// What a project exports to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CodeTarget {
    /// `index.html` + `styles.css`.
    #[default]
    Html,
    /// `App.jsx` with inline styles (or Tailwind classes).
    React,
    /// `index.html` with Tailwind classes.
    Tailwind,
    /// `View.swift`.
    SwiftUI,
    /// `Screen.kt`.
    Compose,
}

impl CodeTarget {
    pub const ALL: [CodeTarget; 5] = [CodeTarget::Html, CodeTarget::React, CodeTarget::Tailwind, CodeTarget::SwiftUI, CodeTarget::Compose];

    pub fn label(self) -> &'static str {
        match self {
            CodeTarget::Html => "HTML + CSS",
            CodeTarget::React => "React",
            CodeTarget::Tailwind => "Tailwind",
            CodeTarget::SwiftUI => "SwiftUI",
            CodeTarget::Compose => "Compose",
        }
    }

    /// The `language` value of `desigma_generate_code` and `--code`.
    pub fn key(self) -> &'static str {
        match self {
            CodeTarget::Html => "html",
            CodeTarget::React => "react",
            CodeTarget::Tailwind => "tailwind",
            CodeTarget::SwiftUI => "swiftui",
            CodeTarget::Compose => "compose",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim().to_ascii_lowercase();
        Self::ALL.into_iter().find(|t| t.key() == s).or(match s.as_str() {
            "css" | "html+css" | "web" => Some(CodeTarget::Html),
            "jsx" | "tsx" => Some(CodeTarget::React),
            "swift" => Some(CodeTarget::SwiftUI),
            "kotlin" | "jetpack" => Some(CodeTarget::Compose),
            _ => None,
        })
    }

    /// The main source file of the project.
    pub fn entry(self) -> &'static str {
        match self {
            CodeTarget::Html | CodeTarget::Tailwind => "index.html",
            CodeTarget::React => "App.jsx",
            CodeTarget::SwiftUI => "View.swift",
            CodeTarget::Compose => "Screen.kt",
        }
    }
}

/// Options of [`export_project`].
#[derive(Clone, Debug, Default)]
pub struct CodeOptions {
    pub target: CodeTarget,
    /// React only: Tailwind classes instead of inline styles.
    pub react_tailwind: bool,
    /// Write `tokens.css` with the document's variables.
    pub tokens: bool,
    /// Write `fonts.css` and the font files the design uses.
    pub embed_fonts: bool,
    /// Where the assets folder sits, relative to the project root and without a
    /// trailing slash (`assets` by default).
    pub asset_dir: String,
}

impl CodeOptions {
    pub fn new(target: CodeTarget) -> Self {
        Self { target, tokens: true, embed_fonts: true, asset_dir: "assets".into(), ..Default::default() }
    }
}

/// Builds the project for `node` (a frame, section, component or page).
pub fn export_project(doc: &Document, node: NodeId, opts: &CodeOptions) -> Result<Project, String> {
    let n = doc.get(node).ok_or_else(|| format!("node {} not found", node.0))?;
    if !matches!(n.kind, NodeKind::Frame(_) | NodeKind::Section | NodeKind::Component(_) | NodeKind::ComponentSet(_) | NodeKind::Page { .. } | NodeKind::Group) {
        return Err("export a frame, section, component or page (a single shape has no layout to export)".into());
    }
    let assets = Assets::new(doc, node, &opts.asset_dir);
    // Web targets only: `tokens.css` and `fonts.css` are CSS, and the Swift and
    // Kotlin output refers to tokens through its own placeholders (`Tokens.x`,
    // `Color("x")`) and to fonts through the platform's font registry.
    let web = matches!(opts.target, CodeTarget::Html | CodeTarget::Tailwind | CodeTarget::React);
    // The generated CSS references tokens as `var(--name)`, so tokens.css has to
    // be written (and linked) whenever the document has variables, or every bound
    // color and spacing renders as an invalid value.
    let tokens = (opts.tokens && web).then(|| tokens_css(doc)).flatten();
    // The markup names the design's fonts; without the files the browser falls
    // back to a system face and every line breaks differently.
    let fonts = if opts.embed_fonts && web { fonts_css(doc, &opts.asset_dir) } else { None };
    let mut files = Vec::new();
    match opts.target {
        CodeTarget::Html => {
            files.push(File::text("styles.css", stylesheet(doc, node, &assets)));
            files.push(File::text("index.html", html_page(doc, node, &assets, false, tokens.is_some(), fonts.as_ref())));
        }
        CodeTarget::Tailwind => {
            files.push(File::text("index.html", html_page(doc, node, &assets, true, tokens.is_some(), fonts.as_ref())));
        }
        CodeTarget::React => {
            let code = if opts.react_tailwind {
                crate::react_with(doc, node, &crate::ReactOptions { tailwind: true })
            } else {
                crate::react_for(doc, node)
            };
            // The generators write `/assets/x.svg`, an absolute path that only
            // works when the project is served from its own root. A project on
            // disk is opened directly, so the imports must be relative.
            let code = code.replace("src=\"/assets/", "src=\"./assets/");
            files.push(File::text("App.jsx", format!("{}\n", code.trim_end())));
            files.push(File::text("index.jsx", react_entry(tokens.is_some(), fonts.is_some())));
        }
        CodeTarget::SwiftUI => files.push(File::text("View.swift", format!("{}\n", crate::swiftui_for(doc, node).trim_end()))),
        CodeTarget::Compose => files.push(File::text("Screen.kt", format!("{}\n", crate::compose_for(doc, node).trim_end()))),
    }
    if let Some(tokens) = tokens {
        files.push(File::text("tokens.css", tokens));
    }
    if let Some(f) = fonts {
        files.push(File::text("fonts.css", f.css));
        files.extend(f.files);
    }
    files.extend(assets.files(doc));
    Ok(Project { files })
}

/// The React entry point; it imports the stylesheets the project has.
fn react_entry(with_tokens: bool, with_fonts: bool) -> String {
    let mut imports = String::new();
    if with_tokens {
        imports.push_str("import './tokens.css';\n");
    }
    if with_fonts {
        imports.push_str("import './fonts.css';\n");
    }
    format!("import React from 'react';\nimport {{ createRoot }} from 'react-dom/client';\n{imports}import {{ App }} from './App.jsx';\n\ncreateRoot(document.getElementById('root')).render(<App />);\n")
}

/// `@font-face` rules plus the font files for the families the design uses.
struct Fonts {
    css: String,
    files: Vec<File>,
}

/// Embeds the fonts the document's text names, taking the bundled and cached
/// files from `desigma-fonts`. Families it cannot resolve (not bundled, not in
/// the cache) are skipped: the browser then falls back exactly as the design
/// tool would.
fn fonts_css(doc: &Document, asset_dir: &str) -> Option<Fonts> {
    let dir = asset_dir.trim_matches('/');
    let dir = if dir.is_empty() { "assets" } else { dir };
    let cache = desigma_fonts::FontCache::new().offline();
    let mut css = String::from("/* Fonts used by the design */\n");
    let mut files = Vec::new();
    let mut seen: std::collections::BTreeSet<String> = Default::default();
    for req in desigma_fonts::requests_for(doc) {
        let family = desigma_fonts::substitute(&req.family).map(str::to_string).unwrap_or_else(|| req.family.clone());
        let effective = desigma_fonts::FontRequest::new(family.clone(), req.weight, req.italic);
        let Some(data) = desigma_fonts::bundled_for(&effective).map(|a| a.data.to_vec()).or_else(|| cache.cached(&effective)) else { continue };
        let slug = format!("{}-{}-{}", kebab(&family), req.weight, if req.italic { "italic" } else { "normal" });
        if !seen.insert(slug.clone()) {
            continue;
        }
        let path = format!("{dir}/{slug}.ttf");
        css.push_str(&format!(
            "@font-face {{\n  font-family: '{}';\n  font-style: {};\n  font-weight: {};\n  font-display: block;\n  src: url('{path}') format('truetype');\n}}\n\n",
            family.replace('\'', ""),
            if req.italic { "italic" } else { "normal" },
            req.weight
        ));
        files.push(File { path, contents: data });
    }
    (files.len() > 0).then_some(Fonts { css, files })
}


// ------------------------------------------------------------------ assets

/// The SVG and image assets a subtree references, and the paths the markup uses.
struct Assets {
    /// Vector layer -> file name (`assets/icon.svg`).
    vectors: BTreeMap<NodeId, String>,
    /// Image ref -> file name (`assets/photo.png`).
    images: BTreeMap<String, String>,
}

impl Assets {
    fn new(doc: &Document, root: NodeId, dir: &str) -> Self {
        let dir = dir.trim_matches('/').to_string();
        let dir = if dir.is_empty() { "assets".to_string() } else { dir };
        let mut vectors = BTreeMap::new();
        let mut used: BTreeMap<String, usize> = BTreeMap::new();
        for id in vector_assets(doc, root) {
            let Some(n) = doc.get(id) else { continue };
            let base = kebab(&n.name);
            let count = used.entry(base.clone()).or_insert(0);
            let name = if *count == 0 { base.clone() } else { format!("{base}-{}", *count) };
            *count += 1;
            vectors.insert(id, format!("{dir}/{name}.svg"));
        }
        let mut images = BTreeMap::new();
        for id in subtree(doc, root) {
            let Some(n) = doc.get(id) else { continue };
            for p in &n.fills {
                if let PaintKind::Image { image_ref, .. } = &p.kind {
                    if images.contains_key(image_ref) || !doc.images.contains_key(image_ref) {
                        continue;
                    }
                    let ext = image_ext(doc.images.get(image_ref).map(Vec::as_slice).unwrap_or_default());
                    let name = format!("{dir}/{}.{ext}", kebab(image_ref));
                    images.insert(image_ref.clone(), name);
                }
            }
        }
        Self { vectors, images }
    }

    /// The path markup uses for an image ref, or `None` when the document lacks the bytes.
    fn image(&self, image_ref: &str) -> Option<&str> {
        self.images.get(image_ref).map(String::as_str)
    }

    /// Replaces the generators' `url(<image_ref>)` placeholders by the real file.
    fn rewrite_images(&self, css: &str) -> String {
        let mut out = String::with_capacity(css.len());
        let mut rest = css;
        while let Some(start) = rest.find("url(<") {
            out.push_str(&rest[..start]);
            let after = &rest[start + 5..];
            let Some(end) = after.find(">)") else { break };
            let image_ref = &after[..end];
            match self.image(image_ref) {
                Some(path) => out.push_str(&format!("url({path})")),
                // the bytes are gone: keep the declaration valid but empty
                None => out.push_str("url(about:blank)"),
            }
            rest = &after[end + 2..];
        }
        out.push_str(rest);
        out
    }

    fn files(&self, doc: &Document) -> Vec<File> {
        let mut out = Vec::new();
        for (id, path) in &self.vectors {
            let svg = crate::export_svg(doc, *id, &SvgOptions::default());
            out.push(File::text(path.clone(), svg));
        }
        for (image_ref, path) in &self.images {
            if let Some(bytes) = doc.images.get(image_ref) {
                out.push(File { path: path.clone(), contents: bytes.clone() });
            }
        }
        out
    }
}

fn subtree(doc: &Document, root: NodeId) -> Vec<NodeId> {
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        let Some(n) = doc.get(id) else { continue };
        out.push(id);
        stack.extend(n.children.iter().copied());
    }
    out
}

fn image_ext(bytes: &[u8]) -> &'static str {
    match image::guess_format(bytes) {
        Ok(image::ImageFormat::Jpeg) => "jpg",
        Ok(image::ImageFormat::WebP) => "webp",
        Ok(image::ImageFormat::Gif) => "gif",
        _ => "png",
    }
}

// ------------------------------------------------------------------ HTML

/// The page for `node`. `tailwind` writes the utility-class markup and loads
/// the Tailwind CDN build; otherwise the page links the generated `styles.css`.
fn html_page(doc: &Document, node: NodeId, assets: &Assets, tailwind: bool, tokens: bool, fonts: Option<&Fonts>) -> String {
    let Some(n) = doc.get(node) else { return String::new() };
    let title = if n.name.trim().is_empty() { doc.name.clone() } else { n.name.clone() };
    let (head, body) = if tailwind {
        ("  <script src=\"https://cdn.tailwindcss.com\"></script>\n".to_string(), indent(&crate::tailwind::tailwind_html(doc, node), 1))
    } else {
        let mut body = String::new();
        html_node(doc, node, assets, 1, &mut body);
        ("  <link rel=\"stylesheet\" href=\"styles.css\" />\n".to_string(), body)
    };
    // tokens.css and fonts.css must come before styles.css: its rules use their
    // custom properties and families.
    let mut tokens_link = String::new();
    if tokens {
        tokens_link.push_str("  <link rel=\"stylesheet\" href=\"tokens.css\" />\n");
    }
    if fonts.is_some() {
        tokens_link.push_str("  <link rel=\"stylesheet\" href=\"fonts.css\" />\n");
    }
    format!(
        "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n  <meta charset=\"utf-8\" />\n  <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\" />\n  <title>{}</title>\n{tokens_link}{head}  <style>\n    body {{ margin: 0; }}\n  </style>\n</head>\n<body>\n{body}</body>\n</html>\n",
        crate::util::esc(&title)
    )
}

/// Shifts every line of `s` right by `depth` two-space steps.
fn indent(s: &str, depth: usize) -> String {
    let pad = "  ".repeat(depth);
    let mut out = String::new();
    for line in s.lines() {
        if !line.trim().is_empty() {
            out.push_str(&pad);
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

fn html_node(doc: &Document, id: NodeId, assets: &Assets, depth: usize, out: &mut String) {
    let Some(n) = doc.get(id) else { return };
    if !n.visible {
        return;
    }
    let indent = "  ".repeat(depth);
    let class = class_name(n);
    if let Some(path) = assets.vectors.get(&id) {
        // The class carries where the icon sits (absolute layers keep their
        // offsets); the attributes keep its size.
        out.push_str(&format!(
            "{indent}<img class=\"{class}\" src=\"{}\" alt=\"{}\" width=\"{}\" height=\"{}\" />\n",
            crate::util::esc(path),
            crate::util::esc(&n.name),
            num(n.size.x),
            num(n.size.y)
        ));
        return;
    }
    if let Some(t) = text_of(n) {
        out.push_str(&format!("{indent}<p class=\"{class}\">{}</p>\n", crate::util::esc(&t.content)));
        return;
    }
    let tag = if matches!(n.kind, NodeKind::Section) { "section" } else { "div" };
    let kids = visible_children(doc, n);
    out.push_str(&format!("{indent}<{tag} class=\"{class}\">\n"));
    for k in kids {
        html_node(doc, k.id, assets, depth + 1, out);
    }
    out.push_str(&format!("{indent}</{tag}>\n"));
}

/// A stable class name for a layer: its kebab-cased name, disambiguated by id.
///
/// A CSS identifier cannot start with a digit, so `9:41` becomes `l-9-41-254`.
fn class_name(n: &Node) -> String {
    let base = kebab(&n.name);
    let base = if base.is_empty() { "layer".to_string() } else { base };
    let base = if base.starts_with(|c: char| c.is_ascii_digit()) { format!("l-{base}") } else { base };
    format!("{base}-{}", n.id.0)
}

/// `styles.css`: one rule per layer, in document order.
///
/// The Dev Mode snippet describes one layer in isolation; a whole tree needs
/// three things it does not say, and they are added here:
///
/// * `box-sizing: border-box`, because Figma's width and height include padding
///   and border while CSS's default content box does not.
/// * `flex-shrink: 0` on auto-layout children that are not filling, because
///   Figma never squeezes a child below its size and CSS's default flex item does.
/// * `position: relative` on a container whose children are absolutely
///   positioned, or they would be placed against the page instead.
fn stylesheet(doc: &Document, root: NodeId, assets: &Assets) -> String {
    let mut out = String::from(
        "/* Generated by desigma */\n\n*,\n*::before,\n*::after {\n  box-sizing: border-box;\n}\n\n/* Figma text has no default margin; the browser's <p> does. */\np {\n  margin: 0;\n}\n\n",
    );
    for id in subtree(doc, root) {
        let Some(n) = doc.get(id) else { continue };
        if !n.visible || matches!(n.kind, NodeKind::Page { .. }) {
            continue;
        }
        let mut lines: Vec<String> = assets
            .rewrite_images(&css_for(doc, id))
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect();
        if assets.vectors.contains_key(&id) {
            // The SVG is an <img>: only where it sits matters, not how it paints.
            lines.retain(|l| ["position:", "left:", "top:", "transform:", "opacity:", "width:", "height:"].iter().any(|p| l.starts_with(p)));
        } else {
            fix_layout(doc, n, &mut lines);
        }
        if lines.is_empty() {
            continue;
        }
        out.push_str(&format!(".{} {{\n", class_name(n)));
        for line in lines {
            out.push_str("  ");
            out.push_str(&line);
            out.push('\n');
        }
        out.push_str("}\n\n");
    }
    out
}

/// Adds the declarations a whole-tree layout needs but the per-layer snippet omits.
fn fix_layout(doc: &Document, n: &Node, lines: &mut Vec<String>) {
    let has = |lines: &[String], prop: &str| lines.iter().any(|l| l.starts_with(prop));
    // `css_for` gives a child `position: absolute` whenever its parent is not an
    // auto-layout frame, so the parent must anchor it. Without this the child is
    // placed against the page and lands in the top-left corner.
    let own_al = n.kind.frame_props().and_then(|f| f.auto_layout);
    let abs_children = own_al.is_none() && n.children.iter().filter_map(|c| doc.get(*c)).any(|c| c.visible);
    if abs_children && n.kind.is_container() && !has(lines, "position:") {
        lines.push("position: relative;".into());
    }
    // Figma's auto-layout children keep their size; CSS flex items shrink by default.
    let parent_al = n.parent.and_then(|p| doc.get(p)).and_then(|p| p.kind.frame_props()).and_then(|f| f.auto_layout);
    let in_flow = parent_al.is_some() && !n.layout_child.absolute;
    if in_flow && !has(lines, "flex:") && !has(lines, "flex-shrink:") {
        lines.push("flex-shrink: 0;".into());
    }
}

// ------------------------------------------------------------------ tokens

/// `:root` custom properties for every variable in its collection's default mode.
///
/// Aliases are followed, so `--color-primary: #0D99FF` is the value the design
/// actually paints with rather than a reference to another custom property.
fn tokens_css(doc: &Document) -> Option<String> {
    if doc.variables.is_empty() {
        return None;
    }
    let mut out = String::from("/* Design tokens generated by desigma */\n:root {\n");
    let mut written = 0;
    let mut ids: Vec<&String> = doc.variables.keys().collect();
    ids.sort();
    for id in ids {
        let Some(v) = doc.variables.get(id) else { continue };
        let Some(coll) = doc.variable_collections.get(&v.collection) else { continue };
        let Some(literal) = resolve_default(doc, v, &coll.default_mode) else { continue };
        let name: String = crate::codegen::css_var(v);
        let name = name.strip_prefix("var(").and_then(|s| s.strip_suffix(')')).unwrap_or(&name).to_string();
        out.push_str(&format!("  {name}: {literal};\n"));
        written += 1;
    }
    if written == 0 {
        return None;
    }
    out.push_str("}\n");
    Some(out)
}

/// The CSS literal of `v` in `mode`, following aliases.
fn resolve_default(doc: &Document, v: &Variable, mode: &str) -> Option<String> {
    let mut current = v;
    for _ in 0..32 {
        let value = current.values.get(mode).or_else(|| current.values.values().next())?;
        match value {
            VariableValue::Alias(next) => current = doc.variables.get(next)?,
            VariableValue::Color(c) => return Some(css_color(c, 1.0)),
            VariableValue::Number(n) => return Some(num(*n)),
            VariableValue::String(s) => return Some(s.clone()),
            VariableValue::Boolean(b) => return Some(b.to_string()),
        }
    }
    None
}

/// File name of the exported source for `target` (`index.html`, `App.jsx`, ...).
pub fn entry_file(target: CodeTarget) -> &'static str {
    target.entry()
}

/// A suggested project directory name for `node`.
pub fn project_name(doc: &Document, node: NodeId) -> String {
    let n = doc.get(node);
    let raw = n.map(|n| n.name.as_str()).filter(|s| !s.trim().is_empty()).unwrap_or(&doc.name);
    let name = snake(raw);
    if name.is_empty() {
        "export".into()
    } else {
        name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card() -> (Document, NodeId) {
        crate::codegen::fixture::card()
    }

    #[test]
    fn targets_roundtrip() {
        for t in CodeTarget::ALL {
            assert_eq!(CodeTarget::parse(t.key()), Some(t));
        }
        assert_eq!(CodeTarget::parse("HTML"), Some(CodeTarget::Html));
        assert_eq!(CodeTarget::parse("css"), Some(CodeTarget::Html));
        assert_eq!(CodeTarget::parse("kotlin"), Some(CodeTarget::Compose));
        assert_eq!(CodeTarget::parse("cobol"), None);
    }

    #[test]
    fn html_project_has_page_styles_and_assets() {
        let (d, id) = card();
        let p = export_project(&d, id, &CodeOptions::new(CodeTarget::Html)).unwrap();
        let html = p.text("index.html").expect("index.html");
        assert!(html.starts_with("<!DOCTYPE html>"), "{html}");
        assert!(html.contains("<link rel=\"stylesheet\" href=\"styles.css\" />"), "{html}");
        // tokens.css must load before styles.css, which references its custom properties
        let tokens_at = html.find("tokens.css").expect("tokens.css is linked");
        let styles_at = html.find("styles.css").expect("styles.css is linked");
        assert!(tokens_at < styles_at, "{html}");
        assert!(html.contains("Hello world"), "{html}");
        assert!(html.contains("class=\"card-"), "{html}");
        assert!(!html.contains("style=\"display"), "layout lives in styles.css, not inline");
        let css = p.text("styles.css").expect("styles.css");
        assert!(css.contains("border-radius: 8px;"), "{css}");
        assert!(css.contains(".card-"), "{css}");
        assert!(p.text("tokens.css").is_some(), "the fixture has variables");
        // the bundled Inter travels with the project, or text reflows in the browser
        let fonts = p.text("fonts.css").expect("fonts.css");
        assert!(fonts.contains("font-family: 'Inter'"), "{fonts}");
        assert!(p.files.iter().any(|f| f.path.ends_with(".ttf")), "the font file is written");
    }

    #[test]
    fn tailwind_project_uses_the_cdn() {
        let (d, id) = card();
        let p = export_project(&d, id, &CodeOptions::new(CodeTarget::Tailwind)).unwrap();
        let html = p.text("index.html").unwrap();
        assert!(html.contains("cdn.tailwindcss.com"), "{html}");
        assert!(html.contains("class=\"flex flex-col"), "{html}");
        assert!(p.file("styles.css").is_none());
    }

    #[test]
    fn react_project_has_an_entry_and_styles() {
        let (d, id) = card();
        let p = export_project(&d, id, &CodeOptions::new(CodeTarget::React)).unwrap();
        let app = p.text("App.jsx").unwrap();
        assert!(app.contains("export function Card()"), "{app}");
        assert!(!app.contains("src=\"/assets/"), "asset paths must be relative: {app}");
        // inline styles carry the layout, so no stylesheet is written
        assert!(p.file("styles.css").is_none());
        let entry = p.text("index.jsx").unwrap();
        assert!(entry.contains("createRoot"), "the entry mounts the component");
        assert!(entry.contains("import './tokens.css'"), "the entry loads the tokens the styles use");
    }

    #[test]
    fn react_tailwind_skips_the_stylesheet() {
        let (d, id) = card();
        let mut o = CodeOptions::new(CodeTarget::React);
        o.react_tailwind = true;
        let p = export_project(&d, id, &o).unwrap();
        assert!(p.text("App.jsx").unwrap().contains("className="));
        assert!(p.file("styles.css").is_none());
    }

    #[test]
    fn swiftui_and_compose_write_one_file() {
        let (d, id) = card();
        for (target, path, needle) in [
            (CodeTarget::SwiftUI, "View.swift", "VStack"),
            (CodeTarget::Compose, "Screen.kt", "Column("),
        ] {
            let p = export_project(&d, id, &CodeOptions::new(target)).unwrap();
            assert!(p.text(path).unwrap().contains(needle), "{target:?}");
            // CSS and web fonts mean nothing to Swift or Kotlin
            assert!(p.file("tokens.css").is_none(), "{target:?} wrote tokens.css");
            assert!(p.file("fonts.css").is_none(), "{target:?} wrote fonts.css");
        }
    }

    #[test]
    fn a_single_shape_is_refused() {
        let mut d = Document::new("t");
        let page = d.add_page("p");
        let id = d.alloc_id();
        d.insert(page, Node::rectangle(id, Rect::new(0.0, 0.0, 10.0, 10.0))).unwrap();
        assert!(export_project(&d, id, &CodeOptions::new(CodeTarget::Html)).is_err());
    }

    #[test]
    fn writing_never_overwrites() {
        let (d, id) = card();
        let p = export_project(&d, id, &CodeOptions::new(CodeTarget::Html)).unwrap();
        let dir = std::env::temp_dir().join(format!("desigma_code_export_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let written = p.write_to(&dir).unwrap();
        assert!(written.iter().any(|w| w.ends_with("index.html")));
        assert!(p.write_to(&dir).is_err(), "a second export must not clobber the first");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_refused_export_writes_nothing() {
        let (d, id) = card();
        let p = export_project(&d, id, &CodeOptions::new(CodeTarget::Html)).unwrap();
        let dir = std::env::temp_dir().join(format!("desigma_code_export_partial_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // One file from a previous export blocks the whole run.
        std::fs::write(dir.join("styles.css"), "old").unwrap();
        assert!(p.write_to(&dir).is_err());
        assert_eq!(std::fs::read_to_string(dir.join("styles.css")).unwrap(), "old", "the existing file is untouched");
        assert!(!dir.join("index.html").exists(), "no partial project is left behind");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tokens_css_lists_variables() {
        let (d, id) = card();
        let p = export_project(&d, id, &CodeOptions::new(CodeTarget::Html)).unwrap();
        let tokens = p.text("tokens.css").expect("tokens.css");
        assert!(tokens.contains("--color-primary: #0D99FF;"), "{tokens}");
        assert!(tokens.contains("--color-text-muted: #666;"), "{tokens}");
        assert!(tokens.contains("--spacing-md: 12;"), "{tokens}");
    }

    #[test]
    fn tokens_follow_aliases() {
        let (mut d, id) = card();
        d.variables.insert(
            "alias".into(),
            Variable {
                id: "alias".into(),
                name: "Color/Brand".into(),
                collection: "c1".into(),
                kind: VariableKind::Color,
                values: BTreeMap::from([("light".into(), VariableValue::Alias("primary".into()))]),
                description: String::new(),
                scopes: vec![],
                hidden_from_publishing: false,
                key: None,
            },
        );
        let p = export_project(&d, id, &CodeOptions::new(CodeTarget::Html)).unwrap();
        let tokens = p.text("tokens.css").unwrap();
        assert!(tokens.contains("--color-brand: #0D99FF;"), "{tokens}");
    }

    #[test]
    fn class_names_are_valid_css_identifiers() {
        let mut d = Document::new("t");
        let page = d.add_page("p");
        let id = d.alloc_id();
        let mut n = Node::frame(id, Rect::new(0.0, 0.0, 10.0, 10.0));
        n.name = "9:41".into();
        let id = d.insert(page, n).unwrap();
        let name = class_name(d.get(id).unwrap());
        assert!(name.starts_with("l-9-41-"), "{name}");
        assert!(!name.starts_with(|c: char| c.is_ascii_digit()), "{name}");
    }

    #[test]
    fn project_name_is_snake_case() {
        let (d, id) = card();
        assert_eq!(project_name(&d, id), "card");
    }
}
