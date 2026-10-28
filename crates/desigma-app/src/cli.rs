//! Command line: `desigma [file] [--screenshot out.png] [--size WxH] [--select name|none] [--menu name]`.

use std::path::PathBuf;

#[derive(Debug, Default, PartialEq)]
pub struct Options {
    pub file: Option<PathBuf>,
    /// A `desigma://join/...` link: opens the Join session dialog with it filled in.
    pub join: Option<String>,
    pub screenshot: Option<PathBuf>,
    pub size: Option<(u32, u32)>,
    /// Device pixel ratio of `--screenshot` (`--scale 2` writes a PNG twice the `--size`, with crisp UI and text).
    pub scale: Option<f32>,
    /// Select the first layer with this name after loading (handy for screenshots).
    pub select: Option<String>,
    /// Open a menu after loading: `main`, `main/File`, `shape`, `palette` (for screenshots).
    pub menu: Option<String>,
    /// Built-in test document instead of the demo (`showcase`: the Marlow product-design file the README screenshots use; `proto-vars`: a shop prototype with variables; `no-selection`: the Figma page-level reference;
    /// `variables`; `comments`: Figma comments imported, one thread open in comment mode;
    /// `board`: a sample whiteboard; `board-empty`: an empty one).
    pub fixture: Option<String>,
    /// Right panel tab to show: `design` or `prototype`.
    pub tab: Option<String>,
    /// Open the Present view on start.
    pub present: bool,
    /// Start presentation in native fullscreen.
    pub fullscreen: bool,
    /// Open a panel on start: `variables` (full view) or `variables-floating`.
    pub panel: Option<String>,
    /// Initial zoom in percent (`--zoom 800`), centred on the selection or the page.
    pub zoom: Option<f32>,
    /// Show the rulers.
    pub rulers: bool,
    /// Draw the canvas with the GPU renderer (`--screenshot` otherwise always uses the
    /// deterministic CPU renderer; normal launches use the GPU unless it is turned off).
    pub gpu: bool,
    /// --bridge: accept MCP tool calls from desigma-mcp for this launch.
    pub bridge: bool,
    /// Force the pixel grid on (it appears from 800% zoom).
    pub grid: bool,
    /// Enter text editing on the first text layer with this name (select it with `--select-text`).
    pub edit_text: Option<String>,
    /// Character range selected while editing: `START-END` (in chars); default is everything.
    pub select_text: Option<(usize, usize)>,
    /// Add a layout grid to the selected frame: `grid`, `columns` or `rows`.
    pub layout_grid: Option<String>,
    /// Show the projects home screen (default when launched without a file).
    pub home: bool,
    /// Library root directory (`--library DIR`, default `$DESIGMA_LIBRARY` or the user data dir).
    pub library: Option<PathBuf>,
    /// Open this library file id on start (used by "Open in new window").
    pub open_id: Option<String>,
    /// Open the Find and replace panel with this query (`--find TEXT`).
    pub find: Option<String>,
    /// Replacement text for `--find`; also shows the replace row.
    pub replace_with: Option<String>,
    /// Open Dev mode with this code language: `css`, `tailwind`, `swiftui`, `compose` or `react`.
    pub dev: Option<String>,
    /// With `--dev`: mark the selected layer Ready for dev and give it a demo annotation and link.
    pub dev_demo: bool,
    /// Open Motion mode with a demo animation on the selected layer (screenshot aid).
    pub motion: bool,
    /// Write the showcase sample files into this library directory and exit (screenshot aid).
    pub seed_library: Option<PathBuf>,
    /// `--export-animation FILE`: render the animation of the file's first animated page (or the
    /// layer named by `--select`) to `--out` and exit, without opening a window.
    pub export_animation: Option<PathBuf>,
    /// `--format gif|webm|png` for `--export-animation` (default: from the `--out` extension).
    pub format: Option<String>,
    /// `--out PATH` of `--export-animation`.
    pub out: Option<PathBuf>,
    /// `--fps N` of `--export-animation` (default: the timeline's fps).
    pub fps: Option<u32>,
    /// `--dither` of `--export-animation`: Floyd-Steinberg dithering for GIF output (less banding
    /// on gradients, larger file).
    pub dither: bool,
    pub help: bool,
    /// Print `desigma <version>` and exit.
    pub version: bool,
}

