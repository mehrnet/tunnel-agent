# tunnel-agent

`tunnel-agent` is a small Rust runtime for declared tunnel contracts. It replaces
the Python control loop around the existing HPNSSH, Jet, and WSTunnel deployment
without reimplementing any tunnel protocol or changing an existing wire shape.

The critical ownership boundary is:

- A cluster owner chooses a transport contract for each lane and node role.
- MehrControl compiles a node-specific manifest.
- `tunnel-agent` validates it, explicitly installs pinned artifacts from ordered
  mirrors, renders contract-owned configuration, and supervises the processes.
- A running agent never polls peers, calls SSH/systemd, or downloads updates.

Legacy v1 raw `processes` manifests remain supported for staged migration.

## Transport contracts

| Contract | Role | Purpose |
| --- | --- | --- |
| `hpnssh.forward.v1` | SSH initiator | HPNSSH remote SOCKS, remote TCP, or local SOCKS lane. |
| `hpnssh.server.v1` | SSH server | Runs a prepared `hpnsshd` configuration. |
| `wstunnel.server.v1` | WSTunnel server | TLS WSS listener with a restricted path prefix. |
| `wstunnel.reverse.v1` | WSTunnel client | Reverse SOCKS or reverse fixed-TCP listener over WSS. |
| `rathole.service.v1` | Rathole server/client | Token-authenticated TCP service forwarding. |
| `trusttunnel.endpoint.v1` | TrustTunnel endpoint | VPN endpoint with supplied settings and TLS host files. |

TrustTunnel is an endpoint contract, not a generic inter-node reverse tunnel.
Rathole has distinct server and client roles. Full role and configuration details
are in [docs/CONTRACTS.md](docs/CONTRACTS.md).

## Artifact mirrors

Every artifact is versioned and SHA-256 pinned. Mirrors are explicitly scoped as
`domestic`, `foreign`, or `any`; the agent never guesses a server's geography.
A domestic node can therefore prefer a provider-hosted Iranian source while
retaining `tunnel-agent.mehrnet.com` as a verified global fallback.

```json
{
  "id": "wstunnel",
  "version": "11.3.0",
  "binary": "wstunnel",
  "package": "binary",
  "sha256": "<64-hex-package-sha256>",
  "sources": [
    {
      "url": "https://iran-mirror.example/wstunnel-11.3.0-amd64",
      "scope": "domestic",
      "priority": 10
    },
    {
      "url": "https://tunnel-agent.mehrnet.com/artifacts/wstunnel/11.3.0/linux-amd64",
      "scope": "any",
      "priority": 100
    }
  ]
}
```

`sha256` covers the downloaded package. For a `tar_gz` package,
`binary_sha256` can additionally pin the extracted binary. Files are installed
atomically under `/var/lib/tunnel-agent/artifacts/<id>/<version>/`.

## Commands

```sh
# No network and no process start.
tunnel-agent validate --config /etc/tunnel-agent.json
tunnel-agent plan --config /etc/tunnel-agent.json

# Explicit artifact installation. This is the only agent command that accesses mirrors.
tunnel-agent sync --config /etc/tunnel-agent.json

# Render contract-owned files and supervise already-installed artifacts.
tunnel-agent run --config /etc/tunnel-agent.json
```

`distribution/install.sh` is a small POSIX bootstrap intended for static hosting
at `https://tunnel-agent.mehrnet.com/install.sh`. It requires an out-of-band
package SHA-256 and accepts ordered source URLs. It never fetches an unverified
`latest` binary.

## Resource model

- One single-thread Tokio runtime supervises every child on a host.
- No one-thread-per-lane model, polling loop, shell probe, or peer seed task.
- Child output is discarded by default; protocol logs remain protocol-owned.
- Failed child processes use bounded exponential backoff.
- It cannot remove HPNSSH/WSTunnel/Rathole data-plane cost. It replaces the
  Python manager and lane guard overhead and removes their continual probes.

## MehrControl integration

`control-node` remains the generic node agent. It obtains this static artifact,
writes the node-specific manifest and protected inputs, runs `tunnel-agent sync`,
then manages `tunnel-agent run` as a daemon. `control-api` owns topology,
secrets, health policy, rollout gates, and mirror selection. The runtime remains
usable without MehrControl through plain local JSON and files.

## Releases

Tags matching `v*` build private GitHub Release assets for Linux `amd64` and
`arm64`, statically linked with musl. `--version` contains the Git tag injected
by CI. The Cloudflare static site mirrors exact immutable release assets; it
must not replace desired-state pinning with a mutable binary URL.

## Development

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --locked
cargo build --release
```
