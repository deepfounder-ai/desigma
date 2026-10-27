//! Proxy configuration for every HTTP request desigma makes.
//!
//! The proxy is chosen by the user in **Preferences → Network proxy**, not by
//! whatever happens to be in the environment:
//!
//! * [`ProxyMode::Custom`] uses the URL and bypass list saved in `settings.json`.
//! * [`ProxyMode::Off`] never uses a proxy.
//! * [`ProxyMode::System`] (the default) asks the system. A launch that came
//!   from a shell uses exactly that shell's `HTTP(S)_PROXY` / `ALL_PROXY` /
//!   `NO_PROXY` (a shell without them means no proxy, even when the desktop has
//!   one); a launch from the app menu, Finder or Explorer has no shell
//!   environment and uses the desktop-wide proxy instead.
//!
//! `ureq` reads the environment itself but ignores `NO_PROXY`, and it has no
//! idea about desktop-wide settings, so the decision is made here and the agent
//! is built with an explicit [`ureq::Proxy`] or none at all.

use std::io::IsTerminal;

/// Where the proxy comes from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ProxyMode {
    /// Read it from the launching shell, else from the desktop settings.
    #[default]
    System,
    /// Use the configured URL.
    Custom,
    /// Never use a proxy.
    Off,
}

impl ProxyMode {
    pub const ALL: [ProxyMode; 3] = [ProxyMode::System, ProxyMode::Custom, ProxyMode::Off];

    pub fn label(self) -> &'static str {
        match self {
            ProxyMode::System => "System",
            ProxyMode::Custom => "Custom",
            ProxyMode::Off => "Off",
        }
    }

    /// The value stored in `settings.json`.
    pub fn key(self) -> &'static str {
        match self {
            ProxyMode::System => "system",
            ProxyMode::Custom => "custom",
            ProxyMode::Off => "off",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.key() == s.trim().to_ascii_lowercase())
    }
}

/// The proxy settings as saved by the app.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProxyConfig {
    pub mode: ProxyMode,
    /// `<scheme>://[user:password@]host[:port]` for [`ProxyMode::Custom`].
    pub url: String,
    /// Hosts that skip the proxy, one per line or comma separated.
    pub no_proxy: String,
}

impl ProxyConfig {
    /// The proxy to use, or `None` to connect directly.
    pub fn resolve(&self) -> Option<Proxy> {
        match self.mode {
            ProxyMode::Off => None,
            ProxyMode::Custom => {
                let url = self.url.trim();
                if url.is_empty() {
                    None
                } else {
                    Some(Proxy { url: url.to_string(), no_proxy: parse_no_proxy(&self.no_proxy) })
                }
            }
            ProxyMode::System => system_proxy(),
        }
    }

    /// Reads the `proxy` object of `settings.json` (`{"mode": ..., "url": ..., "no_proxy": ...}`).
    pub fn from_json(v: &serde_json::Value) -> Self {
        let mode = v.get("mode").and_then(|m| m.as_str()).and_then(ProxyMode::parse).unwrap_or_default();
        Self {
            mode,
            url: v.get("url").and_then(|u| u.as_str()).unwrap_or_default().to_string(),
            no_proxy: v.get("no_proxy").and_then(|u| u.as_str()).unwrap_or_default().to_string(),
        }
    }

    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({"mode": self.mode.key(), "url": self.url, "no_proxy": self.no_proxy})
    }

    /// Whether `url` should be fetched through the proxy.
    pub fn uses_proxy_for(&self, url: &str) -> bool {
        match self.resolve() {
            Some(p) => !p.bypasses(url),
            None => false,
        }
    }
}

/// A resolved proxy and its bypass list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Proxy {
    pub url: String,
    pub no_proxy: Vec<String>,
}

impl Proxy {
    /// Whether `url`'s host is in the bypass list.
    pub fn bypasses(&self, url: &str) -> bool {
        let Some(host) = host_of(url) else { return false };
        self.no_proxy.iter().any(|e| matches_entry(e, &host))
    }
}

