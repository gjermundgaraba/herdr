---
title: Client-side actions
description: Bind the navigation and unread actions the Herdr TUI handles itself, on whichever machine it presents.
---

This page covers the `[keys]` actions this build adds to the TUI client. Tools
that read or steer a client use the socket API instead; see
[Client contract](/docs/client-contract/).

## Actions

These `[keys]` actions are handled inside the TUI client: the key is not
forwarded to a runtime server. What an action then does can still be a request
to the machine that owns a pane: opening a picker's selection or stepping
through history focuses the pane there, and `mark_unread` sends
`agent.mark_unread`. The actions work while any machine is selected, and their
bindings are always read from the local config, at startup and on every config
reload, even when an endpoint keybinding profile is in effect. All are unset by
default:

```toml
[keys]
agent_picker = "prefix+a"
workspace_list = "prefix+shift+l"
history_back = "alt+left"
history_forward = "alt+right"
mark_unread = "prefix+u"
```

Only these five bindings stay local under an endpoint keybinding profile. The
prefix key is the profile's, so `prefix+a` then means the profile's prefix
followed by `a`. In terminal and prefix mode, one of these actions also wins a
key the profile binds to another action.

Navigate mode is narrower, with or without a profile. It matches the
`navigate_workspace_*` and `navigate_pane_*` keys in effect, which are the
profile's under a profile, the prefix key, and its fixed keys first, so an
action that shares a key with one of them does not run there. It then
consults only `prefix+` bindings, by the key after the prefix: `prefix+u`
runs on `u`, and a direct binding such as `alt+left` does nothing.

`agent_picker` opens the session navigator as a flat list of every agent across
connected machines, in the Priority order described below: by each daemon's
`priority_key`, with the agents of machines that are not online last. The top
row is selected: `j`/`k` or `↑`/`↓` move, `/` focuses the search, `Enter` opens
the selection. Agents whose space carries a non-empty `space_priority` token
show its value before their label. `workspace_list` opens the navigator
listing only workspaces, with the search focused: type to search, and matching
panes appear under their workspaces while a search is active. `↑`/`↓` or
`ctrl+n`/`ctrl+p` move, `Esc` leaves the search and then closes. Both share the
navigator's search, status filters, and keys. The Goto picker (`prefix+g`) is
unchanged.

`history_back` and `history_forward` step Back and Forward through the panes
the presented machine has focused, the way a browser walks its pages. Each
machine keeps its own trail, and a step never leaves the presented machine.
The trail is read from the machine's snapshots, so focus changed from anywhere,
by a key, the sidebar, a script, or another client, is a visit, and rapid
intermediate focus changes that never reached a snapshot are not. Going
somewhere new drops the visits ahead, as in a browser. A trail is in memory,
holds at most 100 panes, starts over when its machine restarts, and starts
empty with each TUI. A step skips panes that have closed and the pane already
focused, and a key with nowhere to go does nothing.

A step asks the machine to focus the pane. While that request is in flight,
a second step continues from the first one's target, and the focus the first
one brings moves along the trail instead of dropping the visits ahead. Once the
machine has answered, whatever focus arrived is where the next step starts.

`mark_unread` sends `agent.mark_unread` for the focused pane to the machine
that owns it. That daemon keeps the hold, so every client attached to it shows
the same thing: the completion badge returns once you leave the pane by any
means, and arriving at the pane again clears it. Only an acknowledged
completion, shown as idle, can be held; the daemon rejects working, blocked,
and still-badged agents, and the TUI shows its answer. See
[Client contract](/docs/client-contract/) for the rule.

The TUI keeps no record of the completions it has shown. Every agent status,
and so every completion badge, is the one its machine's daemon reports: all
clients attached to that daemon show the same badge, and it clears for all of
them when one of them goes to the agent's tab. A held completion needs more
than its tab: it clears when a client arrives at its pane, so it keeps its
badge when you only move to a neighbouring split.

The Priority order comes from each machine's daemon. Every agent carries a
`priority_key`; the TUI sorts by it and merges machines by comparing keys, with
ties going to the machine listed first and then to the agent's place in its
machine's list. Agents of a machine that is reconnecting follow every agent of
the machines that are online. A restored completion queues behind the
completions that were already waiting in its Priority group, and new
completions queue after it. The sidebar, agent picker, navigation, and mobile
list share this order. The `space_priority` lift and status groups are part of
the key, which each machine's daemon computes. Custom sorts apply when the
endpoint supplies their definitions. Older endpoints supply
view membership and an initial order, and send no keys, so their agents keep
that order, after every agent that has a key. Spaces retains the supplied order.

Pinned sections of the Space panel are the TUI's own: it matches its local
`[[ui.sidebar.spaces.pinned]]` against the spaces of every machine it shows.

During reconnect, agent lists follow the latest endpoint metadata while the
pane retains its previous frame. Reconnecting entries are dimmed and skipped
by indexed and next/previous agent navigation. Explicit selections on an
unavailable remote are rejected. Local selections still reach the runtime so
they can cancel a pending remote switch.
