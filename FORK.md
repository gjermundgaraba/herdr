# Fork context

This checkout is `gjermundgaraba/herdr`, a thin personal fork of
`herdrdev/herdr`. It exists to support Herdr plugins that need things stock
Herdr does not offer. Machine-specific paths live in `FORK.local.md`, which is
gitignored and may be absent.

## Remotes and branches

- `origin`: `gjermundgaraba/herdr`
- `upstream`: `herdrdev/herdr`
- `master`: an unmodified mirror of upstream `master`
- `custom`: the branch that gets built and run: the latest upstream release
  tag plus a short stack of local commits. It keeps this name; a restructured
  stack replaces it, and the old one moves to `backup/custom-<date>`
- `custom-v2`, `backup/*`, `temp/*`: historical; do not modify or delete them

List the local changes with:

```sh
git log --oneline "$(git describe --tags --abbrev=0 --match 'v*' custom)..custom"
```

## What the fork adds

- Client-side `[keys]` actions: agent and workspace lists, Back/Forward
  history, and `mark_unread`, which asks the daemon for the unread hold,
  documented in `docs/next/website/src/content/docs/client-actions.md`
- The client contract, documented in
  `docs/next/website/src/content/docs/client-contract.md`: a client sends
  `client.report` with its cross-host Agents list, its endpoints, and whether
  its window is focused, and handles `endpoint.client-jump.v1`. Tools read
  `client.list`, follow `client.changed`, and call `client.jump`; a newer jump
  to the same client replaces a pending one, whose caller gets
  `jump_superseded`
  (`src/api/schema/clients.rs`, `src/server/headless/client_reports.rs`,
  `src/server/headless/jumps.rs`, `src/client/client_contract.rs`)
- The unread hold in the daemon: `agent.mark_unread` restores a seen
  completion once the marking client stops presenting the pane, so every
  client shows the same badge (`src/app/unread_hold.rs`)
- Agent status comes from the daemon alone: the TUI's per-client completion
  ledger and the `endpoint.agent-completions.v1` control are removed, so the
  TUI shows each agent's `agent_status` as the snapshot sends it, like Herdr
  GPUI. A completion in the daemon's current tab therefore never becomes a
  badge while every client presents another machine; see "Agent status" in
  the client contract
- Space groups: the `space_group` workspace metadata token puts runs of
  spaces under collapsible, draggable sidebar headers. `workspace.set_group`
  assigns and places a worktree family, moved families take the group of
  where they land, and a family's spaces are kept together, checkout first
  (`src/space_order.rs`, `src/app/api/space_groups.rs`,
  `src/client/shell/space_groups.rs`)
- Pinned space sections: each `[[ui.sidebar.spaces.pinned]]` entry shows a
  collapsible section at the top of every machine's space list with copies of
  the spaces whose given workspace metadata token is non-empty, optionally
  matched with `equals`, `contains`, or `starts_with`. Copies focus and open
  menus like the real rows, but do not drag or take drops, and navigation
  skips them. The daemon resolves the sections and sends them in the snapshot
  as `pinned_sections` (`src/config/sidebar/pinned.rs`,
  `src/server/client_shell.rs`, `src/client/shell/space_groups.rs`)
- Agent priority tokens: `ui.agent_priority_tokens` lists workspace metadata
  token keys, and a space carrying any of them with a non-empty value lifts
  its blocked and done agents to the top of the `priority` agent order; the
  agent picker shows the value of the first listed token that has a non-empty
  one. That order also queues blocked and done agents oldest state change
  first. The daemon sends it as a
  `priority_key` per agent, which clients compare as plain strings, also
  across hosts, and sends the mark as `priority_mark` (`src/agent_priority.rs`)
- One invoke path: `type = "plugin"` keybindings and `plugin.invoke`
  (which replaces `plugin.action.invoke`) run any manifest entrypoint with a
  target pane, tab, or space and its selection as context. Actions and panes
  share one id namespace and one resolver (`src/app/api/plugins/entrypoint.rs`)
- Plugin invocation context: the target a `plugin.invoke` call names
  supplies the context instead of focus; actions, panes, and startup hooks
  also get the invoking TUI's `client_id` (`HERDR_CLIENT_ID`), the target
  pane's agent session, the space's tabs, and every space
  (`src/app/api/plugins/context.rs`)
- Persisted workspace tokens: `workspace.report_metadata` with `persist`
  saves tokens in the session snapshot, so plugin state such as space groups
  and priority tokens survives restarts and handoffs (`src/metadata_tokens.rs`)
- Plugin-facing events and starts: `pane.closed` and `pane.exited` carry
  `tab_id`, `pane.cwd_changed` reports shell cwd changes, and `agent.start`
  waits for a starting shell and reports `agent_present`,
  `pane_command_running`, or `shell_not_ready`
