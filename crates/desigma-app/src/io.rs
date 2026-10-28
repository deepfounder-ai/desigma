//! Opening and saving documents.

use desigma_doc::{Document, Vec2};
use std::path::{Path, PathBuf};

pub const NATIVE_EXT: &str = "desigma";

/// Loads a document from disk, dispatching on the file extension.
pub fn load_path(path: &Path) -> Result<Document, String> {
    load_path_noted(path).map(|(doc, _)| doc)
}

/// [`load_path`], plus notes on what an import could not carry over (empty for every format
/// but `.pen`).
pub fn load_path_noted(path: &Path) -> Result<(Document, Vec<String>), String> {
    let file = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    guard(&file, || load_unguarded(path))
}

/// Runs an import so that a bug in an importer reports an error instead of closing the app.
/// The panic hook has already written the details to the crash log.
fn guard<T>(file: &str, load: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(load)).unwrap_or_else(|payload| {
        Err(format!(
            "Cannot open {file}: desigma hit an internal error ({}). Details were saved to {}; please attach that file to a bug report.",
            crate::crash::payload_message(payload.as_ref()),
            crate::crash::log_path().display()
        ))
    })
}

fn load_unguarded(path: &Path) -> Result<(Document, Vec<String>), String> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let mut notes = Vec::new();
    let mut doc = match ext.as_str() {
        "fig" => {
            let bytes = std::fs::read(path).map_err(|e| format!("Cannot read {}: {e}", path.display()))?;
            desigma_fig::import_fig(&bytes).map_err(|e| format!("Cannot import .fig: {e}"))?
        }
        "pen" => {
            let bytes = std::fs::read(path).map_err(|e| format!("Cannot read {}: {e}", path.display()))?;
            let opts = desigma_pen::Options { base_dir: path.parent().map(Path::to_path_buf), ..Default::default() };
            let import = desigma_pen::import_with(&bytes, &opts).map_err(|e| format!("Cannot import .pen: {e}"))?;
            notes = import.warnings;
            import.document
        }
        "json" => {
            let text = std::fs::read_to_string(path).map_err(|e| format!("Cannot read {}: {e}", path.display()))?;
            match desigma_fig::import_rest_json(&text) {
                Ok(d) => d,
                Err(fig_err) => Document::from_json(&text).map_err(|e| format!("Not a Figma REST file ({fig_err}) nor a desigma file ({e})"))?,
            }
        }
        "svg" => {
            let text = std::fs::read_to_string(path).map_err(|e| format!("Cannot read {}: {e}", path.display()))?;
            let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("svg");
            let mut doc = Document::new(name);
            let page = doc.add_page("Page 1");
            desigma_svg::import_svg_named(&text, name, &mut doc, page, Vec2::ZERO).map_err(|e| format!("Cannot import SVG: {e}"))?;
            doc
        }
        _ => {
            let text = std::fs::read_to_string(path).map_err(|e| format!("Cannot read {}: {e}", path.display()))?;
            Document::from_json(&text).map_err(|e| format!("Invalid desigma file: {e}"))?
        }
    };
    if doc.name.is_empty() {
        doc.name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("Untitled").to_string();
    }
    ensure_page(&mut doc);
    Ok((doc, notes))
}

pub fn ensure_page(doc: &mut Document) {
    doc.pages.retain(|p| doc.nodes.contains_key(p));
    if doc.pages.is_empty() {
        doc.add_page("Page 1");
    }
}

/// Writes the native format; returns the final path (extension enforced).
pub fn save_path(doc: &Document, path: &Path) -> Result<PathBuf, String> {
    let mut p = path.to_path_buf();
    if p.extension().is_none() {
        p.set_extension(NATIVE_EXT);
    }
    let json = doc.to_json().map_err(|e| format!("Cannot serialize: {e}"))?;
    std::fs::write(&p, json).map_err(|e| format!("Cannot write {}: {e}", p.display()))?;
    Ok(p)
}

pub fn pick_open() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .add_filter("Design files", &["fig", "pen", "json", "desigma", "svg"])
        .add_filter("Figma file", &["fig"])
        .add_filter("Pencil file", &["pen"])
        .add_filter("SVG", &["svg"])
        .add_filter("JSON", &["json"])
        .add_filter("desigma", &[NATIVE_EXT])
        .pick_file()
}

