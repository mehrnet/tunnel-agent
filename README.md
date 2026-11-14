# tunnel-agent

`tunnel-agent` is a low-footprint Rust supervisor for declared tunnel lanes.
It installs one immutable transport bundle under `/opt/tunnel-agent` and starts
only the lanes selected in its local manifest. It does not SSH into hosts,
discover topology, seed files peer-to-peer, or download a binary when a lane is
toggled.

The product has two public origins and one installed binary:

```text
https://tunnel-api.mehrnet.com/api/*
https://tunnel-files.mehrnet.com/*
tunnel-agent (the binary installed on each node)
```

- `tunnel-api.mehrnet.com` is a Cloudflare Worker: Telegram webhook,
  ownership, clusters, enrollment, immutable bundle metadata, and node
  documents in D1. It intentionally serves no website or release assets.
- `tunnel-files.mehrnet.com` is an assets-only Cloudflare deployment: website,
  installer, versioned bundle manifests, and immutable archive chunks.

## Static bundle

A release archive contains every supported data-plane dependency for one Linux
architecture:

```text
/opt/tunnel-agent/current/
├── bin/tunnel-agent
├── bin/wstunnel
├── bin/rathole
├── bin/trusttunnel_endpoint
└── hpnssh/
    ├── bin/hpnssh
    ├── sbin/hpnsshd
    └── libexec/
```

Cloudflare static assets have a 25 MiB object limit, so a bundle is published
as a small `.tar.gz.parts` manifest and immutable sub-25 MiB archive chunks.
The installer verifies each chunk, reconstructs the archive, and verifies the
archive SHA-256 received from the Worker before it publishes
`/opt/tunnel-agent/releases/<version>` and atomically advances the `current`
symlink. A tunnel toggle therefore only changes process selection; it never
blocks on an artifact download.

Domestic nodes receive an Iranian mirror URL first, then the static site as a
verified fallback. Foreign nodes receive the static site directly. An Iranian
mirror may serve either the same versioned manifest plus every referenced
chunk, or one full versioned `.tar.gz` release archive. Both forms are checked
against the final archive hash from the Worker.

## Contracts

| Contract | Role | Purpose |
| --- | --- | --- |
| `hpnssh.forward.v1` | SSH initiator | HPNSSH remote SOCKS, remote TCP, or local SOCKS lane. |
| `hpnssh.server.v1` | SSH server | Runs a prepared HPN SSHD configuration. |
| `wstunnel.server.v1` | WSTunnel server | TLS WSS listener with a restricted path prefix. |
| `wstunnel.reverse.v1` | WSTunnel client | Reverse SOCKS or fixed-TCP listener over WSS. |
| `rathole.service.v1` | Rathole server/client | Token-authenticated TCP service forwarding. |
| `trusttunnel.endpoint.v1` | TrustTunnel endpoint | VPN endpoint with supplied settings and TLS host files. |

Full role and configuration details are in [docs/CONTRACTS.md](docs/CONTRACTS.md).

## Manifest versions

- Version 1 is the legacy raw-process format.
- Version 2 supports independently fetched, SHA-256-pinned artifacts during
  migration only.
- Version 3 is the production bundle format. It requires `bundle.root`, allows
  typed contracts only, and resolves known transport IDs from the installed
  `/opt/tunnel-agent/current` release.

```json
{
  "version": 3,
  "node_scope": "domestic",
  "bundle": { "root": "/opt/tunnel-agent/current" },
  "tunnels": []
}
```

## Telegram ownership flow

`@TunnelOwnersBot` is the owner-facing control surface:

```text
/cluster_add w4r-canary
/server_add w4r-canary domestic ir-6 2.144.4.192
/server_add w4r-canary foreign de-1 138.199.236.120
/install ir-6
```

A server can be attached to multiple clusters, but it must belong to at least
one before `/install` can create a single-use enrollment grant. The API stores
membership independently from the node record so the compiler can reject every
host-level listener collision across all of that node's clusters.

The bot token and webhook secret are Cloudflare Worker secrets. Do not place
either in `wrangler.jsonc`, release assets, or Git history.

## Commands

```sh
tunnel-agent validate --config /etc/tunnel-agent/manifest.json
tunnel-agent plan --config /etc/tunnel-agent/manifest.json
tunnel-agent run --config /etc/tunnel-agent/manifest.json
```

`sync` remains available only for legacy v2 manifests. It intentionally fails
for v3 bundles because production lanes are preinstalled.

## Development

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --locked

cd tunnel-api
npm install
npm run check
```

Tags matching `v*` build musl `amd64` and `arm64` bundles in GitHub Actions.
Use `tools/publish.sh` to split a release into immutable chunks, publish them
to `tunnel-files.mehrnet.com`, and activate the matching bundle metadata
through `tunnel-api.mehrnet.com`.

The complete Cloudflare, bundle mirror, Telegram, and W4R canary sequence is in
[docs/BOOTSTRAP.md](docs/BOOTSTRAP.md).

## Repository layout

| Path | Responsibility |
| --- | --- |
| `tunnel-agent/` | Rust node runtime and manifest compiler. |
| `tunnel-api/` | Cloudflare Worker, D1 migrations, and Telegram bot. |
| `tunnel-files/` | Assets-only Cloudflare configuration and static installer/bundles. |
| `tools/` | Release assembly and static bundle publication tooling. |
| `docs/` | Protocol contracts, API operations, and bootstrap runbooks. |
