//! Streaming downloads of release files: https only (plain http to localhost in tests), every redirect hop
//! checked against the host allow-list in [`super::assets`], a size cap, timeouts, and the SHA-256 computed
//! while the bytes go to disk.

use super::assets::{check_asset_url, check_redirect_url};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// The largest release file (or total) we accept: the biggest asset today is under 40 MB.
pub const MAX_DOWNLOAD: u64 = 300 << 20;
/// The checksum file is a few hundred bytes.
pub const MAX_SUMS: u64 = 1 << 20;
const MAX_HOPS: usize = 5;
/// How often the progress callback runs.
const PROGRESS_EVERY: Duration = Duration::from_millis(100);

/// Why a download did not finish.
#[derive(Debug, PartialEq)]
pub enum Fail {
    /// The user pressed Cancel.
    Cancelled,
    Failed(String),
}

impl From<String> for Fail {
    fn from(e: String) -> Fail {
        Fail::Failed(e)
    }
}

type Body = Box<dyn Read + Send + Sync + 'static>;

/// Opens `url` and follows redirects by hand so that each hop is checked. Returns the body and its length.
fn open(url: &str, max: u64, cancel: &AtomicBool) -> Result<(Body, Option<u64>), Fail> {
    let mut url = check_asset_url(url)?;
    for _ in 0..=MAX_HOPS {
        if cancel.load(Ordering::Relaxed) {
            return Err(Fail::Cancelled);
        }
        // a plain-http source is a local test server: no proxy
        let tls = url.scheme() == "https";
        let opts = desigma_http::AgentOptions {
            redirects: 0,
            timeout_connect: Duration::from_secs(10),
            timeout_read: Some(Duration::from_secs(30)),
            timeout_write: Some(Duration::from_secs(30)),
            https_only: tls,
            ..Default::default()
        }
        .user_agent(format!("desigma/{}", crate::VERSION));
        let agent = if tls { desigma_http::agent_with(&opts, url.as_str()) } else { opts.agent_for(None, url.as_str()) };
        let response = agent.get(url.as_str()).set("Accept", "application/octet-stream").call().map_err(|e| match e {
            ureq::Error::Status(403 | 429, _) => "GitHub's limit was reached. Try again later".to_string(),
            ureq::Error::Status(code, _) => format!("GitHub answered {code}"),
            ureq::Error::Transport(t) => {
                eprintln!("desigma: update download: {t}");
                "GitHub can't be reached. Check your connection".to_string()
            }
        })?;
        match response.status() {
            200 => {
                let length = response.header("Content-Length").and_then(|l| l.trim().parse::<u64>().ok());
                if length.is_some_and(|l| l > max) {
                    return Err(Fail::Failed(format!("the download is larger than the {} MB limit", max >> 20)));
                }
                return Ok((response.into_reader(), length));
            }
            301 | 302 | 303 | 307 | 308 => {
                let location = response.header("Location").ok_or_else(|| Fail::Failed("GitHub sent a redirect without a target".into()))?;
                let next = url.join(location).map_err(|_| "GitHub sent an invalid redirect".to_string())?;
                url = check_redirect_url(next.as_str())?;
            }
            code => return Err(Fail::Failed(format!("GitHub answered {code}"))),
        }
    }
    Err(Fail::Failed("too many redirects".into()))
}

