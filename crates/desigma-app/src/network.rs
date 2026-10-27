//! Preferences → Network proxy.
//!
//! The proxy is set here rather than read from the environment, so what the app
//! uses is what the user chose. The three modes are:
//!
//! * **System** (default) — a launch from a shell obeys that shell's
//!   `HTTP(S)_PROXY` / `ALL_PROXY` / `NO_PROXY`; a launch from the app menu,
//!   Finder or Explorer has no shell environment and uses the desktop-wide
//!   proxy instead.
//! * **Custom** — the URL and bypass list typed here.
//! * **Off** — never proxy anything.
//!
//! Saving installs the settings process-wide (`desigma_http::set_config`), so
//! fonts, Figma imports, plugins and update checks all pick it up at once.

use crate::app::App;
use crate::theme;
use crate::widgets;
use egui::{Align2, Color32, Context, Id, Key, Modifiers, Order, Pos2, Rect, Sense, Ui, Vec2};

/// The open dialog's editable state.
#[derive(Clone, Debug, Default)]
pub struct ProxyDialog {
    pub config: desigma_http::ProxyConfig,
    /// The proxy the system resolves to right now, for the System mode's summary.
    pub detected: Option<String>,
    /// Why the last save failed, shown inside the dialog.
    pub error: Option<String>,
    fresh: bool,
}

impl ProxyDialog {
    pub fn open(config: desigma_http::ProxyConfig) -> Self {
        let resolved = config.resolve();
        Self { config, detected: resolved.map(|p| p.url), error: None, fresh: true }
    }
}

impl App {
    /// Preferences → Network proxy…: opens the dialog on the saved settings.
    pub fn open_proxy_dialog(&mut self) {
        self.proxy_dialog = Some(ProxyDialog::open(self.proxy.clone()));
    }

    /// Saves the dialog's settings and installs them. Returns the error to show
    /// inside the dialog, so a typo keeps it open instead of closing silently.
    pub fn save_proxy(&mut self, config: desigma_http::ProxyConfig) -> Result<(), String> {
        if config.mode == desigma_http::ProxyMode::Custom && !config.url.trim().is_empty() {
            desigma_http::validate_proxy_url(&config.url)?;
        }
        desigma_http::set_config(config.clone());
        self.proxy = config.clone();
        if let Some(lib) = &self.lib {
            crate::comments::settings::save_proxy(lib.root(), &config)?;
        }
        self.show_toast(format!("Network proxy: {}", describe(&config)), false);
        Ok(())
    }
}

/// One-line summary of a configuration.
pub fn describe(c: &desigma_http::ProxyConfig) -> String {
    match c.mode {
        desigma_http::ProxyMode::Off => "off".into(),
        desigma_http::ProxyMode::Custom => {
            let url = c.url.trim();
            if url.is_empty() {
                "custom (no URL set)".into()
            } else {
                format!("custom, {url}")
            }
        }
        desigma_http::ProxyMode::System => match c.resolve() {
            Some(p) => format!("system, {}", p.url),
            None => "system (no proxy)".into(),
        },
    }
}

const W: f32 = 460.0;
const PAD: f32 = 20.0;
const BTN_H: f32 = 32.0;

