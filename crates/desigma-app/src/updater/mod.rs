//! Updates. Release builds ask GitHub for a newer desigma a few seconds after launch and then once a day, and
//! show a banner and a dialog with the release notes. Nothing is downloaded until the user presses
//! **Update now**; then, on a background thread (`job`):
//!
//! 1. `SHA256SUMS` of the same release is fetched and must list the file ([`assets`]);
//! 2. the file for this platform and install type is downloaded (`download`: https to GitHub's hosts only,
//!    redirects checked hop by hop, 300 MB cap, timeouts) with a progress bar and Cancel;
//! 3. its SHA-256 must match, else it is deleted;
//! 4. it is unpacked next to what it replaces ([`install::prepare`]); nothing is replaced yet.
//!
//! The dialog then offers **Restart to update**. Before anything is replaced it asks what opening another file
//! would ask (`leave_ok`: "Stop sharing?" / "Leave the session?", then the unsaved-changes dialog in
//! `unsaved.rs`). After that [`install::commit`] swaps the files (Windows installer: starts the new setup
//! silently; Windows portable: renames the running exe aside; macOS: starts a helper script; Linux: renames the
//! new binary over the old), the window closes and the new version is launched. "Open release page" stays as a
//! link and as the fallback for every failure. [`install`] describes each platform in detail.
//!
//! Builds from source never check on their own and never update themselves.

mod assets;
mod download;
pub mod install;
mod job;
pub mod ui;

use crate::app::App;
use crate::comments::settings;
use semver::Version;
use std::io::Read;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;
use std::time::Duration;

/// Every desigma release so far is a pre-release, so `/releases/latest` answers 404; list them instead.
const RELEASES: &str = "https://api.github.com/repos/deepfounder-ai/desigma/releases?per_page=20";
const MAX_LISTING: u64 = 2 << 20;
/// Seconds after launch before the first automatic check, then between checks.
const FIRST_CHECK: f64 = 5.0;
const CHECK_EVERY: f64 = 24.0 * 60.0 * 60.0;

#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    pub version: Version,
    /// The GitHub release page ("Open release page" opens it).
    pub page: String,
    /// Plain-text lines of the "What's Changed" notes.
    pub notes: Vec<String>,
    /// The files attached to the release.
    pub assets: Vec<assets::Asset>,
}

/// Where "Update now" stands.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Install {
    #[default]
    Idle,
    Downloading {
        done: u64,
        total: Option<u64>,
    },
    /// The download is complete and its checksum is being compared.
    Verifying,
    Preparing,
    /// Downloaded, verified and unpacked: only the restart is left.
    Ready(install::Prepared),
    /// Why the update did not happen. The release page link is the fallback.
    Failed(String),
}

impl Install {
    /// A background step is running.
    pub fn busy(&self) -> bool {
        matches!(self, Install::Downloading { .. } | Install::Verifying | Install::Preparing)
    }
}

/// The running download-and-prepare thread.
struct Job {
    receiver: Receiver<Install>,
    cancel: Arc<AtomicBool>,
}

#[derive(Default)]
pub struct UpdaterState {
    /// The newer release the last check found.
    pub available: Option<Release>,
    pub dialog: bool,
    /// The banner was closed (Later or ×) for this session.
    pub dismissed: bool,
    /// Preferences → "Check for updates automatically" is off.
    pub no_auto_check: bool,
    /// The version "Skip this version" hid; automatic checks stay quiet until a newer one.
    pub skip: Option<String>,
    /// When the next automatic check runs (`App::now`); None when this process never checks on its own.
    next_check: Option<f64>,
    /// The running check reports every outcome (Help → Check for updates…).
    manual: bool,
    receiver: Option<Receiver<Result<Option<Release>, String>>>,
    /// "Update now" for `available`.
    pub install: Install,
    job: Option<Job>,
    /// The unsaved-changes dialog asks whether to restart (see `unsaved.rs`).
    pub pending_restart: bool,
    /// Every question is answered: restart on the next frame.
    restart: bool,
}

