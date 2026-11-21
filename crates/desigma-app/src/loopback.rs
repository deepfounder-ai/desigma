//! One-request HTTP/1.1 parsing shared by the plugin panel server and the agent bridge.
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    net::TcpStream,
    sync::atomic::{AtomicBool, Ordering},
    time::Instant,
};

pub const MAX_BODY: usize = 32 * 1024 * 1024;
pub const MAX_HEADER: usize = 8192;
pub type Failure = (&'static str, &'static str);

#[derive(Debug)]
pub struct Request {
    pub method: String,
    pub path: String,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

/// Reads one HTTP/1.1 request within `deadline`; `stopped` aborts early. The body is consumed before a
/// rejection only when its `Content-Length` is within `MAX_BODY`.
pub fn read_request(stream: &mut TcpStream, stopped: &AtomicBool, deadline: Instant) -> Result<Request, Failure> {
    let mut header = Vec::new();
    while !header.ends_with(b"\r\n\r\n") {
        if header.len() >= MAX_HEADER {
            return Err(("431 Request Header Fields Too Large", "Headers too large"));
        }
        let mut byte = [0];
        read_exact(stream, &mut byte, stopped, deadline)?;
        header.push(byte[0]);
    }
    let header = std::str::from_utf8(&header).map_err(|_| ("400 Bad Request", "Invalid headers"))?;
    let mut lines = header.split("\r\n");
    let parts: Vec<_> = lines.next().unwrap_or_default().split(' ').collect();
    if parts.len() != 3 || parts[2] != "HTTP/1.1" {
        return Err(("400 Bad Request", "Invalid request"));
    }
    let mut headers = BTreeMap::new();
    for line in lines.filter(|line| !line.is_empty()) {
        let (name, value) = line.split_once(':').ok_or(("400 Bad Request", "Invalid header"))?;
        if name.is_empty()
            || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
            || headers.insert(name.to_ascii_lowercase(), value.trim().to_string()).is_some()
        {
            return Err(("400 Bad Request", "Invalid or duplicate header"));
        }
    }
    if headers.contains_key("transfer-encoding") {
        return Err(("400 Bad Request", "Transfer encoding is unsupported"));
    }
    // Consume a bounded, framed body before rejecting a request: closing a Windows socket with unread bytes can RST the error response away.
    let length = match headers.get("content-length") {
        Some(value) => value.parse::<usize>().map_err(|_| ("400 Bad Request", "Invalid content length"))?,
        None if parts[0] == "POST" => return Err(("411 Length Required", "Content length required")),
        None => 0,
    };
    if length > MAX_BODY {
        return Err(("413 Payload Too Large", "Request too large"));
    }
    let mut body = vec![0; length];
    read_exact(stream, &mut body, stopped, deadline)?;
    Ok(Request { method: parts[0].to_string(), path: parts[1].to_string(), headers, body })
}

fn read_exact(stream: &mut TcpStream, mut bytes: &mut [u8], stopped: &AtomicBool, deadline: Instant) -> Result<(), Failure> {
    while !bytes.is_empty() {
        if stopped.load(Ordering::Acquire) || Instant::now() >= deadline {
            return Err(("408 Request Timeout", "Request timed out"));
        }
        match stream.read(bytes) {
            Ok(0) => return Err(("400 Bad Request", "Incomplete request")),
            Ok(n) => bytes = &mut bytes[n..],
            Err(e) if matches!(e.kind(), std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted) => {}
            Err(_) => return Err(("400 Bad Request", "Cannot read request")),
        }
    }
    Ok(())
}

pub fn write_response(stream: &mut TcpStream, status: &str, mime: &str, body: &[u8], extra_headers: &str) {
    let headers = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\n{extra_headers}\r\n",
        body.len()
    );
    let _ = stream.write_all(headers.as_bytes()).and_then(|()| stream.write_all(body));
}

pub fn json_error(message: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({"error": message})).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::Write, net::TcpListener, thread, time::Duration};

    fn roundtrip(raw: &str) -> Result<Request, Failure> {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let raw = raw.to_string();
        let client = thread::spawn(move || {
            let mut s = TcpStream::connect(addr).unwrap();
            s.write_all(raw.as_bytes()).unwrap();
            s
        });
        let (mut stream, _) = listener.accept().unwrap();
        stream.set_read_timeout(Some(Duration::from_millis(250))).unwrap();
        let result = read_request(&mut stream, &AtomicBool::new(false), Instant::now() + Duration::from_secs(2));
        drop(client.join().unwrap());
        result
    }

    #[test]
    fn parses_a_post_with_body_and_lowercases_headers() {
        let r = roundtrip("POST /t/call HTTP/1.1\r\nHost: x\r\nX-Desigma-Token: abc\r\nContent-Length: 2\r\n\r\n{}").unwrap();
        assert_eq!((r.method.as_str(), r.path.as_str(), r.headers["x-desigma-token"].as_str()), ("POST", "/t/call", "abc"));
        assert_eq!(r.body, b"{}");
    }

    #[test]
    fn rejects_malformed_oversized_and_unframed_requests() {
        assert_eq!(roundtrip("GET /x HTTP/1.0\r\n\r\n").unwrap_err().0, "400 Bad Request");
        assert_eq!(roundtrip("POST /x HTTP/1.1\r\nHost: x\r\n\r\n").unwrap_err().0, "411 Length Required");
        assert_eq!(roundtrip(&format!("POST /x HTTP/1.1\r\nHost: x\r\nContent-Length: {}\r\n\r\n", MAX_BODY + 1)).unwrap_err().0, "413 Payload Too Large");
        assert_eq!(roundtrip("GET /x HTTP/1.1\r\nHost: a\r\nHost: b\r\n\r\n").unwrap_err().0, "400 Bad Request");
        assert_eq!(roundtrip("GET /x HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n").unwrap_err().0, "400 Bad Request");
    }
}
