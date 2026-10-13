---
title: Client contract
description: What a Herdr daemon computes for its clients, what each client reports back, and how tools read and steer clients through the socket API.
---

This build runs more than one client against a daemon: the TUI and the native
Herdr GPUI app. The daemon computes what they must agree on and sends it in the
snapshot. Each client reports its Agents list to the daemon, and tools read
those reports and ask a client to jump through the ordinary socket API.

The private binary protocol is unchanged (`protocol` stays 23). Everything on
this page is JSON: optional snapshot fields, client methods the endpoint
advertises in its welcome, three endpoint control kinds, and socket API methods.

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
the same length. An empty key, which is what an agent of a daemon that sends no
keys decodes to, sorts after every non-empty key; agents with equal keys, empty
ones included, keep the tie order described below. A client that sorts by
space instead ignores the key.

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

### Agent status

A client shows each agent's `agent_status` exactly as the snapshot sends it.
Whether a finished agent is `done`, a completion nobody has seen, or `idle` is
the daemon's seen state, so every client attached to a daemon shows the same
badge, and an agent's status always matches its `priority_key`. A client keeps
no record of the completions it has shown.

The daemon counts a completion as seen when it lands in the tab the daemon is
on, unless the window of its foreground client is known to be in the
background. A client's command-lane request puts the daemon on that client's
tab, and a focus request, from a client or from the socket API, then moves it.
The foreground client is the client with an active surface that acted last in
any way: a command-lane request, input to a pane, a resize, its surface
becoming active, or its window gaining focus. That is usually the client whose
tab the daemon is on, but not always.

A badge clears when the daemon goes to the agent's tab, which a client causes
with `pane.focus`, `tab.focus`, `workspace.focus`, or any other command-lane
request it sends from that tab. A badge in the tab the daemon is already on
clears when a client whose window is focused acts in one of the ways listed
above, and so becomes or stays the foreground client; the window gaining focus
is one of them. The daemon then marks the tab it is on, whichever tab that
client is on. While the foreground client's window is focused, the daemon
also marks its tab again when it handles an agent's status change, and with
most socket API requests.

Two consequences follow from the daemon being on one tab at a time. A
completion in that tab is seen even while every client presents another
machine, so it never becomes a badge. And a second client that sits on a
different tab does not clear a badge there until it sends a command-lane
request: one that takes the command lease, such as `pane.focus`.
`client.report`, `client.jump.resolve`, and `ui.pick.resolve` do not take it
and change nothing here.

This build's daemon does not send the `endpoint.agent-completions.v1` control
and does not advertise the `agent_completions` capability. A client attached to
a daemon that still sends the control ignores it.

## Unread hold

`agent.mark_unread` takes `{"pane_id": "..."}` and holds an acknowledged
completion: an agent that shows as `idle`. A client sends it as a command-lane
request, which needs an active surface and takes the command lease, and it is
also a socket API method. It returns the agent as `agent_info`. Other agents
are rejected with `agent_not_idle`, a pane without an agent with
`agent_not_found`, and an unknown pane with `pane_not_found`.

The daemon keeps the hold, so every client shows the same thing:

1. While the client that marked the pane still presents it, nothing changes.
   A client presents the focused pane of the tab it is on, while its surface
   for this endpoint is active.
2. When that client focuses another pane, tab, or space, switches to another
   machine, or detaches, the completion comes back. The agent's `agent_status`
   is `done` again, and it gets a new `state_change_seq` and `priority_key`. It
   queues behind the completions already waiting in its priority group, and
   later completions queue behind it. `pane.agent_status_changed` reports the
   change; plugin event hooks do not run for it.
3. The badge stays while clients merely keep the pane in view. It clears when a
   client arrives at the pane: it starts presenting it, or its window regains
   focus while it does. A client whose window is known to be unfocused does
   not arrive, whatever it presents. The agent is `idle` again and, with its
   newer time, leads the idle agents.

A mark from the socket API has no client, so the completion comes back as soon
as no client presents the pane, at once if none does. A real status change ends
a hold. Holds are in memory and do not survive a server restart or handoff.

## Client report

A client tells the daemon what its Agents list shows, across every machine it
is attached to, with the client-lane method `client.report`. A report replaces
the client's previous one. It needs neither an active surface nor the command
lease, so the daemon takes it while the client presents another machine or
has another command in flight:

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
  `null` until the client knows. The TUI knows only after its terminal's first
  focus event, so a tool treats `null` as unknown, not as unfocused. The field
  travels in the report because a client tells only the machine it presents
  about focus.
