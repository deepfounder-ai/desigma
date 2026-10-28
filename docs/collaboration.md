# Live collaboration — design and plan

Status: phases 1–5 done: core sync, transport, app integration, presence, and polish (reconnect, fonts, image cache, leave question, headless server, links on every OS). Target: 2–10 people editing one file live, on Windows, macOS and Linux,
with no account and no server to run.

## How it works for users

1. The host opens a file and clicks **Share**. Desigma shows a link
   `desigma://join/<ticket>` and, optionally, a password. The host picks whether guests can
   edit or only view, and whether each joiner needs approval.
2. A guest clicks the link (or pastes it into **Join session** on the home screen), enters a
   name and the password, and the file opens with everyone's cursors on it.
3. The session lasts while the host keeps the file open. When it ends, guests keep a local
   copy they can save.

## Architecture

```
guest ─┐                 ┌─ guest
guest ─┼── iroh QUIC ── HOST ── iroh QUIC ──┼─ guest
guest ─┘   (encrypted,   (orders every edit) └─ guest
           P2P, relay fallback)
```

* **Star topology.** Every guest has one connection to the host. Guests never talk to each
  other. The host is the single authority: it numbers every change and broadcasts it, so all
  peers converge on the same document.
* **Transport: [iroh](https://github.com/n0-computer/iroh).** Pure Rust QUIC, dialled by public
  key, with NAT hole punching and public relays (over HTTPS) as fallback. The ticket in the link
  holds the host's endpoint id plus its addresses. No ports to open, no tunnel binary.
  ALPN `desigma/collab/1`.
* **Encoding.** JSON frames, length-prefixed, zstd for large ones. The document model uses
  `skip_serializing_if`, so it needs a self-describing format.

## Syncing the document

### Changes are found by diffing, not by hooking every edit path

Edits reach `App::doc` from many places: `Tx`/`History`, live drags and scrubs (which change the
document every frame before committing), the agent bridge, plugins, version restore. Instead of
instrumenting each one, the sync layer keeps a **shadow** copy of the last synced state. After
any frame where the document changed (and at most ~20 times a second), it diffs `doc` against the
shadow and turns the difference into **ops**. This also means other people see drags live, not
just when the mouse is released.

Ops (crate `desigma-collab`):

| Op | Meaning |
|---|---|
| `Create { node, parent, after }` | New node with all fields, placed after sibling `after` (or first). |
| `Patch { id, fields }` | Changed top-level `Node` fields, as `field name → JSON value`. |
| `Move { id, parent, after }` | Reparent or reorder. |
| `Delete { id }` | Remove a node and its subtree. |
| `DocPatch { fields }` | Changed whole-value document fields: `name`, `kind`, `prototype_device`, `library`, `library_page` and the `pages` order. |
| `DocEntry { field, key, value }` | One entry of a keyed document collection (styles, variables, comments, board votes, flows, guides, timelines): set it, or remove it when `value` is `None`. `key` is a path (see below). |
| `Image { hash }` | A new embedded image is available; bytes are fetched on demand. |

The diff only compares nodes that changed since the last diff; unchanged ones are equal by
`PartialEq` and skipped.

### Conflict rules

* **Properties: last writer wins, per field.** Two people changing the same layer's fill and
  position both win. The same field: the host's order decides.
* **Optimistic local edits.** A guest applies its own edits immediately and sends them. While a
  guest has unacknowledged changes to a field, incoming changes to that field are held back, so
  values don't flicker back and forth. The host's acknowledgement then settles the final value.
* **Document collections: last writer wins, per entry.** Styles, variables, comments, board votes,
  flows, guides and timelines are not sent as whole values. The sync layer sees each as a map from an
  entry key to a value and sends only the entries that changed (`Op::DocEntry`), so two people adding
  or editing different entries both win. The same entry: the host's order decides, like a property.
  Removing an entry removes what is under it. Entry keys:

  | Field | Entries |
  |---|---|
  | `styles` (with `style_meta`) | by style id; the style and its metadata are one entry, so a concurrent edit never mixes the two |
  | `variables`, `variable_collections` | by id. A collection's `variables` list is merged: variables two people added to the same collection at once are both listed |
  | `comments` | `[thread]` is the thread (anchor, resolved) without its messages, `[thread, message]` one message. Two replies to one thread both survive, and resolving while someone replies keeps both. A reply to a thread deleted at the same moment is dropped (delete wins). Threads are ordered by the time of their first message, messages by `created`, ties by id, on every peer |
  | `board_session` | `[timer]` (the whole timer is one value), `[vote]` (votes per person and `ended`), `[vote, node, voter]` (one person's count on one node; the node's tally is the sum). Every person writes only their own voter entries, so votes on the same node at once all count |
  | `flows` | by start frame (a frame starts at most one flow); kept in start-node order |
  | `guides`, `timelines` | by page; a page's guide list is one value |

  Task status and `@mentions` on comments are plain-text conventions inside a message, not
  fields: an opening message that starts with `[ ]`, `[/]` or `[x]` makes the thread a to-do,
  in-progress or done task (the pin takes the status colour; "Mark done" rewrites the marker),
  and `@name` tokens are highlighted. Both travel as the message text, so nothing above changes.

  Whole-value last writer wins stays for `name`, `kind`, `prototype_device`, `library` and
  `library_page`. `pages` keeps its own merge: the order is the last writer's, pages it did not
  know survive.
* **Structure is validated by the host.** Moving a node under its own descendant, into a
  non-container, or under a deleted parent is rejected. The host then sends the guest a
  correction: the current state of the affected nodes.
* **Order uses sibling anchors** ("after X"), so two people inserting into the same frame both
  keep their layers.
* **Delete wins over edit.** Patches to a deleted node are dropped.

### Node ids

`NodeId` is a counter, so two peers would hand out the same ids. On join, each peer gets an index
(host 0, guests 1, 2, …) and allocates ids in its own range `(peer << 40) | n`. Existing ids keep
working.

Ids of the other keyed collections are made so that two people creating one at the same time cannot
pick the same: variable and collection ids are `prefix:d{peer}-{n}` (plain `prefix:d{n}` outside a
session), style keys carry `(p{peer})`, comment threads and messages are random UUIDs, flows are keyed
by their start node.

### Undo

Each person's undo reverts only their own changes. When a remote patch arrives, the matching
fields in the stored inverse commands of the local `History` are updated to the remote values.
Undoing an earlier local edit then does not revert someone else's newer change.
The commands that snapshot a whole collection (`SetStyles`, `SetComments`, `SetVariables`,
`SetBoardSession`, `SetFlows`, `SetGuides`, `SetTimeline`) are rebased per entry the same way: a
remote entry is set in the stored snapshot (or removed from it), the rest stays. Undoing my style
leaves yours. One consequence: if someone edits an entry I created, my undo of the creation leaves
it, with their edit. An undo is an
ordinary local edit, so it syncs through the diff like any other.

### Joining

The host sends a snapshot: the document without image bytes, plus the current op sequence
number, and the digest of every image. The guest reads the images it has cached from disk and
requests only the others. Fonts the guest cannot load are requested from the host, which sends
the file if it has it; otherwise they show up in the missing-font toast.

### Reconnecting

When the connection drops (network change, sleep), the guest keeps the document, shows a
"Reconnecting…" banner and stays editable. It dials the same ticket again after 1, 2, 5 and 10 s,
then every 15 s, for two minutes, showing the secret it got at join. The host recognises the secret,
gives the guest its old peer index and id range back (so ids cannot collide) and sends a fresh
snapshot. The guest lays the changes the host never confirmed over that snapshot with the usual
per-field rules (its own write is the later one, deleted nodes stay deleted) and sends them as a
normal batch. After two minutes it gives up: "Connection lost — you can save a copy".

Documents are small enough that a snapshot is cheaper than replaying a log, and the host needs no
per-guest history for this.

## Presence (not part of the document)

Sent separately, not sequenced, last state wins, about 20 Hz:

* cursor position and page;
* selection;
* viewport (for **Follow**);
* name, colour, role.

On the canvas: coloured cursors with name tags and selection outlines in each person's colour.
The top bar gets an avatar row; clicking an avatar follows that person.

## Security

* The ticket is the secret: only people with the link can find the host.
* Optional password. It is checked over the already-encrypted connection, compared in constant
  time, with failed attempts rate-limited.
* The host can require approval for each joiner, kick anyone, and end the session.
* Roles: **can edit** / **can view**. The host drops ops from viewers.
* Off until the host clicks Share. Nothing listens otherwise.

## Platforms

iroh runs on Windows, macOS and Linux; the protocol is the same everywhere. The
`desigma://` link handler is registered per OS: Inno Setup registry keys on Windows,
`CFBundleURLTypes` in `Info.plist` on macOS, `x-scheme-handler` in the `.desktop` file on Linux.
Pasting the link always works as a fallback. See "Links and files reach the running editor" below.

Every peer must speak the same protocol version. A mismatch is refused with "update desigma". Protocol 3 sends keyed collections per entry: `Op::DocEntry`, `Ack::entries` (the host's value of each
entry the batch wrote) and `Fix::Entries`.

## Limits of the first version

* The session ends when the host closes the file. Going home or opening another file asks first.
* Reconnecting works for guests only, and only while the host keeps the file shared: the secret is
  valid for the session, and a restarted session has a new link. Edits a guest made to nodes the host
  deleted in the meantime are dropped (delete wins).
* Joining needs the desktop app (no browser client yet).
* About 2–20 people per session, limited by the host's upload bandwidth when people join.
* Fonts: files over 20 MB are not shared. Font files are shared only between the participants of a
  session, for rendering; they are loaded for that run of the app and never installed.
* Images added during a session are fetched from the host once (and cached after that); the digest
  list covers the images present at join.

## Phases

1. **Core sync (`crates/desigma-collab`, no network).**
   * Ops, diff and apply.
   * Id ranges.
   * Host sequencer with structure validation.
   * Guest reconciliation (pending fields, corrections).
   * History rebase.
   * Snapshot.
   * In-process tests with simulated peers, including randomized "everyone edits at once" runs
     that must converge to identical documents.
2. **Transport (`crates/desigma-net`).**
   * iroh endpoint, ticket and link encoding, and the handshake (protocol version, password,
     name, approval).
   * Framed messages, peer join and leave events, broadcast.
   * A background tokio runtime with channels to the UI thread.
   * Loopback tests with two endpoints in one process (no relay).
3. **App integration (done).**
   * Share and Join dialogs and the session lifecycle.
   * Applying remote ops to `App::doc`, plus the shadow and diff loop.
   * Roles, kick and end session.
   * The `desigma://` handler on all three OSes.
4. **Presence (done).** Cursors, selections, avatars, Follow.
5. **Polish.**
   * Reconnect after a dropped connection (done).
   * Font transfer (done).
   * Image cache (done).
   * Leave questions and a refreshed share link (done).
   * A headless `desigma-server` that can host a file permanently (same protocol).

## Implementation notes (phases 3 and 4)

Code: `crates/desigma-app/src/collab/` (`mod.rs` session and per-frame pump, `link.rs` transport traits and
the desigma-net adapters, `presence.rs`, `ui.rs`, `draw.rs`, `tests.rs`).

* **Frame loop.** `collab::pump` runs in `Shell::logic` (also while the window is occluded). Drain the
  transport, hand guest messages to the sync core, number our own edits when the canvas revision changed
  (at most 20 Hz, flushed once idle), send presence, ease remote cursors, follow.
* **Wire.** Reliable lane carries `ToGuest { Sync, Role, Images, Font }` and `ToHost { Sync, Fonts }` (`desigma_collab::wire`) as JSON;
  desigma-net deflates big frames. The host fans out itself (per peer), so a joiner only gets messages after
  its snapshot. Presence is a datagram of at most 1000 bytes (selection cut to fit); the host stamps the sender
  and relays it to the others, replays known presence to a newcomer, and sends `gone` when someone leaves.
  Silent people expire after 12 s; keepalives go out every 2 s.
* **Transport traits.** `HostLink` / `GuestLink` wrap desigma-net so tests can use in-memory channels.
  `DESIGMA_COLLAB_NETWORK=direct` skips relays (LAN, loopback).
* **Undo.** Guest messages go through `rebase_history`; the host rebases its own history for guest ops. An undo
  that fails with NotFound after a remote delete is silent.
* **Read-only.** `App::read_only()` (old version or view-only guest) gates `run_tx` and the canvas. The host
  still rejects viewer ops and corrects the guest.
* **Ids.** The app only allocates through `Document::alloc_id`; nothing resets `next_id`. A shared document
  keeps its id range until the session ends. Viewing or restoring old versions is refused while hosting.
* **Joining.** The shared file replaces the open document (like File, Open), is not tied to a path, and
  "Save a copy" uses Save to library. The display name is the "Your name" setting (OS user name by default).
* **Link scheme.** `desigma desigma://join/...` opens Join prefilled. Registered by the Windows installer
  (`scripts/windows/desigma.iss`), `CFBundleURLTypes` (macOS) and `x-scheme-handler/desigma` (Linux).

* **Links and files reach the running editor.** On macOS, links and Finder opens arrive as Apple events,
  which winit does not surface; `src/macos.rs` registers an `NSAppleEventManager` handler (`GURL` and
  `odoc`) at startup and again once the app has launched. On Windows and Linux a second
  `desigma desigma://join/...` (or `desigma FILE`) process connects to the running editor and exits without
  a window (`src/instance.rs`): a `127.0.0.1` listener on an ephemeral port, with the port and a random token
  in `instance.json` (per-user runtime or local data dir, 0600 on unix). It accepts only
  `<token> open-link desigma://join/...`, `open-file <absolute path>` and `ping`, in lines of at most
  4 KiB. A stale file (connection refused) just means a normal start. Both paths feed one inbox that the UI
  drains each frame: Join opens prefilled, a file opens like a dropped one (both ask "Discard unsaved changes?" with Save / Discard / Cancel first when the editor has unsaved edits or a live session), and the window is
  un-minimized and focused (or asks for attention when the OS refuses focus).

Known gaps of phases 3 and 4: the macOS handlers are compiled in CI but untested by hand; the board has a floating
Share strip instead of the inspector row.

## Implementation notes (phase 5: reconnect, fonts, image cache)

**Wire.** Protocol version 2. Image and font bytes are base64 strings inside the JSON message
(`desigma_collab::b64`), not number arrays. Measured with 200 000 bytes of incompressible image data
(`base64_images_cost_about_their_raw_size...`): JSON 267 KB against 714 KB before; on the wire, after the
frame's deflate, 201 KB against 268 KB (454 KB with the old deflate level, see below). So base64 costs
about the raw size and a separate binary message kind would add a second code path for almost
nothing. The frame deflate level went from 1 to 4: zlib-rs only uses fixed Huffman codes at level 1, and
those cannot shrink base64 text at all.

**Core (`desigma-collab`).**
* `HostSession::add_peer` also creates a random secret (two UUIDs); `resume_msg(peer)` is the
  `ServerMsg::Resume { token }` that gives it to the guest, sent after the snapshot.
* `remove_peer` keeps the peer index and the secret (and remembers the role) for the rest of the
  session; `forget_peer` drops them (kick). `resume(token, role)` re-seats the guest, also when the old
  connection was never noticed as dead; the caller sends `snapshot(peer)` and `resume_msg(peer)`.
* `GuestSession::rejoin(doc, snapshot)`: the unconfirmed work is `diff(base, doc)`, where `base` is the
  guest's replica of the host's last known state. It is applied to the new snapshot, `doc` is moved to
  that result in place, and the shadow becomes the snapshot, so the next `local_changes` sends exactly
  what the host still misses. Refuses a snapshot for another peer.
* The in-memory simulation cuts and resumes guests at random while everyone edits and must converge.

**Transport (`desigma-net`).** `Hello.resume` (opaque string), `Guest::join_resume`, and on the host
`allow_resume(token)` / `revoke_resume` (a guest showing a trusted secret skips the approval prompt
and the peer limit; the password is still checked), `join_token(peer)` (the secret a joining peer showed,
handed out once, ask right after `PeerJoined`). `Host::sever` cuts a connection like a network failure,
for tests.

**App (`collab/`).**
* `reconnect.rs`: backoff, the dial in progress, giving up. A refusal that cannot change (wrong password,
  declined, version) ends it at once. `GuestState::live()` is false while reconnecting; presence and
  batches wait, the document stays editable.
* `assets.rs`: `ImageCache` (SHA-256 of the bytes as file name, under the app cache dir
  `collab-images`, 500 MB, least recently used removed first by mtime, a read counts as a use), the
  `FontIo` trait (host: file of an installed font; guest: load into the renderer for this run) and the
  wire types. Image keys in a document (`img-3`) are unique only inside it, which is why the cache is
  addressed by digest and the host announces `key -> digest` (`ToGuest::Images`) before the snapshot. What
  arrives is checked against the announced digest.
* `exchange.rs`: the guest answers an image request from the cache when it can, asks the host for fonts
  the background load could not resolve and the document uses (`ToHost::Fonts`), the host answers per
  font with `ToGuest::Font` (empty bytes: none, over 20 MB, or not used by the file). Those not shared are
  reported through the existing "Font unavailable" toast.
* Leaving: `App::leave_ok(then)` guards `go_home`, opening, new file and Join; it shows "Stop sharing?" /
  "Leave the session?" and runs `then` after `end_collab`. `replace_document` still ends a session as a
  backstop.
* The Share popover shows `HostLink::link()` (the transport's current ticket) while it is open.
* A message that arrives in the same batch right behind the snapshot is no longer dropped.

Known gaps: the cache cannot know in advance about images that appear after the join, so a later
session asks the host once for each; a host restarting its session gets a new link, and guests do not
follow; a font the host could not send is not retried.

## Phase 5: `desigma-server`

Code: `crates/desigma-server` (library plus the `desigma-server` binary; usage, systemd unit and security
notes in its README).

* **Same protocol, same code.** The wire types (`ToGuest`, `ToHost`, `ImageInfo`, `FontKey`, the SHA-256
  `digest`) and the UI-free host routing live in `desigma_collab::wire`; the app host and the server both call
  it. `admit` is the one join sequence: the resume secret the transport saw (`join_token`) re-seats a returning
  guest as the same peer (`HostSession::resume`), otherwise `add_peer` + `allow_resume`; a stale connection of
  that peer is kicked; then `Role`, `Images` (when the file has images), `Snapshot`, `Resume`. `route` delivers
  `Out`s by `Dest`, `relay` passes presence on, `answer_fonts` answers `ToHost::Fonts` through a callback. The
  transport is the small `HostWire` trait (the app implements it for its `HostLink`, the server on
  `desigma_net::Host`). What stays in each host: the app's toasts, approval, undo rebase and cursors; the
  server's autosave. A test pins the JSON of every variant.
* **Reconnect and fonts.** Returning guests get their peer back with no approval. The server has no font
  system, so every `Fonts` request is answered with empty bytes ("cannot share").
* **Authority and persistence.** It applies guests' ops to its own document, autosaves when changed (default
  10 s) and on Ctrl-C / `SIGTERM`, always temp file plus rename, with rotating `FILE.bak.N` backups.
  `.fig` / `.pen` / REST `.json` / `.svg` inputs are imported once and saved as `.desigma` next to the source.
* **Stable link.** `HostOptions` gained `secret_key` (fixed endpoint identity) and `port`; the server keeps the
  key in `.<name>.desigma-key` (mode 0600 on Unix). `Ticket::relay_only` drops direct addresses so the link
  does not change when the IP does.
* **No approval.** Nobody is there to click Accept, so everyone passing the password is let in; the password
  (preferably via `--password-env`) is the gate, and `--view-only` makes everyone a viewer.

## Keyed collections (protocol 3)

Code: `crates/desigma-collab/src/entries.rs`. Before protocol 3 every document-level field travelled as one
`DocPatch` value, so two people adding a comment, a vote, a style or a variable at the same time lost one.

* **One op, a path as key.** `Op::DocEntry { field, key, value }` sets one entry (`value: Some`) or removes it
  (`None`). `entries(doc, field)` reads a field as a flat map from key path to JSON, `diff` compares the maps of
  the shadow and the document, and `apply` writes one entry. Parents come before children (a comment thread
  before its replies) and removals before sets; a removed parent takes its children with it.
* **A write into a missing parent** (a reply to a deleted thread, a vote when the session is gone) is refused by
  the host with a correction (the host's value of that entry, `Fix::Entries`), and dropped by guests.
* **Guests.** Pending writes are tracked per entry; a host write to an entry with an unacknowledged local write is
  held back, as for node fields. `Ack::entries` carries the host's value of every entry the batch wrote, which
  settles entries that lost the race. Entries that came from the host with a value different from the local one
  are in `Received::applied`, so history is rebased.
* **History.** `rebase_history` sets (or removes) the entry in the stored snapshots of `SetStyles`, `SetComments`,
  `SetVariables`, `SetBoardSession`, `SetFlows`, `SetGuides` and `SetTimeline`; for variables the list of a
  collection in the snapshot is cut to the variables the snapshot has.
* **Reconnect.** `GuestSession::rejoin` works on diffs, so offline entries are laid over the new snapshot per
  entry.
* **Order.** Comment threads and flows are kept sorted after every entry write (threads by first message time
  then id, messages by `created` then id, flows by start node), so all peers end up with the same order. Local
  edits that append out of that order are put in order by the next entry write from anyone.
* **Votes per person.** A vote is the entry `[vote, node, voter]` -> that voter's count, and a node's tally is the
  sum over its voters, so two people voting on one sticky at once both count. The voter id is a random id kept per
  install in `settings.json` (`voter_id`), not the peer index, so it survives restarts and reconnects and works
  offline. Each person only writes their own voter entries, and "N votes each" counts only that person's votes.
  Files saved before this kept a plain count per node; they load as the votes of the voter `legacy` (totals stay
  the same) and save in the new shape (`{"node": {"voter": n}}`). The wire shape of the key changed without a
  version bump, as protocol 3 had not shipped.
* **Ids made at the same time.** New variable and collection ids are `prefix:d{peer}-{n}` when the document has an id
  peer (`Document::id_peer()`, set by the session) and `prefix:d{n}` otherwise, so existing ids and files are
  unchanged; style keys likewise (`Color style {n} (p{peer})`, `Document::fresh_style_key`). Comment thread and
  message ids are random UUIDs and flows are keyed by start node, so those were already unique.
* **Known limits.** Removing the first message of a thread changes where the thread sorts.
* **Tests.** `sim.rs`: focused cases (concurrent comments, replies, resolve with reply, delete with reply, votes,
  styles, same-style edit, variables, flows/guides/timelines, undo of own style/comment/vote/variable, viewer,
  invalid entries) and `run_keyed_sim`, where every peer only adds distinct keys (also across outages) and every
  addition must be in the final document; the general random simulations now include keyed edits too.
