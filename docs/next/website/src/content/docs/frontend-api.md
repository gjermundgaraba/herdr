---
title: Frontend API and client actions
description: Drive one local Herdr TUI through its socket, and bind the client-side navigation actions.
---

This page covers the two additions this build makes to the TUI client: a
per-TUI socket for same-user tools on the TUI host, and a handful of `[keys]`
actions handled inside the client.

## Client-side actions

These `[keys]` actions are handled inside the TUI client and never reach a
runtime server. They work while any machine is selected and are always read
from the local config, even when an endpoint keybinding profile is in effect.
All are unset by default:

```toml
[keys]
agent_picker = "prefix+a"
workspace_list = "prefix+shift+l"
history_back = "alt+left"
history_forward = "alt+right"
mark_unread = "prefix+u"
```

`agent_picker` opens the session navigator as a flat list of every agent across
connected machines, ordered by status priority and then recency, with the
search focused. `workspace_list` opens the navigator with every workspace
collapsed. Both share the navigator's search, status filters, and keys: type to
search, `↑`/`↓` or `ctrl+n`/`ctrl+p` move, `Enter` opens the selection, `Esc`
leaves the search and then closes. The tree navigator (`prefix+g`) is
unchanged.

`history_back` and `history_forward` step through pane visits recorded per TUI
across all machines. A visit is recorded once per accepted snapshot whose
focused pane changed, so rapid intermediate focus changes coalesce. History is
in memory, holds at most 100 visits, prunes machines that go away or restart,
skips offline machines, and starts empty with each TUI.

`mark_unread` restores the focused agent's completion badge once you leave the
pane by any means; presenting the pane again clears it as usual. Only an
acknowledged completion, shown as idle, can be restored. Working, blocked, and
still-badged agents are left alone.

## Frontend socket

The socket exposes one TUI, including its attached SSH endpoints, to same-user
tools on the TUI host. It is separate from the runtime socket API and the
frozen endpoint codecs, and it is Unix only.

Protocol 7 is private and lockstep: the client and its consumers ship together,
with no fallback to older versions.

### Discovery

Each TUI listens at `/tmp/herdr-clients-<uid>/<client_id>.sock`, or inside
`HERDR_CLIENT_API_DIR` when set. The directory is owner-only (0700) and the
socket is 0600, the same isolation Herdr uses for its private client socket.
The socket grants input and mutation authority over this TUI. A socket whose
TUI has exited refuses connections; tools that list the directory should treat
a refused connect as a dead entry.

The transport is NDJSON, one JSON object per line, with a 1 MiB request limit.
Every connection receives a `hello` first:

```json
{"type":"hello","protocol":7,"client_id":"…"}
```

### Observe

```json
{"type":"snapshot","protocol":7,"id":1}
{"type":"subscribe","protocol":7,"id":2}
```

Both reply with `{"type":"snapshot","id":1,"snapshot":{…}}`. A snapshot
request refreshes the inventory in the client loop and returns it. A
subscription consumes its connection: it sends an initial snapshot, then pushes
full replacement snapshots whenever the projection changes, coalesced to once
per client-loop turn. There is no heartbeat; EOF means the TUI is gone.

The snapshot contains `client_id`, `revision`, `focused`, `input_ready`,
`active_endpoint_id`, `input_target`, and `endpoints`. `focused` is outer
terminal focus, or null until reported. `input_ready` is true while the active
endpoint's presentation lease is available and not frozen. `input_target`
names the pane receiving ordinary typed input, or null while a mode, overlay,
or unavailable lease prevents pane input.

Each `endpoints` entry has `endpoint_id`, `label`, `status`, `boot_id`, and
the cached `ClientShellSnapshot` in `snapshot`.

### Routes

A route names an endpoint and the server boot that produced the ids the tool
captured:

```json
{"endpoint_id":"ssh:…","boot_id":"…"}
```

Pane, tab, and workspace ids are per-server counters, so a request whose boot
id no longer matches the endpoint's retained snapshot is rejected with
`stale_route` rather than acting on a reused id. Refresh the snapshot and retry
once. A subscription always holds current routes.

### Select

```json
{"type":"select","protocol":7,"id":3,"route":{…},"target":{"pane":"pane_9"}}
```

The target is exactly one of `{"pane":id}`, `{"tab":id}`, or
`{"workspace":id}`. On the active endpoint this is an ordinary
`workspace.focus`, `tab.focus`, or `pane.focus` call and the reply carries the
endpoint's result. On another endpoint it starts a native activation carrying
the target, and the reply is `{"ok":true}` once the target is observed
focused, or `timeout` after 12 seconds with the outcome unknown.

### Input

```json
{"type":"input","protocol":7,"id":4,"text":"ls\n"}
{"type":"input","protocol":7,"id":5,"keys":["ctrl+c","enter"]}
```

Provide exactly one of `text` or `keys`. Key names match `pane.send_keys`.
Input follows current TUI focus, including overlays. `{"ok":true}`
acknowledges dispatch, not pane delivery.

### Call

```json
{"type":"call","protocol":7,"id":6,"route":{…},"method":"agent.prompt","params":{"target":"pane_9","text":"hello"}}
```

Any method the endpoint advertises on its client command lane, with the
runtime API's parameter shape. The server owns that list; this build adds
`agent.prompt` to it, without `wait`. The endpoint must already be active with
an available lease, otherwise the call rejects with `inactive_endpoint`. The
reply carries the endpoint result.

### Errors

```json
{"type":"error","id":6,"error":{"code":"stale_route","message":"…"}}
```

Codes include `stale_route`, `unknown_endpoint`, `endpoint_unavailable`,
`endpoint_not_ready`, `inactive_endpoint`, `unsupported_method`,
`invalid_params`, `busy`, `timeout`, `cancelled`, `client_unavailable`,
`request_too_large`, `invalid_request`, `unsupported_protocol`, and
endpoint-provided API errors.

`client_unavailable`, `cancelled`, and timeouts mean an unknown mutation
outcome. Never replay a mutation on that evidence. A missing advertised method
disables only that call.