- `active_endpoint_id` names the machine the client presents, or is `null`
  when the report does not list that machine. Endpoint ids are the client's
  own.
- `endpoints[].status` is `connecting`, `online`, `reconnecting`, `attention`,
  or `disabled`. `boot_id` is `null` before the endpoint's first snapshot.
- `agents` is the list in the client's display order. Each row's `agent` is the
  record from its endpoint's snapshot, unchanged. The row's own
  `focused` marks the one agent the client has focused; `agent.focused` is the
  focus inside the row's endpoint.

The daemon checks a report in this order, and rejects it with `invalid_params`
naming the first check that failed, keeping the client's previous report, when:

1. it lists more than 64 endpoints or more than 512 agents;
2. an endpoint, taken in list order, has an `endpoint_id` that is empty or
   longer than 128 bytes, a `boot_id` longer than 128 bytes, or the
   `endpoint_id` of an endpoint listed before it;
3. `active_endpoint_id` names an endpoint the report does not list;
4. a row's `endpoint_id` names an endpoint the report does not list;
5. more than one row is `focused`;
6. a row's `agent.pane_id` is empty or longer than 128 bytes, or its
   `agent.workspace_id` or `agent.tab_id` is longer than 128 bytes.

The daemon then cuts text longer than 512 bytes, on a character boundary: each
endpoint's `label`; each row's `workspace_label`, `cwd`, and `foreground_cwd`;
and the agent's `name`, `display_agent`, `agent`, `title`, `terminal_title`,
`terminal_title_stripped`, `priority_mark`, and `priority_key`. An agent keeps
its first 32 `state_labels` and first 32 `tokens`, each key and value cut the
same way. A daemon's `priority_key` is far shorter than the limit, so only a
key a client made up is ever cut.

The whole request must also fit one client-lane command, which is at most
1 MiB (`MAX_ENDPOINT_COMMAND_BYTES`) of JSON, and the daemon closes a
connection that sends a larger one. 512 rows of long text can exceed that, so a
client measures the encoded request and drops rows from the end of `agents`
until it fits.

A client sends its report to exactly one daemon: the local one. It sends none
to the other machines it is attached to, so each client appears with a report
in one daemon's `client.list`, and that is the daemon tools ask to jump it. A
client without a local connection, or whose local daemon does not advertise
`client.report`, reports to nobody.

When a client-lane request leaves a client depends on the client. The TUI
sends a report, a jump's answer, and a picker's answer at once, beside a
command it has in flight. Herdr GPUI holds one request per connection, so it
queues all three behind the request that connection is waiting on, until that
one is answered. A report that waited is the latest one, not a backlog. A
request that is not answered within 60 seconds ends the connection instead:
what was queued behind it is never sent, the daemon sees the client detach,
and a jump that client had not answered ends as `client_departed`.

The daemon keeps the latest report of each client in memory and drops it when
the client disconnects. A client is one connection, identified by the
`client_id` the daemon gave it. A client with several connections to one
daemon, as Herdr GPUI has per window, reports on one of them. A tool jumps a
client by the `client_id` it read the report under, so the jump arrives on the
connection that carries the report.

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
sends a report that differs from its last one, or detaches, and names only the
client:

```json
{"event":"client_changed","data":{"type":"client_changed","client_id":7}}
```

Call `client.list` to see what changed; a client it no longer lists has
detached. The event carries no report, so the daemon's event history never
holds copies of reports. Subscribe first and call `client.list` afterwards, so
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
| `client_not_reporting` | The client is attached but has not sent a report, so it does not answer jumps. |
| `jump_superseded` | A newer jump to the same client replaced this one before the client answered. |
| `client_departed` | The client detached before it answered, or the daemon could not send it the jump. |
| `stale_boot` | The client reports that the endpoint now runs another boot. |
| `jump_target_not_found` | The client knows no such endpoint or pane. |
| `jump_endpoint_unavailable` | The endpoint is not online, or the client has no snapshot of it. |
| `jump_timeout` | The client did not see the pane focused in time, or did not answer within 20 seconds. |
| `jump_failed` | The client could not perform the jump. The message is the client's, trimmed and cut to 512 bytes, or the daemon's own when the client sent none. |
| `invalid_params` | An id is empty or longer than 128 bytes. |

