# tunnel-agent

`tunnel-agent` is a low-footprint Rust supervisor for declared tunnel lanes.
It installs one immutable transport bundle under `/opt/tunnel-agent` and starts
only the lanes selected in its local manifest. It does not SSH into hosts,
discover topology, seed files peer-to-peer, or download a binary when a lane is
toggled.

The product has one public origin:

```text
https://tunnel-agent.mehrnet.com/
```

- `/api/*` is a Cloudflare Worker: Telegram webhook, ownership, clusters,
  enrollment, immutable bundle metadata, and node documents in D1.
- Every other path is a static asset: website, installer, versioned bundles,
  and checksums.

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

The installer verifies the archive SHA-256 received from the Worker before it
publishes `/opt/tunnel-agent/releases/<version>` and atomically advances the
`current` symlink. A tunnel toggle therefore only changes process selection;
it never blocks on an artifact download.

Domestic nodes receive an Iranian mirror URL first, then the static site as a
verified fallback. Foreign nodes receive the static site directly. Mirrors
always serve the same versioned archive and hash.

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

cd distribution
npm install
npm run check
```

Tags matching `v*` build musl `amd64` and `arm64` bundles in GitHub Actions.
The static-site artifact is named `tunnel-agent-static-assets-vX.Y.Z`.
