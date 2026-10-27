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
  through each machine's own trail of focused panes (a port of Herdr GPUI's
  per-endpoint trail, `src/client/shell/history.rs`), and `mark_unread`, which
  asks the daemon for the unread hold, documented in
  `docs/next/website/src/content/docs/client-actions.md`
- The unread hold in the daemon: `agent.mark_unread` restores a seen
  completion once the marking client stops presenting the pane, so every
  client shows the same badge (`src/app/unread_hold.rs`)
- Agent status comes from the daemon alone: the TUI's per-client completion
  ledger is removed, so the TUI shows each agent's `agent_status` as the
  snapshot sends it, like Herdr GPUI. The daemon still sends upstream's
  `endpoint.agent-completions.v1` control, which both clients ignore. A
  completion in the daemon's current tab therefore never becomes a badge
  while every client presents another machine; see "Agent status" in
  `docs/next/website/src/content/docs/client-contract.md`
- Agent priority: a space whose `space_priority` workspace metadata token,
  the key the space-priority plugin writes, has a non-empty value lifts its
  blocked and done agents to the top of the `priority` agent order, and the
  agent picker shows the token's value. That order also queues blocked and
  done agents oldest state change first. The daemon sends it as a
  `priority_key` per agent, which clients compare as plain strings, also
  across hosts, and sends the mark as `priority_mark` (`src/agent_priority.rs`)
- One invoke path: `type = "plugin"` keybindings and `plugin.invoke`
  (which replaces `plugin.action.invoke`) run any manifest entrypoint with a
  target pane, tab, or space and its selection as context. Actions and panes
  share one id namespace and one resolver; a pane opens through upstream's
  `plugin.pane.open` handler (`src/app/api/plugins/entrypoint.rs`)
- Plugin invocation context: the target a `plugin.invoke` call names
  supplies the context instead of focus, and actions, panes, and startup
  hooks also get the `client_id` (`HERDR_CLIENT_ID`) of the invoking shell
  client, the TUI or Herdr GPUI. Plugins read anything else, such as tabs,
  spaces, or a pane's agent session, through the socket API
  (`src/app/api/plugins/entrypoint.rs`)
- Persisted workspace tokens: `workspace.report_metadata` with `persist`
  saves tokens in the session snapshot, so plugin state such as the
  `space_priority` token survives restarts and handoffs
  (`src/metadata_tokens.rs`)
- Plugin-facing events: `pane.closed` and `pane.exited` carry `tab_id`, and
  `pane.cwd_changed` reports shell cwd changes. `agent.start` is upstream's:
  the plugin SDK's `start_agent` retries a starting shell as upstream's CLI
  does
- Native pickers: `ui.pick` blocks while the invoking shell client, the TUI
  or Herdr GPUI, shows a filterable list, and returns the picked id, created
  text, or cancellation
  (`src/server/headless/picks.rs`, `src/client/shell/picks.rs`)
- Agent state from OSC 7501 program status reports, applied to the session a
  state-reporting integration recorded; the bundled Pi integration (v10)
  reports only the session. Herdr does not answer the protocol's support
  query, so Pi reports only with `PI_PROGRAM_STATUS=1` in its pane, which the
  user's shell config sets inside Herdr panes, on every machine that runs Pi
  under Herdr; "Pi" in `docs/next/website/src/content/docs/integrations.mdx`
  has the lines. Without it Herdr falls back to reading Pi's screen and never
  sees it blocked. A Pi started directly as a layout, popup, plugin pane or
  custom command, rather than from a pane's shell, does not read the shell
  config, so it does not report through 7501. The bundled extension no
  longer reports working or idle itself, so whenever Pi does not report
  through 7501 (`PI_PROGRAM_STATUS` unset, or Pi launched directly), every
  state falls back to screen detection
  (`src/pane/program_status.rs`, `src/terminal/state/in_band.rs`)
- Smaller sidebar and navigation fixes; see the log above

