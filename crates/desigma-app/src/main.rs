mod app;
mod board;
mod bridge;
mod canvas;
mod cli;
mod clipboard;
mod code_export_ui;
mod collab;
mod comments;
mod comp_ops;
mod decks;
mod demo;
mod dev;
mod edit;
mod find;
mod fonts;
mod history;
mod home;
mod icons;
mod instance;
mod io;
mod layout;
mod libraries;
#[cfg(test)]
mod lint;
mod loopback;
#[cfg(target_os = "macos")]
mod macos;
mod motion;
mod network;
mod ops;
mod panels;
mod plugins;
mod present;
mod proto;
mod protovars;
mod richtext;
mod shell;
mod showcase;
mod styles;
mod textedit;
mod textmeasure;
mod theme;
mod unsaved;
mod updater;
mod util;
mod variables;
mod vecfixture;
mod vecops;
mod video;
mod widgets;

use std::sync::atomic::Ordering;

/// The running version: the release tag that CI stamps in as `DESIGMA_VERSION`, else the crate version.
pub const VERSION: &str = match option_env!("DESIGMA_VERSION") {
    Some(v) => v,
    None => env!("CARGO_PKG_VERSION"),
};
/// Only release builds carry a stamped version; builds from source never look for updates.
pub const RELEASE_BUILD: bool = option_env!("DESIGMA_VERSION").is_some();

/// The app icon (256 px PNG embedded at build time) for the taskbar, Dock and alt-tab. Every window
/// desigma opens (including Present and "open in new window" processes) starts through `main`, so
/// they all get it.
fn window_icon() -> Option<egui::IconData> {
    // macOS shows the window icon in the Dock when desigma runs as a bare binary (cargo run), where
    // the squircle needs the same transparent margin the .icns has; elsewhere the edge-to-edge art
    // fills title bars and taskbars as intended.
    #[cfg(target_os = "macos")]
    const PNG: &[u8] = include_bytes!("../../../assets/icon/desigma.iconset/icon_256x256.png");
    #[cfg(not(target_os = "macos"))]
    const PNG: &[u8] = include_bytes!("../../../assets/icon/png/icon_256.png");
    let img = image::load_from_memory_with_format(PNG, image::ImageFormat::Png).ok()?.into_rgba8();
    let (width, height) = img.dimensions();
    Some(egui::IconData { rgba: img.into_raw(), width, height })
}

