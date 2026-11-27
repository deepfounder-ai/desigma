//! Animation export from the app: the background job behind the inspector's "Export animation"
//! block (GIF / WebM / PNG frames, with progress and cancel) and the headless
//! `desigma --export-animation` command.

use crate::app::App;
use desigma_doc::{Document, NodeId};
use desigma_export::anim::{export_animation, write_png_dir, AnimExport, AnimFormat, Animation, Progress};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{mpsc, Arc};

/// The fps choices of the dropdown (0 = the page's timeline fps).
pub const FPS_CHOICES: [u32; 8] = [0, 12, 15, 24, 25, 30, 50, 60];
/// The scale choices of the dropdown.
pub const SCALE_CHOICES: [f32; 5] = [0.5, 1.0, 1.5, 2.0, 3.0];

/// What the export block remembers between exports.
#[derive(Default)]
pub struct AnimExportUi {
    pub opts: AnimExport,
    job: Option<Job>,
}

struct Job {
    cancel: Arc<AtomicBool>,
    done: Arc<AtomicU32>,
    total: Arc<AtomicU32>,
    result: mpsc::Receiver<Result<(PathBuf, usize), String>>,
    name: String,
}

impl AnimExportUi {
    pub fn running(&self) -> bool {
        self.job.is_some()
    }

    /// Fraction finished (0..=1) of the running export.
    pub fn progress(&self) -> f32 {
        match &self.job {
            Some(j) => {
                let t = j.total.load(Ordering::Relaxed).max(1);
                (j.done.load(Ordering::Relaxed) as f32 / t as f32).clamp(0.0, 1.0)
            }
            None => 0.0,
        }
    }

    pub fn cancel(&mut self) {
        if let Some(j) = &self.job {
            j.cancel.store(true, Ordering::Relaxed);
        }
    }
}

/// Short label of a format in the dropdown.
pub fn format_label(f: AnimFormat) -> &'static str {
    match f {
        AnimFormat::Gif => "GIF",
        AnimFormat::Webm => "WebM",
        AnimFormat::PngZip => "PNG",
    }
}

pub fn fps_label(fps: u32) -> String {
    if fps == 0 {
        "Auto".into()
    } else {
        fps.to_string()
    }
}

/// `1x`, `1.5x`.
pub fn scale_label(s: f32) -> String {
    format!("{}x", s)
}

fn write_job(doc: &Document, node: NodeId, opts: &AnimExport, name: &str, path: &Path, progress: &mut dyn FnMut(Progress) -> bool) -> Result<usize, String> {
    let bytes = export_animation(doc, node, opts, name, Some(progress)).map_err(|e| e.to_string())?;
    // write next to the target first so a failed or cancelled export never leaves half a file
    let tmp = path.with_extension(format!("{}.part", opts.format.extension()));
    std::fs::write(&tmp, &bytes).map_err(|e| format!("Cannot write {}: {e}", path.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("Cannot write {}: {e}", path.display())
    })?;
    Ok(bytes.len())
}

impl App {
    /// Whether the export block of the inspector applies to `node` (its page is animated).
    pub fn has_motion_export(&self, node: NodeId) -> bool {
        desigma_export::anim::has_motion(&self.doc, node)
    }

    /// "Export animation": asks for a file, then renders and encodes on a background thread.
    pub fn export_animation_dialog(&mut self, node: NodeId) {
        if self.canvas.motion.export.running() {
            return;
        }
        let name = self.doc.get(node).map(|n| n.name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| self.doc.name.clone());
        let f = self.canvas.motion.export.opts.format;
        let file = format!("{}.{}", desigma_export::sanitize_file_name(&name), f.extension());
        let label = f.label();
        let ext = f.extension();
        self.pick_file_async(
            move || rfd::FileDialog::new().add_filter(label, &[ext]).set_file_name(&file).save_file(),
            move |app, path| app.export_animation_to(node, path),
        );
    }