/// Checks a proxy URL written by the user: a scheme desigma supports, a host,
/// and a valid port.
///
/// `ureq::Proxy::new` alone is too lenient — it accepts `not a url`, since
/// anything without `://` becomes a host with the default port — so a typo would
/// be saved and every request would then fail with a confusing network error.
pub fn validate_proxy_url(url: &str) -> Result<(), String> {
    let url = url.trim();
    if url.is_empty() {
        return Err("Enter the proxy address".into());
    }
    if url.chars().any(char::is_whitespace) {
        return Err("The proxy address cannot contain spaces".into());
    }
    let (scheme, rest) = url.split_once("://").ok_or("The proxy address needs a scheme, like http://host:port")?;
    // ureq speaks plain HTTP to a proxy (CONNECT for https targets) and SOCKS;
    // it has no TLS-to-proxy transport, so `https://` is not a valid scheme here.
    if !matches!(scheme.to_ascii_lowercase().as_str(), "http" | "socks4" | "socks4a" | "socks5" | "socks") {
        return Err(format!("'{scheme}' is not a proxy scheme desigma supports (http, socks4, socks5)"));
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host_port = authority.rsplit_once('@').map(|(_, h)| h).unwrap_or(authority);
    let (host, port) = if let Some(inner) = host_port.strip_prefix('[') {
        let (h, tail) = inner.split_once(']').ok_or("The proxy's IPv6 address is missing its closing bracket")?;
        (h, tail.strip_prefix(':'))
    } else {
        match host_port.rsplit_once(':') {
            Some((h, p)) => (h, Some(p)),
            None => (host_port, None),
        }
    };
    if host.is_empty() {
        return Err("The proxy address needs a host".into());
    }
    if let Some(p) = port {
        if p.is_empty() || p.parse::<u16>().is_err() {
            return Err(format!("'{p}' is not a valid port"));
        }
    }
    ureq::Proxy::new(url).map(|_| ()).map_err(|e| format!("Cannot use that proxy address: {e}"))
}

/// Splits a `NO_PROXY` value on commas and whitespace.
pub fn parse_no_proxy(s: &str) -> Vec<String> {
    s.split([',', ' ', '\t', '\n', '\r', ';']).map(str::trim).filter(|e| !e.is_empty()).map(|e| e.to_ascii_lowercase()).collect()
}

/// The host of an `http(s)://` URL, lowercased, without credentials or port.
pub fn host_of(url: &str) -> Option<String> {
    let rest = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    let authority = rest.split(['/', '?', '#']).next()?;
    let host_port = authority.rsplit_once('@').map(|(_, h)| h).unwrap_or(authority);
    let host = if let Some(inner) = host_port.strip_prefix('[') {
        // IPv6 literal: [::1]:8080
        inner.split_once(']').map(|(h, _)| h)?
    } else {
        host_port.split(':').next()?
    };
    Some(host.trim().to_ascii_lowercase()).filter(|h| !h.is_empty())
}

fn matches_entry(entry: &str, host: &str) -> bool {
    if entry == "*" {
        return true;
    }
    // `host:port` entries only match that port; we do not know the port here, so
    // compare the host part alone (the conservative reading: bypass the host).
    let entry = entry.rsplit_once(':').filter(|(_, p)| p.parse::<u16>().is_ok()).map(|(h, _)| h).unwrap_or(entry);
    let entry = entry.trim_start_matches('.');
    if entry.is_empty() {
        return false;
    }
    host == entry || host.ends_with(&format!(".{entry}"))
}

// ------------------------------------------------------------------ system proxy

/// The proxy the system should use for this launch.
///
/// A process started from a shell (stdin or stderr is a terminal) is governed by
/// that shell: its proxy variables, or no proxy at all. Anything else (app menu,
/// Finder, Explorer, a desktop autostart) reads the desktop-wide settings.
pub fn system_proxy() -> Option<Proxy> {
    if from_shell() {
        return env_proxy();
    }
    desktop_proxy().or_else(env_proxy)
}

/// Whether this process was started from a shell.
///
/// Two signals, because neither is enough alone:
///
/// * the parent process is a known shell (`bash`, `zsh`, `fish`, …). This is the
///   reliable one: `desigma > log 2>&1 &` keeps its shell parent.
/// * stdin or stderr is a terminal, which catches a shell wrapper that is not in
///   the list below and a launch from a terminal emulator's own command line.
///
/// A desktop launch (app menu, Finder, Explorer, autostart) has a session
/// manager as its parent and no terminal on its streams, so both are false.
pub fn from_shell() -> bool {
    parent_is_shell() || std::io::stdin().is_terminal() || std::io::stderr().is_terminal()
}

/// Shells whose child is a shell launch.
const SHELLS: [&str; 18] = [
    "sh", "bash", "zsh", "fish", "dash", "ksh", "mksh", "ash", "tcsh", "csh", "nu", "elvish", "xonsh", "pwsh", "powershell", "cmd", "yash", "oil",
];

/// Processes that mean "this launch came from the desktop", not from a shell.
const DESKTOP_PARENTS: [&str; 22] = [
    "systemd", "init", "gnome-shell", "gnome-session-b", "gnome-session", "kdeinit5", "kdeinit6", "ksmserver", "plasmashell", "xfce4-session",
    "mate-session", "lxsession", "cinnamon", "sway", "hyprland", "i3", "openbox", "launchd", "explorer", "explorer.exe", "wininit", "services",
];

/// Whether the launch came from a shell.
///
/// The parent chain is walked (a shell script's own `comm` is the script's name,
/// so the shell that ran it is one level up), stopping at the first shell or at
/// a desktop/session process. A terminal on stdin or stderr is the fallback for
/// a wrapper not covered here.
fn parent_is_shell() -> bool {
    let mut pid = std::process::id();
    for _ in 0..4 {
        let Some((ppid, name)) = process_parent(pid) else { return false };
        let name = bare_name(&name);
        if SHELLS.iter().any(|s| s.eq_ignore_ascii_case(&name)) {
            return true;
        }
        if DESKTOP_PARENTS.iter().any(|s| s.eq_ignore_ascii_case(&name)) || ppid <= 1 {
            return false;
        }
        pid = ppid;
    }
    false
}

/// `-bash`, `/usr/bin/zsh`, `bash.exe` -> `bash`.
fn bare_name(name: &str) -> String {
    let name = name.trim().trim_start_matches('-');
    let name = name.rsplit(['/', '\\']).next().unwrap_or(name);
    name.strip_suffix(".exe").unwrap_or(name).to_string()
}

#[cfg(target_os = "linux")]
fn process_parent(pid: u32) -> Option<(u32, String)> {
    // /proc/<pid>/stat: `pid (comm) state ppid ...`; comm may contain spaces and
    // parentheses, so split at the last ')'.
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let ppid = stat.rsplit_once(')')?.1.split_whitespace().nth(1)?.parse().ok()?;
    let comm = std::fs::read_to_string(format!("/proc/{ppid}/comm")).ok()?;
    Some((ppid, comm.trim().to_string()))
}

#[cfg(target_os = "macos")]
fn process_parent(pid: u32) -> Option<(u32, String)> {
    let out = std::process::Command::new("ps").args(["-o", "ppid=,comm=", "-p", &pid.to_string()]).output().ok()?;
    let line = String::from_utf8(out.stdout).ok()?;
    let line = line.trim();
    let (ppid, comm) = line.split_once(char::is_whitespace)?;
    Some((ppid.trim().parse().ok()?, comm.trim().to_string()))
}

#[cfg(target_os = "windows")]
fn process_parent(_pid: u32) -> Option<(u32, String)> {
    // No cheap parent lookup without a Win32 call; the terminal check decides.
    None
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn process_parent(_pid: u32) -> Option<(u32, String)> {
    None
}

/// The proxy named by `ALL_PROXY` / `HTTPS_PROXY` / `HTTP_PROXY` (either case).
pub fn env_proxy() -> Option<Proxy> {
    let get = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
    let url = ["ALL_PROXY", "all_proxy", "HTTPS_PROXY", "https_proxy", "HTTP_PROXY", "http_proxy"].iter().find_map(|k| get(k))?;
    let no_proxy = ["NO_PROXY", "no_proxy"].iter().find_map(|k| get(k)).map(|s| parse_no_proxy(&s)).unwrap_or_default();
    Some(Proxy { url: url.trim().to_string(), no_proxy })
}

/// The desktop-wide proxy: GNOME/KDE settings on Linux, `scutil` on macOS, the
/// Internet Settings on Windows. `None` when there is no proxy or it could not
/// be read.
pub fn desktop_proxy() -> Option<Proxy> {
    #[cfg(target_os = "linux")]
    {
        linux_desktop_proxy()
    }
    #[cfg(target_os = "macos")]
    {
        macos_desktop_proxy()
    }
    #[cfg(target_os = "windows")]
    {
        windows_desktop_proxy()
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        None
    }
}

#[cfg(target_os = "linux")]
fn linux_desktop_proxy() -> Option<Proxy> {
    // GNOME (and anything that reads its settings, including most desktops)
    let mode = run("gsettings", &["get", "org.gnome.system.proxy", "mode"])?;
    let mode = mode.trim().trim_matches('\'').to_ascii_lowercase();
    if mode == "none" {
        return None;
    }
    if mode == "manual" {
        let no_proxy = run("gsettings", &["get", "org.gnome.system.proxy", "ignore-hosts"]).map(|s| parse_gsettings_list(&s)).unwrap_or_default();
        for scheme in ["https", "http"] {
            let host = run("gsettings", &["get", &format!("org.gnome.system.proxy.{scheme}"), "host"]).map(|s| s.trim().trim_matches('\'').to_string()).unwrap_or_default();
            let port = run("gsettings", &["get", &format!("org.gnome.system.proxy.{scheme}"), "port"]).map(|s| s.trim().to_string()).unwrap_or_default();
            if !host.is_empty() {
                let port = if port.is_empty() || port == "0" { "8080".to_string() } else { port };
                return Some(Proxy { url: format!("http://{host}:{port}"), no_proxy });
            }
        }
    }
    // KDE keeps its proxy in kioslaverc; `kreadconfig` prints the same keys.
    let http = run("kreadconfig6", &["--file", "kioslaverc", "--group", "Proxy Settings", "--key", "httpProxy"]).or_else(|| run("kreadconfig5", &["--file", "kioslaverc", "--group", "Proxy Settings", "--key", "httpProxy"]))?;
    let http = http.trim();
    if http.is_empty() {
        return None;
    }
    let no_proxy = run("kreadconfig6", &["--file", "kioslaverc", "--group", "Proxy Settings", "--key", "NoProxyFor"])
        .or_else(|| run("kreadconfig5", &["--file", "kioslaverc", "--group", "Proxy Settings", "--key", "NoProxyFor"]))
        .map(|s| parse_no_proxy(&s))
        .unwrap_or_default();
    Some(Proxy { url: http.to_string(), no_proxy })
}

#[cfg(target_os = "linux")]
fn parse_gsettings_list(s: &str) -> Vec<String> {
    let t = s.trim();
    let inner = t.strip_prefix('[').and_then(|t| t.strip_suffix(']')).unwrap_or(t);
    parse_no_proxy(&inner.replace('\'', ""))
}

#[cfg(target_os = "macos")]
fn macos_desktop_proxy() -> Option<Proxy> {
    let out = run("scutil", &["--proxy"])?;
    //  <dictionary> {
    //    HTTPSEnable : 1
    //    HTTPSProxy : proxy.example.com
    //    HTTPSPort : 8080
    //    ExceptionsList : <array> { 0 : *.local 1 : 169.254/16 }
    //  }
    let value = |key: &str| {
        out.lines().find_map(|l| {
            let (k, v) = l.split_once(':')?;
            (k.trim() == key).then(|| v.trim().to_string())
        })
    };
    let enabled = |key: &str| value(key).is_some_and(|v| v == "1");
    let mut no_proxy = Vec::new();
    let mut in_list = false;
    for line in out.lines() {
        let t = line.trim();
        if t.starts_with("ExceptionsList") {
            in_list = true;
            continue;
        }
        if in_list {
            if t.starts_with('}') {
                break;
            }
            if let Some((_, v)) = t.split_once(':') {
                no_proxy.extend(parse_no_proxy(v));
            }
        }
    }
    for (flag, host_key, port_key) in [("HTTPSEnable", "HTTPSProxy", "HTTPSPort"), ("HTTPEnable", "HTTPProxy", "HTTPPort")] {
        if !enabled(flag) {
            continue;
        }
        let Some(host) = value(host_key).filter(|h| !h.is_empty()) else { continue };
        let port = value(port_key).filter(|p| p.parse::<u16>().is_ok()).unwrap_or_else(|| "8080".into());
        return Some(Proxy { url: format!("http://{host}:{port}"), no_proxy });
    }
    None
}

#[cfg(target_os = "windows")]
fn windows_desktop_proxy() -> Option<Proxy> {
    let out = run("reg", &["query", r"HKCU\Software\Microsoft\Windows\CurrentVersion\Internet Settings"])?;
    let value = |key: &str| {
        out.lines().find_map(|l| {
            let mut parts = l.split_whitespace();
            let k = parts.next()?;
            (k.eq_ignore_ascii_case(key)).then(|| parts.collect::<Vec<_>>().join(" "))
        })
    };
    if value("ProxyEnable").as_deref() != Some("0x1") {
        return None;
    }
    let server = value("ProxyServer")?;
    // "host:port" or "http=host:port;https=host:port"
    let url = if let Some((_, v)) = server.split(';').find_map(|e| e.split_once('=').filter(|(s, _)| s.eq_ignore_ascii_case("https"))) {
        format!("http://{v}")
    } else if let Some((_, v)) = server.split(';').find_map(|e| e.split_once('=').filter(|(s, _)| s.eq_ignore_ascii_case("http"))) {
        format!("http://{v}")
    } else {
        format!("http://{}", server.split(';').next().unwrap_or(&server))
    };
    let no_proxy = value("ProxyOverride").map(|s| parse_no_proxy(&s.replace(';', ","))).unwrap_or_default();
    Some(Proxy { url, no_proxy })
}

/// Runs a command and returns its stdout, or `None` when it is missing or fails.
fn run(cmd: &str, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new(cmd).args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8(out.stdout).ok()
}

// ------------------------------------------------------------------ agents

/// Options shared by every request desigma makes.
#[derive(Clone, Debug)]
pub struct AgentOptions {
    pub user_agent: String,
    pub timeout_connect: std::time::Duration,
    pub timeout_read: Option<std::time::Duration>,
    pub timeout_write: Option<std::time::Duration>,
    pub timeout: Option<std::time::Duration>,
    pub redirects: u32,
    /// Refuse plain HTTP (and HTTPS→HTTP redirects).
    pub https_only: bool,
}

impl Default for AgentOptions {
    fn default() -> Self {
        Self {
            user_agent: format!("desigma/{}", env!("CARGO_PKG_VERSION")),
            timeout_connect: std::time::Duration::from_secs(10),
            timeout_read: Some(std::time::Duration::from_secs(30)),
            timeout_write: Some(std::time::Duration::from_secs(30)),
            timeout: None,
            redirects: 5,
            https_only: false,
        }
    }
}

impl AgentOptions {
    pub fn user_agent(mut self, ua: impl Into<String>) -> Self {
        self.user_agent = ua.into();
        self
    }

    /// Builds an agent for `url`, going through `proxy` unless the URL bypasses it.
    ///
    /// The environment is never consulted: the proxy decision was already made by
    /// [`ProxyConfig::resolve`], so a stray `HTTP_PROXY` cannot leak in.
    pub fn agent_for(&self, proxy: Option<&Proxy>, url: &str) -> ureq::Agent {
        let mut b = ureq::AgentBuilder::new()
            .timeout_connect(self.timeout_connect)
            .redirects(self.redirects)
            .https_only(self.https_only)
            .try_proxy_from_env(false)
            .user_agent(&self.user_agent);
        if let Some(t) = self.timeout_read {
            b = b.timeout_read(t);
        }
        if let Some(t) = self.timeout_write {
            b = b.timeout_write(t);
        }
        if let Some(t) = self.timeout {
            b = b.timeout(t);
        }
        if let Some(p) = proxy.filter(|p| !p.bypasses(url)) {
            if let Ok(up) = ureq::Proxy::new(&p.url) {
                b = b.proxy(up);
            }
        }
        b.build()
    }

    /// Same, resolving the proxy from `config`.
    pub fn agent(&self, config: &ProxyConfig, url: &str) -> ureq::Agent {
        self.agent_for(config.resolve().as_ref(), url)
    }
}

// ------------------------------------------------------------------ global config

static CONFIG: std::sync::RwLock<Option<ProxyConfig>> = std::sync::RwLock::new(None);

/// Installs the settings the whole process uses. Called once at startup (and
/// again when the user saves Preferences → Network proxy).
pub fn set_config(config: ProxyConfig) {
    if let Ok(mut c) = CONFIG.write() {
        *c = Some(config);
    }
}

/// The installed settings, or the defaults (system proxy) when none were set.
pub fn config() -> ProxyConfig {
    CONFIG.read().ok().and_then(|c| c.clone()).unwrap_or_default()
}

/// An agent for `url` using the installed settings and the default options.
pub fn agent_for(url: &str) -> ureq::Agent {
    AgentOptions::default().agent(&config(), url)
}

/// An agent for `url` with custom options, using the installed settings.
pub fn agent_with(opts: &AgentOptions, url: &str) -> ureq::Agent {
    opts.agent(&config(), url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_roundtrips() {
        for m in ProxyMode::ALL {
            assert_eq!(ProxyMode::parse(m.key()), Some(m));
        }
        assert_eq!(ProxyMode::parse("nonsense"), None);
    }

    #[test]
    fn custom_url_is_used_and_off_is_not() {
        let c = ProxyConfig { mode: ProxyMode::Custom, url: " http://127.0.0.1:3128 ".into(), ..Default::default() };
        assert_eq!(c.resolve().unwrap().url, "http://127.0.0.1:3128");
        let c = ProxyConfig { mode: ProxyMode::Off, url: "http://127.0.0.1:3128".into(), ..Default::default() };
        assert!(c.resolve().is_none());
        let c = ProxyConfig { mode: ProxyMode::Custom, url: "  ".into(), ..Default::default() };
        assert!(c.resolve().is_none());
    }

    #[test]
    fn hosts_are_extracted() {
        assert_eq!(host_of("https://api.figma.com/v1/files/x").as_deref(), Some("api.figma.com"));
        assert_eq!(host_of("http://user:pw@Example.COM:8080/p").as_deref(), Some("example.com"));
        assert_eq!(host_of("http://[::1]:8080/").as_deref(), Some("::1"));
        assert_eq!(host_of("example.com/path").as_deref(), Some("example.com"));
    }

    #[test]
    fn bypass_list_matches_hosts_and_suffixes() {
        let p = Proxy { url: "http://p:1".into(), no_proxy: parse_no_proxy("localhost, .example.com, 10.0.0.5") };
        assert!(p.bypasses("http://localhost:3000/x"));
        assert!(p.bypasses("https://api.example.com/x"));
        assert!(p.bypasses("https://example.com/x"));
        assert!(!p.bypasses("https://notexample.com/x"));
        assert!(p.bypasses("http://10.0.0.5/x"));
        assert!(!p.bypasses("http://10.0.0.6/x"));
    }

    #[test]
    fn wildcard_bypasses_everything() {
        let p = Proxy { url: "http://p:1".into(), no_proxy: vec!["*".into()] };
        assert!(p.bypasses("https://anything.example/x"));
    }

    #[test]
    fn host_port_entries_match_the_host() {
        let p = Proxy { url: "http://p:1".into(), no_proxy: vec!["example.com:8080".into()] };
        assert!(p.bypasses("https://example.com/x"));
        assert!(!p.bypasses("https://other.com/x"));
    }

    #[test]
    fn process_names_reduce_to_the_bare_name() {
        for (raw, want) in [("-bash", "bash"), ("/usr/bin/zsh", "zsh"), ("/bin/sh", "sh"), ("bash.exe", "bash"), ("  fish  ", "fish")] {
            assert_eq!(bare_name(raw), want, "{raw}");
        }
    }

    #[test]
    fn shells_and_desktop_processes_are_disjoint() {
        for shell in SHELLS {
            assert!(!DESKTOP_PARENTS.iter().any(|d| d.eq_ignore_ascii_case(shell)), "{shell} is in both lists");
        }
    }

    #[test]
    fn proxy_urls_are_validated() {
        for ok in ["http://127.0.0.1:3128", "http://proxy.example.com:8080", "socks5://u:p@host:1080", "http://[::1]:3128", "http://proxy"] {
            assert!(validate_proxy_url(ok).is_ok(), "{ok}: {:?}", validate_proxy_url(ok));
        }
        for bad in ["", "not a url", "proxy:8080", "ftp://h:21", "https://h:443", "http://h:notaport", "http://", "http://[::1", "http://h:99999"] {
            assert!(validate_proxy_url(bad).is_err(), "{bad} should be rejected");
        }
    }

    #[test]
    fn settings_json_roundtrip() {
        let c = ProxyConfig { mode: ProxyMode::Custom, url: "socks5://u:p@h:1080".into(), no_proxy: "localhost".into() };
        assert_eq!(ProxyConfig::from_json(&c.to_json()), c);
        let empty = ProxyConfig::from_json(&serde_json::json!({}));
        assert_eq!(empty.mode, ProxyMode::System);
    }

    #[test]
    fn agent_builds_with_and_without_proxy() {
        let opts = AgentOptions::default();
        let _ = opts.agent_for(None, "https://example.com");
        let p = Proxy { url: "http://127.0.0.1:3128".into(), no_proxy: vec![] };
        let _ = opts.agent_for(Some(&p), "https://example.com");
        // a bypassed host builds a direct agent
        let p = Proxy { url: "http://127.0.0.1:3128".into(), no_proxy: vec!["example.com".into()] };
        let _ = opts.agent_for(Some(&p), "https://example.com");
    }
}