fn main() {
    let opts = match cli::parse(std::env::args().skip(1)) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("desigma: {e}\n{}", cli::USAGE);
            std::process::exit(2);
        }
    };
    if opts.help {
        println!("{}", cli::USAGE);
        return;
    }
    if opts.version {
        println!("desigma {VERSION}");
        return;
    }
    if let Some(proxy) = &opts.proxy {
        match parse_proxy_flag(proxy) {
            Some(config) => desigma_http::set_config(config),
            None => {
                eprintln!("desigma: invalid --proxy '{proxy}' (expected off, system or a URL like http://host:port)");
                std::process::exit(2);
            }
        }
    }
    if let Some(file) = &opts.export_code {
        match code_export_ui::run_cli(file, opts.code.as_deref(), opts.out.as_deref(), opts.select.as_deref(), opts.code_tailwind, opts.no_tokens) {
            Ok(msg) => println!("desigma: {msg}"),
            Err(e) => {
                eprintln!("desigma: {e}");
                std::process::exit(1);
            }
        }
        return;
    }
    if let Some(file) = &opts.export_animation {
        match motion::playback::export::run_cli(file, opts.format.as_deref(), opts.out.as_deref(), opts.fps, opts.scale, opts.select.as_deref(), opts.dither) {
            Ok(msg) => println!("desigma: {msg}"),
            Err(e) => {
                eprintln!("desigma: {e}");
                std::process::exit(1);
            }
        }
        return;
    }
    if let Some(dir) = &opts.seed_library {
        match showcase::seed_library(dir) {
            Ok(()) => println!("desigma: wrote the showcase sample files to {}", dir.display()),
            Err(e) => {
                eprintln!("desigma: cannot seed {}: {e}", dir.display());
                std::process::exit(1);
            }
        }
        return;
    }
    // A plain `desigma FILE|desigma://join/LINK` (what the OS runs for a click) goes to the editor
    // that is already running, without opening a window here.
    let only_open = cli::Options { file: opts.file.clone(), join: opts.join.clone(), ..Default::default() } == opts;
    if only_open && (opts.file.is_some() || opts.join.is_some()) {
        let dir = instance::default_dir();
        let mut opens = opts.join.iter().cloned().map(instance::Open::Link).chain(opts.file.iter().cloned().map(instance::Open::File));
        if opens.all(|o| instance::forward(&dir, &o)) {
            return;
        }
    }
    #[cfg(target_os = "macos")]
    macos::register();
    let (w, h) = opts.size.unwrap_or((theme::DEFAULT_WINDOW.0 as u32, theme::DEFAULT_WINDOW.1 as u32));
    let screenshot_mode = opts.screenshot.is_some();
    let mut viewport = egui::ViewportBuilder::default()
        .with_title(theme::APP_TITLE)
        .with_inner_size([w as f32, h as f32])
        .with_min_inner_size([theme::MIN_WINDOW.0, theme::MIN_WINDOW.1]);
    if let Some(icon) = window_icon() {
        viewport = viewport.with_icon(icon);
    }
    if screenshot_mode {
        viewport = viewport.with_min_inner_size([200.0, 200.0]).with_resizable(true);
    }
    let native = eframe::NativeOptions { viewport, ..Default::default() };
    let result = eframe::run_native(theme::APP_TITLE, native, Box::new(move |cc| Ok(Box::new(shell::Shell::new(cc, opts)))));
    if let Err(e) = result {
        eprintln!("desigma: {e}");
        std::process::exit(1);
    }
    // normally started when the window closed; this catches a close that skipped that frame
    updater::install::run_stashed();
    if screenshot_mode && !shell::SHOT_OK.load(Ordering::SeqCst) {
        eprintln!("desigma: screenshot was not captured");
        std::process::exit(1);
    }
}

/// `--proxy off|system|<url>`: what this run should use.
fn parse_proxy_flag(v: &str) -> Option<desigma_http::ProxyConfig> {
    let v = v.trim();
    if v.eq_ignore_ascii_case("off") {
        return Some(desigma_http::ProxyConfig { mode: desigma_http::ProxyMode::Off, ..Default::default() });
    }
    if v.eq_ignore_ascii_case("system") {
        return Some(desigma_http::ProxyConfig::default());
    }
    desigma_http::validate_proxy_url(v).ok()?;
    Some(desigma_http::ProxyConfig { mode: desigma_http::ProxyMode::Custom, url: v.to_string(), no_proxy: String::new() })
}

#[cfg(test)]
mod proxy_flag_tests {
    use super::*;

    #[test]
    fn parses_the_three_forms() {
        assert_eq!(parse_proxy_flag("off").unwrap().mode, desigma_http::ProxyMode::Off);
        assert_eq!(parse_proxy_flag("System").unwrap().mode, desigma_http::ProxyMode::System);
        let c = parse_proxy_flag("http://127.0.0.1:3128").unwrap();
        assert_eq!((c.mode, c.url.as_str()), (desigma_http::ProxyMode::Custom, "http://127.0.0.1:3128"));
        assert!(parse_proxy_flag("not a url").is_none());
    }
}

#[cfg(test)]
mod icon_tests {
    #[test]
    fn embedded_window_icon_decodes() {
        let icon = super::window_icon().expect("icon decodes");
        assert_eq!((icon.width, icon.height), (256, 256));
        assert_eq!(icon.rgba.len(), 256 * 256 * 4);
    }
}
