//! Native deck PDF export. Each PDF page preserves the rendered slide appearance;
//! slide content is rasterized, so the PDF is not an editable vector document.

use crate::app::App;
use desigma_doc::{NodeId, NodeKind};

impl App {
    /// All slides uses document page order, then each page's slide order.
    pub fn deck_pdf_slides(&self, all: bool) -> Result<Vec<NodeId>, String> {
        if !self.is_deck() {
            return Err("Open a deck to export slides as PDF.".into());
        }
        let ids: Vec<NodeId> = if all {
            self.doc
                .pages
                .iter()
                .filter_map(|page| self.doc.get(*page))
                .flat_map(|page| page.children.iter().copied())
                .filter(|id| self.doc.get(*id).is_some_and(|node| node.slide.is_some() && matches!(node.kind, NodeKind::Frame(_))))
                .collect()
        } else {
            self.active_slide().into_iter().collect()
        };
        if ids.is_empty() {
            return Err(if all { "This deck has no slides to export." } else { "Select a slide to export." }.into());
        }
        Ok(ids)
    }

    /// Export never changes selection, notes drafts, undo history or document content.
    pub fn deck_pdf_bytes(&self, all: bool) -> Result<Vec<u8>, String> {
        desigma_export::export_pdf(&self.export_doc(), &self.deck_pdf_slides(all)?)
    }

    pub fn export_deck_pdf(&mut self, all: bool) {
        let slides = match self.deck_pdf_slides(all) {
            Ok(slides) => slides,
            Err(error) => {
                self.show_toast(error, true);
                return;
            }
        };
        let name = if all {
            safe_filename(&self.doc.name)
        } else {
            let slide = self.doc.get(slides[0]).map_or("Slide", |node| node.name.as_str());
            safe_filename(&format!("{} - {slide}", self.doc.name))
        };
        crate::canvas::interact::finish_text(self, true);
        let slides_count = slides.len();
        self.pick_file_async(
            move || rfd::FileDialog::new().add_filter("PDF document", &["pdf"]).set_file_name(format!("{name}.pdf")).save_file(),
            move |app, path| {
                let result = app.deck_pdf_bytes(all).and_then(|bytes| std::fs::write(&path, bytes).map_err(|error| format!("Cannot save the PDF: {error}")));
                match result {
                    Ok(()) => app.show_toast(format!("Exported {} {} to {}", slides_count, if slides_count == 1 { "slide" } else { "slides" }, path.display()), false),
                    Err(error) => app.show_toast(error, true),
                }
            },
        );
    }
}

fn safe_filename(name: &str) -> String {
    let name: String =
        name.chars().take(100).map(|character| if character.is_control() || "<>:\"/\\|?*".contains(character) { '-' } else { character }).collect();
    let name = name.trim().trim_matches('.').trim();
    if name.is_empty() {
        return "Deck".into();
    }
    let stem = name.split('.').next().unwrap_or(name).to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4 && (stem.starts_with("COM") || stem.starts_with("LPT")) && stem.as_bytes()[3].is_ascii_digit());
    if reserved {
        format!("Deck-{name}")
    } else {
        name.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use desigma_doc::{Document, Node, Rect, SlideProps};

    #[test]
    fn pdf_scope_uses_active_slide_or_every_document_page_in_order() {
        let mut app = App::new(crate::decks::new_document("Talk"));
        let first = app.active_slide().unwrap();
        app.add_slide();
        let second = app.active_slide().unwrap();
        let page = app.doc.add_page("Appendix");
        let appendix = app.doc.alloc_id();
        let mut node = Node::frame(appendix, Rect::new(0.0, 0.0, 1280.0, 720.0));
        node.slide = Some(SlideProps::default());
        app.doc.insert(page, node).unwrap();
        let ordinary = app.doc.alloc_id();
        app.doc.insert(page, Node::frame(ordinary, Rect::new(0.0, 0.0, 100.0, 100.0))).unwrap();
        assert_eq!(app.deck_pdf_slides(false).unwrap(), vec![second]);
        assert_eq!(app.deck_pdf_slides(true).unwrap(), vec![first, second, appendix]);
        app.doc.pages.swap(0, 1);
        assert_eq!(app.deck_pdf_slides(true).unwrap(), vec![appendix, first, second]);
    }

    #[test]
    fn pdf_export_preserves_document_selection_history_and_unsaved_notes() {
        let mut app = App::new(crate::decks::new_document("Talk"));
        let slide = app.active_slide().unwrap();
        app.select_slide(slide);
        app.decks.notes_draft = "Private speaking points".into();
        let before = app.doc.to_json().unwrap();
        let selection = app.selection.clone();
        let history = (app.hist.can_undo(), app.hist.can_redo());
        let bytes = app.deck_pdf_bytes(false).unwrap();
        assert!(bytes.starts_with(b"%PDF-"));
        assert_eq!(app.doc.to_json().unwrap(), before);
        assert_eq!(app.selection, selection);
        assert_eq!((app.hist.can_undo(), app.hist.can_redo()), history);
        assert_eq!(app.decks.notes_draft, "Private speaking points");
    }

    #[test]
    fn non_decks_and_empty_decks_cannot_export_pdf() {
        let app = App::new(Document::new("Design"));
        assert!(app.deck_pdf_bytes(true).is_err());
        let mut app = App::new(crate::decks::new_document("Empty"));
        app.delete_slide();
        assert!(app.deck_pdf_bytes(false).is_err());
        assert!(app.deck_pdf_bytes(true).is_err());
    }

    #[test]
    fn export_filename_is_safe_and_keeps_readable_unicode() {
        assert_eq!(safe_filename("研究 / review: v2"), "研究 - review- v2");
        assert_eq!(safe_filename(" .. "), "Deck");
        assert_eq!(safe_filename("CON"), "Deck-CON");
        assert_eq!(safe_filename("LPT1.txt"), "Deck-LPT1.txt");
        assert_eq!(safe_filename("Project\nname"), "Project-name");
    }

    #[test]
    #[ignore = "writes review artifacts only when explicitly requested"]
    fn write_demo_pdf_preview() {
        let app = App::new(crate::decks::demo::document());
        let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../out");
        std::fs::create_dir_all(&out).unwrap();
        std::fs::write(out.join("desigma-decks-slide.pdf"), app.deck_pdf_bytes(false).unwrap()).unwrap();
        std::fs::write(out.join("desigma-decks.pdf"), app.deck_pdf_bytes(true).unwrap()).unwrap();
    }
}
