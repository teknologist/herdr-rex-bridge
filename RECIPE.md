# Rex bridge for Herdr

Rex sessions appear as another machine group in Herdr's existing Spaces sidebar,
beside this computer and its SSH machines. Selecting one shows Rex's own tabs
and terminal splits; typing and arranging panes works through Herdr's normal
controls. The person keeps stock Herdr and herdr-core. This is an external
protocol bridge, not a JavaScript mod or replacement layout.

![Rex bridge for Herdr](preview.png)

## What you see

Reuse the person's current Herdr layout, theme, fonts and shipped mods. Add a
machine group named rex through normal machine registration. In the shipped
Spaces sidebar this means the usual 26px server header, small name, 6px connection
dot and collapse chevron, with 30px space rows beneath it. Each Rex session is a
space, each window a tab and each tiled terminal block a pane. Do not paint a new
sidebar, global backend selector, custom title bar or alternate terminal view.

The screenshot is stock Herdr showing a private test server: its rex group,
selected demo session and two native terminal panes. Local spaces remain visible
above it. Existing SSH machines remain available in real use. Herdr's own
connection-state text and theme colors indicate connecting, connected or lost
connections; the bridge adds no separate notification UI.

## How it behaves

Selecting a Rex space opens that server's session without switching Herdr away
from its other machines. The person can type, paste, use terminal mouse modes,
focus, resize, split, rename and close using normal Herdr actions. Changes to
sessions, tabs and terminal splits are real Rex changes and remain shared with
other Rex clients. Connecting and disconnecting must not close sessions or
restart their programs.

Working, blocked, done and idle come from Rex terminal status reports. Publish
them as ordinary core terminal/status events; Herdr applies its own attention
rules, agent colors and completed-work handling. Do not add a separate status
palette or pretend every shell is an agent. The bridge adds no keys or hover
behavior; those stay with Herdr and the foreground terminal program.

On attachment or reconnect, show the current terminal screen, retained history,
styles, cursor, primary/alternate screen state and input modes before live
output. A session renamed or split from another Rex client updates the same
space. Unknown or removed objects do not become new terminals by accident.
A slow viewer may lose its attachment and reconnect; it must not silently miss
individual output bytes or block unrelated RPC work.

A Herdr mod such as Easel may sit beside a Rex terminal. It remains a Herdr mod,
with its own props and pane identity. Keep its placement through Rex layout
updates and bridge reconnects without creating a dummy terminal or custom Rex
block. If its terminal anchor disappears, omit the orphaned mod. For a canvas
with no terminal beside it, use a local Herdr mod tab.

## Built from

Create a standalone executable named herdr-rex-bridge, with no layout or mod
folder. It speaks the existing generation-1 hc connect protocol on stdin/stdout
and translates it to Rex API revision 7. Herdr's existing machine transport is
the integration point:

```sh
hc machine add --name rex --via '/absolute/path/herdr-rex-bridge --server unix:///run/user/1000/rex/server.sock via'
```

The bridge's via entry point accepts the remote connect command appended by hc;
it never evaluates or executes that shell line. A direct connect entry point
serves the same protocol for tests. Keep all diagnostics on stderr. The first
hello reply is one JSON line; switch afterward to framed mode when requested.
Give the endpoint a stable core identity. Report protocol compatibility and a
separate bridge version; do not claim to implement every hc operation.

Use the public hc schema and Rex protocols as the wire contracts:

- hc frames have a 9-byte header: little-endian channel u32, kind u8, payload
  length u32. Channel zero carries JSON-RPC. Other channels carry DATA, CREDIT,
  CLOSE, RESIZE, FOCUS and FACTS. Reject payloads over 16 MiB before allocating.
- Support terminal run/list/get/attach/read/label/keep/kill, event subscriptions,
  and the Herdr layout document get/set operations used by the client. Return
  explicit errors for unsupported operations. A hello names supported methods.
- Give each stream an independent credit gate, initially 256 KiB; send chunks
  no larger than 16 KiB. CREDIT replenishes it; CLOSE wakes blocked workers.
  Bound upstream terminal queues. Keep long RPC calls off the reader thread.
- Rex control uses a WebSocket at /api/control with rex.control.v1. Handle
  correlated replies, ordered events, timeouts, disconnects and concurrent
  requests. Data uses /api/data and rex.data.terminal.v1, with resume support.
  Unix sockets, ws and wss endpoints are supported. Use system certificate
  roots for TLS and leave authorization to Rex. Refuse credentials, query
  strings, fragments or arbitrary endpoint paths.
- Split each terminal stream into its initial GHOSTSNP snapshot, live VT bytes
  and nonce-authorized rex1 APC messages. Validate framing, lengths, grid size
  and CRC32C. Reject a corrupt snapshot without handing it to the viewer.
