//! One running editor per user for links and files: a second `desigma desigma://join/...` or
//! `desigma file.desigma` (what the OS runs when a link is clicked or a file double-clicked on
//! Windows and Linux) hands its argument to the running editor and exits without a window.
//!
//! The running editor listens on `127.0.0.1:<ephemeral port>` and writes `<dir>/instance.json`
//! (port and a random token, 0600 on unix). A client reads that file and sends one line,
//! `<token> open-link <url>`, `<token> open-file <path>` or `<token> ping`, and gets `ok` or
//! `err <why>` back. Lines are capped at [`MAX_LINE`] bytes, links must be `desigma://join/...`,
//! and nothing else is understood, so a stray local process can at most open the Join dialog.
//! A stale file (the editor crashed) refuses the connection and the new process starts normally.
//!
//! Opens that arrive from elsewhere (macOS Apple events, see `macos.rs`) go through the same
//! [`push`] / [`take`] inbox, which the UI drains once a frame.

use serde::{Deserialize, Serialize};
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, OnceLock,
    },
    thread,
    time::Duration,
};

pub const DISCOVERY_FILE: &str = "instance.json";
/// Longest accepted line, newline included.
pub const MAX_LINE: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Open {
    /// A `desigma://join/...` link.
    Link(String),
    File(PathBuf),
}

/// What a line asks for.
#[derive(Debug, PartialEq, Eq)]
enum Request {
    Ping,
    Open(Open),
}

#[derive(Serialize, Deserialize)]
struct Discovery {
    port: u16,
    token: String,
    pid: u32,
}

/// Where the discovery file lives: the per-user runtime dir on Linux, else the local data dir.
pub fn default_dir() -> PathBuf {
    dirs::runtime_dir().or_else(dirs::data_local_dir).unwrap_or_else(std::env::temp_dir).join("desigma")
}

/// A `desigma://join/...` link without control characters and of a sane length.
pub fn valid_link(link: &str) -> bool {
    link.len() <= MAX_LINE - 32
        && link.get(..15).is_some_and(|p| p.eq_ignore_ascii_case("desigma://join/"))
        && link.len() > 15
        && !link.chars().any(char::is_control)
}

/// A `desigma://open` link, optionally with a fragment or query (e.g. what an
/// embedding host appends): hands the running app the foreground only. The
/// remainder is never interpreted or logged.
pub fn is_open_link(link: &str) -> bool {
    if link.len() > MAX_LINE || link.chars().any(char::is_control) {
        return false;
    }
    if !link.get(..14).is_some_and(|p| p.eq_ignore_ascii_case("desigma://open")) {
        return false;
    }
    let rest = &link[14..];
    rest.is_empty() || rest.starts_with('#') || rest.starts_with('?')
}

/// Human-readable rejection for a `desigma://` link that is neither a join
/// link nor an open link, capped so a hostile URL cannot flood the toast.
pub fn unsupported_link_message(link: &str) -> String {
    let cleaned: String = link.chars().filter(|c| !c.is_control()).take(80).collect();
    format!("Unsupported link: {cleaned}")
}

/// Checks the token and parses one protocol line (without its newline).
fn parse_line(line: &str, token: &str) -> Result<Request, &'static str> {
    if line.len() >= MAX_LINE {
        return Err("too long");
    }
    let (given, rest) = line.split_once(' ').ok_or("bad request")?;
    if !same(given, token) {
        return Err("bad token");
    }
    if rest == "ping" {
        return Ok(Request::Ping);
    }
    if let Some(link) = rest.strip_prefix("open-link ") {
        return if valid_link(link) || is_open_link(link) { Ok(Request::Open(Open::Link(link.to_string()))) } else { Err("not a desigma link") };
    }
    if let Some(path) = rest.strip_prefix("open-file ") {
        let path = Path::new(path);
        return if path.is_absolute() && !path.to_string_lossy().chars().any(char::is_control) {
            Ok(Request::Open(Open::File(path.to_path_buf())))
        } else {
            Err("not an absolute path")
        };
    }
    Err("unknown command")
}

/// Equality without an early exit on the first differing byte.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

// The inbox shared by the listener thread and the macOS Apple event handler.

static INBOX: Mutex<Vec<Open>> = Mutex::new(Vec::new());
static CTX: OnceLock<egui::Context> = OnceLock::new();

/// Remembers the context so [`push`] can wake the UI.
pub fn set_context(ctx: &egui::Context) {
    let _ = CTX.set(ctx.clone());
}

/// Queues an open request and asks for a repaint. Callable before the UI exists.
pub fn push(open: Open) {
    if let Ok(mut q) = INBOX.lock() {
        q.push(open);
    }
    if let Some(ctx) = CTX.get() {
        ctx.request_repaint();
    }
}

/// Everything queued since the last call.
pub fn take() -> Vec<Open> {
    INBOX.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default()
}

// The listener.

