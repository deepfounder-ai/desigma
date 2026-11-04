# desigma marketing kit

Images, sources and copy for announcing desigma. Everything here is free to use
for writing about, linking to or promoting desigma.

## Images

| File | Size | Use it for |
|---|---|---|
| [`png/launch-banner-1600x900.png`](png/launch-banner-1600x900.png) | 1600×900 | Release posts, blog headers, X/LinkedIn link images |
| [`png/github-social-preview-1280x640.png`](png/github-social-preview-1280x640.png) | 1280×640 | GitHub repo social preview (Settings → General → Social preview), Open Graph |
| [`png/square-post-1080x1080.png`](png/square-post-1080x1080.png) | 1080×1080 | Instagram, LinkedIn, Mastodon, Bluesky |
| [`png/story-1080x1920.png`](png/story-1080x1920.png) | 1080×1920 | Instagram/LinkedIn stories, TikTok/Shorts covers |
| [`png/carousel-board-mode-1080x1350.png`](png/carousel-board-mode-1080x1350.png) | 1080×1350 | Carousel slide: Board mode |
| [`png/carousel-variables-1080x1350.png`](png/carousel-variables-1080x1350.png) | 1080×1350 | Carousel slide: Variables |
| [`png/stars-1000-linkedin-1200x627.png`](png/stars-1000-linkedin-1200x627.png) | 1200×627 | 1,000 stars: LinkedIn link image |
| [`png/stars-1000-linkedin-square-1080x1080.png`](png/stars-1000-linkedin-square-1080x1080.png) | 1080×1080 | 1,000 stars: LinkedIn square |
| [`png/stars-1000-linkedin-portrait-1080x1350.png`](png/stars-1000-linkedin-portrait-1080x1350.png) | 1080×1350 | 1,000 stars: LinkedIn portrait |
| [`png/stars-1000-linkedin-cover-1584x396.png`](png/stars-1000-linkedin-cover-1584x396.png) | 1584×396 | 1,000 stars: LinkedIn page cover |
| [`png/stars-1000-threads-1-star-1080x1350.png`](png/stars-1000-threads-1-star-1080x1350.png) | 1080×1350 | 1,000 stars: Threads carousel 1/3 |
| [`png/stars-1000-threads-2-thanks-1080x1350.png`](png/stars-1000-threads-2-thanks-1080x1350.png) | 1080×1350 | 1,000 stars: Threads carousel 2/3 |
| [`png/stars-1000-threads-3-shipped-1080x1350.png`](png/stars-1000-threads-3-shipped-1080x1350.png) | 1080×1350 | 1,000 stars: Threads carousel 3/3 |
| [`png/stars-1000-threads-stickies-1080x1350.png`](png/stars-1000-threads-stickies-1080x1350.png) | 1080×1350 | 1,000 stars: Threads single (stickies) |
| [`png/stars-1000-threads-terminal-1080x1350.png`](png/stars-1000-threads-terminal-1080x1350.png) | 1080×1350 | 1,000 stars: Threads single (terminal) |

Product screenshots for articles are in [`../docs/images/`](../docs/images/), all
captured from the real app with its `--screenshot` mode.

## Sources

[`source/`](source/) holds each image as a standalone HTML page sized to the
exact pixel dimensions. Edit the copy or colors there, then re-render with any
Chromium browser. From the repository root, with Chrome or Edge:

```sh
for f in marketing/source/*.html; do
  stem=$(basename "$f" .html); size=${stem##*-}; w=${size%x*}; h=${size#*x}
  chrome --headless=new --hide-scrollbars --force-device-scale-factor=1 \
    --virtual-time-budget=8000 --window-size=$w,$h \
    --screenshot="marketing/png/$stem.png" "file://$PWD/$f"
done
```

The pages load Bricolage Grotesque and IBM Plex Mono from Google Fonts, so
render with a network connection. Use `--force-device-scale-factor=2` for
retina-size exports.

## Brand

- **Name:** desigma, always lowercase, including at the start of a sentence.
- **Logo mark:** [`../docs/brand/desigma-mark.svg`](../docs/brand/desigma-mark.svg). Keep it square with clear space of
  at least a quarter of its width; don't recolor the inner shapes, rotate or
  add effects. On a blue background, use the inverted version (white square,
  blue shapes) as on the GitHub preview.
- **Colors:**

| Role | Hex |
|---|---|
| desigma blue | `#0C8CE9` |
| Deep blue (small text on light, badges) | `#0A5FA8` |
| Ink | `#0E0F12` |
| Canvas light | `#ECEBE6` |
| Panel dark | `#2C2C2C` |
| Instance purple | `#9747FF` |
| Sticky yellow | `#FFD66B` |

- **Type:** Bricolage Grotesque (headlines, 800; body, 400–600) and IBM Plex
  Mono (labels, measurements, URLs). Both are free under the OFL.
- **Visual language:** the promo images are drawn as if on desigma's own canvas:
  dotted ground, rulers, selection boxes with handles and size badges,
  component-instance outlines, prototype connectors, floating panels and
  cursors. Reuse those elements rather than gradients or stock imagery.

## Copy

Use these as-is or adapt them. Only state facts that are true of the current
release; check [the README](../README.md) before quoting feature lists.

**Taglines**

- The design tool you actually own.
- No plans. No seats. Just design.
- Design freely.
- One switch. Every screen. *(variables)*
- A whiteboard, in the same app. *(Board mode)*

**One line**

> desigma is a free, open-source design tool built in pure Rust: native, offline, and able to open your .fig files.

**Short description**

> desigma is a free, open-source design tool for Windows, macOS and Linux, written in pure Rust. It has the everyday design surface you expect (frames, auto layout, components and variants, variables with modes, prototyping, Dev mode code generation and whiteboards) and imports Figma's .fig and REST JSON files. No account, no subscription, and your files stay on your machine.

**Release post** *(edit the version and highlights)*

> desigma v0.1.0-alpha.3 is out. New in this release: a GPU canvas, real text lists and OpenType features, vector networks and a Shape Builder, much better .fig import, variables and conditions in prototypes, Tailwind/SwiftUI/Compose code in Dev mode, and a whiteboard mode. Free and open source, for Windows, macOS and Linux: https://github.com/deepfounder-ai/desigma/releases

**Short social post**

> We're building a design tool you actually own. desigma is free, open source, offline and written in Rust, and it opens .fig files. Alpha builds for Windows, macOS and Linux: https://github.com/deepfounder-ai/desigma

**Before you post**

- desigma is early alpha. Say so, and link to the release notes rather than
  promising parity with commercial tools.
- Release builds aren't code-signed yet; point new users to
  [docs/install.md](../docs/install.md) for the first-launch warning.
- Figma is a trademark of Figma, Inc. Mention it only to describe file
  compatibility ("opens .fig files"), and keep the line
  "desigma is not affiliated with Figma, Inc." on any material that names it.
