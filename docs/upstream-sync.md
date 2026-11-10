# Upstream sync discipline

fusion-desigma is a shell-layer fork of [deepfounder-ai/desigma](https://github.com/deepfounder-ai/desigma).
The heart stays upstream; this repository only maintains the fusion shell
(branding, packaging, defaults, service integration).

## Policy

- **Cadence**: weekly check, plus on-demand (an upstream fix or feature we need).
- **Upstream remote** is wired as `upstream`:

  ```sh
  git remote add upstream https://github.com/deepfounder-ai/desigma.git
  git fetch upstream
  ```

- **Fast-forward only.** `main` must always be a fast-forward of
  `upstream/main`. Never rewrite or cherry-pick over upstream history.

  ```sh
  git checkout main
  git merge --ff-only upstream/main
  git push origin main
  ```

- **Pin**: every release tag records the upstream commit it carries
  (`crates/` are byte-identical with `upstream/main` at that commit).

## Carried local patches

Upstream files are **never** modified inside `crates/`. All fusion adaptation
lives in shell-layer files that upstream does not have, so merges stay
conflict-free by construction:

| Path | Purpose |
|---|---|
| `NOTICE` | upstream attribution (MIT / Apache-2.0, Inter font) |
| `start.sh` | supervisor-compatible lifecycle script (start/stop/restart/status/log) |
| `bin/fusion-desigma` | launcher: fusion defaults, `~/.fusion-desigma` layout |
| `scripts/package-fusion-macos.sh` | branded macOS packaging wrapper |
| `.github/workflows/release.yml` | macOS job calls the branded wrapper instead of the upstream packaging script (single-line carried patch; everything else matches upstream) |
| `docs/fusion-shell.md` | shell-layer architecture and integration notes |
| `docs/upstream-sync.md` | this policy |

If a future change seems to require editing an upstream file, stop and design
a shell-layer alternative (wrapper, env var, symlink, post-package step). If
truly impossible, upstream the change first (file an issue / PR against
deepfounder-ai/desigma) and sync it down.

## Breaking changes in alpha

desigma is early alpha; the document model and CLI surface can change between
alpha tags. Sync checklist:

1. Read the upstream release notes / commit log since the pinned commit.
2. Run the full gate after every sync: `cargo test --workspace --locked`,
   `cargo clippy --workspace --all-targets --locked -- -D warnings`.
3. Re-verify the shell scripts still match upstream reality (binary names,
   packaging artifact names, CLI flags used by `start.sh` and
   `scripts/package-fusion-macos.sh`).
4. Tag a new `-fusion.N` release (see "Versioning").

## Versioning

Fork releases are tagged `<upstream-tag>-fusion.<n>`, e.g.
`v0.1.0-alpha.17-fusion.1` — the upstream base is visible in the tag itself.
The `Cargo.toml` version stays upstream-owned and is not bumped in this fork.
