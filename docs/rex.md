# Rex protocol mapping

The bridge adapts the Rex API revision-7 control and terminal data protocols.
Its source transport documentation is retained in module comments.

## Who owns what

Rex is authoritative. Herdr is one more Rex client, next to Rex.app and the
`rex` CLI, and everything it shows is shared with them live:

| Rex | Herdr |
| --- | --- |
| session | space |
| window (in order) | tab |
| tiled layer's split tree | the tab's panes |
| terminal block | terminal (its address is the block id) |
| detached block | a running terminal without a pane (peek, agents list) |
| OSC 7501 program status | agent status (working, blocked, done, idle) |

What only Herdr keeps (a space's folder, whether it's named automatically,
and Herdr's own ids for things it created) lives in
`$XDG_STATE_HOME/herdr-rex-bridge/<endpoint>.json`. Focus, zoom and settled tabs stay per
screen, as with herdr-core; focusing a pane also focuses its block in Rex
and claims its size, as Rex.app does.


## Bridge boundaries

The local, unchanged herdr-core owns machine registration, address qualification
and the persistent connection. `src/wire.rs` serves the remote `hc connect`
protocol on stdin/stdout. `src/rex/adapter.rs` translates calls to the Rex server;
`src/rex/project.rs` maps layouts without depending on GPUI. `src/screen.rs`
converts Ghostty snapshots into ordinary VT sequences for the stock display.

The bridge starts no Rex server. The `via` entrypoint only accepts the connect
command appended by hc; it never executes that shell line. All diagnostics go
to stderr so stdout stays a binary protocol stream.

Each terminal stream has its own credit gate. It starts with 256 KiB of credit
and sends chunks of at most 16 KiB; CREDIT replenishes it, CLOSE wakes waiters,
and RPC work is not held behind a stalled stream. Frame sizes are checked before
allocation. The Rex stream decoder bounds snapshots, grid dimensions and APCs.
Slow terminal readers have a bounded upstream queue and lose their attachment
rather than silently losing individual bytes; clients may reattach.

The wire `hello` exposes protocol generation 1 and the supported methods.
`version` denotes compatibility with the tested hc protocol, not an installed
herdr-core server; `bridge_version` reports the standalone bridge package.
