//! Connect the installed app to Claude.
//!
//! Claude Desktop and Claude Code launch a local program and talk to it over stdin. That program
//! ships next to `desigma.exe`. This screen writes Claude's config with that program and the
//! library folder, so nobody clones the repository or pastes a path.

use crate::app::App;
use crate::comments::ui::{button, button_width};
use crate::{theme, widgets};
use egui::{Align2, Color32, Context, Id, Key, Modifiers, Order, Pos2, Rect, Sense, Vec2};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

const SERVER: &str = "desigma";

/// `desigma-mcp` beside the running executable.
pub fn mcp_program() -> Option<PathBuf> {
    let dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    let name = if cfg!(windows) { "desigma-mcp.exe" } else { "desigma-mcp" };
    let path = dir.join(name);
    path.is_file().then_some(path)
}

/// Claude Desktop's config. Created on connect when Claude has not been opened yet.
pub fn claude_desktop_config() -> PathBuf {
    dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join("Claude").join("claude_desktop_config.json")
}

fn workspace_dir(app: &App) -> PathBuf {
    app.lib.as_ref().map(|l| l.root().to_path_buf()).unwrap_or_else(desigma_library::Library::default_root)
}

fn server_entry(command: &Path, workspace: &Path) -> Value {
    json!({
        "command": command.to_string_lossy(),
        "args": ["--workspace", workspace.to_string_lossy()],
    })
}

/// Inserts `desigma` into `mcpServers`, keeping every other server and every other setting.
pub fn merge_mcp_server(text: &str, entry: Value) -> Result<String, String> {
    let mut root: Value =
        if text.trim().is_empty() { json!({}) } else { serde_json::from_str(text).map_err(|e| format!("Claude's config is not valid JSON ({e})"))? };
    let obj = root.as_object_mut().ok_or("Claude's config is not a JSON object")?;
    let servers = obj.entry("mcpServers").or_insert_with(|| json!({}));
    let servers = servers.as_object_mut().ok_or("mcpServers in Claude's config is not an object")?;
    servers.insert(SERVER.to_string(), entry);
    serde_json::to_string_pretty(&root).map_err(|e| e.to_string())
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// Whether `text` already launches `command`.
pub fn config_points_at(text: &str, command: &Path) -> bool {
    let Ok(root) = serde_json::from_str::<Value>(text) else { return false };
    let Some(cmd) = root.get("mcpServers").and_then(|s| s.get(SERVER)).and_then(|d| d.get("command")).and_then(|c| c.as_str()) else {
        return false;
    };
    same_file(Path::new(cmd), command)
}

fn write_server(path: &Path, entry: &Value) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("Cannot create {}: {e}", parent.display()))?;
    }
    let existing = std::fs::read_to_string(path).unwrap_or_default();
    let text = merge_mcp_server(&existing, entry.clone())?;
    desigma_library::write_atomic(path, text.as_bytes()).map_err(|e| format!("Cannot write {}: {e}", path.display()))
}

/// Registers Desigma with Claude and turns the agent bridge on.
pub fn connect(app: &mut App, ctx: &Context) -> Result<String, String> {
    let program = mcp_program().ok_or_else(|| "The MCP program was not found next to Desigma. Reinstall Desigma.".to_string())?;
    let workspace = workspace_dir(app);
    std::fs::create_dir_all(&workspace).map_err(|e| format!("Cannot prepare the library folder: {e}"))?;
    let entry = server_entry(&program, &workspace);
    write_server(&claude_desktop_config(), &entry)?;
    let code = register_claude_code(&program, &workspace);
    if app.bridge.bridge.is_none() {
        app.start_bridge(ctx)?;
        if let Some(lib) = &app.lib {
            crate::comments::settings::save_flag(lib.root(), crate::bridge::FLAG, true)?;
        }
    }
    if let Some(path) = app.file_path.clone() {
        app.publish_shared_copy(&path);
    }
    let note = if code { " Claude Code was registered too." } else { "" };
    Ok(format!("Connected. Quit Claude completely, then open it again.{note}"))
}

/// Claude Code keeps its own live config. Ask its CLI to register the server instead of rewriting
/// that file. A missing CLI, or a server that is already registered, leaves Claude Desktop as the
/// connection that was just written.
fn register_claude_code(program: &Path, workspace: &Path) -> bool {
    let Ok(status) = std::process::Command::new("claude")
        .args(["mcp", "add", "--scope", "user", SERVER, "--"])
        .arg(program)
        .arg("--workspace")
        .arg(workspace)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
    else {
        return false;
    };
    status.success()
}