- Native pickers: `ui.pick` blocks while the invoking TUI shows a filterable
  list, and returns the picked id, created text, or cancellation
  (`src/server/headless/picks.rs`, `src/client/shell/picks.rs`)
- Agent state from OSC 7501 program status reports, applied to the session a
  state-reporting integration recorded; the bundled Pi integration (v10)
  reports only the session (`src/pane/osc.rs`, `src/terminal/state.rs`)
- Smaller sidebar and navigation fixes; see the log above

The main consumers are the plugins in the `herdr-plugins` repository. Its
device tools, Micro and Deck, use only the socket API: `client.list`,
`client.changed`, and `client.jump`.

## Sibling fork: Herdr GPUI

`gjermundgaraba/herdr-gpui` on branch `custom` is the user's fork of the native
client, which attaches to this fork's daemon. The goal is the same features in
the TUI here and in that client. Its path is in `FORK.local.md`, and its
`FORK.md` has the matching section; read that before changing anything of the
first two kinds below.

- Contract: the wire shapes a daemon and its clients exchange, such as snapshot
  fields, endpoint control kinds, and client methods. `src/protocol` here
  defines them, and Herdr GPUI's `crates/herdr-protocol` copies them. Add or
  update the fixture in `tests/fixtures/` with every change to one; that crate
  keeps a copy, so a shape that changed on one side fails a test.
- Mirrored client logic: `src/client` behavior Herdr GPUI also implements,
  listed in the table below. Keep it portable: logic and its test cases apart
  from terminal drawing, with the counterpart named in the module's header
  comment.
- Single owner: whatever the daemon computes or a plugin does. Prefer computing
  a result in the daemon and sending it in the snapshot over logic each client
  has to repeat.

