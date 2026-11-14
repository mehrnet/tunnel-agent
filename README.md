# tunnel-agent

`tunnel-agent` is a low-footprint Rust supervisor for external tunnel binaries. It is intentionally not a proxy, protocol implementation, or control plane. It receives a declarative process manifest and keeps those processes running with bounded backoff.

It is designed to replace the Python *control plane* of the legacy HPN/Jet/WSTunnel deployment, not the HPN or WSTunnel data planes themselves. Keeping those binaries external preserves their existing wire behavior.

## Resource model

- One `current_thread` Tokio reactor per agent process, managing every child on the host.
- Child exit notifications are asynchronous. There is no one-thread-per-lane model and no polling loop.
- Process standard streams are sent directly to `/dev/null`; the supervisor never buffers child output.
- No periodic `curl`, `ssh`, `systemctl`, or shell probes. Health policy belongs to MehrControl and is expressed through its desired-state document.
- Every failed child uses bounded exponential backoff. A broken peer cannot create a fork storm.

The agent cannot eliminate resources used by its children. For example, a foreign WSTunnel node with five reverse workers still has five WSTunnel processes and their TLS buffers. It replaces only the Python manager and lane guard overhead.

## Manifest

The manifest is generic: no cluster, protocol, port, or vendor is built into this binary.

```json
{
  "version": 1,
  "processes": [
    {
      "id": "wstunnel-reverse-ir-1",
      "argv": [
        "/usr/local/bin/wstunnel",
        "client",
        "--remote-to-local",
        "socks5://127.0.0.1:10910",
        "wss://198.51.100.10:4444"
      ],
      "env": {},
      "restart": {
        "initial_backoff_ms": 1000,
        "max_backoff_ms": 60000,
        "reset_after_secs": 60
      }
    }
  ]
}
```

```sh
tunnel-agent validate --config /etc/tunnel-agent.json
tunnel-agent run --config /etc/tunnel-agent.json
```

Configuration changes are applied by atomically writing the manifest then restarting this agent through systemd. A later protocol revision will add in-place reload after the canary proves the simpler restart path stable.

## MehrControl integration

`control-node` remains the generic node agent. It downloads this binary as an artifact, writes the compiled manifest, and declares `tunnel-agent` as a daemon process. `control-api` owns topology, secrets, health policy, rollouts, and process arguments.

This separation is deliberate:

- `control-node`: generic artifact, file, process, telemetry, and sync primitives.
- `tunnel-agent`: tiny local child-process supervisor.
- HPNSSH/WSTunnel: data-plane binaries.
- `control-api`: desired state and policy.

## Releases

Tags matching `v*` produce private GitHub Release assets for:

- `x86_64-unknown-linux-musl`
- `aarch64-unknown-linux-musl`

Each binary is statically linked and accompanied by a SHA-256 file. `--version` uses the Git tag injected by the release workflow.

## Development

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release
```
