//! Native plugin bundle storage. Installation never evaluates downloaded code.
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use url::Url;

const REPOSITORY: &str = "deepfounder-ai/desigma-plugins";
const MAX_BYTES: usize = 2 * 1024 * 1024;
const MAX_TOTAL_BYTES: usize = 8 * 1024 * 1024;
static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InstalledPlugin {
    pub id: String,
    pub name: String,
    pub version: String,
    pub license: String,
    pub source_url: String,
    pub manifest_url: String,
    pub main: String,
    pub permissions: Vec<String>,
    pub runtime_supported: bool,
    pub installed_at: u64,
    #[serde(default = "schema_one")]
    pub schema_version: u32,
    #[serde(default)]
    pub panel: Option<String>,
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(default)]
    pub network: Vec<String>,
}

fn schema_one() -> u32 {
    1
}

#[derive(Debug)]
pub struct Bundle {
    pub plugin: InstalledPlugin,
    pub panel: String,
    pub files: BTreeMap<String, Vec<u8>>,
    pub network: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema_version: u32,
    id: String,
    name: String,
    version: String,
    license: String,
    main: String,
    permissions: Vec<String>,
    #[serde(default)]
    panel: Option<String>,
    #[serde(default)]
    files: Vec<String>,
    #[serde(default)]
    network: Vec<String>,
}

/// Canonical raw manifest URL. This intentionally accepts only the native registry.
pub fn validate_url(input: &str) -> Result<String, String> {
    let url = Url::parse(input.trim()).map_err(|_| "Paste a valid HTTPS plugin manifest URL.".to_string())?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("Use an HTTPS manifest URL without credentials, a port, query, or fragment.".into());
    }
    // Reject encoded separators and dot segments before URL normalization can hide them.
    if input.contains('%') || input.contains('\\') || input.split('/').any(|p| p == "." || p == "..") {
        return Err("Plugin URLs cannot contain escaped paths or parent-directory segments.".into());
    }
    let parts: Vec<_> = url.path().trim_start_matches('/').split('/').collect();
    let native = match url.host_str() {
        Some("github.com") if parts.len() == 7 && parts[2] == "blob" => vec![parts[0], parts[1], parts[3], parts[4], parts[5], parts[6]],
        Some("raw.githubusercontent.com") if parts.len() == 6 => parts,
        _ => return Err(format!("Install a native manifest from https://github.com/{REPOSITORY}. Figma Community plugins are not supported.")),
    };
    if native[0] != "deepfounder-ai"
        || native[1] != "desigma-plugins"
        || native[3] != "plugins"
        || native[5] != "manifest.json"
        || !safe_part(native[2])
        || !safe_part(native[4])
    {
        return Err(format!("Use a manifest at {REPOSITORY}/blob/<ref>/plugins/<name>/manifest.json."));
    }
    Ok(format!("https://raw.githubusercontent.com/{}", native.join("/")))
}

