//! AI images: generate an image from a prompt, or regenerate part of an image, with the
//! user's own OpenRouter key.
//!
//! * **Preferences -> AI images…** stores the key in the OS credential store ([`keys`]) and the
//!   chosen model in `settings.json`. The model list comes from OpenRouter.
//! * **Generate image…** sends the prompt, and the selected layers as reference images, and
//!   places the result on the canvas (or fills the selected empty shape with it).
//! * **Edit image with AI…** sends an image layer's picture with a prompt. With a region drawn
//!   on the preview, only that region is taken from the answer ([`compose::paste_region`]),
//!   so the rest of the picture cannot change.
//!
//! Requests run on a background thread ([`api`]); the dialogs in [`ui`] show progress and
//! errors. Nothing is sent until the user presses Generate, and the key never enters a
//! document, a log or a collaboration session.

pub mod api;
pub mod compose;
mod keys;
pub mod ui;

use crate::app::App;
use crate::comments::settings;
use compose::{Mask, Region};
use desigma_doc::{AiVersion, ImageScaleMode, NodeId, NodeKind, Paint, PaintKind};
use std::sync::mpsc::{self, Receiver, TryRecvError};

const MODEL_SETTING: &str = "ai_image_model";
/// Most selected layers sent as references with one request.
const MAX_REFERENCES: usize = 4;
/// Longest side of a layer rendered as a reference.
const REFERENCE_SIDE: f32 = 1024.0;
pub const MAX_PROMPT: usize = 4000;
/// Most pictures kept in a layer's AI history.
const MAX_VERSIONS: usize = 24;
/// Most of them shown in the dialog (the newest).
const SHOWN_VERSIONS: usize = 8;
/// Brush radii of the dialog, in screen pixels.
pub const BRUSHES: [f32; 3] = [10.0, 22.0, 44.0];

/// The model list of the settings dialog.
#[derive(Default)]
pub enum Models {
    #[default]
    Unknown,
    Loading,
    Loaded(Vec<api::Model>),
    Failed(String),
}

/// Preferences -> AI images….
pub struct Settings {
    /// What was typed; empty keeps the stored key.
    pub key: String,
    pub model: String,
    pub models: Models,
    pub error: Option<String>,
    pub scroll: f32,
    pub fresh: bool,
}

/// What a generated image is for.
pub enum Target {
    /// A new image layer in the middle of the view.
    Place,
    /// The image fill of this empty shape.
    Fill(NodeId),
    /// Replaces the image of fill `paint` of `node`.
    Edit { node: NodeId, paint: usize, original: image::RgbaImage },
}

/// The Generate / Edit dialog.
pub struct Prompt {
    pub target: Target,
    pub text: String,
    /// Index in [`compose::ASPECTS`].
    pub aspect: usize,
    /// Selected layers, rendered, sent along as references.
    pub references: Vec<Vec<u8>>,
    /// Edit: what the brush marked to change; nothing marked regenerates the whole picture.
    pub mask: Option<Mask>,
    /// Index in [`BRUSHES`].
    pub brush: usize,
    /// The last point of the stroke being painted, in mask cells.
    pub last: Option<(f32, f32)>,
    /// The mark as a texture; rebuilt when `mask_dirty`.
    pub mask_tex: Option<egui::TextureHandle>,
    pub mask_dirty: bool,
    /// Edit: the layer's earlier pictures (the newest few), to bring one back.
    pub versions: Vec<Thumb>,
    pub preview: Option<egui::TextureHandle>,
    pub error: Option<String>,
    pub running: bool,
    /// When the running request was sent (`App::now`), for the elapsed time.
    pub started: f64,
    pub fresh: bool,
}

/// One picture of a layer's AI history, in the dialog.
pub struct Thumb {
    /// Index in the layer's `ai_versions`.
    pub index: usize,
    pub prompt: String,
    pub current: bool,
    pub tex: egui::TextureHandle,
}

impl Prompt {
    fn new(target: Target, aspect: usize, references: Vec<Vec<u8>>) -> Self {
        Prompt {
            target,
            text: String::new(),
            aspect,
            references,
            mask: None,
            brush: 1,
            last: None,
            mask_tex: None,
            mask_dirty: false,
            versions: Vec::new(),
            preview: None,
            error: None,
            running: false,
            started: 0.0,
            fresh: true,
        }
    }
    /// The box around what the brush marked.
    pub fn region(&self) -> Option<Region> {
        self.mask.as_ref().and_then(Mask::bounds).filter(|r| !r.is_tiny())
    }
}

