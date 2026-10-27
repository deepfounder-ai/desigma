//! Blocking Figma REST client (cargo feature `net`).
//!
//! The personal access token is supplied by the caller at runtime. It is only
//! ever sent as the `X-Figma-Token` header and is never logged or put in a URL
//! or an error message.

use crate::{Document, FigError};
use std::collections::BTreeMap;
use std::io::Read;

const API: &str = "https://api.figma.com";
const MAX_BODY: u64 = 1 << 30;

fn net_err(what: &str, e: ureq::Error) -> FigError {
    match e {
        ureq::Error::Status(code, _) => FigError::Unsupported(format!("{what}: HTTP {code}")),
        ureq::Error::Transport(t) => FigError::Unsupported(format!("{what}: network error ({:?})", t.kind())),
    }
}

fn get(url: &str, token: Option<&str>, what: &str) -> Result<Vec<u8>, FigError> {
    let agent = desigma_http::agent_for(url);
    let mut req = agent.get(url);
    if let Some(t) = token {
        req = req.set("X-Figma-Token", t);
    }
    let resp = req.call().map_err(|e| net_err(what, e))?;
    let mut out = Vec::new();
    resp.into_reader().take(MAX_BODY).read_to_end(&mut out).map_err(|e| FigError::Unsupported(format!("{what}: read failed ({})", e.kind())))?;
    Ok(out)
}

fn valid_key(key: &str) -> Result<(), FigError> {
    if !key.is_empty() && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        Ok(())
    } else {
        Err(FigError::Unsupported("invalid file key".into()))
    }
}

/// `GET /v1/files/:key?geometry=paths` -> response body (feed to `import_rest_json`).
pub fn fetch_file(key: &str, token: &str) -> Result<String, FigError> {
    valid_key(key)?;
    let body = get(&format!("{API}/v1/files/{key}?geometry=paths"), Some(token), "fetch file")?;
    String::from_utf8(body).map_err(|_| FigError::Format("response is not UTF-8".into()))
}

/// `GET /v1/files/:key/images` -> imageRef -> temporary S3 URL for every image fill in the file.
pub fn fetch_image_urls(key: &str, token: &str) -> Result<BTreeMap<String, String>, FigError> {
    valid_key(key)?;
    let body = get(&format!("{API}/v1/files/{key}/images"), Some(token), "fetch image urls")?;
    let v: serde_json::Value = serde_json::from_slice(&body)?;
    let map = v
        .get("meta")
        .and_then(|m| m.get("images"))
        .and_then(|i| i.as_object())
        .map(|o| o.iter().filter_map(|(k, u)| Some((k.clone(), u.as_str()?.to_string()))).collect())
        .unwrap_or_default();
    Ok(map)
}

/// Downloads one image from a URL returned by [`fetch_image_urls`] (pre-signed, no token sent).
pub fn fetch_image(url: &str) -> Result<Vec<u8>, FigError> {
    if !url.starts_with("https://") {
        return Err(FigError::Unsupported("refusing non-https image url".into()));
    }
    get(url, None, "fetch image")
}

/// Fetches the file plus every image it references, ready to render.
pub fn fetch_document(key: &str, token: &str) -> Result<Document, FigError> {
    let mut doc = crate::import_rest_json(&fetch_file(key, token)?)?;
    let wanted = crate::image_refs(&doc);
    if !wanted.is_empty() {
        let urls = fetch_image_urls(key, token)?;
        for r in wanted {
            if let Some(u) = urls.get(&r) {
                if let Ok(bytes) = fetch_image(u) {
                    doc.images.insert(r, bytes);
                }
            }
        }
    }
    Ok(doc)
}