/// Draws the dialog when it is open. Call once per frame.
pub fn show(app: &mut App, ctx: &Context) {
    let Some(mut d) = app.proxy_dialog.take() else { return };
    let screen = ctx.content_rect();
    let (enter, esc) = ctx.input_mut(|i| (i.consume_key(Modifiers::NONE, Key::Enter), i.consume_key(Modifiers::NONE, Key::Escape)));
    let text_w = W - PAD * 2.0;
    let custom = d.config.mode == desigma_http::ProxyMode::Custom;
    let field_h = 54.0;
    let mut h = PAD + 24.0 + 12.0;
    h += 26.0 * 3.0 + 12.0; // the three mode rows
    h += 34.0; // summary / hint
    if custom {
        h += field_h * 2.0 + 8.0;
    }
    if d.error.is_some() {
        h += 34.0;
    }
    h += BTN_H + PAD;
    let rect = Rect::from_center_size(screen.center() - Vec2::new(0.0, 24.0), Vec2::new(W, h));

    let mut submit = enter;
    let mut cancel = esc;
    let mut save = None;
    egui::Area::new(Id::new("network-proxy")).order(Order::Foreground).fixed_pos(screen.min).interactable(true).show(ctx, |ui| {
        let scrim = ui.allocate_rect(screen, Sense::click_and_drag());
        let p = ui.painter().clone();
        p.rect_filled(screen, 0.0, Color32::from_black_alpha(140));
        if scrim.clicked() && !rect.contains(scrim.interact_pointer_pos().unwrap_or(Pos2::ZERO)) {
            cancel = true;
        }
        widgets::menu_panel(&p, rect, theme::MENU_RADIUS);
        let x0 = rect.left() + PAD;
        let mut y = rect.top() + PAD;
        p.text(Pos2::new(x0, y + 12.0), Align2::LEFT_CENTER, "Network proxy", theme::font_large_strong(), theme::TEXT);
        y += 24.0 + 12.0;

        for (i, mode) in desigma_http::ProxyMode::ALL.into_iter().enumerate() {
            let r = Rect::from_min_size(Pos2::new(x0, y), Vec2::new(text_w, 26.0));
            let resp = ui.interact(r, Id::new(("proxy-mode", i)), Sense::click());
            let on = d.config.mode == mode;
            if on {
                p.rect_filled(r.shrink2(Vec2::new(2.0, 1.0)), theme::cr(theme::RADIUS_MEDIUM), theme::BG_HOVER);
            }
            let cx = Pos2::new(r.left() + 10.0, r.center().y);
            p.circle_stroke(cx, 6.0, egui::Stroke::new(1.5, if on { theme::BG_BRAND } else { theme::TEXT_TERTIARY }));
            if on {
                p.circle_filled(cx, 3.5, theme::BG_BRAND);
            }
            p.text(Pos2::new(r.left() + 26.0, r.center().y), Align2::LEFT_CENTER, mode.label(), theme::font_body(), theme::TEXT);
            widgets::record(ui, &format!("proxy-mode-{i}"), r);
            if resp.clicked() {
                d.config.mode = mode;
            }
            y += 26.0;
        }
        y += 6.0;

        match d.config.mode {
            desigma_http::ProxyMode::System => {
                let text = match &d.detected {
                    Some(url) => format!("Using {url}"),
                    None => "No proxy found for this launch.".to_string(),
                };
                let g = p.layout(text, theme::font_small(), theme::TEXT_SECONDARY, text_w);
                p.galley(Pos2::new(x0, y), g, theme::TEXT_SECONDARY);
                y += 18.0;
                let hint = if desigma_http::from_shell() {
                    "Started from a terminal: this launch reads that shell's HTTP_PROXY, HTTPS_PROXY, ALL_PROXY and NO_PROXY."
                } else {
                    "Started from the desktop: this launch reads the desktop-wide proxy settings."
                };
                let g = p.layout(hint.to_string(), theme::font_small(), theme::TEXT_TERTIARY, text_w);
                p.galley(Pos2::new(x0, y), g, theme::TEXT_TERTIARY);
            }
            desigma_http::ProxyMode::Custom => {
                let mut url = std::mem::take(&mut d.config.url);
                let mut no_proxy = std::mem::take(&mut d.config.no_proxy);
                let fresh = d.fresh;
                d.fresh = false;
                field(ui, &p, x0, &mut y, "Proxy URL", "proxy-url", &mut url, "http://host:port", fresh);
                field(ui, &p, x0, &mut y, "Bypass for", "proxy-no-proxy", &mut no_proxy, "localhost, .internal.example", false);
                d.config.url = url;
                d.config.no_proxy = no_proxy;
            }
            desigma_http::ProxyMode::Off => {
                let g = p.layout("Every request connects directly.".to_string(), theme::font_small(), theme::TEXT_SECONDARY, text_w);
                p.galley(Pos2::new(x0, y), g, theme::TEXT_SECONDARY);
            }
        }

        if let Some(e) = &d.error {
            let g = p.layout(e.clone(), theme::font_small(), theme::DANGER, text_w);
            p.galley(Pos2::new(x0, y), g, theme::DANGER);
        }
        let ok_w = widgets::text_width(&p, "Save", &theme::font_body_strong()).max(48.0) + 32.0;
        let ok_r = Rect::from_min_size(Pos2::new(rect.right() - PAD - ok_w, rect.bottom() - PAD - BTN_H), Vec2::new(ok_w, BTN_H));
        let resp = ui.interact(ok_r, Id::new("proxy-ok"), Sense::click());
        let bg = if resp.is_pointer_button_down_on() {
            theme::BG_BRAND_HOVER
        } else if resp.hovered() {
            theme::BG_BRAND_HOVER
        } else {
            theme::BG_BRAND
        };
        p.rect_filled(ok_r, theme::cr(theme::RADIUS_MEDIUM), bg);
        p.text(ok_r.center(), Align2::CENTER_CENTER, "Save", theme::font_body_strong(), Color32::WHITE);
        widgets::record(ui, "proxy-ok", ok_r);
        if resp.clicked() {
            submit = true;
        }
        let cw = widgets::text_width(&p, "Cancel", &theme::font_body()) + 32.0;
        let cr = Rect::from_min_size(Pos2::new(ok_r.left() - 8.0 - cw, ok_r.top()), Vec2::new(cw, BTN_H));
        let cresp = ui.interact(cr, Id::new("proxy-cancel"), Sense::click());
        widgets::button_bg(&p, cr, &cresp, false, theme::cr(theme::RADIUS_MEDIUM));
        p.text(cr.center(), Align2::CENTER_CENTER, "Cancel", theme::font_body(), theme::TEXT);
        widgets::record(ui, "proxy-cancel", cr);
        if cresp.clicked() {
            cancel = true;
        }
        if submit {
            save = Some(d.config.clone());
        }
    });

    if cancel {
        return;
    }
    if let Some(config) = save {
        match app.save_proxy(config) {
            Ok(()) => return,
            Err(e) => {
                // Keep the dialog open with the message, so the URL can be fixed.
                d.error = Some(e);
                app.proxy_dialog = Some(d);
                return;
            }
        }
    }
    app.proxy_dialog = Some(d);
}