/// Reads `body` into `sink` in chunks, stopping at `max` bytes or when `cancel` is set. Returns the byte count
/// and SHA-256 hex digest.
fn pump(
    body: &mut Body,
    sink: &mut dyn Write,
    length: Option<u64>,
    max: u64,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64, Option<u64>),
) -> Result<(u64, String), Fail> {
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 << 10];
    let (mut done, mut last) = (0u64, Instant::now());
    progress(0, length);
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(Fail::Cancelled);
        }
        let n = body.read(&mut buf).map_err(|e| format!("the download was interrupted ({e})"))?;
        if n == 0 {
            break;
        }
        done += n as u64;
        if done > max {
            return Err(Fail::Failed(format!("the download is larger than the {} MB limit", max >> 20)));
        }
        hasher.update(&buf[..n]);
        sink.write_all(&buf[..n]).map_err(|e| format!("cannot write the download ({e})"))?;
        if last.elapsed() >= PROGRESS_EVERY {
            progress(done, length);
            last = Instant::now();
        }
    }
    if length.is_some_and(|l| l != done) {
        return Err(Fail::Failed("the download ended early".into()));
    }
    sink.flush().map_err(|e| format!("cannot write the download ({e})"))?;
    progress(done, length);
    Ok((done, hasher.finalize().iter().map(|b| format!("{b:02x}")).collect()))
}

/// A small text file (`SHA256SUMS`) as a string.
pub fn to_string(url: &str, max: u64, cancel: &AtomicBool) -> Result<String, Fail> {
    let (mut body, length) = open(url, max, cancel)?;
    let mut bytes = Vec::new();
    pump(&mut body, &mut bytes, length, max, cancel, &mut |_, _| {})?;
    String::from_utf8(bytes).map_err(|_| Fail::Failed("the checksum file is not text".into()))
}

