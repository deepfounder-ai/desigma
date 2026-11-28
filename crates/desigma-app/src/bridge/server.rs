//! Loopback agent bridge: desigma-mcp sends tool calls here; the UI thread executes them.
use crate::loopback::{self, Request};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc::{self, Receiver, SyncSender},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

pub const DISCOVERY_FILE: &str = "bridge.json";

/// Connections served at once. Each one lives at most the read deadline plus the reply deadline,
/// so this only bounds a misbehaving local client; past it a connection is closed unanswered.
const MAX_CONNECTIONS: usize = 16;

/// Owned failure: `(status, message)`. Unlike `loopback::Failure` the message is owned so it can
/// carry the editor's actual error text (e.g. a 422 reply) without leaking it.
type Fail = (&'static str, String);

#[derive(Serialize, Deserialize)]
pub struct Discovery {
    pub version: u32,
    pub port: u16,
    pub token: String,
    pub pid: u32,
    pub document: String,
    pub path: Option<PathBuf>,
}

pub struct BridgeRequest {
    pub name: String,
    pub args: Value,
    pub reply: SyncSender<Result<Value, String>>,
    pub cancelled: Arc<AtomicBool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CallBody {
    name: String,
    #[serde(default)]
    args: Value,
}

pub struct Bridge {
    port: u16,
    token: String,
    discovery: Option<PathBuf>,
    receiver: Receiver<BridgeRequest>,
    stopped: Arc<AtomicBool>,
    state: Arc<Mutex<Shared>>,
    pub last_call: Arc<Mutex<Option<String>>>,
}

/// What `ping` reports; rewritten when the editor switches documents.
struct Shared {
    document: String,
    path: Option<PathBuf>,
}

impl Bridge {
    /// Binds `127.0.0.1:0`, writes `<root>/bridge.json` (0600 on unix) when `root` is given, and
    /// spawns the listener thread.
    pub fn start(root: Option<&Path>, document: &str, path: Option<&Path>, ctx: egui::Context) -> Result<Self, String> {
        let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| format!("Cannot start agent bridge: {e}"))?;
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        let token = uuid::Uuid::new_v4().simple().to_string();
        let (sender, receiver) = mpsc::sync_channel(4);
        let stopped = Arc::new(AtomicBool::new(false));
        let state = Arc::new(Mutex::new(Shared { document: document.into(), path: path.map(Path::to_path_buf) }));
        let last_call = Arc::new(Mutex::new(None));
        let server = Arc::new(Server {
            host: format!("127.0.0.1:{port}"),
            token: token.clone(),
            sender,
            stopped: stopped.clone(),
            state: state.clone(),
            last_call: last_call.clone(),
            ctx,
        });
        thread::Builder::new()
            .name("agent-bridge".into())
            .spawn(move || {
                let open = Arc::new(AtomicUsize::new(0));
                while !server.stopped.load(Ordering::Acquire) {
                    match listener.accept() {
                        // Each connection gets its own thread. Served in turn, a call waiting on the
                        // editor held every other connection for up to the reply deadline, so another
                        // client's 300 ms ping went unanswered: desigma-mcp then reported the bridge as
                        // off, and a second window's `remove_stale` deleted this editor's discovery file.
                        Ok((mut stream, _)) => {
                            if open.fetch_add(1, Ordering::AcqRel) >= MAX_CONNECTIONS {
                                open.fetch_sub(1, Ordering::AcqRel);
                                continue;
                            }
                            let (server, done) = (server.clone(), open.clone());
                            let spawned = thread::Builder::new().name("agent-bridge-call".into()).spawn(move || {
                                server.handle(&mut stream);
                                done.fetch_sub(1, Ordering::AcqRel);
                            });
                            if spawned.is_err() {
                                open.fetch_sub(1, Ordering::AcqRel);
                            }
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => thread::sleep(Duration::from_millis(15)),
                        Err(_) => break,
                    }
                }
            })
            .map_err(|e| format!("Cannot start agent bridge: {e}"))?;
        // Written last: if this fails, the thread above must still exit (nothing would ever
        // remove it, since no `Bridge` is returned for `Drop` to run on).
        let discovery = match root {
            Some(root) => match write_discovery(root, port, &token, document, path) {
                Ok(()) => Some(root.join(DISCOVERY_FILE)),
                Err(e) => {
                    stopped.store(true, Ordering::Release);
                    return Err(e);
                }
            },
            None => None,
        };
        Ok(Self { port, token, discovery, receiver, stopped, state, last_call })
    }

    pub fn port(&self) -> u16 {
        self.port
    }
    #[cfg(test)]
    pub fn token(&self) -> &str {
        &self.token
    }
    pub fn try_recv(&self) -> Option<BridgeRequest> {
        self.receiver.try_recv().ok()
    }

    /// Rewrites the discovery file after the editor switches documents.
    pub fn update_document(&self, document: &str, path: Option<&Path>) {
        if let Ok(mut s) = self.state.lock() {
            s.document = document.into();
            s.path = path.map(Path::to_path_buf);
        }
        if let Some(file) = &self.discovery {
            let _ = write_discovery(file.parent().unwrap_or(Path::new(".")), self.port, &self.token, document, path);
        }
    }
}

impl Drop for Bridge {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
        while let Ok(request) = self.receiver.try_recv() {
            request.cancelled.store(true, Ordering::Release);
        }
        if let Some(file) = &self.discovery {
            let _ = std::fs::remove_file(file);
        }
    }
}

fn write_discovery(root: &Path, port: u16, token: &str, document: &str, path: Option<&Path>) -> Result<(), String> {
    let d = Discovery { version: 1, port, token: token.into(), pid: std::process::id(), document: document.into(), path: path.map(Path::to_path_buf) };
    let bytes = serde_json::to_vec_pretty(&d).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(root).map_err(|e| format!("Cannot create {}: {e}", root.display()))?;
    let tmp = root.join(format!("{DISCOVERY_FILE}.{}.tmp", std::process::id()));
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    // Once the temp file exists, any later failure must remove it rather than leave a stray
    // `bridge.json.<pid>.tmp` behind.
    let result = o.open(&tmp).map_err(|e| format!("Cannot write bridge file: {e}")).and_then(|mut f| {
        std::io::Write::write_all(&mut f, &bytes).map_err(|e| e.to_string())?;
        drop(f);
        std::fs::rename(&tmp, root.join(DISCOVERY_FILE)).map_err(|e| format!("Cannot write bridge file: {e}"))
    });
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// Deletes `<root>/bridge.json` when nobody answers its ping (a crashed or killed editor left it
/// behind). Used at startup.
pub fn remove_stale(root: &Path) {
    let file = root.join(DISCOVERY_FILE);
    let Ok(bytes) = std::fs::read(&file) else { return };
    let alive = serde_json::from_slice::<Discovery>(&bytes).ok().is_some_and(|d| ping(d.port, &d.token));
    if !alive {
        let _ = std::fs::remove_file(&file);
    }
}

fn ping(port: u16, token: &str) -> bool {
    let Ok(mut s) = TcpStream::connect_timeout(&std::net::SocketAddr::from(([127, 0, 0, 1], port)), Duration::from_millis(300)) else { return false };
    let _ = s.set_read_timeout(Some(Duration::from_millis(300)));
    let _ = std::io::Write::write_all(&mut s, format!("GET /{token}/ping HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Length: 0\r\n\r\n").as_bytes());
    let mut out = String::new();
    let _ = std::io::Read::read_to_string(&mut s, &mut out);
    out.starts_with("HTTP/1.1 200")
}

struct Server {
    host: String,
    token: String,
    sender: SyncSender<BridgeRequest>,
    stopped: Arc<AtomicBool>,
    state: Arc<Mutex<Shared>>,
    last_call: Arc<Mutex<Option<String>>>,
    ctx: egui::Context,
}

impl Server {
    fn handle(&self, stream: &mut TcpStream) {
        let _ = stream.set_read_timeout(Some(Duration::from_millis(250)));
        let _ = stream.set_write_timeout(Some(Duration::from_secs(1)));
        let (status, body) = match self.request(stream) {
            Ok(body) => ("200 OK", body),
            Err((status, message)) => (status, loopback::json_error(&message)),
        };
        loopback::write_response(stream, status, "application/json", &body, "");
    }

    fn request(&self, stream: &mut TcpStream) -> Result<Vec<u8>, Fail> {
        let req: Request = loopback::read_request(stream, &self.stopped, Instant::now() + Duration::from_secs(2)).map_err(|(s, m)| (s, m.to_string()))?;
        if req.headers.get("host").map(String::as_str) != Some(self.host.as_str()) {
            return Err(("403 Forbidden", "Invalid host".to_string()));
        }
        // Browsers always send Origin; the bridge is for local processes only.
        if req.headers.contains_key("origin") {
            return Err(("403 Forbidden", "Browser access is not allowed".to_string()));
        }
        let path = req.path.strip_prefix(&format!("/{}/", self.token)).ok_or(("404 Not Found", "Not found".to_string()))?;
        match (req.method.as_str(), path) {
            ("GET", "ping") => {
                let s = self.state.lock().map_err(|_| ("503 Service Unavailable", "Editor is busy".to_string()))?;
                Ok(serde_json::to_vec(&json!({"ok":true,"document":s.document,"path":s.path})).unwrap_or_default())
            }
            ("POST", "call") => {
                if req.headers.get("x-desigma-token").map(String::as_str) != Some(self.token.as_str()) {
                    return Err(("403 Forbidden", "Invalid bridge token".to_string()));
                }
                if req.headers.get("content-type").map(|s| s.split(';').next().unwrap_or_default().trim()) != Some("application/json") {
                    return Err(("415 Unsupported Media Type", "JSON required".to_string()));
                }
                let body: CallBody = serde_json::from_slice(&req.body).map_err(|_| ("400 Bad Request", "Invalid call request".to_string()))?;
                if !body.name.starts_with("desigma_") || body.name.len() > 64 {
                    return Err(("400 Bad Request", "Unknown tool".to_string()));
                }
                self.dispatch(body)
            }
            _ => Err(("405 Method Not Allowed", "Unsupported operation".to_string())),
        }
    }

    fn dispatch(&self, body: CallBody) -> Result<Vec<u8>, Fail> {
        let cancelled = Arc::new(AtomicBool::new(false));
        let (reply, receive) = mpsc::sync_channel(1);
        if let Ok(mut l) = self.last_call.lock() {
            *l = Some(body.name.clone());
        }
        self.sender
            .try_send(BridgeRequest { name: body.name, args: body.args, reply, cancelled: cancelled.clone() })
            .map_err(|_| ("503 Service Unavailable", "Editor is busy".to_string()))?;
        self.ctx.request_repaint();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if self.stopped.load(Ordering::Acquire) || Instant::now() >= deadline {
                cancelled.store(true, Ordering::Release);
                return Err((
                    "503 Service Unavailable",
                    "Editor did not answer within 5 s; the call may still have been applied — read the document before retrying".to_string(),
                ));
            }
            match receive.recv_timeout(Duration::from_millis(25)) {
                Ok(Ok(result)) => return Ok(serde_json::to_vec(&json!({"result":result})).unwrap_or_default()),
                Ok(Err(message)) => return Err(("422 Unprocessable Entity", message)),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    cancelled.store(true, Ordering::Release);
                    return Err(("503 Service Unavailable", "Editor session ended".to_string()));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    fn bridge(dir: &Path) -> Bridge {
        Bridge::start(Some(dir), "Doc", None, egui::Context::default()).unwrap()
    }

    fn exchange(port: u16, method: &str, path: &str, extras: &str, body: &str) -> String {
        let host = format!("127.0.0.1:{port}");
        let mut stream = TcpStream::connect(&host).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(8))).unwrap();
        write!(stream, "{method} {path} HTTP/1.1\r\nHost: {host}\r\n{extras}Content-Length: {}\r\n\r\n{body}", body.len()).unwrap();
        let mut result = String::new();
        // Oversized or malformed requests may leave bytes unread. Windows can
        // reset that socket after sending the complete HTTP response.
        if let Err(error) = stream.read_to_string(&mut result) {
            assert_eq!(error.kind(), std::io::ErrorKind::ConnectionReset);
            let (header, body) = result.split_once("\r\n\r\n").expect("complete response headers before reset");
            let length: usize = header.lines().find_map(|line| line.strip_prefix("Content-Length: ")).unwrap().trim().parse().unwrap();
            assert_eq!(body.len(), length, "complete response body before reset");
        }
        result
    }

    fn tmp() -> PathBuf {
        let d = std::env::temp_dir().join(format!("desigma-bridge-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn writes_discovery_file_and_answers_ping() {
        let dir = tmp();
        let b = bridge(&dir);
        let d: Discovery = serde_json::from_slice(&std::fs::read(dir.join(DISCOVERY_FILE)).unwrap()).unwrap();
        assert_eq!((d.version, d.port, d.token.as_str(), d.document.as_str()), (1, b.port(), b.token(), "Doc"));
        let r = exchange(b.port(), "GET", &format!("/{}/ping", b.token()), "", "");
        assert!(r.starts_with("HTTP/1.1 200") && r.contains("\"document\":\"Doc\""), "{r}");
        drop(b);
        assert!(!dir.join(DISCOVERY_FILE).exists());
    }

    #[test]
    fn rejects_browsers_wrong_tokens_and_bad_bodies() {
        let dir = tmp();
        let b = bridge(&dir);
        let call = format!("/{}/call", b.token());
        let auth = format!("X-Desigma-Token: {}\r\nContent-Type: application/json\r\n", b.token());
        let body = r#"{"name":"desigma_get_document","args":{}}"#;
        assert!(exchange(b.port(), "POST", &call, &format!("Origin: http://127.0.0.1:1\r\n{auth}"), body).starts_with("HTTP/1.1 403"));
        assert!(exchange(b.port(), "POST", &call, "X-Desigma-Token: nope\r\nContent-Type: application/json\r\n", body).starts_with("HTTP/1.1 403"));
        assert!(exchange(b.port(), "POST", "/wrongtoken/call", &auth, body).starts_with("HTTP/1.1 404"));
        assert!(exchange(b.port(), "POST", &call, &auth, "not json").starts_with("HTTP/1.1 400"));
        assert!(b.try_recv().is_none());
    }

    #[test]
    fn dispatches_call_to_editor_and_returns_its_reply() {
        let dir = tmp();
        let b = bridge(&dir);
        let (port, token) = (b.port(), b.token().to_string());
        let worker = thread::spawn(move || {
            exchange(
                port,
                "POST",
                &format!("/{token}/call"),
                &format!("X-Desigma-Token: {token}\r\nContent-Type: application/json\r\n"),
                r#"{"name":"desigma_get_node","args":{"node_id":7}}"#,
            )
        });
        let deadline = Instant::now() + Duration::from_secs(3);
        let req = loop {
            if let Some(r) = b.try_recv() {
                break r;
            }
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(10));
        };
        assert_eq!((req.name.as_str(), req.args["node_id"].as_u64()), ("desigma_get_node", Some(7)));
        req.reply.send(Err("node 7 not found".into())).unwrap();
        let r = worker.join().unwrap();
        assert!(r.starts_with("HTTP/1.1 422") && r.contains("node 7 not found"), "{r}");
    }

    #[test]
    fn answers_ping_while_a_call_waits_for_the_editor() {
        let dir = tmp();
        let b = bridge(&dir);
        let (port, token) = (b.port(), b.token().to_string());
        let t = token.clone();
        let worker = thread::spawn(move || {
            exchange(
                port,
                "POST",
                &format!("/{t}/call"),
                &format!("X-Desigma-Token: {t}\r\nContent-Type: application/json\r\n"),
                r#"{"name":"desigma_get_document","args":{}}"#,
            )
        });
        let deadline = Instant::now() + Duration::from_secs(3);
        let req = loop {
            if let Some(r) = b.try_recv() {
                break r;
            }
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(10));
        };
        // The call is now waiting for the editor's reply. Another client's probe uses the same
        // 300 ms ping as desigma-mcp, and must not queue behind it.
        assert!(ping(port, &token), "ping went unanswered while a call was pending");
        // Nor may a second window take this live editor's discovery file for a crashed one.
        remove_stale(&dir);
        assert!(dir.join(DISCOVERY_FILE).exists());
        req.reply.send(Ok(json!({"ok":true}))).unwrap();
        let r = worker.join().unwrap();
        assert!(r.starts_with("HTTP/1.1 200"), "{r}");
    }

    #[test]
    fn remove_stale_deletes_only_unanswered_files() {
        let dir = tmp();
        std::fs::write(
            dir.join(DISCOVERY_FILE),
            serde_json::to_vec(&Discovery { version: 1, port: 1, token: "x".into(), pid: 0, document: "".into(), path: None }).unwrap(),
        )
        .unwrap();
        remove_stale(&dir);
        assert!(!dir.join(DISCOVERY_FILE).exists());
        let b = bridge(&dir);
        remove_stale(&dir);
        assert!(dir.join(DISCOVERY_FILE).exists());
        drop(b);
    }

    #[test]
    fn write_discovery_cleans_up_its_temp_file_on_failure() {
        let dir = tmp();
        // `bridge.json` is a directory here, so the final rename (file onto an existing
        // directory) fails after the temp file has already been written.
        std::fs::create_dir_all(dir.join(DISCOVERY_FILE)).unwrap();
        let result = write_discovery(&dir, 1, "tok", "Doc", None);
        assert!(result.is_err(), "{result:?}");
        let leftovers: Vec<_> =
            std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok()).map(|e| e.file_name()).filter(|name| name.to_string_lossy().contains(".tmp")).collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }
}