pub struct AiState {
    /// The API root; tests point it at a local server.
    pub base: String,
    /// The chosen model id (empty until one is picked).
    pub model: String,
    pub settings: Option<Settings>,
    pub prompt: Option<Prompt>,
    listing: Option<Receiver<Result<Vec<api::Model>, String>>>,
    generating: Option<Receiver<Result<Vec<u8>, String>>>,
}

impl Default for AiState {
    fn default() -> Self {
        Self { base: api::BASE.into(), model: String::new(), settings: None, prompt: None, listing: None, generating: None }
    }
}

/// Tests: the key as if it had been saved (the test store, never the real keychain).
#[cfg(test)]
pub fn set_test_key(key: &str) {
    keys::save(key).unwrap();
}

/// Runs `work` on its own thread; `wake` repaints when the answer is in.
fn spawn<T: Send + 'static>(ctx: &egui::Context, work: impl FnOnce() -> T + Send + 'static) -> Receiver<T> {
    let (tx, rx) = mpsc::channel();
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        let _ = tx.send(work());
        ctx.request_repaint();
    });
    rx
}

fn image_paint(image_ref: String) -> Paint {
    Paint { kind: PaintKind::Image { image_ref, scale_mode: ImageScaleMode::Fill }, opacity: 1.0, visible: true, blend: desigma_doc::BlendMode::Normal }
}

/// A document image key for `bytes` (the same bytes get the same key).
fn image_key(bytes: &[u8]) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut h);
    format!("ai-{:016x}", h.finish())
}

/// The instruction sent with an edit: the user's words plus where to change the picture.
pub fn edit_instruction(text: &str, region: Option<Region>) -> String {
    let scope = match region {
        Some(r) => format!("Change only the area {} (measured from the top-left corner) and keep everything outside it exactly as it is.", r.percent()),
        None => "Keep the composition unless the request says otherwise.".to_string(),
    };
    format!("{}\n\nThe attached image is the one to edit. {scope} Return the whole image with the same framing and proportions.", text.trim())
}

impl App {
    /// Reads the saved model (on startup, once the library folder is known).
    pub fn load_ai_settings(&mut self) {
        if let Some(m) = self.lib.as_ref().and_then(|l| settings::load_string(l.root(), MODEL_SETTING)) {
            self.ai.model = m;
        }
    }

    pub fn open_ai_settings(&mut self, ctx: &egui::Context, error: Option<&str>) {
        self.ai.settings = Some(Settings {
            key: String::new(),
            model: self.ai.model.clone(),
            models: Models::Unknown,
            error: error.map(str::to_string),
            scroll: 0.0,
            fresh: true,
        });
        if keys::load().is_some() {
            self.ai_load_models(ctx);
        }
    }

    /// Asks OpenRouter for its image models, with the typed key or else the stored one.
    pub fn ai_load_models(&mut self, ctx: &egui::Context) {
        let Some(s) = self.ai.settings.as_mut() else { return };
        let typed = s.key.trim().to_string();
        let Some(key) = Some(typed).filter(|k| !k.is_empty()).or_else(keys::load) else {
            s.models = Models::Failed("Enter your OpenRouter API key first".into());
            return;
        };
        s.models = Models::Loading;
        let base = self.ai.base.clone();
        self.ai.listing = Some(spawn(ctx, move || api::list_models(&base, &key)));
    }

    /// Stores the key and the model and closes the dialog; on failure the dialog says why.
    pub fn ai_save_settings(&mut self) {
        let Some(mut s) = self.ai.settings.take() else { return };
        if !s.key.trim().is_empty() {
            if let Err(e) = keys::save(&s.key) {
                s.error = Some(e);
                self.ai.settings = Some(s);
                return;
            }
        }
        self.ai.model = s.model.trim().to_string();
        if let Some(l) = &self.lib {
            let _ = settings::save_string(l.root(), MODEL_SETTING, &self.ai.model);
        }
        self.ai.listing = None;
    }

