# Driving desigma from an agent

Notes from a team that keeps its design source in a workspace directory and edits it over
the [MCP server](../crates/desigma-mcp/README.md) instead of by hand: one Flutter client,
one Vue admin, one design library, an agent in the loop. It is a starting point, not a
specification — the shapes below are the ones that survived contact with real work.

A generated example is in [`examples/button-kit.desigma`](examples/button-kit.desigma): one
page, a main component, two instances, and an override on the second.

## Session model, in one paragraph

`desigma-mcp --workspace DIR` either attaches to a running editor whose agent bridge is on
(`"session": "editor"`) or works on an isolated file session (`"session": "file"`). The
first call picks the mode and the server keeps it. Editor mode shares the user's undo stack
and lays edits out like the editor; file mode is a batch workspace where nothing reaches
disk until `desigma_save_file`. Agents should treat editor mode as "someone is watching"
and file mode as "prepare a revision, then hand it back".

## Workspace conventions

- **One workspace, inside the repository.** `designs/` at the repo root, passed as
  `--workspace designs`. Every path in every tool call is relative to it; absolute paths are
  only accepted inside that tree, and reads canonicalize before they check. Keeping it in
  the repository means every checkout, worktree and CI job sees the same directory.
- **One file per concern.** `designs/buttons/button-kit.desigma`,
  `designs/screens/sign-in.desigma`. A file is a document; a screen-splitting file costs a
  re-open for every change.
- **Edit the file you opened, do not rebuild it.** Node IDs are per document and stay stable
  while the file is edited in place, so the mapping "layer → widget token" that agents keep
  in their head survives. Recreating a document every run throws that away.
- **Never cache IDs across runs.** `desigma_get_document` → `desigma_list_nodes` →
  `desigma_get_node` at the start of every session; IDs from a previous session mean nothing.
- **Revisions get new names, or an explicit overwrite.** `desigma_save_file` refuses an
  existing path unless `overwrite: true`. Agents save `sign-in.v2.desigma` while iterating
  and overwrite the reviewed file once a human has looked at the export.
- **Exports live in the same workspace** (`designs/exports/`) because export paths are
  validated the same way. Scale 2 PNGs are small enough to commit and clear enough to review.
- **Unsaved work is the exception, not the state.** A file session refuses
  `desigma_open_file` while it has unsaved edits; ending a run without `desigma_save_file`
  loses it. Make saving the last step of every script.

## The loop we run

1. `desigma_list_files` → `desigma_open_file` (or `desigma_new_document`).
2. `desigma_get_document` for pages and `session`, then `desigma_list_nodes`
   (`query` for search, `parent_id` for direct children) and `desigma_get_node` for the
   layers that will change. Read before writing: the reply is the only ground truth.
3. Plan the edits, then send them as one sequence of calls. Each mutating call is one undo
   step, so a whole revision can be walked back with repeated `desigma_undo`.
4. Verify with `desigma_export_image` (does it look right?) and/or `desigma_generate_code`
   (is the structure right?). Read the PNG; do not assume.
5. `desigma_save_file`.

Anything that runs long is cheaper as one review at the end: export the frame, look at it,
fix, export again, and only then save.

## `generate_code` or the node tree?

Both read the same document, and picking the wrong one wastes an iteration. The short
version: `generate_code` answers "what should this look like", the node tree answers "what
does this project already have".

Use `desigma_generate_code` when:

- the layer is finished and the question is handoff — `css` and `tailwind` for the admin,
  `compose` and `swiftui` for the clients, `svg` for icons;
- you want a diffable second opinion on geometry, spacing and typography;
- a human will read the output.

Read the node tree and write code yourself when:

- the target is an existing component with its own API. A `Button / Primary` layer is a
  `PrimaryButton(size: md)`, not a `Container(color: 0xFF2563EB)`; only the node name,
  geometry and variable bindings carry that mapping, and the generator does not know the
  component library;
- the work spans several layers or a whole screen — the generators emit one layer;
- layout semantics matter: whether a frame uses auto layout, what `layout_child` asks for,
  which node constrains which. Generated code flattens that into absolute numbers;
- you need to compare before and after an edit, or find the layer to change.

A practical split: read the tree to decide *what* to build, generate code for the layer that
does not exist in the codebase yet, and hand-write the parts that plug into existing
components and theme tokens.

## Components

- A family of variants is one main component plus instances, not copies. Converting a frame
  with `desigma_create_component` keeps its children and its name.
- Change an instance's own properties instead of detaching: a fill, opacity or text change on
  a layer inside an instance is recorded as an override on that layer
  (`component.overrides` lists them), and a sync keeps it.
- A change to the main component reaches its instances in the same undo step, so an agent
  that edits a main does not have to sync by hand. `desigma_sync_instances` is for the case
  where overrides must be reconciled explicitly; on a component nobody instanced it reports
  `{"changed": false}` and pushes no undo step.
- Detach (`desigma_detach_instance`) when the divergence is real, not to get one change
  through.

## Guardrails

- In editor mode the document is the user's. Do not pass `discard_changes: true` to
  `desigma_open_file` outside a throwaway file; ask, or work in the file session.
- Export a PNG before and after a bulk edit. "It returned 200" is not evidence that the
  screen looks right.
- One concern per run. A run that renames layers, changes colours and restructures a page in
  one go is unreviewable and its undo trail is useless.
- Names are the API between the design and the code: `Button / Primary`, `Label`,
  `Toolbar`. Agents and humans both search by them.