| Feature | Here (TUI) | Herdr GPUI | In step |
| --- | --- | --- | --- |
| Sidebar `separator_before` | `src/config/sidebar.rs`, `src/ui/sidebar/tokens.rs` | `crates/herdr-gpui/src/config/sidebar.rs`, `crates/herdr-gpui/src/sidebar/tokens.rs` | yes |
| Plugin picker | `src/client/shell/picks.rs`, `src/client/shell/pick_overlay.rs` | `crates/herdr-gpui/src/picker.rs`, `crates/herdr-gpui/src/picker/` | yes, except a picker that arrives while a menu or dialog is open: Herdr GPUI answers `cancelled`, the TUI replaces its overlay and shows the picker |
| Agent priority order | `src/client/shell/aggregate_navigation.rs` | `crates/herdr-gpui/src/sidebar/agents.rs`, `crates/herdr-gpui/src/sidebar/order.rs` | yes, except under a plugin agent view: Herdr GPUI keeps each host's own view order, the TUI applies the active host's view across hosts |
| Space groups and pinned sections | `src/client/shell/space_groups.rs`, `src/space_order.rs` | `crates/herdr-gpui/src/sidebar/space_groups.rs`, `crates/herdr-gpui/src/sidebar/group_row.rs` | no: group headers do not drag in Herdr GPUI, and it keeps collapse state in memory where the TUI saves it to its preferences file |
| Mark unread | `src/client/shell/endpoints.rs`, `src/client/shell/actions.rs` | `crates/herdr-gpui/src/window/commands.rs`, `crates/herdr-gpui/src/client_contract/follow.rs` | yes |
| Client report | `src/client/shell/client_report.rs`, `src/client/client_contract.rs` | `crates/herdr-gpui/src/client_contract.rs`, `crates/herdr-gpui/src/client_contract/report.rs` | yes, except Herdr GPUI sends a report, and a jump or picker answer, only after the request its connection holds is answered |
| Client jump | `src/client/shell/client_jump.rs` | `crates/herdr-gpui/src/client_contract/jump.rs` | yes, except under an open menu (Herdr GPUI leaves it open, so the jump waits for it and can time out) and for a refused cross-host jump (Herdr GPUI answers from the refusal of its `pane.focus`, the TUI's activation ends as `timeout`), and when the user leaves the jump's host before the pane shows (Herdr GPUI keeps the jump open until its 12 seconds are up, the TUI withdraws the request and answers `unavailable`) |
| Agent and workspace pickers | `src/client/shell/aggregate_navigation.rs`, `src/client/shell/overlay_input.rs` | `crates/herdr-gpui/src/picker/navigator.rs`, `crates/herdr-gpui/src/picker/navigator/rows.rs` | no: Herdr GPUI lacks the Tree layout, the second detail line, and the current marker, and under a plugin agent view its agent picker lists each host's own view where the TUI applies the active host's view across hosts |
| Back/Forward history | `src/client/shell/history.rs` | `crates/herdr-gpui/src/history.rs`, `crates/herdr-gpui/src/history/sync.rs` | yes, except the TUI clears all history when Local becomes unavailable and Herdr GPUI keeps it, and between a step's request being accepted and its pane showing, the TUI records a pane off the traversed path as a visit at once, where Herdr GPUI records nothing and takes the step back if the pane has not shown after 5 seconds |
| Client-side keys stay local | `src/config/keybinds.rs` | `crates/herdr-gpui/src/keymap/daemon.rs` | yes, except a direct key equal to the host profile's prefix key (the TUI runs the action, Herdr GPUI arms the prefix) and a local config that does not parse (the TUI keeps the previous five bindings, Herdr GPUI unbinds them) |

Add a row, in both `FORK.md` files, when a mirrored feature lands in either
client.

A change to contract or mirrored code is finished only when the other side
matches. Herdr GPUI is a separate project, and a change there is a separate
task the user asks for, never a side effect of work in this repository: tell
the user which change it now needs, and mark the row as behind until that
lands.

## Working on the commit stack

- Keep one commit per feature on top of the release tag, not one per change.
  Fold fixes and follow-ups into the feature they belong to with
  `git commit --fixup <feature-commit>` and
  `GIT_SEQUENCE_EDITOR=: git rebase -i --autosquash <tag>`. Keep generated
  files and unrelated formatting out of those commits.
- Only the tip of the stack has to build and pass checks; intermediate commits
  exist to keep features reviewable and separable.
- Upstream tags each release on its own branch, so the previous release tag is
  not an ancestor of the next one. Move to a new release with
  `git rebase --onto vNEW vOLD custom`; a plain `git rebase vNEW` replays
  upstream's old release commits too. Never merge `master` into the stack.
- A clean rebase does not prove compatibility. If an upstream change interacts
  with a customization, changes existing behavior, or leaves you unsure whether
  a customization still works, stop and ask the user. Do not remove, adapt, or
  disable a customization on your own.
- Push only when asked, and use `git push --force-with-lease origin custom`.
- Prefer cohesive shared code over minimizing the number of upstream-owned
  files touched. Keep rebases manageable, but do not add duplicate behavior or
  compatibility wrappers solely to reduce the diff surface.

## Upstream guardrails

Upstream's `AGENTS.md` still applies to code changes. This is a custom fork, so
the external contributor guardrail applies to `herdrdev/herdr` itself: never
open issues or pull requests against it, or push to it.

## Build and install

The vendored `libghostty-vt` needs Zig 0.16 or newer. If `zig` on `PATH` is
older, point `ZIG` at a 0.16 binary for every build and test command
(`FORK.local.md` records the path on this machine).

Check, build, and atomically replace the binary on `PATH`:

```sh
just ci && just docs-contract-test   # `just check` also runs windows-lint, which needs a Windows SDK
just build                           # cargo build --release --locked
dest="$(command -v herdr)"
cp -p "$dest" "$dest.bak-<label>"    # keep the previous build for rollback
cp target/release/herdr "$dest.new" && chmod 750 "$dest.new" && mv -f "$dest.new" "$dest"
cmp target/release/herdr "$dest"
herdr config check
```

Always copy to a temporary file and rename it over the installed binary. On
macOS, copying straight over the existing file can leave a stale code-signature
cache that makes the kernel kill the new binary (exit 137). Verify with `cmp`
rather than `herdr --version`, which reports the same release version for every
fork build. `herdr config check` catches config the new build rejects; the TUI
otherwise falls back to default settings silently.

Keep at most one backup, next to the installed binary. Delete older backups
before making a new one, and delete the backup once the user confirms the new
build works. Do not leave backups in other locations such as `/tmp`.

Never symlink the installed binary into `target/`. Keep
`[update] version_check = false` in the Herdr config so the built-in updater
cannot replace the fork build with an upstream release.

## Activating a new build

Running servers and TUI clients keep the old binary. Do not restart anything
unless the user asks; tell them what needs restarting. When they ask, use a live
handoff, which keeps pane processes alive, once per running session:

```sh
herdr server live-handoff --import-exe "$dest"
herdr --session <name> server live-handoff --import-exe "$dest"
```

Old TUI clients exit after a handoff, and the user relaunches them. Client-only
changes need just a TUI relaunch. Never use `herdr server stop` or kill Herdr
processes to activate a build. Until servers are handed off, the new CLI may
report a protocol mismatch against them; use the backup binary for those calls.

Plugin daemons such as Micro and Hub may need a restart after a server
handoff; follow their lifecycle docs in the `herdr-plugins` repository.