    /// Forgets the stored key.
    pub fn ai_remove_key(&mut self) {
        let result = keys::save("");
        if let Some(s) = self.ai.settings.as_mut() {
            s.key.clear();
            s.models = Models::Unknown;
            s.error = result.err();
        }
        self.ai.listing = None;
    }

    pub fn ai_key_saved(&self) -> bool {
        keys::load().is_some()
    }

    pub fn ai_key_from_env(&self) -> bool {
        keys::from_env()
    }

    /// The selected layer with an image fill that Edit would change.
    pub fn ai_editable(&self) -> Option<(NodeId, usize)> {
        let [id] = self.selection[..] else { return None };
        let n = self.doc.get(id)?;
        let paint = n.fills.iter().position(|p| p.visible && matches!(p.kind, PaintKind::Image { .. }))?;
        Some((id, paint))
    }

    /// True when a key and a model are set; otherwise opens the settings saying what is missing.
    fn ai_ready(&mut self, ctx: &egui::Context) -> bool {
        if self.read_only() {
            self.show_toast("This file is read-only", true);
            return false;
        }
        if keys::load().is_none() {
            self.open_ai_settings(ctx, Some("Add your OpenRouter API key to generate images."));
            return false;
        }
        if self.ai.model.is_empty() {
            self.open_ai_settings(ctx, Some("Choose an image model first."));
            return false;
        }
        true
    }

    /// Generate image…: fills the selected empty shape, or places a new image with the selected
    /// layers as references.
    pub fn open_ai_generate(&mut self, ctx: &egui::Context) {
        if !self.ai_ready(ctx) {
            return;
        }
        let empty_shape = |app: &App, id: NodeId| {
            app.doc.get(id).is_some_and(|n| {
                matches!(n.kind, NodeKind::Rectangle | NodeKind::Ellipse(_) | NodeKind::Frame(_))
                    && n.children.is_empty()
                    && !n.fills.iter().any(|p| matches!(p.kind, PaintKind::Image { .. }))
            })
        };
        let (target, references, aspect) = match self.selection[..] {
            [id] if empty_shape(self, id) => {
                let size = self.doc.get(id).map(|n| n.size).unwrap_or_default();
                (Target::Fill(id), Vec::new(), compose::nearest_aspect(size.x, size.y))
            }
            _ => (Target::Place, self.ai_references(), 0),
        };
        self.ai.prompt = Some(Prompt::new(target, aspect, references));
    }

    /// The selected layers as PNG files, at most [`MAX_REFERENCES`].
    fn ai_references(&self) -> Vec<Vec<u8>> {
        let doc = self.export_doc();
        self.selection
            .iter()
            .take(MAX_REFERENCES)
            .filter_map(|id| {
                let b = desigma_export::render_bounds(&doc, *id);
                if b.w <= 0.0 || b.h <= 0.0 {
                    return None;
                }
                let scale = (REFERENCE_SIDE / b.w.max(b.h)).min(1.0);
                desigma_export::render_export(&doc, *id, scale)?.encode_png().ok()
            })
            .collect()
    }

    /// Edit image with AI…, for the selected image layer.
    pub fn open_ai_edit(&mut self, ctx: &egui::Context) {
        let Some((node, paint)) = self.ai_editable() else {
            self.show_toast("Select one layer with an image fill", true);
            return;
        };
        if !self.ai_ready(ctx) {
            return;
        }
        self.ai_edit_dialog(ctx, node, paint, String::new());
    }