- Restore snapshots with a pinned libghostty-vt, then format them to VT for
  stock Herdr. Preserve the primary shell/history even when attaching inside
  an alternate-screen program. Retain unfinished escape/UTF-8 continuation.
  Do not leave synchronized-output state freezing the new display.
- Rex already answers device queries. Remove those queries before the display
  sees them, including partial sequences crossing reads. Do not enable a
  second source of in-band resize reports. Omit Kitty images in this prototype,
  including file/shared-memory requests that could address the viewer's files.

Map Rex's session/window/tiled split/block structures into the machine's
herdr-app document. Maintain stable aliases for terminals and pane IDs. Compare
the requested terminal tree with current Rex views and issue a bounded sequence
of real Rex operations, refreshing views after each. Reject stale document
versions. New terminals can wait briefly for a layout to place them, then must
start detached even when a background CLI caller never opens a viewer.

Before planning Rex calls, strip Herdr-only mod branches and return their width
share to the neighboring terminal. Persist the decorations separately with their
terminal anchor, orientation, order, props and sizes. Reapply them to fresh Rex
projections. Do not resize Rex's own split just because a mod was added. Reject
a mod-only Rex tab before mutating the server.

Store aliases, space extras and mod decorations in endpoint-specific JSON under
XDG_STATE_HOME/herdr-rex-bridge, otherwise ~/.local/state/herdr-rex-bridge. Offer
an explicit state-directory override. New metadata fields need defaults so old
state still loads. No settings or documents inside Herdr's installation need a
new schema. No Herdr-specific build, dynamic plugin or GPUI dependency is needed
at runtime; libghostty-vt belongs to the bridge.

One workable source split is hc wire server, Rex control/data transport, stream
decoder, pure layout mapping, snapshot-to-VT conversion, and mod-pane metadata.
Use separate tests for the pure protocol/mapping pieces and the real-server
integration. Build Rust and the pinned Ghostty source with Zig 0.16.0 and Clang.

## Watch out

- This is not a Herdr catalog mod: there is no mods/name/main.js to copy. Build
  the executable and register it using hc's existing --via interface. Never
  change the person's backend, core setting, keys or layout to install it.
- The bridge connects to a Rex server and reads terminal/session data. User
  input and layout commands are sent to that server. Its metadata is local;
  it starts no Rex server and should never stop or restart somebody else's.
- Linux is tested. macOS is untested; the hc --via transport currently rejects
  Windows. Require the intended Rex API revision rather than guessing around
  protocol mismatches.
- Register each Rex endpoint once. Other Rex hosts need separate ordinary hc
  machine registrations; do not inject nested machine names from Rex's host
  list. Simultaneous independent cores writing the same bridge metadata are
  outside this prototype's support.
- Only terminal tiled layers are projected. Keep unknown Rex blocks and floating
  layers safe, but do not claim to display them. Lua client keys/actions are
  not forwarded. General hc exec, remote files/git panels and Kitty images
  are not implemented; expose those limits instead of running them locally.
- A snapshot is binary state, not bytes to print into Herdr. Filtering must be
  streaming and bounded; a partial query or image must not bypass it. Replies
  from the display must not be injected a second time into the real program.
- A saved Herdr mod is not a Rex terminal. Preserve its props as given, keep it
  across a server-side rename, and test a fresh bridge process reading its
  metadata. A temporary in-memory pane is not enough.
- Preserve the original shell and stable addresses on reconnect. Do not rerun
  old commands, launch new shells on every attachment, or reuse an alias still
  bound to a live terminal after a bridge restart.
- Tests must create their own Rex server, core session, app home and headless
  display. Use explicit process handles and shut down only those fixtures.
  Never send fake input on the real display or use a user's live sessions as
  disposable tests.

## Done when

- An unmodified upstream Herdr shows rex alongside local and SSH machines,
  without a global backend switcher or a changed layout file.
- Its normal terminal and layout controls alter an isolated Rex fixture, and
  an external Rex rename/split appears back in Herdr.
- Reconnecting restores existing output, colors, cursor, modes, history and the
  primary shell after leaving an alternate-screen program. No snapshot header
  is displayed as terminal text.
- Keyboard input reaches the selected Rex block; resize and focus reach it;
  device queries and in-band resize reports are not answered twice.
- A background run starts without a viewer and its address remains usable;
  stale document versions fail without changing the server.
- A blocked credit gate affects only that stream, malformed/oversize frames
  are rejected, and closing the stream wakes its waiter.
- An Easel mod beside the terminal survives an external Rex update and bridge
  reconnect, keeps its project props and creates no extra Rex terminal.
- The live server process and sessions survive removing/reconnecting the hc
  registration. Other registered machines and the user's app settings remain.
- Unit tests, an isolated real Rex/hc smoke test and a headless stock-Herdr
  test pass. The screenshot comes from that isolated app, not a mockup.