impl UpdaterState {
    /// Reads the saved preferences; `launch` (a real window, not a screenshot or an "Open in new window"
    /// child) of a release build also schedules the automatic checks unless `DESIGMA_NO_UPDATE_CHECK` is set.
    pub fn start(&mut self, root: Option<&Path>, launch: bool) {
        if let Some(root) = root {
            self.no_auto_check = settings::load_flag(root, "check_updates") == Some(false);
            self.skip = settings::load_string(root, "skip_update");
        }
        if launch && crate::RELEASE_BUILD && !opted_out() {
            self.next_check = Some(FIRST_CHECK);
        }
        // what the last update left behind (the renamed old exe, staged files, the download folder)
        if launch && crate::RELEASE_BUILD {
            if let Ok(exe) = std::env::current_exe() {
                install::remove_later(install::leftovers(assets::Os::current(), &exe));
            }
        }
    }
}

impl App {
    /// Starts a check in the background. A `manual` check also reports "up to date" and errors, ignores the
    /// skipped version and opens the dialog when it finds a release.
    pub fn check_for_updates(&mut self, ctx: &egui::Context, manual: bool) {
        if manual && !crate::RELEASE_BUILD {
            self.show_toast("This is a development build, so it does not check for updates", false);
            return;
        }
        self.updater.manual |= manual;
        if self.updater.receiver.is_some() {
            return;
        }
        let (sender, receiver) = mpsc::channel();
        self.updater.receiver = Some(receiver);
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = sender.send(fetch_listing().and_then(|json| newest(&json, crate::VERSION)));
            ctx.request_repaint();
        });
    }

    /// Runs the automatic checks when they are due and takes in finished ones. Call once per frame.
    pub fn poll_updater(&mut self, ctx: &egui::Context) {
        if let Some(at) = self.updater.next_check {
            if self.now >= at {
                self.updater.next_check = Some(self.now + CHECK_EVERY);
                if !self.updater.no_auto_check {
                    self.check_for_updates(ctx, false);
                }
            } else {
                ctx.request_repaint_after(Duration::from_secs_f64(at - self.now));
            }
        }
        let Some(receiver) = &self.updater.receiver else { return };
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err("the check stopped".into()),
        };
        self.updater.receiver = None;
        let manual = std::mem::take(&mut self.updater.manual);
        self.finish_check(result, manual);
    }

    /// Takes in progress from the update thread and performs a confirmed restart. Call once per frame.
    pub fn poll_install(&mut self, ctx: &egui::Context) {
        if let Some(job) = &self.updater.job {
            let (mut latest, mut gone) = (None, false);
            loop {
                match job.receiver.try_recv() {
                    Ok(state) => latest = Some(state),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        gone = true;
                        break;
                    }
                }
            }
            if let Some(state) = latest {
                self.updater.install = state;
            }
            if gone && self.updater.install.busy() {
                self.updater.install = Install::Failed("the update stopped unexpectedly".into());
            }
            if gone || !self.updater.install.busy() {
                self.updater.job = None;
            }
            if self.updater.install.busy() {
                ctx.request_repaint_after(Duration::from_millis(100));
            }
        }
        if std::mem::take(&mut self.updater.restart) {
            self.restart_now(ctx);
        }
    }

    /// "Update now": downloads, verifies and unpacks the release for this install in the background.
    pub fn start_update(&mut self, ctx: &egui::Context) {
        if self.updater.job.is_some() || matches!(self.updater.install, Install::Ready(_)) {
            return;
        }
        let Some(release) = self.updater.available.clone() else { return };
        let target = if !crate::RELEASE_BUILD {
            Err("this is a development build; it is updated by building from source".to_string())
        } else {
            newer_than(crate::VERSION, &release.version).and_then(|()| install::detect())
        };
        let target = match target {
            Ok(target) => target,
            Err(e) => {
                self.updater.install = Install::Failed(e);
                return;
            }
        };
        let (sender, receiver) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        self.updater.install = Install::Downloading { done: 0, total: None };
        self.updater.job = Some(Job { receiver, cancel: cancel.clone() });
        let ctx = ctx.clone();
        job::spawn(release, target, cancel, sender, move || ctx.request_repaint());
    }

    /// Cancel while downloading: the thread stops at its next chunk and reports `Idle`.
    pub fn cancel_update(&mut self) {
        if let Some(job) = &self.updater.job {
            job.cancel.store(true, Ordering::Relaxed);
        }
    }

    /// "Restart to update": asks what leaving the document asks (a live session, then unsaved changes) and
    /// then restarts. Cancelling any of the questions keeps everything as it is.
    pub fn request_restart(&mut self) {
        if !matches!(self.updater.install, Install::Ready(_)) {
            return;
        }
        self.updater.dialog = false;
        if self.leave_ok(|app| app.restart_unless_unsaved()) {
            self.restart_unless_unsaved();
        }
    }

    /// The session question is settled; unsaved edits get their own Save / Discard / Cancel question.
    fn restart_unless_unsaved(&mut self) {
        if self.edits_unsaved() {
            self.updater.pending_restart = true;
        } else {
            self.updater.restart = true;
        }
    }

    /// Every question is answered (also called by the unsaved-changes dialog).
    pub fn confirm_restart(&mut self) {
        self.updater.pending_restart = false;
        self.updater.restart = true;
    }

    /// Swaps the files and closes the window; the new version starts once it has closed (`install::run_stashed`).
    /// If the swap fails the app stays open with the error in the dialog.
    fn restart_now(&mut self, ctx: &egui::Context) {
        let Install::Ready(prepared) = self.updater.install.clone() else { return };
        match install::commit(&prepared) {
            Ok(launch) => {
                install::stash(launch);
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            Err(e) => {
                self.updater.install = Install::Failed(e);
                self.updater.dialog = true;
            }
        }
    }

    /// Automatic checks stay silent unless they find a release the user has not skipped; manual ones report
    /// every outcome and open the dialog.
    pub(crate) fn finish_check(&mut self, result: Result<Option<Release>, String>, manual: bool) {
        match result {
            Ok(Some(release)) if manual || !skipped(&release.version, self.updater.skip.as_deref()) => {
                if self.updater.available.as_ref().is_none_or(|a| a.version != release.version) {
                    self.updater.dismissed = false;
                    // a different release: what was downloaded for the old one no longer applies
                    self.cancel_update();
                    self.updater.job = None;
                    self.updater.install = Install::Idle;
                }
                self.updater.available = Some(release);
                self.updater.dialog |= manual;
            }
            Ok(_) if manual => self.show_toast(format!("desigma {} is up to date", crate::VERSION), false),
            Ok(_) => {}
            Err(e) if manual => self.show_toast(format!("Couldn't check for updates: {e}"), true),
            Err(e) => eprintln!("desigma: update check failed: {e}"),
        }
    }

    /// Preferences → "Check for updates automatically".
    pub fn toggle_update_check(&mut self) {
        self.updater.no_auto_check = !self.updater.no_auto_check;
        if let Some(lib) = &self.lib {
            if let Err(e) = settings::save_flag(lib.root(), "check_updates", !self.updater.no_auto_check) {
                self.show_toast(e, true);
            }
        }
    }

    /// "Skip this version": automatic checks stay quiet until a newer release.
    pub fn skip_update(&mut self) {
        let Some(release) = self.updater.available.take() else { return };
        self.updater.dialog = false;
        let version = release.version.to_string();
        if let Some(lib) = &self.lib {
            if let Err(e) = settings::save_string(lib.root(), "skip_update", &version) {
                self.show_toast(e, true);
            }
        }
        self.updater.skip = Some(version);
    }
}