    /// Opens the Edit dialog on the layer's current picture, with `text` in the prompt field.
    fn ai_edit_dialog(&mut self, ctx: &egui::Context, node: NodeId, paint: usize, text: String) {
        let bytes = match self.doc.get(node).and_then(|n| n.fills.get(paint)).map(|p| &p.kind) {
            Some(PaintKind::Image { image_ref, .. }) => self.doc.images.get(image_ref),
            _ => None,
        };
        let original = match bytes.ok_or_else(|| "The image of this layer is missing".to_string()).and_then(|b| compose::decode(b)) {
            Ok(img) => img,
            Err(e) => {
                self.show_toast(e, true);
                return;
            }
        };
        let (w, h) = original.dimensions();
        // the preview is a small copy; the edit works on the full picture
        let k = (960.0 / w.max(h) as f32).min(1.0);
        let small = image::imageops::resize(&original, ((w as f32 * k) as u32).max(1), ((h as f32 * k) as u32).max(1), image::imageops::FilterType::Triangle);
        let color = egui::ColorImage::from_rgba_unmultiplied([small.width() as usize, small.height() as usize], small.as_raw());
        let mut p = Prompt::new(Target::Edit { node, paint, original }, compose::nearest_aspect(w as f32, h as f32), Vec::new());
        p.preview = Some(ctx.load_texture("ai-edit-preview", color, egui::TextureOptions::LINEAR));
        p.mask = Some(Mask::new(small.width(), small.height()));
        p.versions = self.ai_thumbs(ctx, node, paint);
        p.text = text;
        self.ai.prompt = Some(p);
    }

    /// Small pictures of the layer's AI history (the newest few), oldest first.
    fn ai_thumbs(&self, ctx: &egui::Context, node: NodeId, paint: usize) -> Vec<Thumb> {
        let Some(n) = self.doc.get(node) else { return Vec::new() };
        let now = match n.fills.get(paint).map(|p| &p.kind) {
            Some(PaintKind::Image { image_ref, .. }) => image_ref.as_str(),
            _ => "",
        };
        let skip = n.ai_versions.len().saturating_sub(SHOWN_VERSIONS);
        n.ai_versions
            .iter()
            .enumerate()
            .skip(skip)
            .filter_map(|(index, v)| {
                let img = compose::decode(self.doc.images.get(&v.image_ref)?).ok()?;
                let k = (128.0 / img.width().max(img.height()) as f32).min(1.0);
                let (w, h) = (((img.width() as f32 * k) as u32).max(1), ((img.height() as f32 * k) as u32).max(1));
                let small = image::imageops::thumbnail(&img, w, h);
                let color = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], small.as_raw());
                let tex = ctx.load_texture(format!("ai-version-{index}"), color, egui::TextureOptions::LINEAR);
                Some(Thumb { index, prompt: v.prompt.clone(), current: v.image_ref == now, tex })
            })
            .collect()
    }

    /// Shows version `index` of the layer's AI history again (one undo step) and reopens the
    /// dialog on it. The history itself is kept, so any version can be picked again.
    pub fn ai_restore_version(&mut self, ctx: &egui::Context, index: usize) {
        let Some(p) = self.ai.prompt.take() else { return };
        let Target::Edit { node, paint, .. } = p.target else {
            self.ai.prompt = Some(p);
            return;
        };
        let Some(key) = self.doc.get(node).and_then(|n| n.ai_versions.get(index)).map(|v| v.image_ref.clone()) else { return };
        self.edit_nodes(&[node], |n| {
            if let Some(PaintKind::Image { image_ref, .. }) = n.fills.get_mut(paint).map(|p| &mut p.kind) {
                *image_ref = key.clone();
            }
        });
        self.ai_edit_dialog(ctx, node, paint, p.text);
    }

    /// Sends the open dialog's request.
    pub fn ai_submit(&mut self, ctx: &egui::Context) {
        let Some(key) = keys::load() else { return };
        let (base, model) = (self.ai.base.clone(), self.ai.model.clone());
        let Some(p) = self.ai.prompt.as_mut() else { return };
        let text = p.text.trim().to_string();
        if p.running || text.is_empty() {
            return;
        }
        if text.len() > MAX_PROMPT {
            p.error = Some(format!("The prompt is longer than {MAX_PROMPT} characters"));
            return;
        }
        let aspect = Some(compose::ASPECTS[p.aspect.min(compose::ASPECTS.len() - 1)].to_string()).filter(|_| p.aspect != 0);
        let mask = p.mask.clone().filter(|m| !m.is_empty());
        let region = mask.as_ref().and_then(Mask::bounds);
        let job: Box<dyn FnOnce() -> Result<Vec<u8>, String> + Send> = match &p.target {
            Target::Edit { original, .. } => {
                let original = original.clone();
                Box::new(move || {
                    let req = api::Request {
                        model,
                        prompt: edit_instruction(&text, region),
                        aspect_ratio: aspect,
                        references: vec![compose::reference_png(&original)?],
                    };
                    let generated = compose::decode(&api::generate(&base, &key, &req)?)?;
                    compose::encode_png(&match &mask {
                        Some(m) => compose::paste_mask(&original, &generated, m),
                        None => compose::fit_to(&original, &generated),
                    })
                })
            }
            Target::Place | Target::Fill(_) => {
                let references = p.references.clone();
                Box::new(move || api::generate(&base, &key, &api::Request { model, prompt: text, aspect_ratio: aspect, references }))
            }
        };
        p.running = true;
        p.started = self.now;
        p.error = None;
        self.ai.generating = Some(spawn(ctx, job));
    }

    /// Cancel: the dialog closes and a running request's answer is dropped.
    pub fn ai_cancel(&mut self) {
        self.ai.prompt = None;
        self.ai.generating = None;
    }

    /// Puts a finished image into the document, as one undo step.
    fn ai_apply(&mut self, target: Target, bytes: Vec<u8>, prompt: String) -> Result<(), String> {
        // every generation is remembered on its layer, so an earlier picture can be brought back
        let remember = |n: &mut desigma_doc::Node, image_ref: &str, prompt: &str| {
            n.ai_versions.push(AiVersion { image_ref: image_ref.to_string(), prompt: prompt.to_string() });
            let extra = n.ai_versions.len().saturating_sub(MAX_VERSIONS);
            n.ai_versions.drain(..extra);
        };
        let (bytes, _, _) = crate::canvas::interact::prepare_image(bytes)?;
        match target {
            Target::Place => {
                let page = self.view.to_page(self.canvas_rect.min, self.canvas_rect.center());
                let before = self.doc.nodes.len();
                let id = crate::canvas::interact::place_image_bytes(self, page, "AI image", bytes);
                let Some(id) = id.filter(|_| self.doc.nodes.len() > before) else { return Err("The image could not be placed".into()) };
                // centre it on the view instead of hanging from the centre point
                self.edit_nodes(&[id], |n| {
                    n.transform.tx -= n.size.x / 2.0;
                    n.transform.ty -= n.size.y / 2.0;
                    if let Some(PaintKind::Image { image_ref, .. }) = n.fills.first().map(|p| p.kind.clone()) {
                        remember(n, &image_ref, &prompt);
                    }
                });
            }
            Target::Fill(id) => {
                let key = image_key(&bytes);
                self.doc.images.insert(key.clone(), bytes);
                self.edit_nodes(&[id], |n| {
                    n.fills = vec![image_paint(key.clone())];
                    remember(n, &key, &prompt);
                });
            }
            Target::Edit { node, paint, .. } => {
                let key = image_key(&bytes);
                self.doc.images.insert(key.clone(), bytes);
                self.edit_nodes(&[node], |n| {
                    let Some(PaintKind::Image { image_ref, .. }) = n.fills.get_mut(paint).map(|p| &mut p.kind) else { return };
                    let before = std::mem::replace(image_ref, key.clone());
                    // the picture the layer had before its first generation opens the history
                    if !n.ai_versions.iter().any(|v| v.image_ref == before) {
                        remember(n, &before, "");
                    }
                    remember(n, &key, &prompt);
                });
            }
        }
        Ok(())
    }
}