/// The running editor's end. Dropping it stops the thread and removes the discovery file.
pub struct Instance {
    dir: PathBuf,
    token: String,
    port: u16,
    stopped: Arc<AtomicBool>,
}

impl Instance {
    /// Starts listening and publishes the discovery file, handing every request to `sink` (on the
    /// listener thread). `Ok(None)` when another live editor already owns the endpoint.
    pub fn start(dir: &Path, sink: impl Fn(Open) + Send + 'static) -> Result<Option<Self>, String> {
        if let Some(d) = read_discovery(dir) {
            if send(&d, "ping") {
                return Ok(None);
            }
        }
        let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        let token = uuid::Uuid::new_v4().simple().to_string();
        write_discovery(dir, &Discovery { port, token: token.clone(), pid: std::process::id() })?;
        let stopped = Arc::new(AtomicBool::new(false));
        let (flag, tok) = (stopped.clone(), token.clone());
        thread::Builder::new()
            .name("instance".into())
            .spawn(move || {
                for stream in listener.incoming() {
                    if flag.load(Ordering::SeqCst) {
                        break;
                    }
                    if let Ok(s) = stream {
                        serve(s, &tok, &sink);
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Some(Self { dir: dir.to_path_buf(), token, port, stopped }))
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::SeqCst);
        // wake the accept loop so the thread ends
        let _ = TcpStream::connect_timeout(&SocketAddr::from(([127, 0, 0, 1], self.port)), Duration::from_millis(200));
        // a newer editor may have replaced the file since
        if read_discovery(&self.dir).is_some_and(|d| d.token == self.token) {
            let _ = std::fs::remove_file(self.dir.join(DISCOVERY_FILE));
        }
    }
}

fn serve(stream: TcpStream, token: &str, sink: &impl Fn(Open)) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(1)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(1)));
    let mut line = String::new();
    // one byte over the limit tells "too long" from "exactly fits"
    let read = BufReader::new((&stream).take(MAX_LINE as u64)).read_line(&mut line);
    let reply = match read {
        Ok(n) if n > 0 && line.ends_with('\n') => match parse_line(line.trim_end_matches(['\r', '\n']), token) {
            Ok(Request::Ping) => "ok\n".to_string(),
            Ok(Request::Open(open)) => {
                sink(open);
                "ok\n".to_string()
            }
            Err(why) => format!("err {why}\n"),
        },
        _ => "err bad request\n".to_string(),
    };
    let _ = (&stream).write_all(reply.as_bytes());
}

fn read_discovery(dir: &Path) -> Option<Discovery> {
    serde_json::from_slice(&std::fs::read(dir.join(DISCOVERY_FILE)).ok()?).ok()
}

fn write_discovery(dir: &Path, d: &Discovery) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("Cannot create {}: {e}", dir.display()))?;
    let tmp = dir.join(format!("{DISCOVERY_FILE}.{}.tmp", std::process::id()));
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    let bytes = serde_json::to_vec(d).map_err(|e| e.to_string())?;
    let result = o
        .open(&tmp)
        .and_then(|mut f| f.write_all(&bytes))
        .and_then(|()| std::fs::rename(&tmp, dir.join(DISCOVERY_FILE)))
        .map_err(|e| format!("Cannot write {DISCOVERY_FILE}: {e}"));
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

// The client.

fn send(d: &Discovery, command: &str) -> bool {
    let Ok(mut s) = TcpStream::connect_timeout(&SocketAddr::from(([127, 0, 0, 1], d.port)), Duration::from_millis(500)) else { return false };
    let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
    let _ = s.set_write_timeout(Some(Duration::from_secs(1)));
    if s.write_all(format!("{} {command}\n", d.token).as_bytes()).is_err() {
        return false;
    }
    let mut reply = String::new();
    BufReader::new(s).read_line(&mut reply).is_ok() && reply.trim_end() == "ok"
}

