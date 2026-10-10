---
title: Client contract
description: What a Herdr daemon computes for its clients, what each client reports back, and how tools read and steer clients through the socket API.
---

This build runs more than one client against a daemon: the TUI and the native
Herdr GPUI app. The daemon computes what they must agree on and sends it in the
snapshot. Each client reports its Agents list to the daemon, and tools read
those reports and ask a client to jump through the ordinary socket API.

The private binary protocol is unchanged (`protocol` stays 23). Everything on
this page is JSON: optional snapshot fields, client-lane methods the endpoint
advertises in its welcome, one endpoint control kind, and socket API methods.

## Snapshot additions

The endpoint snapshot (`shell.snapshot.v1`) carries three optional additions.
A client that lacks them decodes an empty list, an empty key, and no mark.

### Pinned sections

`pinned_sections` is the daemon's `[[ui.sidebar.spaces.pinned]]` config with
the matching spaces already resolved, so a client draws the sections without
reading that config:

```json
"pinned_sections": [
  { "title": "Priority", "fg": "#ff8800", "workspace_ids": ["w1"] },
  { "title": "Platform", "fg": null, "workspace_ids": ["w1", "w2"] }
]
```

Sections come in config order. `workspace_ids` lists the member spaces in the
snapshot's space order and is never empty: a section no space matches is left
out. A space is a member when it carries the section's token with a non-empty
value that meets the section's `equals`, `contains`, or `starts_with`
condition, if it has one. `fg` is `#rrggbb` or `null`.

### Agent priority

Each entry of `agents` has two more fields:

```json
{ "pane_id": "w1:p1", "agent_status": "done", "priority_key": "0.1.00065d7128fb2820", "priority_mark": "high" }
```

`priority_key` is the agent's place in the `priority` order, with the
`ui.agent_priority_tokens` lift already applied. It is opaque. To order agents,
compare the keys as plain strings, byte by byte, smallest first. Every key has
the same length. A client that sorts by space instead ignores the key.

`priority_mark` is the value of the first `ui.agent_priority_tokens` entry the
agent's space carries with a non-empty value, or `null`. The agent picker shows
it before the agent's label.

Keys from different daemons compare the same way, so a client merges the agents
of several machines by sorting all of them on `priority_key`. This works
because the key's last part is not a per-daemon counter. A daemon's
`state_change_seq` counts only its own changes, starts again at every boot, and
says nothing about another daemon's agents. The key uses the time of the
agent's latest status change instead, in microseconds since the Unix epoch,
which each daemon keeps strictly increasing for its own agents.

The comparison is therefore exact within one daemon, and between daemons as
accurate as their clocks agree. Clock skew can only swap agents of different
machines that have the same lift and the same status. Two keys can be equal:
agents that have not changed status since their daemon started all carry time
zero. A client breaks ties by its own machine order, then by the agents' order
in their snapshot.

A client applies its own rules on top: for example, the TUI lists the agents of
a reconnecting machine after all others.

## Unread hold

`agent.mark_unread` takes `{"pane_id": "..."}` and holds an acknowledged
completion: an agent that shows as `idle`. It is available on the client lane
and on the socket API, and returns the agent as `agent_info`. Other agents are
rejected with `agent_not_idle`, a pane without an agent with `agent_not_found`,
and an unknown pane with `pane_not_found`.

The daemon keeps the hold, so every client shows the same thing:

1. While the client that marked the pane still presents it, nothing changes.
   A client presents the focused pane of the tab it is on, while its surface
   for this endpoint is active.
2. When that client focuses another pane, tab, or space, switches to another
   machine, or detaches, the completion comes back. The agent's `agent_status`
   is `done` again, it gets a new `state_change_seq` and `priority_key`, and
   `endpoint.agent-completions.v1` lists that sequence as its completion. It
   queues behind the completions already waiting in its priority group, and
   later completions queue behind it. `pane.agent_status_changed` reports the
   change; plugin event hooks do not run for it.
3. The badge stays while clients merely keep the pane in view. It clears when a
   client arrives at the pane: it starts presenting it, or its window regains
   focus while it does. The agent is `idle` again and, with its newer time,
   leads the idle agents.

A mark from the socket API has no client, so the completion comes back as soon
as no client presents the pane, at once if none does. A real status change ends
a hold. Holds are in memory and do not survive a server restart or handoff.

## Client report

A client tells the daemon what its Agents list shows, across every machine it
is attached to, with the client-lane method `client.report`. A report replaces
the client's previous one. It needs neither an active surface nor the command
lease, so a client sends it while it presents another machine or waits on
another command:

```json
{"id":"client-shell:7","method":"client.report","params":{
  "focused": true,
  "active_endpoint_id": "local",
  "endpoints": [
    {"endpoint_id":"local","label":"Local","status":"online","boot_id":"boot-a"},
    {"endpoint_id":"ssh:0123456789abcdef0123456789abcdef","label":"Build","status":"online","boot_id":"boot-b"}
  ],
  "agents": [
    {"endpoint_id":"local","focused":true,"workspace_label":"web","workspace_number":2,
     "cwd":"/work/web","foreground_cwd":null,
     "agent":{"pane_id":"w2:p1","workspace_id":"w2","tab_id":"w2:t1","name":null,
       "display_agent":"Claude","agent":"claude","title":"Review",
       "terminal_title":"Claude Review","terminal_title_stripped":"Claude Review",
       "agent_status":"blocked","state_change_seq":11,"state_labels":[],
       "tokens":[["task","review"]],"focused":true,
       "priority_key":"1.0.00065d7125600000","priority_mark":null}}
  ]
}}
```