fn safe_part(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 100
        && value != "."
        && value != ".."
        && value.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

fn validate_asset_path(path: &str) -> Result<(), String> {
    if path.len() > 240 || !path.split('/').all(safe_part) || path.contains('\\') {
        return Err("Plugin files must use safe relative paths.".into());
    }
    for part in path.split('/') {
        let stem = part.split('.').next().unwrap_or_default().to_ascii_uppercase();
        if matches!(
            stem.as_str(),
            "CON"
                | "PRN"
                | "AUX"
                | "NUL"
                | "COM1"
                | "COM2"
                | "COM3"
                | "COM4"
                | "COM5"
                | "COM6"
                | "COM7"
                | "COM8"
                | "COM9"
                | "LPT1"
                | "LPT2"
                | "LPT3"
                | "LPT4"
                | "LPT5"
                | "LPT6"
                | "LPT7"
                | "LPT8"
                | "LPT9"
        ) || part.ends_with('.')
        {
            return Err("Plugin file contains a reserved name.".into());
        }
    }
    Ok(())
}

fn validate_manifest(manifest: &Manifest) -> Result<(), String> {
    if !matches!(manifest.schema_version, 1 | 2) {
        return Err("Unsupported plugin schema_version; this build supports versions 1 and 2.".into());
    }
    if !safe_part(&manifest.id) || !safe_part(&manifest.version) || manifest.id.len() + manifest.version.len() > 100 {
        return Err("Plugin id and version must contain only letters, digits, dots, dashes, or underscores (up to 100 characters combined).".into());
    }
    if manifest.name.trim().is_empty() || manifest.name.len() > 200 || manifest.license.trim().is_empty() || manifest.license.len() > 200 {
        return Err("Plugin name and license are required and must be at most 200 characters.".into());
    }
    validate_asset_path(&manifest.main)?;
    if !manifest.main.ends_with(".js") {
        return Err("Plugin main must be a relative .js path.".into());
    }
    let assets = if manifest.schema_version == 1 {
        if !manifest.permissions.is_empty() || manifest.panel.is_some() || !manifest.files.is_empty() || !manifest.network.is_empty() {
            return Err("Schema 1 plugins cannot declare permissions, panel, files, or network.".into());
        }
        vec![manifest.main.clone()]
    } else {
        let panel = manifest.panel.as_ref().ok_or("Schema 2 requires a panel HTML file.")?;
        validate_asset_path(panel)?;
        if panel.contains('/')
            || !panel.ends_with(".html")
            || manifest.files.is_empty()
            || manifest.files.len() > 16
            || !manifest.files.contains(panel)
            || !manifest.files.contains(&manifest.main)
        {
            return Err("Schema 2 must list its HTML panel and main script in files (at most 16 assets).".into());
        }
        let mut permissions = BTreeSet::new();
        if manifest.permissions.iter().any(|p| !matches!(p.as_str(), "document:insert-svg" | "storage:local") || !permissions.insert(p)) {
            return Err("Unsupported or duplicate plugin permission.".into());
        }
        if manifest.network.len() > 1 || manifest.network.iter().any(|origin| origin != "https://api.iconify.design") {
            return Err("Only the https://api.iconify.design network origin is supported.".into());
        }
        manifest.files.clone()
    };
    let mut names =
        BTreeSet::from(["manifest.json".to_string(), "installed.json".to_string(), "license".to_string(), "host.json".to_string(), "insert-svg".to_string()]);
    let mut path_spellings = BTreeMap::new();
    for asset in assets {
        validate_asset_path(&asset)?;
        let mut prefix = String::new();
        for part in asset.split('/') {
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(part);
            if let Some(previous) = path_spellings.insert(prefix.to_ascii_lowercase(), prefix.clone()) {
                if previous != prefix {
                    return Err("Plugin paths differ only by letter case.".into());
                }
            }
        }
        let key = asset.to_ascii_lowercase();
        if !names.insert(key) {
            return Err("Plugin files collide with another asset or reserved metadata.".into());
        }
    }
    for name in &names {
        let mut parent = name.as_str();
        while let Some((prefix, _)) = parent.rsplit_once('/') {
            if names.contains(prefix) {
                return Err("Plugin asset path has a file/directory collision.".into());
            }
            parent = prefix;
        }
    }
    Ok(())
}

fn bundle_directory(root: &Path, id: &str, version: &str) -> PathBuf {
    let encode = |s: &str| s.bytes().map(|b| format!("{b:02x}")).collect::<String>();
    root.join("plugins").join(format!("{}--{}", encode(id), encode(version)))
}

fn read_bundle_file(directory: &Path, relative: &str) -> Result<Vec<u8>, String> {
    let metadata = fs::symlink_metadata(directory).map_err(|e| format!("Cannot locate plugin bundle: {e}"))?;
    if metadata.file_type().is_symlink() {
        return Err("Plugin bundles cannot be symbolic links.".into());
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err("Plugin bundles cannot be reparse points.".into());
        }
    }
    let canonical = fs::canonicalize(directory).map_err(|e| format!("Cannot locate plugin bundle: {e}"))?;
    let mut path = directory.to_path_buf();
    for part in relative.split('/') {
        path.push(part);
        let metadata = fs::symlink_metadata(&path).map_err(|e| format!("Cannot read plugin asset {relative}: {e}"))?;
        if metadata.file_type().is_symlink() {
            return Err("Plugin assets cannot be symbolic links.".into());
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x400 != 0 {
                return Err("Plugin assets cannot be reparse points.".into());
            }
        }
        if !fs::canonicalize(&path).map_err(|e| e.to_string())?.starts_with(&canonical) {
            return Err("Plugin asset escapes its bundle.".into());
        }
    }
    let metadata = fs::metadata(&path).map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.len() > MAX_BYTES as u64 {
        return Err("Invalid or oversized plugin asset.".into());
    }
    let mut bytes = Vec::new();
    fs::File::open(path).map_err(|e| e.to_string())?.take((MAX_BYTES + 1) as u64).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    check_size(bytes)
}

