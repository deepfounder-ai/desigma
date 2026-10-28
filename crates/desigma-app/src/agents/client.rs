//! HTTP client of the agent server: the JSON shapes the panel reads and the requests it
//! sends, run on one background thread so the UI never waits on the network.
//!
//! The contract is small (see "Agents panel" in the README): `GET /api/state`,
//! `GET /api/chat/{id}`, `POST /api/send`, `POST /api/stop`, `POST /api/chat/new`,
//! `POST /api/comment/{thread}/send`, `POST /api/comment/{thread}/resolve` and
//! `POST /api/import`. Unknown fields are ignored and missing ones default, so a server
//! that offers less still works.

use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::mpsc::{Receiver, Sender};
use std::time::Duration;

/// `GET /api/state`: the session the server joined and everything the panel lists.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct ServerState {
    #[serde(default)]
    pub joined: bool,
    #[serde(default)]
    pub joining: bool,
    #[serde(default)]
    pub join_error: Option<String>,
    #[serde(default)]
    pub disconnected: Option<String>,
    #[serde(default)]
    pub doc_name: Option<String>,
    /// The server's own presence name.
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub people: Vec<Person>,
    /// The human's selection as the server sees it through presence.
    #[serde(default)]
    pub selection: Option<Selection>,
    #[serde(default)]
    pub comments: Vec<Thread>,
    #[serde(default)]
    pub chats: Vec<ChatSummary>,
    #[serde(default)]
    pub models: Vec<Model>,
    #[serde(default)]
    pub default_model: Option<String>,
    /// Comment threads an agent is working on right now.
    #[serde(default)]
    pub flows: Vec<Flow>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct Person {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub color: String,
    #[serde(default)]
    pub is_agent: bool,
    #[serde(default)]
    pub page_name: Option<String>,
    #[serde(default)]
    pub selection: Vec<SelNode>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct Selection {
    #[serde(default)]
    pub nodes: Vec<SelNode>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct SelNode {
    #[serde(default)]
    pub name: String,
}

/// A comment thread with its parsed task marker (`[ ]` todo, `[/]` doing, `[x]` done).
#[derive(Clone, Debug, Default, Deserialize)]
pub struct Thread {
    #[serde(default)]
    pub thread_id: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub resolved: bool,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub anchor: Anchor,
    /// The opening message and the replies (only counted here).
    #[serde(default)]
    pub messages: Vec<Value>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct Anchor {
    #[serde(default)]
    pub node_name: Option<String>,
    #[serde(default)]
    pub page_name: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct ChatSummary {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub title: String,
    /// An agent run of this chat is still going.
    #[serde(default)]
    pub active: bool,
    #[serde(default)]
    pub runs: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct Model {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub label: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct Flow {
    #[serde(default)]
    pub thread_id: String,
    #[serde(default)]
    pub agent: String,
}

/// `GET /api/chat/{id}`: one chat with its messages.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct Chat {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub active: bool,
    #[serde(default)]
    pub thread_id: Option<String>,
    #[serde(default)]
    pub messages: Vec<Message>,
}

/// One chat line: `role` user / agent / system, `kind` text / tool / error / result / exit /
/// run / import / flow_done.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct Message {
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub text: String,
    /// The agent's name (agent lines) or the human's (user lines).
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
    /// Tool lines: a one-line summary of the call.
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub tool: Option<String>,
    #[serde(default)]
    pub exit_code: Option<i64>,
    #[serde(default)]
    pub is_error: bool,
    #[serde(default)]
    pub cost_usd: Option<f64>,
    #[serde(default)]
    pub agents: Vec<RunAgent>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct RunAgent {
    #[serde(default)]
    pub name: String,
}

/// `POST /api/send` answer.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct Sent {
    #[serde(default)]
    pub chat_id: String,
}

/// What the panel asks the server for; answered by one [`Reply`] each.
#[derive(Debug)]
pub enum Request {
    State,
    Chat(String),
    /// The `/api/send` body.
    Send(Value),
    Stop(String),
    NewChat,
    /// Runs an agent on a comment thread (`{model}` body).
    RunThread {
        thread: String,
        model: String,
    },
    Resolve {
        thread: String,
        resolved: bool,
    },
    /// The `/api/import` body.
    Import(Value),
}

#[derive(Debug)]
pub enum Reply {
    State(Result<Box<ServerState>, String>),
    Chat(Result<Chat, String>),
    Sent(Result<Sent, String>),
    /// New chat created: its id.
    NewChat(Result<String, String>),
    /// Anything else: a short toast text, or the error.
    Done(Result<String, String>),
}

/// Runs `req` against the server at `base` (no trailing slash).
pub fn call(agent: &ureq::Agent, base: &str, req: Request) -> Reply {
    match req {
        Request::State => Reply::State(get(agent, base, "/api/state").and_then(parse).map(Box::new)),
        Request::Chat(id) => Reply::Chat(get(agent, base, &format!("/api/chat/{id}")).and_then(parse)),
        Request::Send(body) => Reply::Sent(post(agent, base, "/api/send", &body).and_then(parse)),
        Request::Stop(run) => Reply::Done(post(agent, base, "/api/stop", &json!({ "run_id": run })).map(|_| "Stopped".into())),
        Request::NewChat => Reply::NewChat(post(agent, base, "/api/chat/new", &json!({ "title": "New chat" })).and_then(|v| {
            v.get("id")
                .and_then(Value::as_str)
                .or_else(|| v.pointer("/chat/id").and_then(Value::as_str))
                .map(str::to_string)
                .ok_or_else(|| "no chat id in the answer".into())
        })),
        Request::RunThread { thread, model } => Reply::Done(
            post(agent, base, &format!("/api/comment/{thread}/send"), &json!({ "model": model }))
                .map(|v| format!("{} is on it", v.get("agent").and_then(Value::as_str).unwrap_or("An agent"))),
        ),
        Request::Resolve { thread, resolved } => {
            Reply::Done(post(agent, base, &format!("/api/comment/{thread}/resolve"), &json!({ "resolved": resolved })).map(|_| {
                if resolved {
                    "Resolved".into()
                } else {
                    "Reopened".into()
                }
            }))
        }
        Request::Import(body) => Reply::Done(post(agent, base, "/api/import", &body).map(|v| {
            let names: Vec<&str> =
                v.get("frames").and_then(Value::as_array).map(|a| a.iter().filter_map(|f| f.get("name").and_then(Value::as_str)).collect()).unwrap_or_default();
            format!("Imported {} frame(s): {}", names.len(), names.join(", "))
        })),
    }
}

fn parse<T: for<'de> Deserialize<'de>>(v: Value) -> Result<T, String> {
    serde_json::from_value(v).map_err(|e| format!("unexpected answer: {e}"))
}

fn get(agent: &ureq::Agent, base: &str, path: &str) -> Result<Value, String> {
    read(agent.get(&format!("{base}{path}")).call())
}

fn post(agent: &ureq::Agent, base: &str, path: &str, body: &Value) -> Result<Value, String> {
    read(agent.post(&format!("{base}{path}")).set("Content-Type", "application/json").send_string(&body.to_string()))
}

/// A JSON body; the server's `{"error": …}` and transport errors become the message.
fn read(r: Result<ureq::Response, ureq::Error>) -> Result<Value, String> {
    let json = |resp: ureq::Response| resp.into_string().ok().and_then(|t| serde_json::from_str::<Value>(&t).ok());
    let v = match r {
        Ok(resp) => json(resp).ok_or_else(|| "the answer is not JSON".to_string())?,
        Err(ureq::Error::Status(code, resp)) => {
            let error = json(resp).and_then(|v| v.get("error").and_then(Value::as_str).map(str::to_string));
            return Err(error.unwrap_or_else(|| format!("server answered {code}")));
        }
        // "http://…/api/state: Connection Failed: Connect error: Connection refused (os error 61)": keep the cause
        Err(ureq::Error::Transport(t)) => return Err(t.to_string().rsplit(": ").next().unwrap_or_default().to_string()),
    };
    match v.get("error").and_then(Value::as_str) {
        Some(e) => Err(e.to_string()),
        None => Ok(v),
    }
}

/// The agent used for every call: short timeouts, no proxy (a loopback server).
pub fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(2)).timeout(Duration::from_secs(120)).try_proxy_from_env(false).build()
}

/// The worker thread: runs requests in order, one reply per request, and wakes the UI.
pub fn worker(base: String, rx: Receiver<Request>, tx: Sender<Reply>, wake: impl Fn() + Send + 'static) {
    let agent = agent();
    for req in rx {
        if tx.send(call(&agent, &base, req)).is_err() {
            break;
        }
        wake();
    }
}

/// `#rrggbb` (or `#rgb`) from the server, else the fallback.
pub fn color(s: &str, fallback: egui::Color32) -> egui::Color32 {
    let h = s.trim().trim_start_matches('#');
    let byte = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
    match h.len() {
        6 => match (byte(0), byte(2), byte(4)) {
            (Some(r), Some(g), Some(b)) => egui::Color32::from_rgb(r, g, b),
            _ => fallback,
        },
        3 => {
            let n = |i: usize| u8::from_str_radix(&h[i..i + 1], 16).ok().map(|v| v * 17);
            match (n(0), n(1), n(2)) {
                (Some(r), Some(g), Some(b)) => egui::Color32::from_rgb(r, g, b),
                _ => fallback,
            }
        }
        _ => fallback,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_colors() {
        let f = egui::Color32::GRAY;
        assert_eq!(color("#FFA629", f), egui::Color32::from_rgb(0xFF, 0xA6, 0x29));
        assert_eq!(color("#fff", f), egui::Color32::WHITE);
        assert_eq!(color("nope", f), f);
        assert_eq!(color("", f), f);
    }

    #[test]
    fn state_tolerates_missing_and_extra_fields() {
        let s: ServerState = serde_json::from_str(
            r##"{"joined":true,"doc_name":"x","people":[{"name":"Panel","color":"#FFA629","is_agent":true,"peer":4}],"palette":["#0C8CE9"]}"##,
        )
        .unwrap();
        assert!(s.joined && s.people[0].is_agent && s.chats.is_empty());
        let empty: ServerState = serde_json::from_str("{}").unwrap();
        assert!(!empty.joined);
    }

    #[test]
    fn chat_messages_keep_tool_summaries() {
        let c: Chat = serde_json::from_str(r#"{"id":"1","messages":[{"role":"agent","kind":"tool","agent":"Phantom","summary":"desigma_list_nodes"},{"role":"user","kind":"text","text":"hi"}]}"#).unwrap();
        assert_eq!(c.messages[0].summary.as_deref(), Some("desigma_list_nodes"));
        assert_eq!(c.messages[1].text, "hi");
    }

    #[test]
    fn unreachable_server_is_an_error_not_a_panic() {
        // port 9 (discard) is closed on every CI host
        let agent = agent();
        assert!(matches!(call(&agent, "http://127.0.0.1:9", Request::State), Reply::State(Err(_))));
    }
}
