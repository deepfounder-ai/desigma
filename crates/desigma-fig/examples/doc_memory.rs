//! Measures heap memory and time of opening and saving a document.
//!
//!   cargo run --release -p desigma-fig --example doc_memory -- file.fig|file.desigma
//!   cargo run --release -p desigma-fig --example doc_memory -- --synth <instances> <out.desigma>
//!
//! A counting global allocator records the live heap and its peak during each phase
//! (read, import/parse, save, reload), so numbers are comparable across platforms and
//! need no profiler. It also estimates how much of the document is precomputed path
//! geometry and how many nodes are resolved copies inside instances.
//!
//! `--synth` first writes a reproducible large file (one component of vector icons and
//! `<instances>` instances of it), then measures that file. Use it for numbers that can be
//! shared without a private design file.

use desigma_doc::*;
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::time::Instant;

struct Counting;
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(l) };
        if !p.is_null() {
            let now = LIVE.fetch_add(l.size(), Relaxed) + l.size();
            PEAK.fetch_max(now, Relaxed);
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) };
        LIVE.fetch_sub(l.size(), Relaxed);
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, new: usize) -> *mut u8 {
        let q = unsafe { System.realloc(p, l, new) };
        if !q.is_null() {
            if new >= l.size() {
                let now = LIVE.fetch_add(new - l.size(), Relaxed) + (new - l.size());
                PEAK.fetch_max(now, Relaxed);
            } else {
                LIVE.fetch_sub(l.size() - new, Relaxed);
            }
        }
        q
    }
}

#[global_allocator]
static ALLOC: Counting = Counting;

const MB: f64 = 1024.0 * 1024.0;

fn mb(bytes: usize) -> f64 {
    bytes as f64 / MB
}

/// Runs `f`, printing elapsed time, the heap peak reached during it and the heap left after it.
fn phase<T>(name: &str, f: impl FnOnce() -> T) -> T {
    PEAK.store(LIVE.load(Relaxed), Relaxed);
    let t = Instant::now();
    let out = f();
    println!("{name:<10} {:>7.2}s  peak {:>8.1} MB  live after {:>8.1} MB", t.elapsed().as_secs_f64(), mb(PEAK.load(Relaxed)), mb(LIVE.load(Relaxed)));
    out
}

fn path_bytes(paths: &Option<Vec<Path>>) -> usize {
    paths.as_ref().map_or(0, |ps| ps.iter().map(|p| p.cmds.capacity() * size_of::<PathCmd>()).sum())
}

/// Vectors per icon component in the synthetic file.
const SYNTH_VECTORS: usize = 4;
/// Cubic segments per vector outline.
const SYNTH_SEGMENTS: usize = 4;

/// A closed wavy ring of `SYNTH_SEGMENTS` cubics around (`cx`, `cy`).
fn ring(cx: f32, cy: f32, r: f32, seed: usize) -> Path {
    let point = |i: usize| {
        let a = i as f32 / SYNTH_SEGMENTS as f32 * std::f32::consts::TAU;
        let rr = r * (1.0 + 0.15 * ((i + seed) as f32 * 1.7).sin());
        Vec2::new(cx + rr * a.cos(), cy + rr * a.sin())
    };
    let mut cmds = vec![PathCmd::MoveTo(point(0))];
    for i in 1..=SYNTH_SEGMENTS {
        let (a, b) = (point(i - 1), point(i % SYNTH_SEGMENTS));
        let c1 = Vec2::new(a.x + (b.x - a.x) / 3.0, a.y + (b.y - a.y) / 3.0);
        let c2 = Vec2::new(a.x + (b.x - a.x) * 2.0 / 3.0, a.y + (b.y - a.y) * 2.0 / 3.0);
        cmds.push(PathCmd::CubicTo(c1, c2, b));
    }
    cmds.push(PathCmd::Close);
    Path { cmds, fill_rule: FillRule::default() }
}