/// Loads only declared assets, validating the manifest again before execution.
pub fn load_bundle(root: &Path, plugin: &InstalledPlugin) -> Result<Bundle, String> {
    let directory = bundle_directory(root, &plugin.id, &plugin.version);
    let bytes = read_bundle_file(&directory, "manifest.json")?;
    let manifest: Manifest = serde_json::from_slice(&bytes).map_err(|e| format!("Invalid stored manifest: {e}"))?;
    validate_manifest(&manifest)?;
    if manifest.id != plugin.id || manifest.version != plugin.version {
        return Err("Stored manifest identity does not match the installed plugin.".into());
    }
    if manifest.schema_version != 2 {
        return Err("Schema 1 bundles do not support execution.".into());
    }
    let license = read_bundle_file(&directory, "LICENSE")?;
    if std::str::from_utf8(&license).map_or(true, |s| s.trim().is_empty()) {
        return Err("Missing or invalid plugin license.".into());
    }
    let mut total = bytes.len() + license.len();
    let mut files = BTreeMap::new();
    for path in &manifest.files {
        let bytes = read_bundle_file(&directory, path)?;
        total += bytes.len();
        if total > MAX_TOTAL_BYTES {
            return Err("Plugin bundle exceeds 8 MiB.".into());
        }
        files.insert(path.clone(), bytes);
    }
    let mut plugin = plugin.clone();
    plugin.name = manifest.name;
    plugin.license = manifest.license;
    plugin.main = manifest.main;
    plugin.schema_version = manifest.schema_version;
    plugin.panel = manifest.panel.clone();
    plugin.files = manifest.files;
    plugin.permissions = manifest.permissions;
    plugin.network = manifest.network.clone();
    plugin.runtime_supported = true;
    Ok(Bundle { plugin, panel: manifest.panel.expect("validated panel"), files, network: manifest.network })
}

pub fn install(root: &Path, input: &str) -> Result<InstalledPlugin, String> {
    let opts = desigma_http::AgentOptions {
        timeout_connect: Duration::from_secs(10),
        timeout_read: Some(Duration::from_secs(30)),
        timeout_write: Some(Duration::from_secs(30)),
        redirects: 0,
        ..Default::default()
    }
    .user_agent("desigma-plugin-installer");
    let token = std::env::var("DESIGMA_PLUGINS_TOKEN").ok().filter(|value| !value.trim().is_empty());
    install_with(root, input, |url| {
        // All supplied URLs were constructed from the validated native repository path.
        // Send credentials only to GitHub's API, never to downloaded plugin code.
        let source = Url::parse(url).map_err(|_| "Invalid plugin download URL.".to_string())?;
        let segments: Vec<_> = source.path().trim_start_matches('/').split('/').collect();
        let mut api_url = Url::parse(&format!("https://api.github.com/repos/{REPOSITORY}/contents/{}", segments[3..].join("/")))
            .map_err(|_| "Invalid plugin download path.".to_string())?;
        api_url.query_pairs_mut().append_pair("ref", segments[2]);
        let agent = desigma_http::agent_with(&opts, api_url.as_str());
        let mut request = agent.get(api_url.as_str()).set("Accept", "application/vnd.github.raw+json").set("X-GitHub-Api-Version", "2022-11-28");
        if let Some(token) = &token {
            request = request.set("Authorization", &format!("Bearer {}", token.trim()));
        }
        let response = request.call().map_err(|err| -> String { match err {
            ureq::Error::Status(401 | 403 | 404, _) => "The plugin file is unavailable. Check the manifest URL. If GitHub's hourly rate limit was reached, set DESIGMA_PLUGINS_TOKEN to a GitHub token and restart Desigma.".into(),
            _ => "Plugin download failed. Check your connection and the manifest URL.".into(),
        } })?;
        if response.status() != 200 {
            return Err("The plugin URL must return a file directly; redirects are not supported.".into());
        }
        let mut bytes = Vec::new();
        response.into_reader().take((MAX_BYTES + 1) as u64).read_to_end(&mut bytes).map_err(|e| format!("Cannot read plugin download: {e}"))?;
        check_size(bytes)
    })
}

