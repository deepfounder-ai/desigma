use crate::fontsource::{font_id, metadata_url, FontMeta};
use crate::{bundled_for, FontError, FontRequest};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Source of bytes for a URL. The default implementation (feature `net`) is a
/// blocking `ureq` client; tests inject their own.
pub trait Fetch: Send + Sync {
    fn get(&self, url: &str) -> Result<Vec<u8>, FontError>;
}

#[cfg(feature = "net")]
struct UreqFetch;

#[cfg(feature = "net")]
impl Fetch for UreqFetch {
    fn get(&self, url: &str) -> Result<Vec<u8>, FontError> {
        use std::io::Read;
        const MAX: u64 = 64 * 1024 * 1024;
        // One agent per request: the proxy and its bypass list depend on the URL.
        let opts = desigma_http::AgentOptions {
            timeout_connect: std::time::Duration::from_secs(10),
            timeout: Some(std::time::Duration::from_secs(60)),
            ..Default::default()
        }
        .user_agent("desigma-fonts");
        match desigma_http::agent_with(&opts, url).get(url).call() {
            Ok(resp) => {
                let mut buf = Vec::new();
                resp.into_reader().take(MAX).read_to_end(&mut buf).map_err(|e| FontError::Network(e.to_string()))?;
                Ok(buf)
            }
            Err(ureq::Error::Status(404, _)) => Err(FontError::NotFound(url.to_string())),
            Err(e) => Err(FontError::Network(e.to_string())),
        }
    }
}

/// `dirs::cache_dir()/desigma` (falls back to the temp dir): where desigma keeps what it can
/// download or receive again.
pub fn app_cache_dir() -> PathBuf {
    dirs::cache_dir().unwrap_or_else(std::env::temp_dir).join("desigma")
}

/// License record stored next to each cached font (`<name>.json`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sidecar {
    pub family: String,
    pub license: String,
    /// Weight that was requested.
    pub requested_weight: u16,
    /// Weight of the file actually stored (nearest available).
    pub weight: u16,
    pub italic: bool,
    pub source_url: String,
}

/// On-disk font cache with optional network fallback.
///
/// Layout: `<dir>/<fontsource-id>-<weight>-<normal|italic>.ttf` plus a
/// `.json` sidecar holding the license id.
pub struct FontCache {
    dir: PathBuf,
    fetcher: Option<Box<dyn Fetch>>,
}

impl Default for FontCache {
    fn default() -> Self {
        Self::new()
    }
}

impl FontCache {
    /// `dirs::cache_dir()/desigma/fonts` (falls back to the temp dir), with the
    /// built-in HTTP fetcher when the `net` feature is on.
    pub fn new() -> Self {
        Self::with_dir(Self::default_dir())
    }

    pub fn default_dir() -> PathBuf {
        app_cache_dir().join("fonts")
    }

    /// Cache rooted at `dir` (used by tests).
    pub fn with_dir(dir: impl Into<PathBuf>) -> Self {
        #[cfg(feature = "net")]
        let fetcher: Option<Box<dyn Fetch>> = Some(Box::new(UreqFetch));
        #[cfg(not(feature = "net"))]
        let fetcher: Option<Box<dyn Fetch>> = None;
        Self { dir: dir.into(), fetcher }
    }

    /// Replace the network layer.
    pub fn with_fetcher(mut self, f: Box<dyn Fetch>) -> Self {
        self.fetcher = Some(f);
        self
    }

