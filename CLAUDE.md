# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

desigma — an independent, local-first, open-source interface design tool written entirely in Rust (single-language cargo workspace, 16 crates). Native desktop editor (egui + wgpu/winit), deterministic CPU renderer on tiny-skia, Figma REST JSON and `.fig` import, live collaboration, keyframe motion, and a local MCP server. Early alpha.

## Build, Test, Lint

Toolchain is pinned in `rust-toolchain.toml` (1.93.1 with clippy + rustfmt). Bump it together with `RUST_VERSION` in `.github/workflows/*.yml`.

```sh
cargo build --workspace --all-targets --locked
cargo test --workspace --locked                 # full suite (~1,600 tests)
cargo test -p desigma-doc                       # one crate
cargo test -p desigma-doc undo                  # filter tests by name
cargo clippy --workspace --all-targets --locked -- -D warnings   # CI gate: must be warning-free
cargo fmt --all --check                         # advisory only (pre-rustfmt codebase); rustfmt.toml: max_width 160, Max heuristics
cargo run --release -p desigma-app              # editor with demo document
cargo run --release -p desigma-app -- file.fig  # open a Figma file
```

CI (`.github/workflows/ci.yml`) runs build + test + clippy + fmt-check on macOS / Windows / Ubuntu, with `RUSTFLAGS: -D warnings` for the build. Linux needs the windowing/fontconfig dev packages listed in the workflow and README.

Dev builds compile dependencies with `opt-level = 2` (root `Cargo.toml` `[profile.dev.package."*"]`) — dev builds are usable, don't default to `--release` for iterating.

### Editor CLI flags for screenshots / verification

```sh
desigma --screenshot out.png --size 1600x1000 --select Card
desigma --fixture showcase --select Home --size 1600x1000 --scale 2 --screenshot hero.png
desigma --tab prototype --present --screenshot p.png
desigma-diff compare reference.png actual.png --out diff.png
```

`desigma-diff` (SSIM + per-pixel threshold, anti-aliased edges forgiven by default) is the visual-regression harness; rendering changes should keep baseline screenshots matching.

## Architecture

The dependency spine: **doc → engine → (render | app | mcp)**. Everything below hangs off it.

- `desigma-doc` — the one document model: scene graph (frames, auto layout, components/instances, variables, prototype + motion), paints, text. Plain data + undo stack where every edit is a `Command` with an exact inverse.
- `desigma-engine` — editing logic shared by the app and the MCP server: transactions, auto layout, text measuring, component sync. This sharing is why MCP edits and GUI edits behave identically.
- `desigma-render` — deterministic CPU renderer (tiny-skia), spatial index, parallel 256px tile cache. This is the **reference renderer**; tile output must byte-match a whole-page render.
- `desigma-gpu` — wgpu + lyon renderer with CPU fallback; the CPU path stays authoritative.
- `desigma-app` — the egui desktop editor (canvas, layers, inspector, prototype/decks/home/panels). Owns the agent bridge (`--bridge`) that lets desigma-mcp drive the on-screen document.
- `desigma-mcp` — local MCP server, 16 tools for import/inspect/edit/codegen. Without a running editor it works on an isolated file session; with the bridge on it edits the live document and shares its undo.
- `desigma-fig` — Figma REST JSON and `.fig` (fig-kiwi container: deflate/zstd Kiwi) import, REST JSON export. Format is undocumented — keep the decoder tolerant of unknown fields.
- `desigma-svg` / `desigma-pen` — SVG and Pencil `.pen` import.
- `desigma-fonts` — Inter bundled in-binary; other open families fetched from Fontsource and cached; proprietary fonts map to metric-compatible substitutes.
- `desigma-export` — PNG/JPG/SVG/PDF, animated GIF/WebM/PNG sequences, Dev Mode CSS, SwiftUI/Compose code.
- `desigma-library` — projects, team libraries, publishing, version history.
- `desigma-collab` — collaboration core: doc diffs, host sequencer/ordering, conflict rules, per-person undo. Design notes in `docs/collaboration.md`.
- `desigma-net` — peer-to-peer transport over iroh QUIC (links, password, approval, presence); `desigma-server` — headless always-on host that keeps a `.desigma` file shared.
- `desigma-diff` — pixel-diff library + CLI.

## Product direction (constrains design decisions)

Faithful document rendering and an independently designed editor UI. Figma file compatibility and familiar workflows guide development; reproducing Figma's editor chrome is explicitly **not** a goal. Editor regression baselines are approved desigma screenshots, not Figma screenshots. License caution applies to third-party fonts, fixtures, and assets.

## Conventions

- Commit style: short imperative subject with area prefix — `app: assets pop zoom`, `doc: fix undo`. Explain why in the body when non-obvious; add/update tests with behavior changes.
- Keep every crate's public behavior covered; the workspace passes clippy `-D warnings` in CI, so run it before declaring done.
- Releases: `docs/releasing.md`; packaging via `scripts/package.sh`, `scripts/package-macos.sh`, `scripts/package-windows-installer.sh` (CI in `release.yml`).
- MCP tool reference: `crates/desigma-mcp/README.md`; install docs: `docs/install.md`.
- `PLAN.md` is the original phased roadmap, not a completion tracker — trust the README and crate manifests for current state.