fn check_size(bytes: Vec<u8>) -> Result<Vec<u8>, String> {
    if bytes.len() > MAX_BYTES {
        Err("Plugin files must be no larger than 2 MiB each.".into())
    } else {
        Ok(bytes)
    }
}

fn install_with(root: &Path, input: &str, mut fetch: impl FnMut(&str) -> Result<Vec<u8>, String>) -> Result<InstalledPlugin, String> {
    let manifest_url = validate_url(input)?;
    let bytes = check_size(fetch(&manifest_url)?)?;
    let manifest: Manifest = serde_json::from_slice(&bytes).map_err(|e| format!("Invalid native plugin manifest: {e}"))?;
    validate_manifest(&manifest)?;
    let plugins = root.join("plugins");
    let destination = bundle_directory(root, &manifest.id, &manifest.version);
    if destination.exists() {
        return Err(format!("{} {} is already installed.", manifest.name, manifest.version));
    }
    let base = manifest_url.strip_suffix("manifest.json").expect("validated manifest URL");
    let paths = if manifest.schema_version == 2 { manifest.files.clone() } else { vec![manifest.main.clone()] };
    let mut assets = BTreeMap::new();
    let mut total = bytes.len();
    for path in paths {
        let content = check_size(fetch(&format!("{base}{path}"))?)?;
        total += content.len();
        if total > MAX_TOTAL_BYTES {
            return Err("Plugin bundle exceeds 8 MiB.".into());
        }
        assets.insert(path, content);
    }
    let license = check_size(fetch(&format!("{base}LICENSE"))?)?;
    if total + license.len() > MAX_TOTAL_BYTES {
        return Err("Plugin bundle exceeds 8 MiB.".into());
    }
    if std::str::from_utf8(&license).map_or(true, |text| text.trim().is_empty()) {
        return Err("The plugin must include a nonempty UTF-8 LICENSE file beside its manifest.".into());
    }
    let installed = InstalledPlugin {
        id: manifest.id.clone(),
        name: manifest.name.clone(),
        version: manifest.version.clone(),
        license: manifest.license.clone(),
        source_url: input.trim().into(),
        manifest_url,
        main: manifest.main.clone(),
        permissions: manifest.permissions.clone(),
        runtime_supported: manifest.schema_version == 2,
        schema_version: manifest.schema_version,
        panel: manifest.panel.clone(),
        files: manifest.files.clone(),
        network: manifest.network.clone(),
        installed_at: SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs(),
    };
    fs::create_dir_all(&plugins).map_err(|e| format!("Cannot create plugin storage: {e}"))?;
    let temporary = plugins.join(format!(".install-{}-{}-{}", std::process::id(), installed.installed_at, NEXT_TEMP.fetch_add(1, Ordering::Relaxed)));
    fs::create_dir(&temporary).map_err(|e| format!("Cannot create temporary plugin directory: {e}"))?;
    let result = (|| -> Result<(), String> {
        for (path, content) in &assets {
            let path = temporary.join(path);
            fs::create_dir_all(path.parent().expect("asset has parent")).map_err(|e| e.to_string())?;
            write_synced(&path, content)?;
        }
        write_synced(&temporary.join("LICENSE"), &license)?;
        write_synced(&temporary.join("manifest.json"), &bytes)?;
        write_synced(&temporary.join("installed.json"), &serde_json::to_vec_pretty(&installed).map_err(|e| e.to_string())?)?;
        // Destination bundles are always nonempty; rename cannot replace an existing install.
        if destination.exists() {
            return Err("This plugin version is already installed.".into());
        }
        fs::rename(&temporary, &destination).map_err(|e| format!("Cannot finish plugin installation: {e}"))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&temporary);
    }
    result?;
    Ok(installed)
}