    /// Starts the background export of `node` to `path` with the remembered settings.
    pub fn export_animation_to(&mut self, node: NodeId, path: PathBuf) {
        if self.canvas.motion.export.running() {
            return;
        }
        let name = self.doc.get(node).map(|n| n.name.clone()).unwrap_or_default();
        let doc = self.export_doc().into_owned();
        let opts = self.canvas.motion.export.opts.clone();
        let (cancel, done, total) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicU32::new(0)), Arc::new(AtomicU32::new(1)));
        let (tx, rx) = mpsc::channel();
        let (c, d, t) = (cancel.clone(), done.clone(), total.clone());
        let out = path.clone();
        let file_name = name.clone();
        std::thread::spawn(move || {
            let mut cb = |p: Progress| {
                d.store(p.done, Ordering::Relaxed);
                t.store(p.total.max(1), Ordering::Relaxed);
                !c.load(Ordering::Relaxed)
            };
            let r = write_job(&doc, node, &opts, &file_name, &out, &mut cb).map(|n| (out, n));
            let _ = tx.send(r);
        });
        self.canvas.motion.export.job = Some(Job { cancel, done, total, result: rx, name });
    }

    /// Per-frame: reports a finished export and keeps the progress bar moving.
    pub fn poll_anim_export(&mut self, ctx: &egui::Context) {
        let Some(job) = &self.canvas.motion.export.job else { return };
        match job.result.try_recv() {
            Ok(r) => {
                let name = job.name.clone();
                self.canvas.motion.export.job = None;
                match r {
                    Ok((path, bytes)) => self.show_toast(format!("Exported {} ({})", path.display(), human_size(bytes)), false),
                    Err(e) if e == "export cancelled" => self.show_toast(format!("Export of {name} cancelled"), false),
                    Err(e) => self.show_toast(format!("Export failed: {e}"), true),
                }
            }
            Err(mpsc::TryRecvError::Empty) => ctx.request_repaint_after(std::time::Duration::from_millis(100)),
            Err(mpsc::TryRecvError::Disconnected) => {
                self.canvas.motion.export.job = None;
                self.show_toast("Export failed", true);
            }
        }
    }
}

pub fn human_size(bytes: usize) -> String {
    if bytes >= 1 << 20 {
        format!("{:.1} MB", bytes as f64 / (1 << 20) as f64)
    } else {
        format!("{} KB", bytes.div_ceil(1024))
    }
}

// ------------------------------------------------------------------------------- the CLI

/// The layer a headless export animates: the first layer called `name` (screens first), else the
/// first page.
fn target(doc: &Document, name: Option<&str>) -> Result<NodeId, String> {
    match name {
        Some(name) => {
            let named = |top: bool| doc.nodes.values().find(|n| n.name == name && (!top || n.parent.is_some_and(|p| doc.pages.contains(&p)))).map(|n| n.id);
            named(true).or_else(|| named(false)).ok_or_else(|| format!("no layer named '{name}'"))
        }
        None => doc
            .pages
            .iter()
            .copied()
            .find(|p| desigma_export::anim::has_motion(doc, *p))
            .or_else(|| doc.pages.first().copied())
            .ok_or_else(|| "the file has no pages".into()),
    }
}