fn field(ui: &mut Ui, p: &egui::Painter, x0: f32, y: &mut f32, label: &str, id: &str, text: &mut String, hint: &str, focus: bool) {
    p.text(Pos2::new(x0, *y + 8.0), Align2::LEFT_CENTER, label, theme::font_small_medium(), theme::TEXT_SECONDARY);
    let r = Rect::from_min_size(Pos2::new(x0, *y + 20.0), Vec2::new(W - PAD * 2.0, 30.0));
    p.rect_filled(r, theme::cr(theme::RADIUS_MEDIUM), theme::BG_SECONDARY);
    let te = egui::TextEdit::singleline(text)
        .id(Id::new(id))
        .font(theme::font_body())
        .text_color(theme::TEXT)
        .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(10, 0)))
        .vertical_align(egui::Align::Center)
        .hint_text(egui::RichText::new(hint).color(theme::TEXT_TERTIARY))
        .min_size(r.size())
        .desired_width(r.width());
    let out = ui.put(r, te);
    if focus {
        out.request_focus();
    }
    widgets::record(ui, id, r);
    *y += 54.0;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describe_covers_each_mode() {
        let off = desigma_http::ProxyConfig { mode: desigma_http::ProxyMode::Off, ..Default::default() };
        assert_eq!(describe(&off), "off");
        let custom = desigma_http::ProxyConfig { mode: desigma_http::ProxyMode::Custom, url: "http://h:1".into(), ..Default::default() };
        assert_eq!(describe(&custom), "custom, http://h:1");
        let empty = desigma_http::ProxyConfig { mode: desigma_http::ProxyMode::Custom, ..Default::default() };
        assert_eq!(describe(&empty), "custom (no URL set)");
    }

    #[test]
    fn dialog_starts_on_the_saved_settings() {
        let c = desigma_http::ProxyConfig { mode: desigma_http::ProxyMode::Custom, url: "http://h:1".into(), ..Default::default() };
        let d = ProxyDialog::open(c.clone());
        assert_eq!(d.config, c);
        assert_eq!(d.detected.as_deref(), Some("http://h:1"));
    }
}
