# Desigma MCP

A local MCP server for inspecting and editing design files with Desigma's document model and code generators. By default it runs an isolated, in-memory file session over stdio. When the editor's agent bridge is on, it attaches to the running editor instead and edits the document open on screen; see [Live editor control](#live-editor-control).

## Build and connect

From the repository root:

```sh
cargo build --locked -p desigma-mcp
```

The executable is `target/debug/desigma-mcp` (`desigma-mcp.exe` on Windows). Use `--release` for a release build and change `debug` to `release` in the paths below. Rust and the repository's normal build prerequisites are required only for building.

Configure an MCP client that supports local stdio servers. These are generic `mcpServers` examples; use the configuration format and location required by your client. Replace both paths with absolute paths on your machine. The workspace directory must already exist.

Windows:

```json
{
  "mcpServers": {
    "desigma": {
      "command": "C:\\path\\to\\desigma\\target\\debug\\desigma-mcp.exe",
      "args": ["--workspace", "C:\\path\\to\\designs"]
    }
  }
}
```

macOS/Linux:

```json
{
  "mcpServers": {
    "desigma": {
      "command": "/path/to/desigma/target/debug/desigma-mcp",
      "args": ["--workspace", "/path/to/designs"]
    }
  }
}
```

The client launches the process and speaks MCP over stdin/stdout. The server does not listen on a network port. No account, API key, Figma connection, or running editor is needed; a running editor is used when available. Configuration is not installed automatically.

## File workflow

1. Call `desigma_open_file` with a path inside the configured workspace, or `desigma_new_document` with a name.
2. Use `desigma_get_document` to discover page IDs, then `desigma_list_nodes` and `desigma_get_node` to inspect layers. IDs are positive integers; obtain them from tool results rather than Figma node IDs.
3. Inspect variables or generate handoff code. Make edits with the creation, update, and deletion tools; undo and redo operate on this session's edit history.
4. Call `desigma_save_file` with an unused `.desigma` filename, or pass `"overwrite": true` to replace an existing one (for example the `path` that `desigma_get_document` reports for the file you opened). Its parent directory must already exist. Open the resulting file in the desktop editor to continue there.

Example tool arguments:

```text
desigma_open_file     {"path":"input.fig"}
desigma_get_document  {}
desigma_list_nodes    {"query":"button","limit":25}
desigma_get_node      {"node_id":42}
desigma_generate_code {"node_id":42,"language":"react"}
desigma_update_node   {"node_id":42,"name":"Primary action","width":160}
desigma_save_file     {"path":"output-v2.desigma"}
```

`42` is illustrative: substitute an ID returned by the preceding queries. Relative paths resolve beneath `--workspace`. Absolute paths are accepted only within that same directory tree. Reads canonicalize paths and reject escapes; writes resolve the existing parent. `desigma_save_file` creates a new file unless `overwrite: true` is passed, in which case it replaces an existing regular `.desigma` file atomically (temporary file + rename) and still never writes through a symlink; `desigma_export_image` overwrites an existing regular `.png` but never writes through a symlink.

Edits remain in memory until saved. Restarting the server loses unsaved work. Opening another file or starting a new document resets undo history and refuses to discard pending edits unless `discard_changes: true` is explicitly supplied. Save each revision under a new filename, or overwrite the file you opened with `overwrite: true`. Requests are handled one at a time in the order they arrive, so a batch of calls written in one go runs and answers in sequence. Tool failures return an MCP error result with an explanatory message.

Imports are limited to 64 MiB per file, 100,000 layers, and 256 levels of nesting. The server validates the imported document graph before using it, rejecting malformed parent/child relationships and cycles.

## Tools

| Tool | Purpose |
| --- | --- |
| `desigma_list_files` | Design files in this user's library, plus `shared/` copies of files opened from outside it while the agent bridge is on. Each `path` is valid for `desigma_open_file`. |
| `desigma_open_file` | Import a native `.desigma`, `.fig`, `.pen` (Pencil), SVG, or supported JSON design into the session. |
| `desigma_new_document` | Start an empty document with one page. |
| `desigma_save_file` | Write the session to a native `.desigma` file: a new one, or an existing one with `overwrite: true`. |
| `desigma_get_document` | Read document name, pages, counts, undo/redo availability, and `session` (`"file"` or `"editor"`). |
| `desigma_get_tree` | Compact layer tree below `node_id` (or all pages), `depth` levels (default 3): id, name, kind, x/y/w/h, a paint summary (`fill`, `stroke`, `text`), `instance_of`, `layout`, bound variables, children; deeper layers show `child_count`. |
| `desigma_get_assets` | Components, styles, variable collections and variables (value per mode name) in one list; `query` filters by name. Read it before building. |
| `desigma_list_nodes` | Search names case-insensitively or list direct children; paginate using `next_offset`. |
| `desigma_get_node` | Read a complete native node, including child IDs, paints, geometry, text, and bindings. |
| `desigma_get_variables` | Read collections and tokens; optionally resolve one node's direct bindings in its inherited modes. |
| `desigma_generate_code` | Return CSS, Tailwind, React, SwiftUI, Compose, or SVG source without writing a file. |
| `desigma_apply_nodes` | Create or update a whole node tree from JSON in one transaction and one undo step; see [Building whole screens](#building-whole-screens). |
| `desigma_create_component` | Turn a layer into a main component: frames, groups and sections in place, other layers wrapped in a component frame. Returns `component_id`; see [Components](#components). |
| `desigma_create_instance` | Create an instance of a component under `parent_id` at `x`/`y`, with `overrides` for text, fill, visibility and swap. One undo step; returns `instance_id`. |
| `desigma_override_instance` | Change layers inside an existing instance by layer name; stored as overrides so they survive component updates. |
| `desigma_set_variables` | Create or update variable collections, modes and values by name (colour, number, string, boolean) in one undo step; repeatable. See [Variables](#variables). |
| `desigma_bind_variable` | Bind a variable (by name) to a layer field such as `fill`, `stroke`, `gap`, `padding`, `radius`, `width`, `height`; `null` unbinds. |
| `desigma_create_node` | Add a frame, rectangle, ellipse, or text layer to a container. |
| `desigma_update_node` | Change name, position, dimensions, visibility, opacity, or plain text. |
| `desigma_delete_node` | Remove a non-page layer and its descendants. |
| `desigma_undo` / `desigma_redo` | Traverse the edit history. In the editor it is the user's own undo stack, so `desigma_undo` may undo the user's latest edit. |
| `desigma_export_image` | Render a node to a PNG at `scale` (0.25–4, default 1) and write it to `path` inside the workspace, overwriting if present; returns `{path,width,height}`. Works in both session types. |
| `desigma_screenshot` | Render a node to PNG at `scale` (0.25-4) without choosing a file: the PNG goes to `.desigma-screenshots/` in the workspace (50 rotating names) unless `path` is given; returns `{path,width,height,scale,node_id}`. Use it after every edit to check the result. Works in both session types. |
| `desigma_get_selection` | Read the editor's current selection. Editor only; fails in a file session. |
| `desigma_select_nodes` | Select 1–500 `node_ids` on one page in the editor (switching to that page) and, with `reveal` (default `true`), scroll/zoom to them. Editor only; fails in a file session. |

`desigma_list_nodes` defaults to 100 results and accepts 1–500 per call. `parent_id` filters direct children, not all descendants. Coordinates are relative to the parent. Width and height must be non-negative, geometry must be finite and within ±1,000,000, and opacity must be between 0 and 1. Replacing text clears rich-text runs and list ranges. Page roots cannot be edited or deleted by the layer tools.

## Building whole screens

`desigma_apply_nodes` builds or edits a complete tree in one call. Use it instead of chains of `create_node`, `update_node` and `set_appearance`: the whole call is one transaction, one undo step, and it is checked with `validate_document` before it is kept.

```json
{"parent_id": 1, "nodes": [{
  "kind": "frame", "name": "Card", "x": 40, "y": 60, "width": 320, "height": "hug",
  "clips_content": true, "corner_radius": [24, 24, 8, 8],
  "fill": {"type": "linear", "angle": 180, "stops": [{"pos": 0, "color": "#ff3366"}, {"pos": 1, "color": "#3366ffcc"}]},
  "effects": [{"type": "drop_shadow", "y": 8, "blur": 24}, {"type": "background_blur", "blur": 20}],
  "auto_layout": {"direction": "vertical", "gap": 12, "padding": [16, 20]},
  "children": [
    {"kind": "text", "name": "Title", "text": "Hello", "width": "fill",
     "text_style": {"font_size": 24, "font_weight": 700, "line_height": "120%", "color": "#111111"}},
    {"kind": "rectangle", "name": "Bar", "width": "fill", "height": 40, "rotation": 15, "blend": "multiply"}
  ]
}]}
```

- **Create or update.** A spec with `kind` and a parent (`parent_id` on the call or on the spec) creates a node. A spec with `node_id` updates that node and changes only the fields you pass. Under an updated node, a child with `node_id` is updated and a child without one is created and appended. `kind` cannot be changed, and `node_id` cannot be combined with `parent_id` (use `desigma_move_node`).
- **Kinds.** `frame`, `group`, `rectangle`, `ellipse`, `text`, `line`, `polygon` and `star` (`sides`, `inner_ratio`). Sizes are pixels, `"hug"` (auto-layout frame or text) or `"fill"` (stretch inside an auto-layout parent). A pixel width on a text node makes it wrap (`text_resize` overrides).
- **Paints** (`fill`, `stroke.color`, `text_style.color`): `"#RRGGBB[AA]"`, `"none"`, a paint object or an array of them, bottom first. Objects: `solid`, `linear`, `radial`, `angular`, `diamond` (with `stops` and an optional `angle` in degrees, 0 up and clockwise, or raw `handles`), and `image` (`image_ref` of an image already in the document, or `image_base64` of a PNG, JPEG, GIF or WebP up to 16 MiB each and 64 MiB of new images per call; `scale_mode` fill, fit, crop or tile). Every paint takes `opacity`, `blend` and `visible`.
- **Effects** replace the node's effect list: `drop_shadow`, `inner_shadow` (`color`, `x`, `y`, `blur`, `spread`, `show_behind` for the drop shadow), `layer_blur`, `background_blur` (`blur`) and `glass` (`light_angle`, `light_intensity`, `refraction`, `depth`, `dispersion`, `frost`).
- **Frames.** `clips_content`, `auto_layout` (`direction`, `gap`, `counter_gap`, `wrap`, `padding` as a number, `[vertical, horizontal]`, `[top, right, bottom, left]` or an object, `primary_align`, `counter_align`, `reverse_z`, `stroke_included`; `null` removes it), `layout_child` (`absolute`, `grow`, `min_width`, `max_width`, `min_height`, `max_height`) and `constraints` (`min`/`max`/`center`/`stretch`/`scale`, with `left`, `right`, `top`, `bottom` as aliases).
- **Also.** `corner_radius` (one number or four, clockwise from top left), `corner_smoothing`, `rotation` (degrees around the top-left corner), `blend`, `opacity`, `visible`, `locked`, `is_mask`, and `stroke` (`color`, `width`, `align`, `cap`, `join`, `dashes`, `side_weights`). Text styles take `font_family`, `font_weight`, `italic`, `font_size`, `line_height` (pixels, `"auto"` or `"120%"`), `letter_spacing`, `paragraph_spacing`, `align_h`, `align_v`, `case`, `decoration`, `color` and OpenType `features`.
- **Layout is solved for you.** Auto-layout frames touched by the call are laid out, and text boxes are measured, before the result is stored, so file sessions get the same positions the editor would compute. Siblings that the layout moves count in `updated`.
- **Errors come back per node and nothing changes.** Every problem in the tree is listed with its path and the node's name, for example `nodes[0].children[2] "Badge": fill: stop pos 2 is outside 0..=1`. Unknown fields are errors, so a typo never silently does nothing. Pass `validate_only: true` to check a tree without applying it.
- **Limits.** 5000 specs per call, 64 nesting levels. Components, instances, variables bindings, vector paths and noise or texture effects are not covered by this tool yet.
- **Speed.** A 300-node screen (49 auto-layout rows with gradients, shadows and text) builds in about 70 ms in a release build (0.8 s unoptimized), including layout and validation.

Two things worth knowing when editing a screen that already exists. Laying out a container that changed also re-solves every auto-layout ancestor above it, so siblings move and hugging parents grow or shrink. And an update with `"replace_children": true` treats `children` as the complete new list: entries with `node_id` keep and reorder that child, entries without are created, children not listed are deleted (`[]` removes all), all in the same single undo step.

## Components

`desigma_create_component {node_id}` makes a main component and returns `component_id`. `desigma_create_instance {component_id, parent_id, x, y, name?, overrides?}` places an instance (`origin` at x/y in the parent). `overrides` maps a layer name inside the component to the changes made there; the first layer with that name, depth first, is used and an unknown name lists the available ones:

```json
{"component_id": 12, "parent_id": 3, "x": 40, "y": 40,
 "overrides": {"Label": {"text": "Buy now", "fill": "#ff3366"},
               "Badge": {"visible": false},
               "Icon":  {"swap": 31}}}
```

An override takes the same properties as a `desigma_apply_nodes` spec (`text`, `fill`, `stroke`, `visible`, `opacity`, `text_style`, ...) plus `swap` (a component id, for nested instances). Every change is recorded as an override, so syncing the component later does not undo it. In `desigma_apply_nodes` a spec may carry `"instance_of": <component id>` with the same `overrides` instead of `kind`/`children`, so a whole screen can mix plain layers and instances; the auto layout is solved around them. `desigma_override_instance {instance_id, overrides}` edits an instance that already exists; swaps run first as their own undo step. Updating a layer inside an instance with `desigma_apply_nodes {node_id}` records overrides in the same way.

## Variables

`desigma_set_variables` upserts by name, so an agent can state the tokens it needs and repeat the call safely:

```json
{"collections": [{"name": "Brand", "modes": ["Light", "Dark"], "variables": [
  {"name": "Color/Primary", "type": "color", "values": {"Light": "#3366ff", "Dark": "#88aaff"}},
  {"name": "Space/M", "type": "number", "value": 16}]}]}
```

`value` sets every mode, `values` sets modes by name. A new collection's first mode is named after `modes[0]`. A call with any problem changes nothing; a call that changes nothing returns `changed:false`. Opacity variables hold percent (0-100), as in Figma.

Bind while building: a `desigma_apply_nodes` spec may carry `"bind": {"fill": "Color/Primary", "gap": "Space/M", "padding": "Space/M", "radius": "Radius/L"}`. Fields: `fill`, `stroke`, `stroke_width`, `gap`, `counter_gap`, `padding` (+ `_top/_right/_bottom/_left`), `radius`, `width`, `height`, `size`, `opacity`, `visible`, `text`, `font_size`, `font_weight`, `line_height`, `letter_spacing`. The value must match the variable's type and scopes; the variable's default-mode value is written into the layer so the layout sees it. `desigma_bind_variable {node_id, bindings}` does the same for an existing layer and `null` unbinds.

## Live editor control

When Desigma runs with its agent bridge on, `desigma-mcp` attaches to it automatically: the editor writes `bridge.json` (port and a per-launch token) into the library folder next to `settings.json` (`$DESIGMA_LIBRARY` honored if set), the server pings it before each call and forwards the call over loopback HTTP. `desigma_get_document` then reports `"session": "editor"` (a file session reports `"file"`) plus the open file's `path` and `has_undo`, which is true while the editor's undo stack is non-empty, including after the user saves. Edits go through the editor's undo stack, so ⌘Z in the editor and `desigma_undo` see the same history, and `desigma_undo` may undo the user's latest edit. Mutating calls are refused while a drag, pen or vector edit, dialog, or presentation is in progress ("Editor is busy"); an open text edit is committed first. A call that takes the editor longer than 5 s fails, but may still have been applied: read the document before retrying.

The first call picks the session mode and the server keeps it. In editor mode, if the bridge goes away later, calls fail with "Editor bridge is off" instead of falling back to a file session; turn the bridge on again or restart `desigma-mcp`. In file mode, an editor that appears later is used from the next call when the file session has no unsaved edits; with unsaved edits, calls fail with "An editor appeared" until you save them with `desigma_save_file` or restart `desigma-mcp`. `desigma_get_selection` and `desigma_select_nodes` fail in a file session with "No editor connected".

In this mode `desigma_open_file` opens the file in the editor (refused while the editor has undo history: ask the user to save first; `discard_changes` throws their edits away), `desigma_save_file` writes a copy to a new `.desigma` file without changing the editor's own path, and `desigma_new_document` starts a new document in the editor. Workspace rules still apply to every path. Set `DESIGMA_BRIDGE_FILE` to point the server at a different discovery file, or to an empty value to disable discovery entirely (used by tests).

Turn the bridge on in **Preferences → Allow MCP agents to control this editor** (remembered) or with `desigma --bridge` (this launch only). A status pill top-right shows `"Agent bridge on"` or `"Agent · <last tool>"` while it runs, with an **Off** button. The bridge never starts for `--screenshot` runs, and a second editor instance refuses to start one. It listens on 127.0.0.1 only, uses a per-launch token in the URL path and the `X-Desigma-Token` header, rejects any request carrying an `Origin` header (browsers), and is removed with the pill's **Off** button or when the editor exits.

## Current limits

The server shares Desigma's importers, document types, variables, undo commands, and exporters. Code generation resolves bound variable values. Import and generated-code fidelity follow the underlying Desigma implementations; generated source is a handoff starting point.

In an isolated file session, layer edits do not run the desktop application's auto-layout reflow, text measurement, connector rerouting, or component-instance synchronization; do not assume that resizing a container rearranges its children or that editing a component updates every instance. With the agent bridge on, edits run through the editor's transactions and are laid out like the editor's own edits (auto-layout reflow, group fit, connector rerouting, component-instance sync). Editing token definitions or arbitrary paint properties is not exposed in this version.

## Verification and evaluation

```sh
cargo test --locked -p desigma-mcp
```

`tests/evaluation.xml` contains ten independent, read-only retrieval questions based on the repository's fixed Figma REST fixture. For each question, start a fresh server with the repository root as `--workspace`, then preload `crates/desigma-fig/tests/fixtures/rest_file.json` using `desigma_open_file`. This preload is test setup; answering the questions requires only the read-only query/code tools. Do not create, update, delete, undo, redo, or save during evaluation. Match the response against the XML answer exactly.

For a short workflow guide for agents (read assets, build, screenshot, fix) with a complete example, see [docs/agents.md](../../docs/agents.md).