    /// Never touch the network; only bundled fonts and cached files resolve.
    pub fn offline(mut self) -> Self {
        self.fetcher = None;
        self
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn stem(req: &FontRequest) -> String {
        format!("{}-{}-{}", font_id(&req.family), req.weight, if req.italic { "italic" } else { "normal" })
    }

    pub fn font_path(&self, req: &FontRequest) -> PathBuf {
        self.dir.join(format!("{}.ttf", Self::stem(req)))
    }

    pub fn sidecar_path(&self, req: &FontRequest) -> PathBuf {
        self.dir.join(format!("{}.json", Self::stem(req)))
    }

    /// License record for a cached font, if present.
    pub fn sidecar(&self, req: &FontRequest) -> Option<Sidecar> {
        serde_json::from_slice(&std::fs::read(self.sidecar_path(req)).ok()?).ok()
    }

    /// Cached bytes only (no bundled lookup, no network).
    pub fn cached(&self, req: &FontRequest) -> Option<Vec<u8>> {
        std::fs::read(self.font_path(req)).ok().filter(|b| !b.is_empty())
    }

    /// Bundled font, else cached file, else fetch from Fontsource and cache it.
    /// Only open-licensed fonts are accepted.
    pub fn resolve(&self, req: &FontRequest) -> Result<Vec<u8>, FontError> {
        if let Some(a) = bundled_for(req) {
            return Ok(a.data.to_vec());
        }
        if let Some(bytes) = self.cached(req) {
            return Ok(bytes);
        }
        let fetcher = self.fetcher.as_deref().ok_or_else(|| FontError::Offline(format!("'{}' is not cached and fetching is disabled", req.family)))?;
        let meta_bytes = fetcher.get(&metadata_url(&req.family)).map_err(|e| match e {
            FontError::NotFound(_) => FontError::NotFound(req.family.clone()),
            other => other,
        })?;
        let meta = FontMeta::parse(&String::from_utf8_lossy(&meta_bytes))?;
        if !meta.is_open() {
            return Err(FontError::NotOpenSource { family: meta.family, license: meta.license });
        }
        let pick = meta.pick(req.weight, req.italic)?;
        let bytes = fetcher.get(&pick.url)?;
        if bytes.len() < 12 || !matches!(&bytes[..4], [0, 1, 0, 0] | b"OTTO" | b"true" | b"ttcf") {
            return Err(FontError::Metadata(format!("{} is not a TTF/OTF file", pick.url)));
        }
        self.store(
            req,
            &bytes,
            &Sidecar {
                family: meta.family,
                license: meta.license,
                requested_weight: req.weight,
                weight: pick.weight,
                italic: pick.italic,
                source_url: pick.url,
            },
        )?;
        Ok(bytes)
    }

    fn store(&self, req: &FontRequest, bytes: &[u8], side: &Sidecar) -> Result<(), FontError> {
        std::fs::create_dir_all(&self.dir)?;
        // Sidecar first so a font file never exists without its license record.
        write_atomic(&self.sidecar_path(req), &serde_json::to_vec_pretty(side).map_err(|e| FontError::Metadata(e.to_string()))?)?;
        write_atomic(&self.font_path(req), bytes)?;
        Ok(())
    }
}

fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    const CARLITO: &str = include_str!("../tests/fixtures/fontsource_carlito.json");

    struct Mock {
        routes: HashMap<String, Vec<u8>>,
        hits: Arc<Mutex<Vec<String>>>,
    }

    impl Fetch for Mock {
        fn get(&self, url: &str) -> Result<Vec<u8>, FontError> {
            self.hits.lock().unwrap().push(url.to_string());
            self.routes.get(url).cloned().ok_or_else(|| FontError::NotFound(url.to_string()))
        }
    }

    fn fake_ttf() -> Vec<u8> {
        let mut v = vec![0, 1, 0, 0];
        v.extend_from_slice(&[7u8; 64]);
        v
    }

    fn tempdir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("desigma-fonts-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    fn mock_cache(dir: &Path, meta: &str, hits: Arc<Mutex<Vec<String>>>) -> FontCache {
        let m = FontMeta::parse(meta).unwrap();
        let mut routes = HashMap::new();
        routes.insert(metadata_url(&m.family), meta.as_bytes().to_vec());
        for subs in m.ttf.values().flat_map(|s| s.values()) {
            for url in subs.values() {
                routes.insert(url.clone(), fake_ttf());
            }
        }
        FontCache::with_dir(dir).with_fetcher(Box::new(Mock { routes, hits }))
    }