impl Options {
    /// Whether a launch with these options starts on the home screen: explicitly
    /// with `--home`, or interactively with nothing else to open.
    pub fn wants_home(&self) -> bool {
        self.open_id.is_none() && (self.home || (self.file.is_none() && self.screenshot.is_none() && self.fixture.is_none() && !self.motion))
    }
}

/// Values of `--panel`: `variables` / `variables-floating` open the local variables view,
/// `libraries` the Libraries modal, `updater` / `updater-banner` check the update feed (`DESIGMA_UPDATE_URL`)
/// and show the update dialog or banner; the others are inspector popups opened on the selection for screenshots.
pub const PANELS: [&str; 29] = [
    "share",
    "collab",
    "comments",
    "plugins",
    "plugin-install",
    "interaction",
    "present-variables",
    "variables",
    "variables-floating",
    "libraries",
    "picker",
    "picker-page",
    "fill-d9",
    "picker-gradient",
    "effects-menu",
    "stroke-settings",
    "type-details",
    "effect-noise",
    "effect-texture",
    "effect-glass",
    "version-history",
    "dev-annotate",
    "dev-compare",
    "motion-inspector",
    "motion-easing",
    "motion-library",
    "motion-clip",
    "updater",
    "updater-banner",
];

pub fn parse_size(s: &str) -> Option<(u32, u32)> {
    let (w, h) = s.split_once(['x', 'X'])?;
    let (w, h) = (w.trim().parse::<u32>().ok()?, h.trim().parse::<u32>().ok()?);
    (w >= 200 && h >= 200 && w <= 8192 && h <= 8192).then_some((w, h))
}

/// Parses a scale like `2` or `1.5` (device pixel ratio of a screenshot, pixel scale of an
/// animation export). 0.1..=4; screenshots need 1..=4, which `parse` checks.
pub fn parse_scale(s: &str) -> Option<f32> {
    let v: f32 = s.trim().trim_end_matches(['x', 'X']).trim().parse().ok()?;
    (v.is_finite() && (0.1..=4.0).contains(&v)).then_some(v)
}

/// Parses a zoom like `800`, `800%` or `12.5%` (percent, 1..=25600).
pub fn parse_zoom(s: &str) -> Option<f32> {
    let v: f32 = s.trim().trim_end_matches('%').trim().parse().ok()?;
    (v.is_finite() && (1.0..=25600.0).contains(&v)).then_some(v)
}

fn parse_range(s: &str) -> Option<(usize, usize)> {
    let (a, b) = s.split_once('-')?;
    Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
}

pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Options, String> {
    let mut o = Options::default();
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--screenshot" => o.screenshot = Some(it.next().ok_or("--screenshot needs an output path")?.into()),
            "--size" => {
                let v = it.next().ok_or("--size needs WxH")?;
                o.size = Some(parse_size(&v).ok_or_else(|| format!("invalid size '{v}' (expected WxH, e.g. 1600x1088)"))?);
            }
            "--scale" => {
                let v = it.next().ok_or("--scale needs a factor (e.g. 2)")?;
                // validated below: --export-animation also accepts 0.1..1
                o.scale = Some(parse_scale(&v).ok_or_else(|| format!("invalid scale '{v}' (expected 1..4, e.g. 2)"))?);
            }
            "--export-animation" => o.export_animation = Some(it.next().ok_or("--export-animation needs a .desigma file")?.into()),
            "--format" => {
                let f = it.next().ok_or("--format needs gif, webm or png")?;
                if desigma_export::anim::AnimFormat::parse(&f).is_none() {
                    return Err(format!("unknown animation format '{f}' (expected gif, webm or png)"));
                }
                o.format = Some(f);
            }
            "--out" => o.out = Some(it.next().ok_or("--out needs an output path")?.into()),
            "--fps" => {
                let v = it.next().ok_or("--fps needs a number (1..120)")?;
                o.fps = Some(v.trim().parse::<u32>().ok().filter(|f| (1..=120).contains(f)).ok_or_else(|| format!("invalid fps '{v}' (expected 1..120)"))?);
            }
            "--dither" => o.dither = true,
            "--select" => o.select = Some(it.next().ok_or("--select needs a layer name")?),
            "--menu" => o.menu = Some(it.next().ok_or("--menu needs a name (main, main/File, shape, palette)")?),
            "--fixture" => o.fixture = Some(it.next().ok_or("--fixture needs a name (no-selection)")?),
            "--tab" => {
                let t = it.next().ok_or("--tab needs design or prototype")?;
                if !matches!(t.as_str(), "design" | "prototype") {
                    return Err(format!("unknown tab '{t}' (expected design or prototype)"));
                }
                o.tab = Some(t);
            }
            "--present" => o.present = true,
            "--fullscreen" => {
                o.present = true;
                o.fullscreen = true;
            }
            "--panel" => {
                let t = it.next().ok_or("--panel needs a name (see --help)")?;
                if !PANELS.contains(&t.as_str()) {
                    return Err(format!("unknown panel '{t}' (expected {})", PANELS.join(", ")));
                }
                o.panel = Some(t);
            }
            "--rulers" => o.rulers = true,
            "--gpu" => o.gpu = true,
            "--bridge" => o.bridge = true,
            "--grid" => o.grid = true,
            "--zoom" => {
                let v = it.next().ok_or("--zoom needs a percentage (e.g. 800)")?;
                o.zoom = Some(parse_zoom(&v).ok_or_else(|| format!("invalid zoom '{v}' (expected 1..25600, e.g. 800 or 800%)"))?);
            }
            "--edit-text" => o.edit_text = Some(it.next().ok_or("--edit-text needs a text layer name")?),
            "--select-text" => {
                let v = it.next().ok_or("--select-text needs START-END")?;
                o.select_text = Some(parse_range(&v).ok_or_else(|| format!("invalid range '{v}' (expected START-END in characters)"))?);
            }
            "--layout-grid" => {
                let k = it.next().ok_or("--layout-grid needs grid, columns or rows")?;
                if !matches!(k.as_str(), "grid" | "columns" | "rows") {
                    return Err(format!("unknown layout grid '{k}' (expected grid, columns or rows)"));
                }
                o.layout_grid = Some(k);
            }
            "--home" => o.home = true,
            "--library" => o.library = Some(it.next().ok_or("--library needs a directory")?.into()),
            "--open-id" => o.open_id = Some(it.next().ok_or("--open-id needs a library file id")?),
            "--find" => o.find = Some(it.next().ok_or("--find needs the text to search for")?),
            "--replace-with" => o.replace_with = Some(it.next().ok_or("--replace-with needs the replacement text")?),
            "--dev" => {
                let l = it.next().ok_or("--dev needs css, tailwind, swiftui, compose or react")?;
                if !matches!(l.as_str(), "css" | "tailwind" | "swiftui" | "compose" | "react") {
                    return Err(format!("unknown language '{l}' (expected css, tailwind, swiftui, compose or react)"));
                }
                o.dev = Some(l);
            }
            "--dev-demo" => o.dev_demo = true,
            "--motion" => o.motion = true,
            "--seed-library" => o.seed_library = Some(it.next().ok_or("--seed-library needs a directory")?.into()),
            "-h" | "--help" => o.help = true,
            "-V" | "--version" => o.version = true,
            s if s.starts_with("--") => return Err(format!("unknown option {s}")),
            s if s.get(..10).is_some_and(|p| p.eq_ignore_ascii_case("desigma://")) => o.join = Some(a),
            // a file manager may hand over `file:///...` URLs (the Linux launcher passes %U)
            s if s.starts_with("file://") => {
                if o.file.is_some() {
                    return Err("only one file can be opened".into());
                }
                o.file = Some(url::Url::parse(s).ok().and_then(|u| u.to_file_path().ok()).ok_or_else(|| format!("invalid file URL {s}"))?);
            }
            _ => {
                if o.file.is_some() {
                    return Err("only one file can be opened".into());
                }
                o.file = Some(a.into());
            }
        }
    }
    if o.export_animation.is_none() {
        if let Some(sc) = o.scale.filter(|s| *s < 1.0) {
            return Err(format!("invalid scale '{sc}' (expected 1..4, e.g. 2)"));
        }
        if o.format.is_some() || o.out.is_some() || o.fps.is_some() || o.dither {
            return Err("--format, --out, --fps and --dither need --export-animation FILE".into());
        }
    } else if o.out.is_none() {
        return Err("--export-animation needs --out PATH".into());
    }
    Ok(o)
}