pub fn show(app: &mut App, ctx: &Context) {
    if !app.mcp_dialog {
        return;
    }
    let screen = ctx.content_rect();
    let esc = ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
    const W: f32 = 460.0;
    const PAD: f32 = 20.0;
    let linked = mcp_program().is_some_and(|p| std::fs::read_to_string(claude_desktop_config()).ok().is_some_and(|t| config_points_at(&t, &p)));
    let body = "Connects this installation to Claude. While that connection is on, a file you open from outside your library is copied into the library so Claude can read it. Nothing is copied before you connect.";
    let status = if linked { "Status: connected to Claude" } else { "Status: not connected" };
    let rect = Rect::from_center_size(screen.center() - Vec2::new(0.0, 24.0), Vec2::new(W, 280.0));
    let mut close = esc;
    let mut connect_clicked = false;
    egui::Area::new(Id::new("mcp-link")).order(Order::Foreground).fixed_pos(screen.min).interactable(true).show(ctx, |ui| {
        ui.allocate_rect(screen, Sense::click_and_drag());
        let p = ui.painter().clone();
        p.rect_filled(screen, 0.0, Color32::from_black_alpha(140));
        widgets::menu_panel(&p, rect, theme::MENU_RADIUS);
        let x0 = rect.left() + PAD;
        let text_w = W - PAD * 2.0;
        p.text(Pos2::new(x0, rect.top() + PAD + 12.0), Align2::LEFT_CENTER, "Claude", theme::font_large_strong(), theme::TEXT);
        let galley = p.layout(body.into(), theme::font_body(), theme::TEXT_SECONDARY, text_w);
        p.galley(Pos2::new(x0, rect.top() + 52.0), galley, theme::TEXT_SECONDARY);
        p.text(Pos2::new(x0, rect.top() + 168.0), Align2::LEFT_CENTER, status, theme::font_body_strong(), theme::TEXT);
        if let Some(err) = &app.mcp_error {
            let g = p.layout(err.clone(), theme::font_small(), theme::DANGER, text_w);
            p.galley(Pos2::new(x0, rect.top() + 188.0), g, theme::DANGER);
        }
        let by = rect.bottom() - PAD - 32.0;
        let connect_w = button_width(&p, "Connect to Claude") + 16.0;
        let close_w = button_width(&p, "Close") + 16.0;
        let connect_r = Rect::from_min_size(Pos2::new(rect.right() - PAD - connect_w, by), Vec2::new(connect_w, 32.0));
        let close_r = Rect::from_min_size(Pos2::new(connect_r.left() - 8.0 - close_w, by), Vec2::new(close_w, 32.0));
        if button(ui, connect_r, "mcp-connect", "Connect to Claude", true, true).clicked() {
            connect_clicked = true;
        }
        widgets::record(ui, "mcp-connect", connect_r);
        if button(ui, close_r, "mcp-close", "Close", false, true).clicked() {
            close = true;
        }
        widgets::record(ui, "mcp-close", close_r);
    });
    if connect_clicked {
        match connect(app, ctx) {
            Ok(msg) => {
                app.mcp_error = None;
                app.mcp_dialog = false;
                app.show_toast(msg, false);
            }
            Err(e) => app.mcp_error = Some(e),
        }
    } else if close {
        app.mcp_dialog = false;
        app.mcp_error = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_keeps_other_servers_and_settings() {
        let existing = r#"{"theme":"dark","mcpServers":{"other":{"command":"other"}}}"#;
        let text = merge_mcp_server(existing, json!({"command": "desigma-mcp", "args": ["--workspace", "lib"]})).unwrap();
        let v: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["theme"], "dark");
        assert_eq!(v["mcpServers"]["other"]["command"], "other");
        assert_eq!(v["mcpServers"]["desigma"]["command"], "desigma-mcp");
        assert!(config_points_at(&text, Path::new("desigma-mcp")));
    }

    #[test]
    fn merge_starts_from_an_empty_file() {
        let text = merge_mcp_server("  ", json!({"command": "d"})).unwrap();
        assert!(config_points_at(&text, Path::new("d")));
    }

    #[test]
    fn merge_refuses_a_broken_config() {
        let err = merge_mcp_server("{", json!({})).unwrap_err();
        assert!(err.contains("not valid JSON"), "{err}");
    }
}