/// `--panel updater` / `updater-banner` (screenshots): checks right away, blocking, and shows the dialog or
/// the banner. Point `DESIGMA_UPDATE_URL` at a file to keep it offline.
pub fn preset(app: &mut App, panel: &str) {
    match fetch_listing().and_then(|json| newest(&json, crate::VERSION)) {
        Ok(Some(release)) => {
            app.updater.available = Some(release);
            app.updater.dialog = panel == "updater";
        }
        Ok(None) => eprintln!("desigma: no release newer than {} in the update feed", crate::VERSION),
        Err(e) => eprintln!("desigma: update check failed: {e}"),
    }
}

/// GitHub's release list, or `DESIGMA_UPDATE_URL` (an http(s) URL or a file path) for tests.
fn fetch_listing() -> Result<Vec<u8>, String> {
    match std::env::var("DESIGMA_UPDATE_URL") {
        Ok(source) if !source.trim().is_empty() => fetch(source.trim()),
        _ => fetch(RELEASES),
    }
}

fn fetch(source: &str) -> Result<Vec<u8>, String> {
    if !source.starts_with("http://") && !source.starts_with("https://") {
        let path = source.strip_prefix("file://").unwrap_or(source);
        return std::fs::read(path).map_err(|e| format!("cannot read {path}: {e}"));
    }
    // a plain-http source is a local test server: no proxy
    let tls = source.starts_with("https://");
    let opts = desigma_http::AgentOptions {
        timeout_connect: Duration::from_secs(10),
        timeout_read: Some(Duration::from_secs(30)),
        timeout_write: Some(Duration::from_secs(30)),
        https_only: tls,
        ..Default::default()
    }
    .user_agent(format!("desigma/{}", crate::VERSION));
    let agent = if tls { desigma_http::agent_with(&opts, source) } else { opts.agent_for(None, source) };
    let response = agent.get(source).set("Accept", "application/vnd.github+json").set("X-GitHub-Api-Version", "2022-11-28").call().map_err(|e| match e {
        ureq::Error::Status(403 | 429, _) => "GitHub's hourly limit was reached. Try again later".to_string(),
        ureq::Error::Status(code, _) => format!("GitHub answered {code}"),
        ureq::Error::Transport(t) => {
            eprintln!("desigma: update check: {t}");
            "GitHub can't be reached. Check your connection".to_string()
        }
    })?;
    let mut body = Vec::new();
    response.into_reader().take(MAX_LISTING + 1).read_to_end(&mut body).map_err(|e| e.to_string())?;
    if body.len() as u64 > MAX_LISTING {
        return Err("the release list is too large".into());
    }
    Ok(body)
}