/// `desigma --export-animation FILE --format gif|webm|png --out PATH [--fps N] [--scale S] [--dither] [--select NAME]`.
/// `png` writes a `.zip` when `out` ends in `.zip`, else a folder of numbered PNGs.
pub fn run_cli(
    file: &Path,
    format: Option<&str>,
    out: Option<&Path>,
    fps: Option<u32>,
    scale: Option<f32>,
    select: Option<&str>,
    dither: bool,
) -> Result<String, String> {
    let doc = crate::io::load_path(file)?;
    let node = target(&doc, select)?;
    let out = out.ok_or("--export-animation needs --out PATH")?;
    let fmt = match format {
        Some(f) => AnimFormat::parse(f).ok_or_else(|| format!("unknown animation format '{f}' (expected gif, webm or png)"))?,
        None => out.extension().and_then(|e| e.to_str()).and_then(AnimFormat::parse).unwrap_or(AnimFormat::Gif),
    };
    if !desigma_export::anim::has_motion(&doc, node) {
        return Err("nothing is animated on that page (add keyframes or an animation preset first)".into());
    }
    let opts = AnimExport { format: fmt, fps: fps.unwrap_or(0), scale: scale.unwrap_or(1.0), gif_dither: dither, ..Default::default() };
    let name = doc.get(node).map(|n| n.name.clone()).unwrap_or_default();
    let started = std::time::Instant::now();
    let mut last = 0u32;
    let mut cb = |p: Progress| {
        let pct = p.done * 100 / p.total.max(1);
        if pct / 10 != last / 10 {
            eprintln!("desigma: {pct}%");
        }
        last = pct;
        true
    };
    if fmt == AnimFormat::PngZip && out.extension().is_none_or(|e| !e.eq_ignore_ascii_case("zip")) {
        let mut anim = Animation::new(&doc, node, opts.scale).map_err(|e| e.to_string())?;
        let fps = opts.effective_fps(&anim.timeline());
        let files = write_png_dir(&mut anim, fps, &name, out, Some(&mut cb)).map_err(|e| e.to_string())?;
        return Ok(format!("wrote {} frames to {} in {:.1}s", files.len(), out.display(), started.elapsed().as_secs_f32()));
    }
    let bytes = export_animation(&doc, node, &opts, &name, Some(&mut cb)).map_err(|e| e.to_string())?;
    std::fs::write(out, &bytes).map_err(|e| format!("Cannot write {}: {e}", out.display()))?;
    Ok(format!("wrote {} ({}) in {:.1}s", out.display(), human_size(bytes.len()), started.elapsed().as_secs_f32()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use desigma_doc::motion::{AnimProperty, AnimValue, Ease, TimelineSettings};
    use desigma_doc::{Color, Node, Paint, Rect, Vec2};

    fn animated() -> (Document, NodeId, NodeId) {
        let mut d = Document::new("anim");
        let page = d.add_page("P");
        let frame = d.alloc_id();
        let mut f = Node::frame(frame, Rect::new(0.0, 0.0, 64.0, 48.0));
        f.name = "Hero".into();
        f.fills = vec![Paint::solid(Color::WHITE)];
        d.insert(page, f).unwrap();
        let sq = d.alloc_id();
        let mut n = Node::rectangle(sq, Rect::new(0.0, 10.0, 16.0, 16.0));
        n.fills = vec![Paint::solid(Color::rgb(0, 128, 255))];
        let pos = n.motion.ensure_track(AnimProperty::Position);
        pos.set_key(0, AnimValue::Vec2(Vec2::new(0.0, 10.0)), Some(Ease::Linear));
        pos.set_key(500, AnimValue::Vec2(Vec2::new(40.0, 10.0)), None);
        d.insert(frame, n).unwrap();
        d.timelines.insert(page, TimelineSettings { duration_ms: 500, fps: 20, looping: true });
        (d, page, frame)
    }

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("desigma-anim-app-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    #[test]
    fn cli_writes_a_gif_from_a_saved_file() {
        let (d, _p, _f) = animated();
        let src = tmp("hero.desigma");
        crate::io::save_path(&d, &src).unwrap();
        let out = tmp("hero.gif");
        let msg = run_cli(&src, Some("gif"), Some(&out), Some(10), None, Some("Hero"), false).unwrap();
        assert!(msg.contains("hero.gif"), "{msg}");
        let bytes = std::fs::read(&out).unwrap();
        let mut dec = gif::DecodeOptions::new().read_info(std::io::Cursor::new(bytes)).unwrap();
        assert_eq!((dec.width(), dec.height()), (64, 48));
        let mut n = 0;
        while dec.read_next_frame().unwrap().is_some() {
            n += 1;
        }
        assert_eq!(n, 5, "500 ms at 10 fps");
    }

    #[test]
    fn cli_png_goes_to_a_zip_or_a_folder_and_rejects_bad_input() {
        let (d, _p, _f) = animated();
        let src = tmp("p.desigma");
        crate::io::save_path(&d, &src).unwrap();
        let zip = tmp("frames.zip");
        run_cli(&src, Some("png"), Some(&zip), Some(10), Some(2.0), None, false).unwrap();
        assert_eq!(&std::fs::read(&zip).unwrap()[..2], b"PK");
        let dir = tmp("frames");
        run_cli(&src, Some("png"), Some(&dir), Some(10), None, None, false).unwrap();
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 5);
        assert!(run_cli(&src, Some("mov"), Some(&zip), None, None, None, false).unwrap_err().contains("unknown animation format"));
        assert!(run_cli(&src, Some("gif"), Some(&zip), None, None, Some("Nope"), false).unwrap_err().contains("no layer named"));
        assert!(run_cli(&src, Some("gif"), None, None, None, None, false).unwrap_err().contains("--out"));
        // a file without animation says so
        let mut plain = Document::new("plain");
        let pg = plain.add_page("P");
        let id = plain.alloc_id();
        plain.insert(pg, Node::rectangle(id, Rect::new(0.0, 0.0, 5.0, 5.0))).unwrap();
        let ps = tmp("plain.desigma");
        crate::io::save_path(&plain, &ps).unwrap();
        assert!(run_cli(&ps, Some("gif"), Some(&tmp("x.gif")), None, None, None, false).unwrap_err().contains("nothing is animated"));
    }

    #[test]
    fn background_export_reports_progress_and_writes_the_file() {
        let (d, _p, frame) = animated();
        let mut app = App::new(d);
        app.canvas.motion.export.opts = AnimExport { format: AnimFormat::Webm, fps: 10, webm_speed: 10, ..Default::default() };
        let out = tmp("job.webm");
        let _ = std::fs::remove_file(&out);
        app.export_animation_to(frame, out.clone());
        assert!(app.canvas.motion.export.running());
        let ctx = egui::Context::default();
        let start = std::time::Instant::now();
        while app.canvas.motion.export.running() {
            assert!(start.elapsed().as_secs() < 120, "export hangs");
            std::thread::sleep(std::time::Duration::from_millis(20));
            app.poll_anim_export(&ctx);
        }
        let bytes = std::fs::read(&out).unwrap();
        assert_eq!(&bytes[..4], &[0x1A, 0x45, 0xDF, 0xA3], "EBML header");
        assert!(app.toast.as_ref().is_some_and(|t| !t.error && t.text.starts_with("Exported")), "{:?}", app.toast.as_ref().map(|t| &t.text));
        assert!(!out.with_extension("webm.part").exists());
    }

    #[test]
    fn background_export_can_be_cancelled() {
        let (mut d, page, frame) = animated();
        d.timelines.insert(page, TimelineSettings { duration_ms: 60_000, fps: 60, looping: true });
        let mut app = App::new(d);
        app.canvas.motion.export.opts = AnimExport { format: AnimFormat::PngZip, fps: 60, ..Default::default() };
        let out = tmp("cancel.zip");
        let _ = std::fs::remove_file(&out);
        app.export_animation_to(frame, out.clone());
        app.canvas.motion.export.cancel();
        let ctx = egui::Context::default();
        let start = std::time::Instant::now();
        while app.canvas.motion.export.running() {
            assert!(start.elapsed().as_secs() < 60, "cancel did not stop the export");
            std::thread::sleep(std::time::Duration::from_millis(20));
            app.poll_anim_export(&ctx);
        }
        assert!(!out.exists(), "a cancelled export leaves no file");
        assert!(app.toast.as_ref().is_some_and(|t| t.text.contains("cancelled")));
    }
}
