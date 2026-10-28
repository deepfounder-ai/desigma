//! Agents panel (rail "Agents"): chat with a local agent server over HTTP.
//!
//! The server is any program that joins the file's live-collaboration session and offers the
//! small JSON API in `client.rs` (the README's "Agents panel" section lists the endpoints).
//! Nothing in the app depends on a particular server: it only needs the URL, taken from
//! `DESIGMA_AGENTS_URL`, else the `agents_url` setting, else `http://127.0.0.1:47900`.
//!
//! All HTTP runs on one worker thread (`client::worker`); the UI thread sends requests
//! through a channel and reads the replies back in `poll_agents` once per frame. While the
//! panel is open the state is polled every 2 s, and the open chat every second while one of its
//! runs is active.

pub mod client;
pub mod ui;

use crate::app::App;
use client::{Chat, Reply, Request, ServerState};
use serde_json::{json, Value};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};

pub const DEFAULT_URL: &str = "http://127.0.0.1:47900";
/// Settings key of the server URL (`settings.json`, beside `author`).
pub const URL_KEY: &str = "agents_url";
const STATE_EVERY: f64 = 2.0;
const CHAT_EVERY: f64 = 1.0;

/// Whether the server answered the last state poll.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Link {
    /// Nothing asked yet (the panel was never opened).
    #[default]
    Idle,
    /// The first answer is on its way.
    Connecting,
    Up,
    Down(String),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tab {
    #[default]
    Chat,
    Notes,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Parallel {
    #[default]
    Split,
    SideBySide,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Iterate {
    #[default]
    Layout,
    Style,
}

#[derive(Default)]
pub struct AgentsState {
    pub url: String,
    pub link: Link,
    pub state: Option<ServerState>,
    /// The chat shown in the messages list.
    pub chat: Option<Chat>,
    pub chat_id: Option<String>,
    pub tab: Tab,
    // ---- composer
    pub prompt: String,
    pub model: Option<String>,
    pub parallel_count: u32,
    pub parallel: Parallel,
    pub variants: u32,
    pub iterate: Iterate,
    pub use_selection: bool,
    /// A send or run is waiting for the server's answer (the Send button is disabled).
    pub sending: bool,
    // ---- import from web
    pub import_url: String,
    pub import_widths: [bool; 4],
    pub importing: bool,
    pub show_resolved: bool,
    pub messages_scroll: f32,
    pub notes_scroll: f32,
    /// Scroll to the newest message on the next frame (after a send or a new message).
    pub scroll_to_end: bool,
    // ---- worker
    tx: Option<Sender<Request>>,
    rx: Option<Receiver<Reply>>,
    state_due: f64,
    chat_due: f64,
    state_pending: bool,
    chat_pending: bool,
    /// The URL changed: the worker is restarted on the next poll.
    restart: bool,
}

/// The four import widths offered as checkboxes.
pub const IMPORT_WIDTHS: [u32; 4] = [390, 768, 1180, 1440];

impl AgentsState {
    /// The env override, else the saved setting, else the default; cached in `url`.
    pub fn resolve_url(&mut self, root: Option<&std::path::Path>) -> String {
        if self.url.is_empty() {
            let env = std::env::var("DESIGMA_AGENTS_URL").ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
            let saved = root.and_then(|r| crate::comments::settings::load_string(r, URL_KEY));
            self.url = env.or(saved).unwrap_or_else(|| DEFAULT_URL.into());
            self.url = self.url.trim_end_matches('/').to_string();
        }
        self.url.clone()
    }

    /// The chat summary of the open chat (its `active` flag is the freshest run state).
    pub fn open_summary(&self) -> Option<&client::ChatSummary> {
        let id = self.chat_id.as_deref()?;
        self.state.as_ref()?.chats.iter().find(|c| c.id == id)
    }

    /// An agent run of the open chat is going.
    pub fn run_active(&self) -> bool {
        self.open_summary().map(|c| c.active).or_else(|| self.chat.as_ref().map(|c| c.active)).unwrap_or(false)
    }

    pub fn models(&self) -> Vec<client::Model> {
        self.state.as_ref().map(|s| s.models.clone()).unwrap_or_default()
    }

    /// The model to send: the picked one, else the server's default, else the first listed.
    pub fn model_id(&self) -> Option<String> {
        self.model.clone().or_else(|| self.state.as_ref().and_then(|s| s.default_model.clone())).or_else(|| self.models().first().map(|m| m.id.clone()))
    }

    fn send(&mut self, req: Request) {
        if let Some(tx) = &self.tx {
            // a dead worker is noticed on the next poll (the receiver reports Disconnected)
            let _ = tx.send(req);
        }
    }

    /// Picks `id` and fetches it at once.
    pub fn open_chat(&mut self, id: Option<String>) {
        self.chat_id = id.clone();
        self.chat = None;
        self.messages_scroll = 0.0;
        self.scroll_to_end = true;
        if let Some(id) = id {
            self.chat_pending = true;
            self.send(Request::Chat(id));
        }
    }

    /// Merges the server's answer in; a closed worker becomes a `Down` link.
    fn handle(&mut self, reply: Reply) -> Option<String> {
        match reply {
            Reply::State(Ok(s)) => {
                self.state_pending = false;
                self.link = Link::Up;
                if self.chat_id.is_none() {
                    self.open_chat(s.chats.first().map(|c| c.id.clone()));
                }
                self.state = Some(*s);
            }
            Reply::State(Err(e)) => {
                self.state_pending = false;
                self.link = Link::Down(e);
            }
            Reply::Chat(Ok(c)) => {
                self.chat_pending = false;
                if self.chat_id.as_deref() == Some(c.id.as_str()) {
                    let grew = self.chat.as_ref().map(|old| old.messages.len() != c.messages.len()).unwrap_or(true);
                    self.scroll_to_end |= grew;
                    self.chat = Some(c);
                }
            }
            Reply::Chat(Err(e)) => {
                self.chat_pending = false;
                return Some(e);
            }
            Reply::Sent(Ok(sent)) => {
                self.sending = false;
                self.open_chat(Some(sent.chat_id));
                self.state_due = 0.0;
            }
            Reply::Sent(Err(e)) => {
                self.sending = false;
                return Some(e);
            }
            Reply::NewChat(Ok(id)) => {
                self.open_chat(Some(id));
                self.state_due = 0.0;
            }
            Reply::NewChat(Err(e)) => return Some(e),
            Reply::Done(Ok(text)) => {
                self.importing = false;
                self.sending = false;
                self.state_due = 0.0;
                return Some(text);
            }
            Reply::Done(Err(e)) => {
                self.importing = false;
                self.sending = false;
                return Some(e);
            }
        }
        None
    }

    /// The `/api/send` body for the composer's text and options.
    pub fn send_body(&self) -> Value {
        let mode = match self.parallel {
            Parallel::Split => "split",
            Parallel::SideBySide => "side_by_side",
        };
        let mut body = json!({
            "prompt": self.prompt.trim(),
            "model": self.model_id(),
            "parallel": { "count": self.parallel_count.clamp(1, 6), "mode": mode },
            "context": { "selection": self.use_selection, "files": [] },
            "chat_id": self.chat_id,
        });
        if self.variants > 1 {
            let mode = match self.iterate {
                Iterate::Layout => "layout",
                Iterate::Style => "style",
            };
            body["iterate"] = json!({ "mode": mode, "variants": self.variants.min(6) });
        }
        body
    }

    /// The `/api/import` body; `None` without a URL.
    pub fn import_body(&self) -> Option<Value> {
        let url = self.import_url.trim();
        if url.is_empty() {
            return None;
        }
        let url = if url.starts_with("http://") || url.starts_with("https://") { url.to_string() } else { format!("https://{url}") };
        let mut widths: Vec<u32> = IMPORT_WIDTHS.iter().zip(self.import_widths).filter(|(_, on)| *on).map(|(w, _)| *w).collect();
        if widths.is_empty() {
            widths.push(1440);
        }
        Some(json!({ "url": url, "widths": widths, "mode": "layers" }))
    }
}

impl App {
    /// Opens the panel in the left column and starts polling the server.
    pub fn open_agents(&mut self, ctx: &egui::Context) {
        self.ui.rail = Some(crate::app::RailItem::Agents);
        self.ui.left_collapsed = false;
        self.start_agents_worker(ctx);
    }

    /// Opened for a headless screenshot: a single frame renders, so the first state fetch
    /// runs on this thread instead of the worker (nothing else would ever show "connected").
    pub fn open_agents_blocking(&mut self, ctx: &egui::Context) {
        self.open_agents(ctx);
        let base = self.agents.url.clone();
        let reply = client::call(&client::agent(), &base, Request::State);
        if let Some(msg) = self.agents.handle(reply) {
            self.show_toast(msg, true);
        }
        if let Some(id) = self.agents.chat_id.clone() {
            let reply = client::call(&client::agent(), &base, Request::Chat(id));
            self.agents.handle(reply);
        }
        self.agents.state_due = self.now + STATE_EVERY;
    }

    fn start_agents_worker(&mut self, ctx: &egui::Context) {
        let root = self.lib.as_ref().map(|l| l.root().to_path_buf());
        let url = self.agents.resolve_url(root.as_deref());
        if self.agents.tx.is_some() {
            return;
        }
        let (tx, worker_rx) = mpsc::channel();
        let (worker_tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || client::worker(url, worker_rx, worker_tx, move || ctx.request_repaint()));
        self.agents.tx = Some(tx);
        self.agents.rx = Some(rx);
        self.agents.link = Link::Connecting;
        self.agents.state_due = 0.0;
        if self.agents.parallel_count == 0 {
            self.agents.parallel_count = 1;
            self.agents.variants = 1;
            self.agents.use_selection = true;
            self.agents.import_widths = [false, false, false, true];
        }
    }

    /// Once per frame: reads the worker's replies and schedules the next polls.
    pub fn poll_agents(&mut self, ctx: &egui::Context) {
        if self.agents.restart {
            self.reconnect_agents(ctx);
        }
        let Some(rx) = &self.agents.rx else { return };
        let (mut replies, mut dead) = (Vec::new(), false);
        loop {
            match rx.try_recv() {
                Ok(reply) => replies.push(reply),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    dead = true;
                    break;
                }
            }
        }
        if dead {
            self.agents.rx = None;
            self.agents.tx = None;
            self.agents.link = Link::Down("the connection thread stopped".into());
        }
        let toasts: Vec<String> = replies.into_iter().filter_map(|r| self.agents.handle(r)).collect();
        for t in toasts {
            let error = t.starts_with("cannot") || t.contains("error") || t.contains("failed");
            self.show_toast(t, error);
        }
        if self.ui.rail != Some(crate::app::RailItem::Agents) || self.home.is_some() {
            return;
        }
        let now = self.now;
        let a = &mut self.agents;
        if !a.state_pending && now >= a.state_due {
            a.state_pending = true;
            a.state_due = now + STATE_EVERY;
            a.send(Request::State);
        }
        if a.run_active() && !a.chat_pending && now >= a.chat_due {
            if let Some(id) = a.chat_id.clone() {
                a.chat_pending = true;
                a.chat_due = now + CHAT_EVERY;
                a.send(Request::Chat(id));
            }
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(500));
    }

    /// Drops the worker and starts a new one on the current URL.
    fn reconnect_agents(&mut self, ctx: &egui::Context) {
        self.agents.tx = None;
        self.agents.rx = None;
        self.agents.restart = false;
        self.agents.state = None;
        self.agents.chat = None;
        self.agents.chat_id = None;
        self.agents.state_pending = false;
        self.agents.chat_pending = false;
        self.start_agents_worker(ctx);
    }

    /// Sends the composer: through the thread for a comment-linked chat, else `/api/send`.
    pub fn agents_send(&mut self) {
        if self.agents.prompt.trim().is_empty() || self.agents.sending {
            return;
        }
        self.agents.sending = true;
        let thread = self.agents.chat.as_ref().and_then(|c| c.thread_id.clone());
        let req = match thread {
            Some(thread) => Request::RunThread { thread, model: self.agents.model_id().unwrap_or_default() },
            None => Request::Send(self.agents.send_body()),
        };
        self.agents.prompt.clear();
        self.agents.send(req);
    }

    pub fn agents_stop(&mut self) {
        let runs: Vec<String> = self.agents.open_summary().map(|c| c.runs.clone()).unwrap_or_default();
        for run in runs {
            self.agents.send(Request::Stop(run));
        }
    }

    pub fn agents_new_chat(&mut self) {
        self.agents.send(Request::NewChat);
    }

    pub fn agents_run_thread(&mut self, thread: &str) {
        let model = self.agents.model_id().unwrap_or_default();
        self.agents.send(Request::RunThread { thread: thread.into(), model });
    }

    pub fn agents_resolve(&mut self, thread: &str, resolved: bool) {
        self.agents.send(Request::Resolve { thread: thread.into(), resolved });
    }

    pub fn agents_import(&mut self) {
        let Some(body) = self.agents.import_body() else {
            self.show_toast("Enter a URL to import", true);
            return;
        };
        self.agents.importing = true;
        self.show_toast("Importing from the web, this takes a while…", false);
        self.agents.send(Request::Import(body));
    }

    /// Preferences → Agent server URL…
    pub fn ask_agents_url(&mut self) {
        let root = self.lib.as_ref().map(|l| l.root().to_path_buf());
        let current = self.agents.resolve_url(root.as_deref());
        self.dialog = Some(crate::home::Dialog::prompt(crate::home::Intent::AgentsUrl, "Agent server URL", &current, "Save"));
    }

    /// Stores the URL (empty goes back to the default); an open panel reconnects on the next frame.
    pub fn set_agents_url(&mut self, url: &str) {
        let url = url.trim().trim_end_matches('/');
        if let Some(lib) = &self.lib {
            if let Err(e) = crate::comments::settings::save_string(lib.root(), URL_KEY, url) {
                self.show_toast(e, true);
            }
        }
        self.agents.url = if url.is_empty() { DEFAULT_URL.into() } else { url.into() };
        self.agents.restart = self.agents.tx.is_some();
    }
}

/// Runs the dialogs of this feature; false when `d` is not one of them.
pub fn finish_dialog(app: &mut App, d: &crate::home::Dialog) -> bool {
    if !matches!(d.intent, crate::home::Intent::AgentsUrl) {
        return false;
    }
    app.set_agents_url(&d.input.clone().unwrap_or_default());
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn send_body_follows_the_composer() {
        let mut a = AgentsState {
            prompt: " a card ".into(),
            parallel_count: 3,
            parallel: Parallel::SideBySide,
            variants: 1,
            use_selection: true,
            ..Default::default()
        };
        let b = a.send_body();
        assert_eq!(b["prompt"], "a card");
        assert_eq!(b["parallel"]["count"], 3);
        assert_eq!(b["parallel"]["mode"], "side_by_side");
        assert_eq!(b["context"]["selection"], true);
        assert!(b.get("iterate").is_none());
        a.variants = 4;
        a.iterate = Iterate::Style;
        assert_eq!(a.send_body()["iterate"], json!({ "mode": "style", "variants": 4 }));
    }

    #[test]
    fn import_body_defaults_the_scheme_and_width() {
        let mut a = AgentsState::default();
        assert!(a.import_body().is_none());
        a.import_url = "example.com".into();
        let b = a.import_body().unwrap();
        assert_eq!(b["url"], "https://example.com");
        assert_eq!(b["widths"], json!([1440]));
        a.import_widths = [true, false, true, false];
        assert_eq!(a.import_body().unwrap()["widths"], json!([390, 1180]));
    }

    #[test]
    fn state_reply_opens_the_first_chat_and_marks_the_link_up() {
        let mut a = AgentsState::default();
        let s: ServerState = serde_json::from_str(r#"{"joined":true,"chats":[{"id":"c1","title":"t","active":true,"runs":["r1"]}]}"#).unwrap();
        assert!(a.handle(Reply::State(Ok(Box::new(s)))).is_none());
        assert_eq!(a.link, Link::Up);
        assert_eq!(a.chat_id.as_deref(), Some("c1"));
        assert!(a.run_active());
        assert_eq!(a.handle(Reply::State(Err("down".into()))), None);
        assert_eq!(a.link, Link::Down("down".into()));
        assert_eq!(a.handle(Reply::Done(Err("x failed".into()))).as_deref(), Some("x failed"));
    }

    #[test]
    fn env_url_wins_over_the_default() {
        let mut a = AgentsState::default();
        let url = a.resolve_url(None);
        assert!(url.starts_with("http"), "{url}");
        assert!(!url.ends_with('/'));
    }
}
