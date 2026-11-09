# fusion-desigma shell layer

fusion-desigma is the customer self-design GUI entry of the fusion pipeline
(companion: fusion-design is the AI-generates path). The core stays upstream:
this repository tracks [deepfounder-ai/desigma](https://github.com/deepfounder-ai/desigma)
and never modifies `crates/`, `packaging/` or `assets/`. See
[upstream-sync.md](upstream-sync.md) for the sync policy and the list of
shell-layer files.

## What the shell layer does

| Concern | Mechanism | File |
|---|---|---|
| Upstream attribution | NOTICE with MIT / Apache-2.0 and font licenses | `NOTICE` |
| Branding | bundle display name rewritten at package time (`CFBundleName` / `CFBundleDisplayName` → `fusion-desigma`); bundle id and executable stay upstream (pinned by upstream tests, keeps sync conflict-free) | `scripts/package-fusion-macos.sh` |
| macOS packaging | wrapper around the upstream packaging script; emits `dist/fusion-desigma-<version>-macos.dmg` and `-universal.tar.gz` | `scripts/package-fusion-macos.sh` |
| Default config | `~/.fusion-desigma/{cache,data,recent}`; the launcher symlinks the macOS app dirs (`~/Library/Caches/desigma`, `~/Library/Application Support/desigma`) into it on first run | `bin/fusion-desigma` |
| Service lifecycle | supervisor-compatible `start.sh` (`start\|stop\|restart\|status\|log`, pidfile `.fusion-desigma.pid`, log `logs/fusion-desigma.log`) | `start.sh` |
| Sync discipline | weekly + on-demand fast-forward-only merges, pinned upstream commit per release | `docs/upstream-sync.md` |

## Usage

```sh
cargo build --release -p desigma-app        # upstream build, unchanged
bin/fusion-desigma                          # launch with fusion defaults
./start.sh status                           # supervisor lifecycle face
scripts/package-fusion-macos.sh 0.1.0-alpha.17   # branded DMG + tar.gz into dist/
```

## fusion-supervisor integration

`start.sh` follows the monorepo lifecycle convention, so the supervisor's
default start path (`bash $FUSION_ROOT/fusion-desigma/start.sh`) works without
a `start_map` override. Registration (port-registry entry, tier pinning,
health probe = process liveness, no HTTP face) is tracked in the
fusion-supervisor repository; the shell side makes no assumptions beyond the
script contract.

## What stays upstream on purpose

- **Bundle id / executable name** (`ai.deepfounder.desigma`, `desigma`):
  pinned by upstream tests in `crates/` and referenced by upstream scripts;
  renaming them would fork the identity and break every sync.
- **Updater, plugins, MCP**: unchanged upstream behavior; the app self-reports
  as desigma internally. Product naming for users comes from the bundle
  display name and the launcher.
