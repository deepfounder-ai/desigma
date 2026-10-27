//! App settings ("Your name", GPU, update checks), stored in `settings.json` in the library root.
//!
//! The app had no settings store, so this is a small JSON object; keys it does not
//! know are preserved so other settings can live beside `author`.

use std::path::Path;

pub const FILE: &str = "settings.json";

/// The OS user name, or "You" when it cannot be determined.
pub fn os_user_name() -> String {
    ["USERNAME", "USER", "LOGNAME"]
        .iter()
        .filter_map(|k| std::env::var(k).ok())
        .map(|s| s.trim().to_string())
        .find(|s| !s.is_empty())
        .unwrap_or_else(|| "You".into())
}

fn read(root: &Path) -> serde_json::Map<String, serde_json::Value> {
    std::fs::read_to_string(root.join(FILE))
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default()
}

/// The saved author name, if one was set.
pub fn load_author(root: &Path) -> Option<String> {
    read(root).get("author").and_then(|v| v.as_str()).map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

/// A boolean app setting stored beside the author name.
pub fn load_flag(root: &Path, key: &str) -> Option<bool> {
    read(root).get(key).and_then(|v| v.as_bool())
}

pub fn save_flag(root: &Path, key: &str, value: bool) -> Result<(), String> {
    let mut map = read(root);
    map.insert(key.into(), value.into());
    write(root, map)
}

/// A string app setting (empty when missing).
pub fn load_string(root: &Path, key: &str) -> Option<String> {
    read(root).get(key).and_then(|v| v.as_str()).map(str::to_string).filter(|s| !s.is_empty())
}

pub fn save_string(root: &Path, key: &str, value: &str) -> Result<(), String> {
    let mut map = read(root);
    map.insert(key.into(), value.into());
    write(root, map)
}

/// A nested object setting (`key` -> JSON object), or `None` when missing.
pub fn load_object(root: &Path, key: &str) -> Option<serde_json::Value> {
    read(root).get(key).filter(|v| v.is_object()).cloned()
}

pub fn save_object(root: &Path, key: &str, value: serde_json::Value) -> Result<(), String> {
    let mut map = read(root);
    map.insert(key.into(), value);
    write(root, map)
}

/// Key of the network proxy settings in `settings.json`.
pub const PROXY_KEY: &str = "proxy";

/// The saved proxy settings (system defaults when the file has none).
pub fn load_proxy(root: &Path) -> desigma_http::ProxyConfig {
    load_object(root, PROXY_KEY).map(|v| desigma_http::ProxyConfig::from_json(&v)).unwrap_or_default()
}

pub fn save_proxy(root: &Path, config: &desigma_http::ProxyConfig) -> Result<(), String> {
    save_object(root, PROXY_KEY, config.to_json())
}

/// Saves the author name (an empty name resets to the OS user name).
pub fn save_author(root: &Path, name: &str) -> Result<(), String> {
    let mut map = read(root);
    let name = name.trim();
    if name.is_empty() {
        map.remove("author");
    } else {
        map.insert("author".into(), name.into());
    }
    write(root, map)
}

fn write(root: &Path, map: serde_json::Map<String, serde_json::Value>) -> Result<(), String> {
    let text = serde_json::to_string_pretty(&serde_json::Value::Object(map)).map_err(|e| e.to_string())?;
    desigma_library::write_atomic(&root.join(FILE), text.as_bytes()).map_err(|e| format!("Cannot save settings: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("desigma_settings_{}_{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn author_roundtrip_keeps_other_keys() {
        let d = tmp("a");
        assert_eq!(load_author(&d), None);
        std::fs::write(d.join(FILE), r#"{"theme":"dark","author":"old"}"#).unwrap();
        assert_eq!(load_author(&d).as_deref(), Some("old"));
        save_author(&d, "  Kir ").unwrap();
        assert_eq!(load_author(&d).as_deref(), Some("Kir"));
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(d.join(FILE)).unwrap()).unwrap();
        assert_eq!(v["theme"], "dark");
        save_author(&d, "").unwrap();
        assert_eq!(load_author(&d), None);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn string_roundtrip_keeps_other_keys() {
        let d = tmp("s");
        assert_eq!(load_string(&d, "skip_update"), None);
        save_flag(&d, "use_gpu", false).unwrap();
        save_string(&d, "skip_update", "0.1.0-alpha.8").unwrap();
        assert_eq!(load_string(&d, "skip_update").as_deref(), Some("0.1.0-alpha.8"));
        assert_eq!(load_flag(&d, "use_gpu"), Some(false));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn broken_settings_file_is_ignored() {
        let d = tmp("b");
        std::fs::write(d.join(FILE), "{ nope").unwrap();
        assert_eq!(load_author(&d), None);
        save_author(&d, "Ann").unwrap();
        assert_eq!(load_author(&d).as_deref(), Some("Ann"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn os_user_is_never_empty() {
        assert!(!os_user_name().is_empty());
    }

    #[test]
    fn proxy_roundtrip_keeps_other_keys() {
        let d = tmp("p");
        assert_eq!(load_proxy(&d), desigma_http::ProxyConfig::default());
        save_author(&d, "Ann").unwrap();
        let c = desigma_http::ProxyConfig {
            mode: desigma_http::ProxyMode::Custom,
            url: "http://127.0.0.1:3128".into(),
            no_proxy: "localhost".into(),
        };
        save_proxy(&d, &c).unwrap();
        assert_eq!(load_proxy(&d), c);
        assert_eq!(load_author(&d).as_deref(), Some("Ann"), "other keys survive");
        let _ = std::fs::remove_dir_all(&d);
    }
}