- `focused` is whether the client's window or outer terminal has focus, or
  `null` until the client knows. It travels in the report because a client
  tells only the machine it presents about focus.
- `active_endpoint_id` names the machine the client presents. Endpoint ids are
  the client's own.
- `endpoints[].status` is `connecting`, `online`, `reconnecting`, `attention`,
  or `disabled`. `boot_id` is `null` before the endpoint's first snapshot.
- `agents` is the list in the client's display order. Each row's `agent` is the
  record from its endpoint's snapshot as the client shows it. The row's own
  `focused` marks the one agent the client has focused; `agent.focused` is the
  focus inside the row's endpoint.

A report lists at most 64 endpoints and 512 agents, and ids are at most 128
bytes. The daemon rejects a report beyond those limits, or one whose rows name
an unlisted endpoint, with `invalid_params`. Display text longer than 512 bytes
is cut, and an agent keeps at most 32 state labels and 32 tokens.

The daemon keeps the latest report of each client in memory and drops it when
the client disconnects. A client is one connection, identified by the
`client_id` the daemon gave it. A client with several connections to one
daemon, as Herdr GPUI has per window, reports on one of them and receives jumps
on that same connection.

## Reading clients

`client.list` returns the attached clients and their reports:

```json
{"id":"req_clients","result":{"type":"client_list","boot_id":"boot-a","clients":[
  {"client_id":7,"report":{"focused":true,"active_endpoint_id":"local","endpoints":[],"agents":[]}},
  {"client_id":8}
]}}
```

`boot_id` is the daemon's own boot. A reported endpoint with the same `boot_id`
is this daemon, so a tool calls the socket API directly for its panes. A client
without `report` has not sent one.

Subscribe to `client.changed` to stay current. It fires when a client attaches,
sends a report that differs from its last one, or detaches:

```json
{"event":"client_changed","data":{"type":"client_changed","client_id":7,"client":{"client_id":7,"report":{}}}}
{"event":"client_changed","data":{"type":"client_changed","client_id":7}}
```

`client` is the client as `client.list` would now return it, and is absent once
the client has detached. Subscribe first and call `client.list` afterwards, so
no change falls between the two. `client.changed` does not run plugin event
hooks.

## Jump

`client.jump` asks one client to show a pane, switching machine when it has to:

```json
{"id":"req_jump","method":"client.jump","params":{"client_id":7,"endpoint_id":"ssh:0123456789abcdef0123456789abcdef","boot_id":"boot-b","pane_id":"w1:p4"}}
```

`endpoint_id` and `boot_id` are taken from that client's report. The daemon
does not navigate. It sends the client the control `endpoint.client-jump.v1`:

```json
{"jump_id":"jump-3","endpoint_id":"ssh:0123456789abcdef0123456789abcdef","boot_id":"boot-b","pane_id":"w1:p4"}
```

The client performs the navigation and answers on the same connection with the
client-lane method `client.jump.resolve`, which needs no active surface either:

```json
{"id":"client-shell:8","method":"client.jump.resolve","params":{"jump_id":"jump-3","outcome":{"type":"focused"}}}
```

The outcome is `focused`, `stale_boot`, `not_found`, `unavailable`, `timeout`,
or `{"type":"failed","message":"..."}`. A jump id the client does not hold
answers `jump_not_found`. The caller of `client.jump` then receives
`{"type":"client_jumped","client_id":7,"endpoint_id":"...","pane_id":"..."}` or
an error:

| Code | Meaning |
| --- | --- |
| `client_not_found` | No client with that id is attached. |
| `jump_pending` | That client is still performing another jump. |
| `client_departed` | The client detached before it answered. |
| `stale_boot` | The client reports that the endpoint now runs another boot. |
| `jump_target_not_found` | The client knows no such endpoint or pane. |
| `jump_endpoint_unavailable` | The endpoint is not online. |
| `jump_timeout` | The client did not see the pane focused in time, or did not answer within 20 seconds. |
| `jump_failed` | The client could not perform the jump; the message is the client's. |
| `invalid_params` | An id is empty or longer than 128 bytes. |

After `client_departed` and `jump_timeout` the jump may still have happened.

## Fixtures

`tests/fixtures/` holds one canonical example of each shape. Herdr GPUI's
protocol crate keeps verbatim copies:

| File | Shape |
| --- | --- |
| `endpoint-snapshot-shared-v1.json` | Snapshot with `pinned_sections`, `priority_key`, and `priority_mark` |
| `endpoint-client-report-v1.json` | `client.report` request |
| `endpoint-client-jump-v1.json` | `endpoint.client-jump.v1` control data |
| `endpoint-client-jump-resolve-v1.json` | `client.jump.resolve` requests, one per outcome |
| `endpoint-agent-mark-unread-v1.json` | `agent.mark_unread` request |
| `endpoint-pick-open-v1.json` | `endpoint.pick.open.v1` control data |
| `endpoint-pick-close-v1.json` | `endpoint.pick.close.v1` control data |
| `endpoint-pick-resolve-v1.json` | `ui.pick.resolve` params, one per outcome |
| `api-client-list-v1.json` | `client.list` response |
| `api-client-changed-v1.json` | `client.changed` events: attached, reported, detached |
| `api-client-jump-v1.json` | `client.jump` request and its success response |
