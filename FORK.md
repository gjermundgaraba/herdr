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

Nothing in the code, for now. The stack ends in a commit whose subject starts
with `revert: return custom to stock`, which returns the code, tests, and docs
to the release tag. The tree differs from the tag only in this file, the
pointer to it in `AGENTS.md`, the `FORK.local.md` entry in `.gitignore`, and
the Linux build workflow (see "Linux builds from CI"). The plugins in the
`herdr-plugins` repository need the fork's features, so they are disabled.

The earlier features, and how they were split between this fork and Herdr GPUI,
are in the history before that commit:

```sh
git show "$(git log --format=%h -1 --grep='^revert: return custom to stock' custom)^:FORK.md"
```

## Sibling fork: Herdr GPUI

`gjermundgaraba/herdr-gpui` on branch `custom` is the user's fork of the native
client, which attaches to this fork's daemon. It is back to stock as well. Its
path is in `FORK.local.md`. When a feature returns, restore the contract,
mirrored-client, and parity-table rules from the earlier `FORK.md` along with
it. Herdr GPUI is a separate project, and a change there is a separate task the
user asks for, never a side effect of work in this repository.

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
- While the code is stock, a new release needs no rebase: start `custom` again
  from the new tag with only the files named under "What the fork adds", and
  move the old branch to `backup/custom-<date>`.
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

Plugin daemons restart through their startup hook after a server handoff;
their lifecycle docs are in the `herdr-plugins` repository.