fn write_synced(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = fs::OpenOptions::new().write(true).create_new(true).open(path).map_err(|e| format!("Cannot write plugin file: {e}"))?;
    file.write_all(bytes).and_then(|_| file.sync_all()).map_err(|e| format!("Cannot save plugin file: {e}"))
}

pub fn list(root: &Path) -> Result<Vec<InstalledPlugin>, String> {
    let plugins = root.join("plugins");
    if !plugins.exists() {
        return Ok(Vec::new());
    }
    let mut installed = Vec::new();
    for entry in fs::read_dir(&plugins).map_err(|e| format!("Cannot read installed plugins: {e}"))? {
        let entry = entry.map_err(|e| e.to_string())?;
        if entry.file_name().to_string_lossy().starts_with('.') || !entry.file_type().map_err(|e| e.to_string())?.is_dir() {
            continue;
        }
        let bytes = read_bundle_file(&entry.path(), "installed.json")?;
        let mut plugin: InstalledPlugin = serde_json::from_slice(&bytes).map_err(|e| format!("Invalid installed plugin metadata: {e}"))?;
        plugin.runtime_supported = false;
        if entry.path() == bundle_directory(root, &plugin.id, &plugin.version) {
            if let Ok(bundle) = load_bundle(root, &plugin) {
                plugin = bundle.plugin;
            }
        }
        installed.push(plugin);
    }
    installed.sort_by(|a, b| a.name.cmp(&b.name).then(a.version.cmp(&b.version)));
    Ok(installed)
}