pub const USAGE: &str = "usage: desigma [FILE|desigma://join/LINK] [--screenshot OUT.png] [--size WxH] [--scale FACTOR] [--select LAYER|none] [--menu main|main/File|shape|palette] [--panel interaction|present-variables|variables|variables-floating|libraries|picker|picker-page|fill-d9|picker-gradient|effects-menu|stroke-settings|type-details|effect-noise|effect-texture|effect-glass|version-history|dev-annotate|dev-compare|motion-inspector|motion-easing|motion-library|motion-clip|updater|updater-banner|share|collab|comments]\n               [--zoom PERCENT] [--rulers] [--grid] [--gpu] [--bridge] [--home] [--library DIR] [--layout-grid grid|columns|rows] [--edit-text LAYER [--select-text START-END]]\n  FILE  .fig, .pen (Pencil), .json (Figma REST or native), .svg or .desigma; without a file the projects home screen opens
  desigma://join/LINK  open the Join session dialog with a shared link filled in
  --scale    with --screenshot: device pixel ratio (2 writes a PNG twice --size, rendered crisp rather than upscaled)
  --home     show the home screen (also in --screenshot mode)
  --present  open presentation preview; --fullscreen starts it in native fullscreen (F11 toggles, Esc returns)
  --gpu      draw the canvas on the GPU; --screenshot uses the CPU renderer unless this is given
  --bridge   let desigma-mcp control this editor (Preferences → Allow MCP agents… makes it permanent)
  --library  use DIR as the projects library instead of the default location
  --dev      open Dev mode with a code language (css|tailwind|swiftui|compose|react); --dev-demo marks the selection Ready for dev with an annotation
  --motion   open Motion mode with a demo animation on the selected layer
  --seed-library  write the showcase sample files (a project with three files) into DIR and exit
  --find     open the Find and replace panel searching for TEXT (--replace-with TEXT shows the replace row)
  --export-animation FILE  render FILE's Motion animation (first animated page, or the layer named by --select) and exit:
               --out PATH [--format gif|webm|png] [--fps N] [--scale 0.1..4] [--dither]; png writes a folder, or a .zip when PATH ends in .zip
  --version  print the version and exit";

#[cfg(test)]
mod tests {
    use super::*;

    fn p(a: &[&str]) -> Result<Options, String> {
        parse(a.iter().map(|s| s.to_string()))
    }

    #[test]
    fn gpu_flag_is_off_unless_given() {
        assert!(!p(&["--screenshot", "x.png"]).unwrap().gpu);
        let o = p(&["--gpu", "--screenshot", "x.png"]).unwrap();
        assert!(o.gpu && o.screenshot.is_some());
    }

