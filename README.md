# Herdr Rex bridge

Use a Rex server from **stock Herdr**, alongside its existing local and SSH spaces.
Rex appears as a machine group in Spaces. No Herdr fork, backend switcher, mod API
extension, or replacement herdr-core is required.

This is a standalone prototype extracted from the Rex integration in
[yycholla/herdr-app](https://github.com/yycholla/herdr-app). It implements the
existing generation-1 `hc connect` protocol and registers through herdr-core's
`machine add --via` transport. Normal machines continue to use herdr-core.

## Build

Requires Rust, **Zig 0.16.0**, libclang/Clang, and the pinned Ghostty submodule.
Build from this repository, independently of either Herdr checkout:

```sh
git submodule update --init --depth 1
cargo build --locked
```

If your default Zig differs, set `ZIG` to the absolute path of Zig 0.16.0.
The binary is `target/debug/herdr-rex-bridge`. It has no runtime dependency on
Herdr's source tree, GPUI, or the fork's binary.

## Connect your server

Use an existing Rex server that authorizes this machine. Register its endpoint
with your normal `hc`; substitute the actual absolute paths:

```sh
hc machine add --name rex --via '/absolute/path/herdr-rex-bridge --server unix:///run/user/1000/rex/server.sock via'
```

A TCP endpoint also works: `--server wss://your-rex-host:7297`.
Use `ws://` only for a listener you intend to reach without TLS. Endpoint
credentials and query strings are refused; Rex controls authorization.

Open stock Herdr. Its existing Spaces mod shows a **rex** group containing the
server's sessions. Select one to use its tabs and panes. Creating and splitting
panes through Herdr changes Rex's actual layout, shared with other Rex clients.
Herdr's machine names are lowercase, so the registration is named `rex`.

Register each Rex endpoint once, with a distinct machine name. Add other Rex
hosts as separate `hc machine add --via` registrations; the bridge does not
import Rex's host list as nested machines. Keep your existing Herdr settings;
do not set `backend`, `rex_server`, or `core` for this integration.

To remove the registration:

```sh
hc machine rm rex
```

The bridge does not ask Rex to close its sessions when the connection ends.

## What works

- Rex sessions → Herdr spaces, windows → tabs, tiled splits → panes.
- Existing sessions and terminals remain authoritative on Rex.
- Live terminal output, keyboard/mouse bytes, resize and focus.
- Attach/reconnect repaint, screen colors/styles, cursor, history, primary and
  alternate screens, keyboard modes, and unfinished VT/UTF-8 input.
- Session/tab/pane changes through the projected Herdr layout document.
- Terminal listing, creation, labels, read, keep, and close through the adapter.
  Background runs materialize even when no viewer opens them.
- Terminal/status/layout events, framed stream credits, and reconnect through
  the normal core's machine transport.

Rex answers device queries itself. The bridge removes those queries before
sending output to Herdr, preventing duplicate answers from the display copy.
The bridge's metadata lives under `$XDG_STATE_HOME/herdr-rex-bridge`, otherwise
`~/.local/state/herdr-rex-bridge`; `--state-dir PATH` overrides it. Metadata is
separate from Herdr's installed data and stores ID aliases/layout extras only.

## Current limits

This prototype has been tested on Linux. macOS has not been tested; herdr-core's
`--via` transport currently rejects Windows. It is not a complete hc server:
its `version` field identifies protocol compatibility, while `bridge_version`
identifies this program. Unsupported operations return errors.

- **Kitty images are omitted**, both in snapshots and live output. In particular,
  file/shared-memory image requests never reach the stock display.
- Remote file browsing, git queries and general `hc exec --on rex` are not
  implemented. Terminal IO still runs on the selected Rex server.
- Rex floating layers, non-terminal blocks, Lua client key bindings and client
  actions are not projected. Existing non-terminal blocks are preserved.
- ID/layout extras assume one registered core connection per Rex endpoint.
  Multiple independent cores writing those extras concurrently are not supported.

## Validation

```sh
cargo test --locked
cargo build --locked
python3 tests/smoke.py
```

`tests/smoke.py` launches its own Rex server and hc session in temporary homes.
It checks the real server, stdin/stdout framing, snapshot conversion, input,
resize/focus/facts, layout edits, background creation, reconnect, `hc --via`,
qualified terminal addresses and remote documents. It stops only its own servers.

For the UI test, use a clean Herdr worktree and binary. Install the dependencies
required by Herdr's `scripts/sandbox` (Weston, Xwayland, uv, ImageMagick):

```sh
HERDR_ORIGIN_WORKTREE=/absolute/path/herdr-app-origin \
HERDR_ORIGIN_BIN=/absolute/path/to/upstream/herdr-app \
python3 tests/ui_smoke.py
```

Pass `CARGO_TARGET_DIR` if the upstream worktree uses a shared target directory.
The test creates an isolated app home/core and headless display, registers its
own Rex server, checks typing and native pane splitting, saves a screenshot,
and tears down the sandbox. It verifies that the upstream worktree stayed clean.

Verified against upstream Herdr `07ba77e`, hc `0.1.14`, and Rex API revision 7.
The captured result is in `artifacts/stock-herdr-rex.png`; the companion JSON
contains Herdr's space/pane inspection and error list.

## Sources

See `NOTICE` and `LICENSE`. The Rex transport, data decoder and layout mapping
are adapted from the fork; Ghostty is a pinned upstream submodule. The bridge's
hc wire server and snapshot-to-VT adapter are separate from the Herdr application.