The main consumers are the plugins in the `herdr-plugins` repository.

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
| Sidebar `separator_before` | `src/config/sidebar.rs`, `src/config/sidebar/separator.rs`, `src/ui/sidebar/tokens.rs` | `crates/herdr-gpui/src/config/sidebar.rs`, `crates/herdr-gpui/src/config/sidebar/separator.rs`, `crates/herdr-gpui/src/sidebar/tokens.rs` | yes |
| Plugin picker | `src/client/shell/picks.rs`, `src/client/shell/pick_overlay.rs` | `crates/herdr-gpui/src/picker.rs`, `crates/herdr-gpui/src/picker/` | yes, except a picker that arrives while a menu, a dialog, or another picker or navigator is open: Herdr GPUI answers `cancelled`, the TUI replaces its overlay and shows the picker; and Herdr GPUI sends a picker's answer only after the request its connection holds is answered |
| Agent priority order | `src/client/shell/aggregate_navigation/agent_order.rs` | `crates/herdr-gpui/src/sidebar/agents/priority.rs`, `crates/herdr-gpui/src/sidebar/agents.rs` | yes: both sort by the daemon's `priority_key`, whose `space_priority` lift neither client configures, and break ties between equal keys by host, then snapshot order, except for agents without a key, from a host that sends none: Herdr GPUI ranks them by upstream's status and recency order, the TUI keeps their listing order; under a plugin agent view: Herdr GPUI keeps each host's own view order, the TUI applies the active host's view across hosts; and for a host whose connection is up but that is not online (a moment during a refused handshake or an outage): Herdr GPUI's panel ranks it as connected, while the TUI ranks it as stale |
| Mark unread | `src/client/shell/mark_unread.rs` | `crates/herdr-gpui/src/mark_unread.rs` | yes |
| Agent and workspace pickers | `src/client/shell/aggregate_navigation/agent_picker.rs`, `src/client/shell/overlay_input.rs` | `crates/herdr-gpui/src/picker/navigator.rs`, `crates/herdr-gpui/src/picker/navigator/rows.rs` | no: Herdr GPUI lacks the Tree layout, the second detail line, and the current marker, and under a plugin agent view its agent picker lists each host's own view where the TUI applies the active host's view across hosts |
| Back/Forward history | `src/client/shell/history.rs` | `crates/herdr-gpui/src/endpoint/history.rs` (upstream's), `crates/herdr-gpui/src/keymap/daemon.rs` (the fork's `history_back` and `history_forward` keys) | yes: the TUI ports upstream's per-host trail, except that after a pane opened from an activity-bar section, Back in Herdr GPUI first returns to that section, which the TUI does not have |
| Client-side keys stay local | `src/config/keybinds/client_actions.rs`, `src/client/shell/client_actions.rs` | `crates/herdr-gpui/src/keymap/daemon/client_actions.rs` | yes, except a direct key equal to the host profile's prefix key (the TUI runs the action, Herdr GPUI arms the prefix) and a local config that does not parse (the TUI keeps the previous five bindings, Herdr GPUI unbinds them) |
| Plugin popup selection | `src/client/shell/actions.rs` | `crates/herdr-gpui/src/palette.rs` | yes: both send the pane selection with a plugin key the daemon advertises as a popup |

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
- Keep fork code out of upstream-owned files: fork logic and its tests live in
  fork-added files or child modules, reached through as few hook lines in
  upstream files as possible. Do not rewrite or reorder upstream code you do
  not need to change, and never copy or wrap upstream code just to avoid
  touching it: a small edit in place beats a duplicate. An upstream test whose
  behavior the fork changes is edited in place.
  `src/client/shell/aggregate_navigation.rs` stays rewritten in two places
  for other fork features. Its rows drop upstream's `recency`: the priority
  order breaks ties between equal keys by the rows' endpoint and snapshot
  order, which its stable sort keeps, not by client-side recency. It drops the
  recency and presentation fields and the `seen` flag, and `seen()` reads the
  daemon's status, because the TUI's completion ledger is removed.
  `src/client/shell/overlay_input.rs` also stays rewritten: upstream's
  `open_navigator_overlay` becomes a layout-taking `open_navigator(layout)`,
  which the fork's agent and workspace pickers share, because a fork hook
  would copy upstream's overlay literal.

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

### Linux builds from CI

Every push to `custom` runs `.github/workflows/fork-build.yml`, which builds
the static Linux binaries and replaces the `custom-latest` prerelease with
them, so a Linux machine can fetch the fork build from
`https://github.com/gjermundgaraba/herdr/releases/download/custom-latest/herdr-linux-x86_64`
(or `-aarch64`). `SHA256SUMS` and `BUILD_INFO-*.txt` there name the commit,
the only way to tell fork builds apart. The rolling tag moves on every push,
so `git fetch --tags origin` refuses to update a stale local copy; delete it
with `git tag -d custom-latest`. Upstream's `ci.yml` does not run on
`custom`, so this workflow only builds; it runs no tests.

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

Plugin daemons such as Hub may need a restart after a server
handoff; follow their lifecycle docs in the `herdr-plugins` repository.