    #[test]
    fn bridge_flag_is_off_unless_given() {
        assert!(!p(&[]).unwrap().bridge);
        assert!(p(&["--bridge"]).unwrap().bridge);
    }

    #[test]
    fn parses_file_and_screenshot() {
        let o = p(&["a.fig", "--screenshot", "out/app.png", "--size", "1600x1088", "--select", "Card"]).unwrap();
        assert_eq!(o.file, Some("a.fig".into()));
        assert_eq!(o.screenshot, Some("out/app.png".into()));
        assert_eq!(o.size, Some((1600, 1088)));
        assert_eq!(o.select.as_deref(), Some("Card"));
    }

    #[test]
    fn parses_dev_options() {
        let o = p(&["--dev", "swiftui", "--dev-demo", "--select", "Card"]).unwrap();
        assert_eq!(o.dev.as_deref(), Some("swiftui"));
        assert!(o.dev_demo);
        assert!(p(&["--dev", "cobol"]).is_err());
        assert!(p(&["--dev"]).is_err());
        assert!(p(&["--panel", "dev-compare"]).is_ok());
        assert!(p(&["--panel", "comments"]).is_ok());
    }

    #[test]
    fn parses_session_link() {
        let o = p(&["desigma://join/abc"]).unwrap();
        assert_eq!(o.join.as_deref(), Some("desigma://join/abc"));
        assert!(o.file.is_none() && o.wants_home());
        assert!(p(&["DESIGMA://join/abc", "x.fig"]).unwrap().file.is_some());
    }

    #[test]
    fn file_urls_become_paths() {
        #[cfg(unix)]
        assert_eq!(p(&["file:///tmp/a.desigma"]).unwrap().file, Some("/tmp/a.desigma".into()));
    }

    #[test]
    fn parses_scale() {
        assert_eq!(p(&["--scale", "2"]).unwrap().scale, Some(2.0));
        assert_eq!(p(&["--scale", "1.5x"]).unwrap().scale, Some(1.5));
        assert_eq!(p(&[]).unwrap().scale, None);
        assert!(p(&["--scale", "0.5"]).is_err() && p(&["--scale", "9"]).is_err() && p(&["--scale"]).is_err());
    }

    #[test]
    fn parses_menu_and_select_none() {
        let o = p(&["--menu", "main/File", "--select", "none"]).unwrap();
        assert_eq!(o.menu.as_deref(), Some("main/File"));
        assert_eq!(o.select.as_deref(), Some("none"));
    }

    #[test]
    fn rejects_bad_input() {
        assert!(p(&["--size", "huge"]).is_err());
        assert!(p(&["--screenshot"]).is_err());
        assert!(p(&["--bogus"]).is_err());
        assert!(p(&["--menu"]).is_err());
        assert!(p(&["--tab", "nope"]).is_err());
        assert!(p(&["--tab"]).is_err());
        assert!(p(&["a", "b"]).is_err());
        assert_eq!(parse_size("1440X900"), Some((1440, 900)));
        assert_eq!(parse_size("10x10"), None);
    }

    #[test]
    fn parses_tab_and_present() {
        let o = p(&["--tab", "prototype", "--present"]).unwrap();
        assert_eq!(o.tab.as_deref(), Some("prototype"));
        assert!(o.present);
    }

    #[test]
    fn parses_panel() {
        assert_eq!(p(&["--panel", "variables"]).unwrap().panel.as_deref(), Some("variables"));
        assert_eq!(p(&["--panel", "variables-floating"]).unwrap().panel.as_deref(), Some("variables-floating"));
        assert!(p(&["--panel", "nope"]).is_err());
        assert!(p(&["--panel"]).is_err());
    }