/// Downloads `url` to `dest` (a partial file is removed on any failure) and returns its SHA-256 hex digest.
pub fn to_file(url: &str, dest: &Path, max: u64, cancel: &AtomicBool, progress: &mut dyn FnMut(u64, Option<u64>)) -> Result<String, Fail> {
    let (mut body, length) = open(url, max, cancel)?;
    let file = std::fs::File::create(dest).map_err(|e| format!("cannot create {} ({e})", dest.display()))?;
    let mut sink = std::io::BufWriter::new(file);
    let result = pump(&mut body, &mut sink, length, max, cancel, progress);
    drop(sink);
    match result {
        Ok((_, digest)) => Ok(digest),
        Err(e) => {
            let _ = std::fs::remove_file(dest);
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufRead;
    use std::net::TcpListener;

    /// Serves canned responses, one per connection, in order; the handle returns each request's first line.
    fn serve(responses: Vec<Vec<u8>>) -> (String, std::thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let mut seen = Vec::new();
            for response in responses {
                let (mut stream, _) = listener.accept().unwrap();
                let head: Vec<String> = std::io::BufReader::new(&stream).lines().map_while(Result::ok).take_while(|l| !l.is_empty()).collect();
                seen.push(head[0].clone());
                let _ = stream.write_all(&response);
            }
            seen
        });
        (base, server)
    }

    fn ok(body: &[u8]) -> Vec<u8> {
        let mut r = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).into_bytes();
        r.extend_from_slice(body);
        r
    }

    fn redirect(to: &str) -> Vec<u8> {
        format!("HTTP/1.1 302 Found\r\nLocation: {to}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").into_bytes()
    }

    fn temp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("desigma_dl_{}_{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    const HELLO_SHA: &str = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";

    #[test]
    fn a_file_is_streamed_to_disk_and_hashed() {
        let (base, server) = serve(vec![ok(b"hello")]);
        let dir = temp("file");
        let dest = dir.join("out.bin");
        let mut steps = Vec::new();
        let digest = to_file(&format!("{base}/a.bin"), &dest, MAX_DOWNLOAD, &AtomicBool::new(false), &mut |d, t| steps.push((d, t))).unwrap();
        assert_eq!(digest, HELLO_SHA);
        assert_eq!(std::fs::read(&dest).unwrap(), b"hello");
        assert_eq!(steps.last(), Some(&(5, Some(5))), "{steps:?}");
        assert_eq!(server.join().unwrap(), ["GET /a.bin HTTP/1.1"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn redirects_are_followed_only_to_allowed_hosts() {
        // local to local is allowed in tests
        let (base, server) = serve(vec![redirect("/next"), ok(b"hello")]);
        let text = to_string(&format!("{base}/first"), MAX_SUMS, &AtomicBool::new(false)).unwrap();
        assert_eq!(text, "hello");
        assert_eq!(server.join().unwrap(), ["GET /first HTTP/1.1", "GET /next HTTP/1.1"]);
        // anywhere else is refused before a request is made to it
        for target in ["https://evil.example/x", "http://127.0.0.2:9/x", "https://github.com/other/repo/releases/download/v1/x"] {
            let (base, server) = serve(vec![redirect(target)]);
            let e = to_string(&format!("{base}/first"), MAX_SUMS, &AtomicBool::new(false)).unwrap_err();
            assert!(matches!(&e, Fail::Failed(m) if m.starts_with("refusing to download from")), "{target}: {e:?}");
            server.join().unwrap();
        }
        // a start URL outside the allow-list never connects
        assert!(to_string("https://evil.example/x", MAX_SUMS, &AtomicBool::new(false)).is_err());
        // a redirect loop ends
        let (base, _server) = serve(vec![redirect("/a"); 8]);
        assert_eq!(to_string(&format!("{base}/a"), MAX_SUMS, &AtomicBool::new(false)).unwrap_err(), Fail::Failed("too many redirects".into()));
    }

    #[test]
    fn the_size_cap_stops_the_download_and_removes_the_file() {
        let dir = temp("cap");
        let dest = dir.join("out.bin");
        // announced too large: refused before any byte is read
        let (base, _s) = serve(vec![ok(&[7u8; 100])]);
        let e = to_file(&format!("{base}/a"), &dest, 50, &AtomicBool::new(false), &mut |_, _| {}).unwrap_err();
        assert!(matches!(&e, Fail::Failed(m) if m.contains("larger than")), "{e:?}");
        assert!(!dest.exists());
        // no Content-Length: counted while reading
        let (base, _s) = serve(vec![b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n0123456789".to_vec()]);
        let e = to_file(&format!("{base}/a"), &dest, 5, &AtomicBool::new(false), &mut |_, _| {}).unwrap_err();
        assert!(matches!(&e, Fail::Failed(m) if m.contains("larger than")), "{e:?}");
        assert!(!dest.exists(), "a partial file is removed");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_short_body_is_an_error() {
        let dir = temp("short");
        let dest = dir.join("out.bin");
        let (base, _s) = serve(vec![b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\nConnection: close\r\n\r\nabc".to_vec()]);
        assert!(matches!(to_file(&format!("{base}/a"), &dest, MAX_DOWNLOAD, &AtomicBool::new(false), &mut |_, _| {}), Err(Fail::Failed(_))));
        assert!(!dest.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn http_errors_are_reported_plainly() {
        for (status, want) in [
            ("404 Not Found", "GitHub answered 404"),
            ("500 Internal Server Error", "GitHub answered 500"),
            ("429 Too Many Requests", "GitHub's limit was reached. Try again later"),
        ] {
            let (base, _s) = serve(vec![format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").into_bytes()]);
            assert_eq!(to_string(&format!("{base}/a"), MAX_SUMS, &AtomicBool::new(false)).unwrap_err(), Fail::Failed(want.into()));
        }
    }

    #[test]
    fn cancel_stops_before_and_during() {
        let dir = temp("cancel");
        let dest = dir.join("out.bin");
        let cancel = AtomicBool::new(true);
        assert_eq!(
            to_file("https://github.com/deepfounder-ai/desigma/releases/download/v1/x", &dest, MAX_DOWNLOAD, &cancel, &mut |_, _| {}),
            Err(Fail::Cancelled)
        );
        // pressed while the bytes arrive
        let (base, _s) = serve(vec![ok(&[1u8; 200_000])]);
        let cancel = AtomicBool::new(false);
        let r = to_file(&format!("{base}/a"), &dest, MAX_DOWNLOAD, &cancel, &mut |_, _| {
            cancel.store(true, Ordering::Relaxed);
        });
        assert_eq!(r, Err(Fail::Cancelled));
        assert!(!dest.exists(), "nothing is left behind");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