#[derive(serde::Deserialize)]
struct Listed {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    assets: Vec<ListedAsset>,
}

#[derive(serde::Deserialize)]
struct ListedAsset {
    name: String,
    browser_download_url: String,
    #[serde(default)]
    size: u64,
}

/// The highest release in a GitHub release list that is newer than `current`. Drafts and tags that are not
/// `v<semver>` are ignored; pre-releases count only while `current` is one.
fn newest(json: &[u8], current: &str) -> Result<Option<Release>, String> {
    let listing: Vec<Listed> = serde_json::from_slice(json).map_err(|e| format!("unexpected release list ({e})"))?;
    let current = Version::parse(current).map_err(|e| format!("this build's version {current} is not semver ({e})"))?;
    let best = listing
        .into_iter()
        .filter(|r| !r.draft)
        .filter_map(|r| Some((Version::parse(r.tag_name.strip_prefix('v')?).ok()?, r)))
        .filter(|(v, r)| !current.pre.is_empty() || (v.pre.is_empty() && !r.prerelease))
        .filter(|(v, _)| *v > current)
        .max_by(|a, b| a.0.cmp(&b.0));
    Ok(best.map(|(version, r)| Release {
        page: format!("{}/releases/tag/{}", crate::home::GITHUB_URL, r.tag_name),
        notes: whats_changed(r.body.as_deref().unwrap_or_default()),
        assets: r.assets.into_iter().map(|a| assets::Asset { name: a.name, url: a.browser_download_url, size: a.size }).collect(),
        version,
    }))
}