    #[test]
    fn fetches_stores_then_serves_from_cache() {
        let dir = tempdir("roundtrip");
        let hits = Arc::new(Mutex::new(Vec::new()));
        let cache = mock_cache(&dir, CARLITO, hits.clone());
        let req = FontRequest::new("Carlito", 600, false);
        assert!(cache.cached(&req).is_none());

        let bytes = cache.resolve(&req).unwrap();
        assert_eq!(bytes, fake_ttf());
        assert_eq!(hits.lock().unwrap().len(), 2); // metadata + ttf
        assert!(cache.font_path(&req).ends_with("carlito-600-normal.ttf"));
        let side = cache.sidecar(&req).unwrap();
        assert_eq!(side.license, "OFL-1.1");
        assert_eq!((side.requested_weight, side.weight, side.italic), (600, 700, false));
        assert!(side.source_url.ends_with("latin-700-normal.ttf"));

        // Second resolve, even through an offline cache, uses the disk copy.
        let offline = FontCache::with_dir(&dir).offline();
        assert_eq!(offline.resolve(&req).unwrap(), fake_ttf());
        assert_eq!(hits.lock().unwrap().len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn offline_miss_is_an_error_not_a_panic() {
        let dir = tempdir("offline");
        let cache = FontCache::with_dir(&dir).offline();
        let err = cache.resolve(&FontRequest::new("Roboto", 400, false)).unwrap_err();
        assert!(matches!(err, FontError::Offline(_)));
        assert!(!dir.exists());
    }

    #[test]
    fn bundled_inter_needs_no_cache_or_network() {
        let cache = FontCache::with_dir(tempdir("bundled")).offline();
        let r = cache.resolve(&FontRequest::new("inter", 700, true)).unwrap();
        assert_eq!(r.as_slice(), crate::bundled().iter().find(|a| a.italic).unwrap().data);
    }

    #[test]
    fn closed_license_is_rejected_and_nothing_is_cached() {
        let dir = tempdir("closed");
        let meta = CARLITO.replace("\"license\":\"OFL-1.1\"", "\"license\":\"Proprietary\"");
        assert_ne!(meta, CARLITO);
        let cache = mock_cache(&dir, &meta, Arc::default());
        let err = cache.resolve(&FontRequest::new("Carlito", 400, false)).unwrap_err();
        assert!(matches!(err, FontError::NotOpenSource { ref license, .. } if license == "Proprietary"), "{err:?}");
        assert!(!dir.exists());
    }

    #[test]
    fn non_font_payload_is_rejected() {
        let dir = tempdir("junk");
        let m = FontMeta::parse(CARLITO).unwrap();
        let url = m.pick(400, false).unwrap().url;
        let mut routes = HashMap::new();
        routes.insert(metadata_url("Carlito"), CARLITO.as_bytes().to_vec());
        routes.insert(url, b"<html>oops</html>".to_vec());
        let cache = FontCache::with_dir(&dir).with_fetcher(Box::new(Mock { routes, hits: Arc::default() }));
        assert!(cache.resolve(&FontRequest::new("Carlito", 400, false)).is_err());
        assert!(!cache.font_path(&FontRequest::new("Carlito", 400, false)).exists());
    }

    #[test]
    fn unknown_family_maps_to_not_found() {
        let cache = FontCache::with_dir(tempdir("nf")).with_fetcher(Box::new(Mock { routes: HashMap::new(), hits: Arc::default() }));
        let err = cache.resolve(&FontRequest::new("No Such Font", 400, false)).unwrap_err();
        assert!(matches!(err, FontError::NotFound(ref f) if f == "No Such Font"), "{err:?}");
    }

    #[test]
    fn default_dir_is_under_desigma_fonts() {
        let d = FontCache::default_dir();
        assert!(d.ends_with(Path::new("desigma").join("fonts")));
    }
}