#[cfg(test)]
mod tests {
    use super::*;
    const URL: &str = "https://github.com/deepfounder-ai/desigma-plugins/blob/main/plugins/hello/manifest.json";
    fn manifest() -> Vec<u8> {
        br#"{"schema_version":1,"id":"hello","name":"Hello","version":"1.0.0","license":"MIT","main":"dist/main.js","permissions":[]}"#.to_vec()
    }
    fn root() -> std::path::PathBuf {
        std::env::temp_dir().join(format!("desigma-plugins-test-{}-{}", std::process::id(), NEXT_TEMP.fetch_add(1, Ordering::Relaxed)))
    }
    #[test]
    fn only_native_repository_manifests_are_accepted() {
        let raw = validate_url(URL).unwrap();
        assert_eq!(raw, "https://raw.githubusercontent.com/deepfounder-ai/desigma-plugins/main/plugins/hello/manifest.json");
        assert_eq!(validate_url(&raw).unwrap(), raw);
        for bad in [
            "https://www.figma.com/community/plugin/735098390272716381/iconify",
            "https://github.com/other/desigma-plugins/blob/main/plugins/hello/manifest.json",
            "https://raw.githubusercontent.com/deepfounder-ai/desigma-plugins/main/plugins/../manifest.json",
            "https://raw.githubusercontent.com/deepfounder-ai/desigma-plugins/main/plugins/%2e%2e/manifest.json",
            "https://user@github.com/deepfounder-ai/desigma-plugins/blob/main/plugins/hello/manifest.json",
        ] {
            assert!(validate_url(bad).is_err(), "{bad}");
        }
    }
    #[test]
    fn install_roundtrip_duplicate_and_inert_script() {
        let root = root();
        let plugin = install_with(&root, URL, |url| {
            Ok(if url.ends_with("manifest.json") {
                manifest()
            } else if url.ends_with("/LICENSE") {
                b"MIT license test notice".to_vec()
            } else {
                assert!(url.ends_with("/plugins/hello/dist/main.js"));
                b"throw new Error('never execute');".to_vec()
            })
        })
        .unwrap();
        assert!(!plugin.runtime_supported);
        assert_eq!(list(&root).unwrap()[0].id, "hello");
        assert!(install_with(&root, URL, |_| Ok(manifest())).unwrap_err().contains("already installed"));
        let entry = fs::read_dir(root.join("plugins")).unwrap().next().unwrap().unwrap();
        assert_eq!(fs::read(entry.path().join("dist/main.js")).unwrap(), b"throw new Error('never execute');");
        assert_eq!(fs::read(entry.path().join("LICENSE")).unwrap(), b"MIT license test notice");
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn failures_leave_no_install_and_reject_capabilities_and_traversal() {
        let root = root();
        assert!(install_with(&root, URL, |url| if url.ends_with("manifest.json") { Ok(manifest()) } else { Err("offline".into()) }).is_err());
        assert!(list(&root).unwrap().is_empty());
        for (key, value) in
            [("main", serde_json::json!("../escape.js")), ("permissions", serde_json::json!(["network"])), ("schema_version", serde_json::json!(2))]
        {
            let mut m: serde_json::Value = serde_json::from_slice(&manifest()).unwrap();
            m[key] = value;
            assert!(install_with(&root, URL, |_| Ok(serde_json::to_vec(&m).unwrap())).is_err());
        }
        assert!(check_size(vec![0; MAX_BYTES + 1]).is_err());
        assert!(!root.exists());
    }
    #[test]
    fn incomplete_storage_is_removed() {
        let root = root();
        let mut m: serde_json::Value = serde_json::from_slice(&manifest()).unwrap();
        // A directory/file collision forces failure after writing the downloaded script.
        m["main"] = serde_json::json!("manifest.json/main.js");
        assert!(install_with(&root, URL, |url| Ok(if url.ends_with("manifest.json") { serde_json::to_vec(&m).unwrap() } else { b"void 0;".to_vec() })).is_err());
        assert!(!root.join("plugins").exists());
        assert!(list(&root).unwrap().is_empty());
        if root.exists() {
            fs::remove_dir_all(root).unwrap();
        }
    }
    fn manifest_v2() -> serde_json::Value {
        serde_json::json!({"schema_version":2,"id":"icons","name":"Icons","version":"1.0.0","license":"MIT","main":"main.js","panel":"index.html","files":["index.html","main.js","style.css"],"permissions":["document:insert-svg","storage:local"],"network":["https://api.iconify.design"]})
    }
    fn install_v2(root: &Path) -> InstalledPlugin {
        install_with(root, URL, |url| Ok(if url.ends_with("manifest.json") { serde_json::to_vec(&manifest_v2()).unwrap() } else { b"test asset".to_vec() }))
            .unwrap()
    }
    #[test]
    fn browser_bundle_roundtrip_and_incomplete_assets() {
        let root = root();
        let plugin = install_v2(&root);
        assert!(plugin.runtime_supported);
        let bundle = load_bundle(&root, &plugin).unwrap();
        assert_eq!(bundle.panel, "index.html");
        assert_eq!(bundle.files.len(), 3);
        assert_eq!(bundle.network, ["https://api.iconify.design"]);
        assert!(list(&root).unwrap()[0].runtime_supported);
        let directory = bundle_directory(&root, &plugin.id, &plugin.version);
        fs::write(directory.join("undeclared.js"), b"never served").unwrap();
        assert!(!load_bundle(&root, &plugin).unwrap().files.contains_key("undeclared.js"));
        fs::remove_file(directory.join("style.css")).unwrap();
        assert!(load_bundle(&root, &plugin).is_err());
        assert!(!list(&root).unwrap()[0].runtime_supported);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn browser_manifest_rejects_capability_and_path_escapes() {
        for (key, value) in [
            ("permissions", serde_json::json!(["network"])),
            ("permissions", serde_json::json!(["storage:local", "storage:local"])),
            ("network", serde_json::json!(["https://api.iconify.design/evil"])),
            ("network", serde_json::json!(["https://other.example"])),
            ("files", serde_json::json!(["index.html", "main.js", "../escape"])),
            ("files", serde_json::json!(["index.html", "main.js", "MAIN.js"])),
            ("files", serde_json::json!(["index.html", "main.js", "manifest.json"])),
            ("files", serde_json::json!(["index.html", "main.js", "HOST.json"])),
            ("files", serde_json::json!(["index.html", "main.js", "insert-svg"])),
            ("files", serde_json::json!(["index.html", "main.js", "LICENSE/notice"])),
            ("files", serde_json::json!(["index.html", "main.js", "foo", "Foo/bar"])),
            ("files", serde_json::json!(["index.html", "main.js", "con.txt"])),
            ("files", serde_json::json!(["index.html", "main.js", "Foo/a", "foo/b"])),
            ("files", serde_json::json!(["index.html", "main.js", "assets./x"])),
            ("files", serde_json::json!(["main.js"])),
        ] {
            let mut value_manifest = manifest_v2();
            value_manifest[key] = value;
            let manifest: Manifest = serde_json::from_value(value_manifest).unwrap();
            assert!(validate_manifest(&manifest).is_err(), "{key}: {manifest:?}");
        }
    }
    #[test]
    fn browser_panel_must_be_at_bundle_root() {
        let mut value = manifest_v2();
        value["panel"] = serde_json::json!("ui/index.html");
        value["files"] = serde_json::json!(["ui/index.html", "main.js"]);
        let manifest: Manifest = serde_json::from_value(value).unwrap();
        assert!(validate_manifest(&manifest).is_err());
    }
    #[test]
    fn installed_metadata_cannot_grant_runtime_capabilities() {
        let root = root();
        let plugin = install_v2(&root);
        let directory = bundle_directory(&root, &plugin.id, &plugin.version);
        let mut fake = plugin.clone();
        fake.permissions = vec!["arbitrary".into()];
        fake.network = vec!["https://other.example".into()];
        fake.main = "undeclared.js".into();
        fs::write(directory.join("installed.json"), serde_json::to_vec(&fake).unwrap()).unwrap();
        let actual = list(&root).unwrap().remove(0);
        assert_eq!(actual.permissions, plugin.permissions);
        assert_eq!(actual.network, plugin.network);
        assert_eq!(actual.main, plugin.main);
        let mut manifest = manifest_v2();
        manifest["id"] = serde_json::json!("imposter");
        fs::write(directory.join("manifest.json"), serde_json::to_vec(&manifest).unwrap()).unwrap();
        assert!(load_bundle(&root, &plugin).is_err());
        assert!(!list(&root).unwrap()[0].runtime_supported);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn schema_one_metadata_without_new_fields_stays_inert() {
        let root = root();
        let plugin = install_with(&root, URL, |url| Ok(if url.ends_with("manifest.json") { manifest() } else { b"test".to_vec() })).unwrap();
        let directory = bundle_directory(&root, &plugin.id, &plugin.version);
        let mut metadata = serde_json::to_value(&plugin).unwrap();
        for field in ["schema_version", "panel", "files", "network"] {
            metadata.as_object_mut().unwrap().remove(field);
        }
        metadata["runtime_supported"] = serde_json::json!(true);
        fs::write(directory.join("installed.json"), serde_json::to_vec(&metadata).unwrap()).unwrap();
        assert!(!list(&root).unwrap()[0].runtime_supported);
        assert_eq!(list(&root).unwrap()[0].schema_version, 1);
        assert!(load_bundle(&root, &plugin).is_err());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn bundle_total_limit_applies_before_storage() {
        let root = root();
        let mut manifest = manifest_v2();
        manifest["files"] = serde_json::json!(["index.html", "main.js", "a", "b"]);
        let result =
            install_with(&root, URL, |url| Ok(if url.ends_with("manifest.json") { serde_json::to_vec(&manifest).unwrap() } else { vec![b'a'; MAX_BYTES] }));
        assert!(result.unwrap_err().contains("8 MiB"));
        assert!(!root.exists());
    }
    #[cfg(unix)]
    #[test]
    fn symbolic_link_assets_are_not_loaded() {
        use std::os::unix::fs::symlink;
        let root = root();
        let plugin = install_v2(&root);
        let directory = bundle_directory(&root, &plugin.id, &plugin.version);
        fs::remove_file(directory.join("main.js")).unwrap();
        symlink(directory.join("style.css"), directory.join("main.js")).unwrap();
        assert!(load_bundle(&root, &plugin).unwrap_err().contains("symbolic links"));
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    #[ignore = "Needs network access to the public plugin registry"]
    fn real_registry_install() {
        let root = root();
        let url = "https://github.com/deepfounder-ai/desigma-plugins/blob/main/plugins/install-demo/manifest.json";
        let plugin = install(&root, url).expect("Install the published native sample");
        let reloaded = list(&root).unwrap();
        assert_eq!(reloaded.len(), 1);
        assert_eq!(reloaded[0].id, plugin.id);
        assert_eq!(reloaded[0].version, plugin.version);
        assert!(!reloaded[0].runtime_supported);
        assert!(install(&root, url).unwrap_err().contains("already installed"));
        fs::remove_dir_all(root).unwrap();
    }
}