    #[test]
    fn parses_canvas_aid_flags() {
        let o = p(&["--zoom", "800%", "--rulers", "--grid", "--layout-grid", "columns", "--edit-text", "Title", "--select-text", "2-7"]).unwrap();
        assert_eq!(o.zoom, Some(800.0));
        assert!(o.rulers && o.grid);
        assert_eq!(o.layout_grid.as_deref(), Some("columns"));
        assert_eq!(o.edit_text.as_deref(), Some("Title"));
        assert_eq!(o.select_text, Some((2, 7)));
        assert_eq!(parse_zoom("50"), Some(50.0));
        assert!(p(&["--zoom", "0"]).is_err() && p(&["--zoom", "x"]).is_err() && p(&["--zoom"]).is_err());
        assert!(p(&["--layout-grid", "diagonal"]).is_err());
        assert!(p(&["--select-text", "5"]).is_err());
    }

    #[test]
    fn parses_home_and_library() {
        let o = p(&["--home", "--library", "/tmp/lib", "--screenshot", "h.png"]).unwrap();
        assert!(o.home && o.wants_home());
        assert_eq!(o.library, Some("/tmp/lib".into()));
        assert!(p(&["--library"]).is_err());
        assert_eq!(p(&["--open-id", "abc"]).unwrap().open_id.as_deref(), Some("abc"));
        assert!(p(&["--open-id"]).is_err());
    }

    #[test]
    fn home_is_the_default_only_for_a_bare_interactive_launch() {
        assert!(p(&[]).unwrap().wants_home());
        assert!(!p(&["a.desigma"]).unwrap().wants_home(), "a file argument opens the editor");
        assert!(!p(&["--screenshot", "x.png"]).unwrap().wants_home(), "existing screenshot flows keep the editor");
        assert!(!p(&["--fixture", "no-selection"]).unwrap().wants_home());
        assert!(!p(&["--home", "--open-id", "x"]).unwrap().wants_home());
    }

    #[test]
    fn parses_find_and_replace_with() {
        let o = p(&["--find", "quarter", "--replace-with", "period"]).unwrap();
        assert_eq!((o.find.as_deref(), o.replace_with.as_deref()), (Some("quarter"), Some("period")));
        assert!(p(&["--find"]).is_err() && p(&["--replace-with"]).is_err());
    }

    #[test]
    fn parses_version() {
        assert!(p(&["--version"]).unwrap().version && p(&["-V"]).unwrap().version);
        assert!(!p(&[]).unwrap().version);
        assert!(p(&["--panel", "updater"]).is_ok() && p(&["--panel", "updater-banner"]).is_ok());
    }

    #[test]
    fn parses_export_animation() {
        let o = p(&["--export-animation", "a.desigma", "--format", "webm", "--out", "a.webm", "--fps", "30", "--scale", "0.5", "--select", "Hero"]).unwrap();
        assert_eq!(o.export_animation, Some("a.desigma".into()));
        assert_eq!((o.format.as_deref(), o.fps, o.scale), (Some("webm"), Some(30), Some(0.5)));
        assert!(!o.dither && p(&["--export-animation", "a", "--out", "o.gif", "--dither"]).unwrap().dither);
        assert!(p(&["--dither"]).is_err(), "--dither needs --export-animation");
        assert_eq!(o.out, Some("a.webm".into()));
        assert!(p(&["--export-animation", "a.desigma"]).is_err(), "--out is required");
        assert!(p(&["--export-animation", "a", "--out", "o", "--format", "mov"]).is_err());
        assert!(p(&["--export-animation", "a", "--out", "o", "--fps", "0"]).is_err());
        assert!(p(&["--export-animation", "a", "--out", "o", "--fps", "500"]).is_err());
        assert!(p(&["--format", "gif"]).is_err(), "--format alone means nothing");
        assert!(p(&["--scale", "0.5"]).is_err(), "sub-1x scales are only for animation export");
        assert_eq!(p(&["--scale", "2"]).unwrap().scale, Some(2.0));
    }

    #[test]
    fn empty_is_default() {
        assert_eq!(p(&[]).unwrap(), Options::default());
    }
}