/// An update only ever moves forward: `release` must be newer than the running `current` version.
fn newer_than(current: &str, release: &Version) -> Result<(), String> {
    let current = Version::parse(current).map_err(|e| format!("this build's version {current} is not semver ({e})"))?;
    if *release > current {
        Ok(())
    } else {
        Err(format!("{release} is not newer than the {current} you have"))
    }
}

/// Whether "Skip this version" covers `version` (the skipped one or older).
fn skipped(version: &Version, skip: Option<&str>) -> bool {
    skip.and_then(|s| Version::parse(s).ok()).is_some_and(|s| *version <= s)
}

/// `DESIGMA_NO_UPDATE_CHECK` set to anything but empty or "0".
fn opted_out() -> bool {
    std::env::var_os("DESIGMA_NO_UPDATE_CHECK").is_some_and(|v| !v.is_empty() && v != "0")
}

/// Longest note line kept, in characters: a hand-written paragraph must not push the
/// dialog's buttons off a small window.
const NOTE_CHARS: usize = 160;

/// The "What's Changed" section of generated release notes as plain text: bullets become "•" and the
/// trailing "in https://…/pull/N" of each entry is dropped; long lines end in "…".
fn whats_changed(body: &str) -> Vec<String> {
    body.lines()
        .skip_while(|l| l.trim() != "## What's Changed")
        .skip(1)
        .take_while(|l| !l.starts_with("## ") && !l.starts_with("**Full Changelog**"))
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|l| {
            let l = l.split(" in https://").next().unwrap_or(l);
            l.strip_prefix("* ").or_else(|| l.strip_prefix("- ")).map_or_else(|| l.to_string(), |item| format!("• {item}"))
        })
        .map(|l| match l.char_indices().nth(NOTE_CHARS) {
            Some((cut, _)) => format!("{}…", l[..cut].trim_end()),
            None => l,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &[u8] = include_bytes!("fixtures/releases.json");

    fn listing(tags: &[(&str, bool, bool)]) -> Vec<u8> {
        let v: Vec<_> = tags.iter().map(|(t, draft, pre)| serde_json::json!({"tag_name": t, "draft": draft, "prerelease": pre})).collect();
        serde_json::to_vec(&v).unwrap()
    }

    fn found(json: &[u8], current: &str) -> Option<String> {
        newest(json, current).unwrap().map(|r| r.version.to_string())
    }

    #[test]
    fn the_real_listing_offers_the_newest_alpha() {
        let r = newest(FIXTURE, "0.1.0-alpha.5").unwrap().unwrap();
        assert_eq!(r.version.to_string(), "0.1.0-alpha.7");
        assert_eq!(r.page, "https://github.com/deepfounder-ai/desigma/releases/tag/v0.1.0-alpha.7");
        assert_eq!(r.notes, ["• fig: nested instance swaps and variable-bound paints in instance overrides by @MrAndrewMal"]);
        assert_eq!(found(FIXTURE, "0.1.0-alpha.7"), None, "up to date");
        assert_eq!(found(FIXTURE, "0.1.0-alpha.9"), None, "a newer local build is not downgraded");
        // a stable build never moves to a pre-release, and every release so far is one
        assert_eq!(found(FIXTURE, "0.0.9"), None);
    }

    #[test]
    fn versions_compare_as_semver() {
        let tags = listing(&[("v0.1.0-alpha.9", false, true), ("v0.1.0-alpha.10", false, true), ("v0.1.0-beta.1", true, true)]);
        assert_eq!(found(&tags, "0.1.0-alpha.8").as_deref(), Some("0.1.0-alpha.10"), "alpha.10 > alpha.9, drafts skipped");
        let tags = listing(&[("v0.1.0-rc.1", false, true), ("v0.1.0", false, false), ("v0.2.0-alpha.1", false, true)]);
        assert_eq!(found(&tags, "0.1.0-alpha.8").as_deref(), Some("0.2.0-alpha.1"), "a pre-release build sees pre-releases");
        assert_eq!(found(&tags, "0.0.5").as_deref(), Some("0.1.0"), "a stable build only sees stable releases");
        let tags = listing(&[("nightly", false, false), ("v1", false, false), ("0.3.0", false, false), ("v0.2.0", false, true)]);
        assert_eq!(found(&tags, "0.1.0"), None, "malformed tags and flagged pre-releases are ignored");
    }

    #[test]
    fn bad_input_is_an_error() {
        assert!(newest(b"{\"message\":\"API rate limit exceeded\"}", "0.1.0").is_err());
        assert!(newest(b"[]", "dev").is_err());
        assert_eq!(newest(b"[]", "0.1.0"), Ok(None));
    }

    #[test]
    fn skipping_hides_that_version_and_older() {
        let v = |s: &str| Version::parse(s).unwrap();
        assert!(skipped(&v("0.1.0-alpha.8"), Some("0.1.0-alpha.8")));
        assert!(skipped(&v("0.1.0-alpha.7"), Some("0.1.0-alpha.8")));
        assert!(!skipped(&v("0.1.0-alpha.9"), Some("0.1.0-alpha.8")));
        assert!(!skipped(&v("0.1.0-alpha.9"), None));
        assert!(!skipped(&v("0.1.0-alpha.9"), Some("garbage")));
    }

    #[test]
    fn notes_keep_only_whats_changed() {
        let body = "### Installing\n\n* not this\n\n## What's Changed\n* a by @x in https://github.com/o/r/pull/1\n- b\nplain\n\n## New Contributors\n* @x";
        assert_eq!(whats_changed(body), ["• a by @x", "• b", "plain"]);
        assert!(whats_changed("no notes").is_empty());
    }

    #[test]
    fn the_endpoint_override_reads_a_file() {
        let path = std::env::temp_dir().join(format!("desigma_updater_{}.json", std::process::id()));
        std::fs::write(&path, FIXTURE).unwrap();
        assert_eq!(fetch(path.to_str().unwrap()).unwrap(), FIXTURE);
        assert_eq!(fetch(&format!("file://{}", path.display())).unwrap(), FIXTURE);
        let _ = std::fs::remove_file(&path);
        assert!(fetch(path.to_str().unwrap()).unwrap_err().starts_with("cannot read"));
    }

    /// A one-shot HTTP server on localhost; the handle returns the request head it got, lower-cased.
    fn serve(status: &'static str, body: &'static [u8]) -> (String, std::thread::JoinHandle<String>) {
        use std::io::{BufRead, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/releases.json", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let head: Vec<String> = std::io::BufReader::new(&stream).lines().map_while(Result::ok).take_while(|l| !l.is_empty()).collect();
            write!(&stream, "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).unwrap();
            (&stream).write_all(body).unwrap();
            head.join("\n").to_lowercase()
        });
        (url, server)
    }

    #[test]
    fn the_endpoint_override_reads_a_local_server() {
        let (url, server) = serve("200 OK", FIXTURE);
        assert_eq!(fetch(&url).unwrap(), FIXTURE);
        let request = server.join().unwrap();
        assert!(request.contains(&format!("user-agent: desigma/{}", crate::VERSION).to_lowercase()), "{request}");
        assert!(request.contains("accept: application/vnd.github+json"));
        let (url, _) = serve("403 Forbidden", b"{}");
        assert!(fetch(&url).unwrap_err().contains("limit"));
        let (url, _) = serve("500 Internal Server Error", b"");
        assert_eq!(fetch(&url).unwrap_err(), "GitHub answered 500");
    }

    #[test]
    fn automatic_checks_are_quiet_and_manual_ones_report() {
        let mut app = App::new(desigma_doc::Document::new("t"));
        let found = || Ok(newest(FIXTURE, "0.1.0-alpha.6").unwrap());
        app.updater.skip = Some("0.1.0-alpha.7".into());
        app.finish_check(found(), false);
        assert!(app.updater.available.is_none(), "the skipped version stays hidden");
        app.finish_check(Err("offline".into()), false);
        app.finish_check(Ok(None), false);
        assert!(app.toast.is_none() && !app.updater.dialog);
        app.finish_check(found(), true);
        assert!(app.updater.available.is_some() && app.updater.dialog, "a manual check ignores the skip and opens the dialog");
        app.updater.dialog = false;
        app.finish_check(Ok(None), true);
        assert!(app.toast.as_ref().is_some_and(|t| !t.error && t.text.ends_with("is up to date")));
        app.finish_check(Err("GitHub answered 500".into()), true);
        assert!(app.toast.as_ref().is_some_and(|t| t.error && t.text == "Couldn't check for updates: GitHub answered 500"));
        // × hides the banner until a different version turns up
        app.updater.skip = None;
        app.updater.dismissed = true;
        app.finish_check(found(), false);
        assert!(app.updater.dismissed);
        app.finish_check(Ok(newest(FIXTURE, "0.1.0-alpha.6").unwrap().map(|r| Release { version: Version::new(9, 0, 0), ..r })), false);
        assert!(!app.updater.dismissed);
    }

    #[test]
    fn only_release_builds_check_on_their_own() {
        let mut s = UpdaterState::default();
        s.start(None, false);
        assert_eq!(s.next_check, None, "screenshots and child windows never check");
        s.start(None, true);
        assert_eq!(s.next_check.is_some(), crate::RELEASE_BUILD && !opted_out());
    }

    #[test]
    fn the_real_releases_carry_assets_that_pass_the_download_checks() {
        use assets::{asset_name, check_asset_url, pick, sums, Kind};
        let r = newest(FIXTURE, "0.1.0-alpha.5").unwrap().unwrap();
        assert_eq!(r.assets.len(), 5);
        for a in &r.assets {
            check_asset_url(&a.url).unwrap_or_else(|e| panic!("{}: {e}", a.name));
            assert!(a.size > 0);
        }
        // this release predates the installer; the other three install types find their file
        for kind in [Kind::MacApp, Kind::WindowsPortable, Kind::LinuxTarball] {
            assert_eq!(pick(&r.assets, kind, &r.version).unwrap().name, asset_name(kind, &r.version));
        }
        assert!(pick(&r.assets, Kind::WindowsInstaller, &r.version).is_err());
        assert_eq!(sums(&r.assets).unwrap().size, 444);
    }

    #[test]
    fn updates_only_move_forward() {
        let v = |s: &str| Version::parse(s).unwrap();
        assert!(newer_than("0.1.0-alpha.7", &v("0.1.0-alpha.8")).is_ok());
        assert!(newer_than("0.1.0-alpha.9", &v("0.1.0-alpha.10")).is_ok(), "numeric, not text, order");
        assert!(newer_than("0.1.0-alpha.7", &v("0.1.0")).is_ok());
        assert!(newer_than("0.1.0-alpha.7", &v("0.1.0-alpha.7")).is_err(), "the same version is not an update");
        assert!(newer_than("0.1.0-alpha.7", &v("0.1.0-alpha.6")).unwrap_err().contains("not newer"));
        assert!(newer_than("0.1.0", &v("0.1.0-rc.1")).is_err());
        assert!(newer_than("dev", &v("1.0.0")).is_err());
    }

    #[test]
    fn long_notes_are_cut() {
        let body = format!("## What's Changed\n* {}\n* short", "ж".repeat(500));
        let notes = whats_changed(&body);
        assert_eq!(notes[0].chars().count(), NOTE_CHARS + 1, "{}", notes[0]);
        assert!(notes[0].ends_with('…'));
        assert_eq!(notes[1], "• short");
    }
}
