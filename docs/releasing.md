# Releasing

Releases are tag-driven. Pushing a tag `vX.Y.Z` runs `.github/workflows/release.yml`.

## Checklist

1. Make sure `main` is green (tests on Linux, Windows and macOS, Clippy).
2. Optionally bump `version` under `[workspace.package]` in the root `Cargo.toml`, run `cargo build --workspace` so `Cargo.lock` updates, and commit both (`release: 0.2.0`). The app's own version comes from the tag either way: the workflow compiles it in as `DESIGMA_VERSION` (About, `desigma --version`, update checks) and fails if `--version` disagrees.
3. Tag and push:

   ```sh
   git tag -a v0.2.0 -m "desigma 0.2.0"
   git push origin v0.2.0
   ```

   A tag containing a hyphen (`v0.2.0-rc.1`) is published as a pre-release.
4. Watch the Release workflow. When all builds pass, a GitHub Release is created with auto-generated notes. Edit the notes if needed.

## What gets built

| Job | Runner | Output |
|---|---|---|
| `build` `x86_64-pc-windows-msvc` | `windows-latest` | `desigma-<version>-<target>.zip` (the exe has the icon and version info embedded by `crates/desigma-app/build.rs`) |
| `build` `x86_64-unknown-linux-gnu` | `ubuntu-latest` | `desigma-<version>-<target>.tar.gz` (adds `desigma.desktop`, `icons/`, `install.sh`) |
| `macos` | `macos-14` (builds `aarch64-apple-darwin` and cross-compiles `x86_64-apple-darwin`; `macos-13` is retired) | `desigma-<version>-macos.dmg` and `desigma-<version>-macos-universal.tar.gz` |

The Windows and Linux archives hold `desigma`, `desigma-diff`, `LICENSE-MIT`, `LICENSE-APACHE` and `README.md`. They are packaged by `scripts/package.sh <target> <version>`, which you can run locally after a release build.

The macOS job runs `scripts/package-macos.sh <version>`. It `lipo`s a universal `desigma` and `desigma-diff`, assembles `desigma.app` (bundle id `ai.deepfounder.desigma`, `Info.plist` from `packaging/macos/Info.plist.in`, `desigma.icns` built with `iconutil` from `assets/icon/desigma.iconset`), ad-hoc signs it, and writes the `.dmg` (app, `Applications` link, `Read me first.txt`) and the universal `.tar.gz` (app, licenses, README). It needs a Mac, so test changes to it with a dry run (below).

The release carries a combined `SHA256SUMS` covering every file (verify with `sha256sum -c SHA256SUMS`).

## App icons

The masters are `assets/icon/desigma-icon.svg` (full bleed, Windows and Linux) and `assets/icon/desigma-icon-macos.svg` (824 px tile on a 1024 canvas). All generated files are committed; regenerate them after editing a master with:

```sh
cargo run -p desigma-app --example gen_icons
```

This rewrites `assets/icon/png/`, `desigma.ico`, `desigma.iconset/` and `desigma.icns`. The window icon is the embedded 256 px PNG.

## Dry run

Run the workflow from the Actions tab (Release, Run workflow) on any branch. It builds and packages everything and uploads the archives as workflow artifacts, but creates no release. Run it after any change to `scripts/package-macos.sh` or the macOS plist, then open the downloaded `.dmg` on a Mac to check the icon, the drag-to-Applications layout and the Open Anyway flow. The version is stamped `0.0.0-dev.<sha>`.

## Update notifications

Release builds compare their stamped version with GitHub's release list (`/repos/deepfounder-ai/desigma/releases`; pre-releases are offered only to pre-release builds) and show a banner when a newer one is out. To preview the banner and dialog offline, stamp a build and point `DESIGMA_UPDATE_URL` at a file (or an `http://127.0.0.1` URL) in GitHub's release-list shape with a newer tag:

```sh
DESIGMA_VERSION=0.1.0-alpha.7 cargo build -p desigma-app
DESIGMA_UPDATE_URL=releases.json target/debug/desigma --fixture showcase --panel updater --screenshot update.png
```

`--panel updater-banner` shows the banner instead of the dialog.

## Not covered yet

- macOS: the app is ad-hoc signed but not signed with a Developer ID or notarized, so Gatekeeper asks for a one-time Open Anyway. "Open With" lists desigma through `CFBundleDocumentTypes`, and a double click in Finder opens the file in the running editor.
- Windows binaries are unsigned (SmartScreen warning). There is no installer; the zip is the download.
- Linux binaries link the build runner's glibc; they run on that distribution or newer.