After `client_departed`, `jump_timeout`, and `jump_superseded` the jump may
still have happened; see [Choosing the outcome](#choosing-the-outcome).

The daemon does not notice a caller that disconnects while it waits. The jump
stays pending until the client answers it, a newer jump replaces it, the
client detaches, or the 20 seconds pass.

### The newest jump wins

A client performs one jump at a time. When `client.jump` names a client that is
still performing a jump, the daemon forwards the new jump under a new
`jump_id`, answers the earlier caller `jump_superseded` at once, and forgets
the earlier jump. A jump the daemon rejects, or cannot forward, replaces
nothing.

A client that receives a jump while it performs another drops the earlier one
and never answers it. If its answer to the earlier jump was already on its way,
the daemon answers that `client.jump.resolve` with `jump_not_found`, which the
client ignores.

The earlier jump's navigation may still be under way when the newer jump
arrives. The client therefore always performs the newer navigation, even when
it already presents the pane the newer jump names, so that navigation is the
last one it starts and the one it ends on. Only a client that is performing no
jump skips the navigation for a pane it already presents, and answers
`focused` at once.

### Choosing the outcome

When a jump arrives, and again on every turn while it waits, the client checks
what it holds. The first of these that applies is the outcome, sent at once:

1. it knows no such endpoint: `not_found`;
2. the endpoint has sent no snapshot yet: `unavailable`;
3. the endpoint's snapshot is of another boot: `stale_boot`;
4. the endpoint is not online: `unavailable`;
5. the endpoint's snapshot has no such pane: `not_found`.

Otherwise the client navigates to the pane the way a click on that agent does,
switching machine when the pane is on another one. Nothing follows that
navigation's own request or switch: the client answers `focused` once it
presents the endpoint with the pane focused, and `timeout` when that has not
happened 12 seconds after the jump arrived. A navigation the endpoint refuses,
the client withdraws, or another selection replaces therefore ends the jump as
`timeout`, unless one of the checks above ends it first. Neither client
answers `failed`.

A client that navigated does not answer `focused` on the turn it started the
navigation, when the pane it shows predates it, nor while it is still
switching machine. The TUI takes no request while it switches, and presents
the machine it switches to shortly before the switch is done, so a jump that
arrives during a switch and names another pane on the machine shown can end
as `timeout`.

## Pickers

A daemon shows a plugin's `ui.pick` on one client by sending it the control
`endpoint.pick.open.v1`, and sends `endpoint.pick.close.v1` when the plugin
closes the picker itself. The client answers with the client-lane method
`ui.pick.resolve` on the connection the picker arrived on. Like
`client.jump.resolve`, it needs neither an active surface nor the command
lease, so a client answers a picker after it switched to another machine:

```json
{"id":"client-pick:3","method":"ui.pick.resolve","params":{"pick_id":"req_pick","outcome":{"type":"cancelled"}}}
```

A client answers every picker the daemon did not close: with the user's choice,
or `cancelled` when the picker goes away for any other reason, such as a switch
to another machine. A picker from a machine the client does not present is
answered `cancelled` as it arrives. Until a client answers, the daemon refuses
that client's next picker with `ui_busy`. A pick id the client does not hold
answers `pick_not_found`, and the socket API answers `ui.pick.resolve` with
`connection_local_only`.

An answer must fit one client-lane command. A client whose `created` text is
too long for that answers `cancelled` instead and tells the user.

## Fixtures

`tests/fixtures/` holds one canonical example of each shape. Each consumer
keeps verbatim copies of the fixtures for the shapes it reads or writes, and
no others: Herdr GPUI's protocol crate, which speaks the endpoint side, and
the plugins SDK, which reads reports through the socket API.

| File | Shape | Copied by |
| --- | --- | --- |
| `endpoint-snapshot-shared-v1.json` | Snapshot with `pinned_sections`, `priority_key`, and `priority_mark` | Herdr GPUI, plugins SDK |
| `endpoint-client-report-v1.json` | `client.report` request | Herdr GPUI, plugins SDK |
| `endpoint-client-jump-v1.json` | `endpoint.client-jump.v1` control data | Herdr GPUI |
| `endpoint-client-jump-resolve-v1.json` | `client.jump.resolve` requests, one per outcome | Herdr GPUI |
| `endpoint-agent-mark-unread-v1.json` | `agent.mark_unread` request | Herdr GPUI |
| `endpoint-pick-open-v1.json` | `endpoint.pick.open.v1` control data | Herdr GPUI |
| `endpoint-pick-close-v1.json` | `endpoint.pick.close.v1` control data | Herdr GPUI |
| `endpoint-pick-resolve-v1.json` | `ui.pick.resolve` params, one per outcome | Herdr GPUI |
| `api-client-list-v1.json` | `client.list` response | plugins SDK |
| `api-client-changed-v1.json` | `client.changed` event | plugins SDK |
| `api-client-jump-v1.json` | `client.jump` request and its success response | plugins SDK |