/// Builds a document shaped like an imported design system file: a component of vectors
/// carrying precomputed fill and stroke geometry (as `.fig` import produces), and
/// `instances` instances of it, each holding its own resolved copy of the subtree.
fn synth(instances: usize) -> Document {
    let mut doc = Document::new("Synthetic");
    let page = doc.add_page("Page 1");
    let id = doc.alloc_id();
    let comp = doc.insert(page, Node::new(id, NodeKind::Component(FrameProps::default()))).expect("component");
    let c = doc.get_mut(comp).expect("component");
    c.name = "Icon".into();
    c.size = Vec2::new(24.0, 24.0);
    c.transform = Affine::translate(0.0, -100.0);
    for v in 0..SYNTH_VECTORS {
        let paths = vec![ring(12.0, 12.0, 3.0 + v as f32, v)];
        let mut n = Node::new(doc.alloc_id(), NodeKind::Vector { paths: paths.clone(), network: None });
        n.name = format!("path {v}");
        n.size = Vec2::new(24.0, 24.0);
        n.fills = vec![Paint::solid(Color::rgb(0x22, 0x22, 0x22))];
        n.fill_geometry = Some(paths);
        n.stroke_geometry = Some(vec![ring(12.0, 12.0, 3.5 + v as f32, v), ring(12.0, 12.0, 2.5 + v as f32, v)]);
        doc.insert(comp, n).expect("vector");
    }
    let cols = (instances as f32).sqrt().ceil().max(1.0) as usize;
    for i in 0..instances {
        let at = Vec2::new((i % cols) as f32 * 32.0, (i / cols) as f32 * 32.0);
        create_instance(&mut doc, comp, page, at).apply(&mut doc).expect("instance");
    }
    doc
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let path = match args.as_slice() {
        [flag, count, out] if flag == "--synth" => {
            let count: usize = count.parse().expect("instance count");
            let doc = phase("generate", || synth(count));
            phase("write", || std::fs::write(out, doc.to_json().expect("serialize")).expect("write file"));
            println!("wrote {out}\n");
            out.clone()
        }
        [path] if !path.starts_with("--") => path.clone(),
        _ => {
            eprintln!("usage: doc_memory <file.fig|file.desigma|file.json>\n       doc_memory --synth <instances> <out.desigma>");
            std::process::exit(2);
        }
    };

    let bytes = phase("read", || std::fs::read(&path).expect("read file"));
    println!("file       {:.1} MB", mb(bytes.len()));
    let doc = phase("open", || {
        if path.ends_with(".fig") {
            desigma_fig::import_fig(&bytes).expect("import .fig")
        } else {
            Document::from_json(std::str::from_utf8(&bytes).expect("utf-8")).expect("parse")
        }
    });
    drop(bytes);
    println!("heap with document only: {:.1} MB", mb(LIVE.load(Relaxed)));

    let (mut geometry, mut in_instance, mut instances) = (0usize, 0usize, 0usize);
    for n in doc.nodes.values() {
        geometry += path_bytes(&n.fill_geometry) + path_bytes(&n.stroke_geometry);
        if matches!(n.kind, NodeKind::Instance { .. }) {
            instances += 1;
        }
        let mut p = n.parent;
        while let Some(id) = p {
            let Some(parent) = doc.nodes.get(&id) else { break };
            if matches!(parent.kind, NodeKind::Instance { .. }) {
                in_instance += 1;
                break;
            }
            p = parent.parent;
        }
    }
    let images: usize = doc.images.values().map(|b| b.len()).sum();
    println!(
        "nodes {} | instances {instances} | nodes inside instances {in_instance} ({:.0}%)",
        doc.nodes.len(),
        in_instance as f64 * 100.0 / doc.nodes.len().max(1) as f64
    );
    println!("precomputed geometry {:.1} MB | image bytes {:.1} MB | Node struct {} B", mb(geometry), mb(images), size_of::<Node>());

    let json = phase("save", || doc.to_json().expect("serialize"));
    println!("saved      {:.1} MB", mb(json.len()));
    drop(doc);
    let back = phase("reload", || Document::from_json(&json).expect("reparse"));
    drop(json);
    println!("reloaded {} nodes; heap {:.1} MB", back.nodes.len(), mb(LIVE.load(Relaxed)));
}