/// Collects finished background work. Called every frame.
pub fn pump(app: &mut App) {
    if let Some(rx) = &app.ai.listing {
        match rx.try_recv() {
            Ok(result) => {
                app.ai.listing = None;
                if let Some(s) = app.ai.settings.as_mut() {
                    s.models = match result {
                        Ok(m) => Models::Loaded(m),
                        Err(e) => Models::Failed(e),
                    };
                }
            }
            Err(TryRecvError::Disconnected) => app.ai.listing = None,
            Err(TryRecvError::Empty) => {}
        }
    }
    let done = match &app.ai.generating {
        Some(rx) => match rx.try_recv() {
            Ok(result) => Some(result),
            Err(TryRecvError::Disconnected) => Some(Err("The request stopped unexpectedly".to_string())),
            Err(TryRecvError::Empty) => None,
        },
        None => None,
    };
    let Some(result) = done else { return };
    app.ai.generating = None;
    let Some(mut p) = app.ai.prompt.take() else { return };
    p.running = false;
    let bytes = match result {
        Ok(bytes) => bytes,
        Err(e) => {
            // the request failed: the dialog stays, with the reason, so the prompt is not lost
            p.error = Some(e);
            app.ai.prompt = Some(p);
            return;
        }
    };
    // the layer may be gone by now (deleted, or another file was opened)
    let alive = match &p.target {
        Target::Place => true,
        Target::Fill(id) | Target::Edit { node: id, .. } => app.doc.get(*id).is_some(),
    };
    let outcome = if alive { app.ai_apply(p.target, bytes, p.text.trim().to_string()) } else { Err("The layer this image was for is gone".into()) };
    match outcome {
        Ok(()) => app.show_toast("Image generated", false),
        Err(e) => app.show_toast(e, true),
    }
}

