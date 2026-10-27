---
title: Client contract
description: What a Herdr daemon computes for its clients, and the requests and controls they exchange beyond upstream's protocol.
---

This build runs more than one client against a daemon: the TUI and the native
Herdr GPUI app. The daemon computes what they must agree on and sends it in the
snapshot, so both clients show the same order and the same badges.

The private binary protocol is unchanged (`protocol` stays 23). Everything on
this page is JSON: optional snapshot fields, client methods the endpoint
advertises in its welcome, two endpoint control kinds, and socket API methods.

## Snapshot additions

The endpoint snapshot (`shell.snapshot.v1`) carries two optional additions.
A client that lacks them decodes an empty key and no mark.

### Agent priority

Each entry of `agents` has two more fields:

```json
{ "pane_id": "w1:p1", "agent_status": "done", "priority_key": "0.1.00065d7128fb2820", "priority_mark": "high" }
```

`priority_key` is the agent's place in the `priority` order, with the
`space_priority` lift already applied: a space whose `space_priority` workspace
metadata token has a non-empty value lifts its blocked and done agents. It is
opaque. To order agents, compare the keys as plain strings, byte by byte,
smallest first. Every key has the same length. An empty key, which is what an
agent of a daemon that sends no keys decodes to, sorts after every non-empty
key; agents with equal non-empty keys keep the tie order described below. A
client may order agents without a key its own way: Herdr GPUI ranks them by
status and recency, the TUI keeps their listing order. A client that sorts by
space instead ignores the key.

`priority_mark` is the value of the agent's space's `space_priority` token
when it is non-empty, or `null`. The agent picker shows it before the agent's
label.

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
`ui.pick.resolve` does not take it and changes nothing here.

The daemon still sends upstream's `endpoint.agent-completions.v1` control and
advertises the `agent_completions` capability, but both clients ignore the
control: neither keeps a record of completions.

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

## Pickers

A daemon shows a plugin's `ui.pick` on one client by sending it the control
`endpoint.pick.open.v1`, and sends `endpoint.pick.close.v1` when the plugin
closes the picker itself. The client answers with the client-lane method
`ui.pick.resolve` on the connection the picker arrived on. It needs neither an
active surface nor the command lease, so a client answers a picker after it
switched to another machine:

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

An answer must fit one client-lane command, which is at most 1 MiB
(`MAX_ENDPOINT_COMMAND_BYTES`) of JSON; the daemon closes a connection that
sends a larger one. A client whose `created` text is too long for that answers
`cancelled` instead and tells the user.

When an answer leaves a client depends on the client. The TUI sends it at
once, beside a command it has in flight. Herdr GPUI holds one request per
connection, so it queues the answer behind the request that connection is
waiting on, until that one is answered.

## Fixtures

`tests/fixtures/` holds one canonical example of each shape. Each consumer
keeps verbatim copies of the fixtures for the shapes it reads or writes, and
no others: Herdr GPUI's protocol crate, which speaks the endpoint side, and
the plugins SDK.

| File | Shape | Copied by |
| --- | --- | --- |
| `endpoint-snapshot-shared-v1.json` | Snapshot with `priority_key` and `priority_mark` | Herdr GPUI, plugins SDK |
| `endpoint-agent-mark-unread-v1.json` | `agent.mark_unread` request | Herdr GPUI |
| `endpoint-pick-open-v1.json` | `endpoint.pick.open.v1` control data | Herdr GPUI |
| `endpoint-pick-close-v1.json` | `endpoint.pick.close.v1` control data | Herdr GPUI |
| `endpoint-pick-resolve-v1.json` | `ui.pick.resolve` params, one per outcome | Herdr GPUI |