/// Hands `open` to the editor running for this user. False when there is none (no file, or a stale
/// one that refuses the connection) or it refused the request; the caller then starts normally.
pub fn forward(dir: &Path, open: &Open) -> bool {
    let Some(d) = read_discovery(dir) else { return false };
    match open {
        Open::Link(l) if valid_link(l) || is_open_link(l) => send(&d, &format!("open-link {l}")),
        Open::File(p) => {
            // the running editor has another working directory
            let Ok(abs) = std::path::absolute(p) else { return false };
            send(&d, &format!("open-file {}", abs.display()))
        }
        Open::Link(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("desigma-instance-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn lines_need_the_token_and_a_known_command() {
        assert_eq!(parse_line("tok ping", "tok"), Ok(Request::Ping));
        assert_eq!(parse_line("tok open-link desigma://join/abc", "tok"), Ok(Request::Open(Open::Link("desigma://join/abc".into()))));
        assert_eq!(parse_line("nope ping", "tok"), Err("bad token"));
        assert_eq!(parse_line("tok", "tok"), Err("bad request"));
        assert_eq!(parse_line("tok quit", "tok"), Err("unknown command"));
        assert_eq!(parse_line("tok open-link https://example.com", "tok"), Err("not a desigma link"));
        assert_eq!(parse_line("tok open-link desigma://join/", "tok"), Err("not a desigma link"));
        assert_eq!(parse_line("tok open-link desigma://join/a\u{7}b", "tok"), Err("not a desigma link"));
    }

    #[test]
    fn links_are_case_insensitive_and_files_must_be_absolute() {
        assert!(valid_link("DESIGMA://join/abc"));
        assert!(!valid_link("desigma://open/abc"));
        assert!(is_open_link("desigma://open"));
        assert!(is_open_link("DESIGMA://OPEN#identity_token=abc"));
        assert!(is_open_link("desigma://open?from=shell"));
        assert!(!is_open_link("desigma://openx"));
        assert!(!is_open_link("desigma://open/abc"));
        assert!(!is_open_link("desigma://join/abc"));
        assert!(!is_open_link(&format!("desigma://open#{}", "a".repeat(MAX_LINE))));
        assert_eq!(parse_line("tok open-link desigma://open", "tok"), Ok(Request::Open(Open::Link("desigma://open".into()))));
        assert_eq!(parse_line("tok open-link desigma://open#frag", "tok"), Ok(Request::Open(Open::Link("desigma://open#frag".into()))));
        assert_eq!(parse_line("tok open-link desigma://openx", "tok"), Err("not a desigma link"));
        assert!(!valid_link("desigma://"));
        let abs = std::env::temp_dir().join("a b.desigma");
        assert_eq!(parse_line(&format!("tok open-file {}", abs.display()), "tok"), Ok(Request::Open(Open::File(abs))));
        assert_eq!(parse_line("tok open-file relative.desigma", "tok"), Err("not an absolute path"));
    }

    #[test]
    fn unsupported_link_messages_are_sanitized_and_capped() {
        assert_eq!(unsupported_link_message("desigma://weird/x"), "Unsupported link: desigma://weird/x");
        let long = format!("desigma://{}", "a".repeat(200));
        let msg = unsupported_link_message(&long);
        assert_eq!(msg.chars().count(), "Unsupported link: ".len() + 80);
        assert_eq!(unsupported_link_message("desigma://a\u{7}b"), "Unsupported link: desigma://ab");
    }

    #[test]
    fn long_lines_are_refused() {
        let long = format!("tok open-link desigma://join/{}", "a".repeat(MAX_LINE));
        assert_eq!(parse_line(&long, "tok"), Err("too long"));
        assert!(!valid_link(&format!("desigma://join/{}", "a".repeat(MAX_LINE))));
    }

    #[test]
    fn a_second_process_reaches_the_listener() {
        let dir = tmp("forward");
        let (tx, rx) = std::sync::mpsc::channel();
        let inst = Instance::start(&dir, move |o| {
            let _ = tx.send(o);
        })
        .unwrap()
        .expect("first instance owns the endpoint");
        let link = Open::Link("desigma://join/abc".into());
        assert!(forward(&dir, &link));
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)).unwrap(), link);
        let file = Open::File(std::env::temp_dir().join("x.desigma"));
        assert!(forward(&dir, &file));
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)).unwrap(), file);
        let open = Open::Link("desigma://open".into());
        assert!(forward(&dir, &open));
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)).unwrap(), open);
        // a second editor does not take over
        assert!(Instance::start(&dir, |_| {}).unwrap().is_none());
        drop(inst);
        assert!(!dir.join(DISCOVERY_FILE).exists());
        assert!(!forward(&dir, &link));
    }

    #[test]
    fn wrong_token_and_garbage_are_refused_over_the_socket() {
        let dir = tmp("refuse");
        let (tx, rx) = std::sync::mpsc::channel();
        let _inst = Instance::start(&dir, move |o| {
            let _ = tx.send(o);
        })
        .unwrap()
        .unwrap();
        let d = read_discovery(&dir).unwrap();
        let ask = |line: &str| {
            let mut s = TcpStream::connect(("127.0.0.1", d.port)).unwrap();
            s.write_all(line.as_bytes()).unwrap();
            let mut r = String::new();
            BufReader::new(s).read_line(&mut r).unwrap();
            r
        };
        assert_eq!(ask("wrong open-link desigma://join/abc\n"), "err bad token\n");
        assert_eq!(ask("\n"), "err bad request\n");
        assert!(ask(&format!("{} open-link desigma://join/{}\n", d.token, "a".repeat(MAX_LINE))).starts_with("err"));
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn a_stale_file_means_nobody_answers() {
        let dir = tmp("stale");
        // a port nobody listens on
        let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        write_discovery(&dir, &Discovery { port, token: "t".into(), pid: 0 }).unwrap();
        assert!(!forward(&dir, &Open::Link("desigma://join/abc".into())));
        // and a new editor takes the endpoint over
        assert!(Instance::start(&dir, |_| {}).unwrap().is_some());
    }
}
