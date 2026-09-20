# herdr-frontend

Rust client for the per-TUI frontend socket served by this fork (protocol 7).
It ships in the same repository as the socket so the two stay lockstep; there
is no fallback to older protocols. Unix only.

```toml
[dependencies]
herdr-frontend = { git = "https://github.com/gjermundgaraba/herdr", branch = "custom-v3" }
```

`FrontendClient` connects to one TUI's socket: `from_env()` reads
`HERDR_FRONTEND_SOCKET`, `connect(path)` takes an explicit socket, and
`discover(&directory())` lists the owner-only sockets in the TUI socket
directory. The methods are `snapshot`, `subscribe`, `navigate`, `input`, and
`call`; `Snapshot::route(endpoint_id)` yields the `Route` a mutation needs.
`attention_order` sorts agents by how much they deserve a glance. The socket
contract itself is documented in
`docs/next/website/src/content/docs/frontend-api.md`.

```sh
cd sdk/frontend && cargo test
```
