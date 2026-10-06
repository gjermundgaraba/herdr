# Fork context

This checkout is `gjermundgaraba/herdr`, a thin personal fork of
`herdrdev/herdr`. It exists to support Herdr plugins that need things stock
Herdr does not offer. Machine-specific paths live in `FORK.local.md`, which is
gitignored and may be absent.

## Remotes and branches

- `origin`: `gjermundgaraba/herdr`
- `upstream`: `herdrdev/herdr`
- `master`: an unmodified mirror of upstream `master`
- `custom-v3`: the branch that gets built and run: the latest upstream release
  tag plus a short stack of local commits
- `custom`, `custom-v2`, `backup/*`, `temp/*`: historical; do not modify or
  delete them

List the local changes with:

```sh
git log --oneline "$(git describe --tags --abbrev=0 --match 'v*' custom-v3)..custom-v3"
```

## What the fork adds

- The per-TUI frontend socket and its protocol, documented in
  `docs/next/website/src/content/docs/frontend-api.md`
- The `herdr-frontend` Rust client crate in `sdk/frontend`
- Client-side `[keys]` actions: agent and workspace lists, Back/Forward
  history, and the unread hold
- The `agent.prompt` method in the client command lane
  (`CLIENT_SHELL_METHODS` in `src/server/client_commands.rs`)
- Space groups: the `space_group` workspace metadata token puts runs of
  spaces under collapsible, draggable sidebar headers
  (`src/client/shell/space_groups.rs`)
- Agent priority tokens: `ui.agent_priority_tokens` lists workspace metadata
  token keys, and a space carrying any of them with a non-empty value lifts
  its blocked and done agents to the top of the `priority` agent order; the
  agent picker shows the first listed token's value. That order also queues
  blocked and done agents oldest state change first (`src/agent_priority.rs`)
- Space context menu items: plugin actions with the `workspace` context appear
  in a space's right-click menu and run against the clicked space
  (`src/app/api/plugins/workspace_actions.rs`)
- `type = "plugin"` keybindings run any manifest entrypoint, opening panes
  directly with the originating pane and selection as context. Actions and
  panes share one id namespace and one resolver
  (`src/app/api/plugins/entrypoint.rs`, `keybind.rs`)
- Plugin invocation context: menu, key, and API targets supply the context
  instead of focus; actions, panes, and startup hooks also get the invoking
  TUI's `client_id` (`HERDR_CLIENT_ID`), the target pane's agent session, the
  space's tabs, and every space (`src/app/api/plugins/context.rs`)
- Smaller sidebar and navigation fixes; see the log above

The main consumers are the Micro and Deck plugins in the `herdr-plugins`
repository. They depend on `herdr-frontend` as a git dependency on the
`custom-v3` branch of `origin`, so their `Cargo.lock` pins a pushed fork
commit.

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
  `git rebase --onto vNEW vOLD custom-v3`; a plain `git rebase vNEW` replays
  upstream's old release commits too. Never merge `master` into the stack.
- A clean rebase does not prove compatibility. If an upstream change interacts
  with a customization, changes existing behavior, or leaves you unsure whether
  a customization still works, stop and ask the user. Do not remove, adapt, or
  disable a customization on your own.
- Push only when asked, and use `git push --force-with-lease origin custom-v3`.
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

`just ci` does not build `sdk/frontend`, which sits outside the Cargo
workspace. When it or the frontend protocol changes, or after a rebase, also
run `cargo test --locked` in `sdk/frontend`.

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

## After changing `sdk/frontend` or the frontend protocol

Plugins only see a change after it is pushed to `origin/custom-v3`. Then run
`cargo update -p herdr-frontend` in the `herdr-plugins` repository and rebuild
the affected plugins by following that repository's `AGENTS.md`. Plugin daemons
such as Micro and Hub may need a restart after a server handoff; follow their
lifecycle docs.