/// `--panel ai-settings|ai-generate|ai-edit|ai-running`: the dialogs (and the canvas effect of a
/// running request) with sample content, for screenshots.
/// Nothing is read from the keychain or sent anywhere.
pub fn preset(app: &mut App, ctx: &egui::Context, panel: &str) {
    let sample = |id: &str, name: &str, references| api::Model { id: id.into(), name: name.into(), references };
    match panel {
        "ai-settings" => {
            let models = vec![
                sample("google/gemini-2.5-flash-image", "Google: Gemini 2.5 Flash Image", true),
                sample("openai/gpt-image-1", "OpenAI: GPT Image 1", true),
                sample("black-forest-labs/flux.2-pro", "Black Forest Labs: FLUX.2 Pro", true),
                sample("bytedance-seed/seedream-4.5", "ByteDance Seed: Seedream 4.5", true),
            ];
            app.ai.settings = Some(Settings {
                key: String::new(),
                model: "openai/gpt-image-1".into(),
                models: Models::Loaded(models),
                error: None,
                scroll: 0.0,
                fresh: false,
            });
        }
        _ => {
            let (w, h) = (600u32, 360u32);
            let original = image::RgbaImage::from_fn(w, h, |x, y| {
                let sky = y < h * 55 / 100;
                let t = x as f32 / w as f32;
                if sky {
                    image::Rgba([(110.0 + 90.0 * t) as u8, (170.0 + 40.0 * t) as u8, 235, 255])
                } else {
                    image::Rgba([(60.0 + 40.0 * t) as u8, (120.0 + 30.0 * t) as u8, 70, 255])
                }
            });
            let edit = panel == "ai-edit";
            let color = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], original.as_raw());
            let mut p = Prompt::new(
                if edit { Target::Edit { node: NodeId(0), paint: 0, original: original.clone() } } else { Target::Place },
                if edit { 0 } else { 2 },
                Vec::new(),
            );
            p.text =
                if edit { "Replace the sky with a warm sunset".into() } else { "A product photo of a ceramic mug on a wooden table, soft daylight".into() };
            p.running = panel == "ai-running";
            p.fresh = false;
            if edit {
                p.preview = Some(ctx.load_texture("ai-edit-preview", color, egui::TextureOptions::LINEAR));
                // the sky, brushed over by hand
                let mut mask = Mask::new(w, h);
                for (i, y) in [40.0f32, 95.0, 150.0].into_iter().enumerate() {
                    mask.stroke((30.0 + 25.0 * i as f32, y), (570.0 - 30.0 * i as f32, y + 12.0), 34.0);
                }
                p.mask = Some(mask);
                p.mask_dirty = true;
                // three earlier pictures, as a history would show them
                for (index, shade) in [(0usize, 1.0f32), (1, 0.8), (2, 0.6)] {
                    let small = image::imageops::thumbnail(&original, 120, 72);
                    let px: Vec<u8> = small.as_raw().chunks(4).flat_map(|c| [(c[0] as f32 * shade) as u8, (c[1] as f32 * shade) as u8, c[2], 255]).collect();
                    let tex =
                        ctx.load_texture(format!("ai-version-{index}"), egui::ColorImage::from_rgba_unmultiplied([120, 72], &px), egui::TextureOptions::LINEAR);
                    p.versions.push(Thumb { index, prompt: if index == 0 { String::new() } else { "An earlier request".into() }, current: index == 2, tex });
                }
            }
            app.ai.prompt = Some(p);
        }
    }
}