pub fn pick_save(default_name: &str) -> Option<PathBuf> {
    rfd::FileDialog::new().add_filter("desigma", &[NATIVE_EXT]).set_file_name(format!("{default_name}.{NATIVE_EXT}")).save_file()
}

pub fn pick_export(default_name: &str) -> Option<PathBuf> {
    rfd::FileDialog::new().add_filter("PNG", &["png"]).set_file_name(format!("{default_name}.png")).save_file()
}

#[cfg(test)]
mod tests {
    use super::*;
    use desigma_doc::{Node, Rect};

    fn tmp(name: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("desigma_test_{}_{name}", std::process::id()));
        p
    }

    #[test]
    fn native_roundtrip() {
        let mut d = Document::new("Saved");
        let page = d.add_page("P");
        let id = d.alloc_id();
        d.insert(page, Node::rectangle(id, Rect::new(1.0, 2.0, 3.0, 4.0))).unwrap();
        let p = save_path(&d, &tmp("a")).unwrap();
        assert_eq!(p.extension().unwrap(), NATIVE_EXT);
        let back = load_path(&p).unwrap();
        assert_eq!(back.name, "Saved");
        assert_eq!(back.nodes.len(), 2);
        std::fs::remove_file(p).ok();
    }

    #[test]
    fn json_falls_back_to_native() {
        let mut d = Document::new("J");
        d.add_page("P");
        let p = tmp("b.json");
        std::fs::write(&p, d.to_json().unwrap()).unwrap();
        let back = load_path(&p).unwrap();
        assert_eq!(back.name, "J");
        std::fs::remove_file(p).ok();
    }

    #[test]
    fn errors_are_readable() {
        let p = tmp("c.desigma");
        std::fs::write(&p, "not json").unwrap();
        assert!(load_path(&p).unwrap_err().contains("Invalid desigma file"));
        std::fs::remove_file(p).ok();
        assert!(load_path(Path::new("/definitely/missing.fig")).is_err());
    }

    #[test]
    fn svg_loads_as_a_document_named_after_the_file() {
        let p = tmp("icon.svg");
        std::fs::write(&p, r##"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><rect width="8" height="8" fill="#f00"/></svg>"##).unwrap();
        let d = load_path(&p).unwrap();
        std::fs::remove_file(&p).ok();
        assert!(d.name.ends_with("icon"));
        assert_eq!(d.pages.len(), 1);
        let page = d.get(d.pages[0]).unwrap();
        assert_eq!(page.name, "Page 1");
        assert_eq!(page.children.len(), 1);
        assert_eq!(d.get(page.children[0]).unwrap().name, d.name);
    }

    #[test]
    fn pen_loads_with_notes_and_is_not_saved_back() {
        let p = tmp("design.pen");
        std::fs::write(
            &p,
            r##"{"version":"2.21","children":[{"id":"a","type":"frame","name":"Card","width":80,"height":40,"fill":"#FF0000","children":[{"id":"s","type":"script","width":10,"height":10}]}]}"##,
        )
        .unwrap();
        let (d, notes) = load_path_noted(&p).unwrap();
        std::fs::remove_file(&p).ok();
        assert!(d.name.ends_with("design"));
        let page = d.get(d.pages[0]).unwrap();
        assert_eq!(page.children.len(), 1);
        assert_eq!(d.get(page.children[0]).unwrap().name, "Card");
        assert_eq!(notes.len(), 1);
        assert!(notes[0].contains("script"));
        let bad = tmp("bad.pen");
        std::fs::write(&bad, [0u8, 1, 2]).unwrap();
        assert!(load_path(&bad).unwrap_err().contains("Cannot import .pen"));
        std::fs::remove_file(&bad).ok();
    }

    #[test]
    fn importer_panics_become_errors() {
        let err = guard("broken.fig", || -> Result<(), String> { panic!("index out of bounds") }).unwrap_err();
        assert!(err.starts_with("Cannot open broken.fig: desigma hit an internal error (index out of bounds)"));
        assert!(err.contains("crash.log"));
        assert_eq!(guard("ok.fig", || Ok::<_, String>(3)), Ok(3));
    }

    #[test]
    fn empty_doc_gets_a_page() {
        let mut d = Document::new("E");
        ensure_page(&mut d);
        assert_eq!(d.pages.len(), 1);
    }
}
