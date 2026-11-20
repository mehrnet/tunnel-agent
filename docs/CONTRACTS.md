# Tunnel Contracts

## Purpose

A tunnel contract describes one local process role with enough typed information
to safely compile its configuration and command line. It is not an inventory of
the whole cluster. The tunnel-agent API produces one manifest per node, so a foreign
node's WSTunnel client and a domestic node's WSTunnel server are different
contracts in different manifests.

This replaces the legacy pattern where every host parsed a global cluster,
inferred roles, launched unrelated workers, and required SSH topology discovery
or peer seeding before it could apply an update.

## Invariants

- IDs contain only letters, numbers, `_`, and `-`.
- Version 3 contracts reference a preinstalled artifact ID from the immutable
  `/opt/tunnel-agent/current` bundle. No lane activation downloads a binary.
- Version 2 source selection remains explicit through `node_scope`; there is no
  geography lookup or reachability inference.
- Every versioned bundle archive is SHA-256 checked before it is published;
  static mirrors serve its signed chunk manifest and every immutable chunk.
- Typed contracts claim local listeners. Two contracts claiming the same
  `bind:port` fail validation before anything starts.
- Every contract defaults to `"enabled": true`. A disabled contract retains its
  ID and topology history but is not compiled, does not need a local artifact,
  and claims no listener. This is the safe toggle/rollback mechanism for a lane.
- Legacy raw processes are opaque and remain only for migration. They cannot
  participate in listener-conflict detection.
- `run` never accesses the network. Legacy v2 `sync` is the only artifact-fetch
  command; v3 bundles deliberately disable it.
- `agent` polls only its own authenticated document, using an ETag and a
  60-second default interval. It accepts v3 manifests only, cannot change its
  bootstrap node scope or bundle root, validates a new revision before stopping
  the active plan, and retains the last working plan on fetch or validation
  failure.
- Owners publish a document through the Telegram bot's **Servers** menu. After
  selecting a server and **Apply manifest**, they attach a v3 JSON file. The
  Worker requires the target node's scope and bundle root, allows only known
  typed contracts, rejects inline credential fields and PEM private keys, and
  atomically advances the revision. This is an early safety boundary, not a
  replacement for the Rust compiler.

## Agent Control Lanes

`agent` is the only top-level object for tunnel-agent runtime behavior. It is
strictly separate from `tunnels`: a management lane never exposes customer
traffic, SOCKS, or a public listener. Version 3 manifests may declare a small,
ordered set of control lanes. The first implemented transport is an HPNSSH
foreign-to-domestic reverse forward:

```json
{
  "agent": {
    "config_poll_interval_secs": 5,
    "poll_interval_secs": 30,
    "max_active_lanes": 2,
    "lanes": [{
      "id": "8NdSLH0jy7Af3Gu9h",
      "transport": "hpnssh",
      "direction": "foreign_to_domestic",
      "role": "client",
      "priority": 10,
      "peer": {
        "node_id": "d6caf939-b959-4471-918f-2e05ca282a37",
        "host": "de-1.example.net",
        "port": 4443
      },
      "listener": {"bind": "127.0.0.1", "port": 42022},
      "hpnssh": {
        "cluster_id": "4cd57b8f-8f97-4f80-8791-90bba10190d0",
        "client": {
          "user": "root",
          "identity_file": "/opt/tunnel-agent/state/control-keys/cluster.pem",
          "known_hosts_file": "/opt/tunnel-agent/state/known-hosts/domestic",
          "known_host": "[ir-6.example.net]:42001 ssh-ed25519 BASE64HOSTKEY",
          "target": {"host": "127.0.0.1", "port": 42022}
        }
      }
    }]
  }
}
```

Lane IDs are exactly 17 base62 characters. The API's `agentLaneId` helper
derives them from the first 96 bits of SHA-256 over this canonical identity:
`v1\0cluster-uuid\0local-node-uuid\0peer-node-uuid\0transport\0direction`.
They are not labels such as `hpn-primary` or `ws-secondary`. The manifest
rejects public or privileged management listeners, duplicate enabled priorities,
and listener collisions with typed tunnel contracts. Every enabled lane is
eligible; after priority ordering, the first `max_active_lanes` are selected
and the rest remain validated standbys. This makes failover safe without
having to issue a new manifest during an outage.

The selected `hpnssh` lane starts two foreign processes: a loopback-only seed
server and the pinned HPNSSH client. Its domestic peer starts only the
restricted `hpnsshd`. The agent rejects a management listener collision with a
typed customer tunnel before either process is started. `wstunnel`, `rathole`,
and `trusttunnel` remain declared but unavailable for agent lanes until their
role-specific management contracts have equivalent isolation and verification.

## Peer-Seeded Bundles

Release updates use a separate, loopback-only HTTP protocol intended to run
inside an established agent lane. It is not an HTTP service for customers and
must never bind a public address.

1. The release archive includes `bundle-index.json`, with each payload file's
   path, SHA-256, exact size, and Unix mode. The index excludes itself to avoid
   a recursive digest.
2. Publication registers the archive SHA-256 and index SHA-256. Only an
   enrolled foreign node can request the API-signed release document.
3. `seed-serve` verifies that its current release matches the signed document,
   then exposes `/v1/release`, `/v1/index`, `/v1/blob/<sha256>`, a signed
   per-domestic-node `/v1/relay/<node-id>` document, and
   `/v1/ping/<node-id>` on a loopback listener. A valid ping records a local
   foreign-relay timestamp for that domestic node. The relay can serve signed
   config and pings even if a release cannot be served. It never serves
   arbitrary paths.
4. A domestic node accepts only a signature made by the compiled API public
   key. It checks the index hash, release version, architecture, every blob's
   size and SHA-256, and every staged file before promotion.
5. The domestic content cache is keyed by SHA-256. Unchanged files are reused
   from the cache or the current indexed bundle, so a peer transfer contains
   only new content.

The domestic `agent.bundle_seed` setting automates polling after an agent lane
maps the foreign seed service to the declared local URL:

```json
{
  "bundle_seed": {
    "source": "http://127.0.0.1:42022",
    "poll_interval_secs": 300
  }
}
```

`source` must be exactly an unprivileged `127.0.0.1` or `[::1]` HTTP listener;
the validator rejects public hosts, HTTPS URLs, paths, credentials, and
privileged ports. It is valid only for a domestic v3 manifest. The offline
domestic agent does not contact the API: after a verified changed release is
activated, it stops its contracts and exits successfully, allowing its
`Restart=always` systemd service to start the new binary.

Desired config polling is independent of bundle updates and telemetry.
`agent.config_poll_interval_secs` defaults to 5 seconds on both types of node
and accepts integers from 5 to 3600. Foreign relays check both their own
document and their authorized domestic snapshots; unchanged responses are
conditional. Domestic agents check their signed document through their
configured management lane only. Keep the optional field out of documents
delivered to agents that predate it. `agent.poll_interval_secs` controls
heartbeat and telemetry (60 seconds by default), and the domestic bundle poll
remains at 300 seconds by default.

## Managed Reverse Tunnels

The Telegram owner editor can create WSTunnel, HPNSSH, and Rathole lanes.
The listening node receives the loopback listener; the connector dials the
control port and reaches the configured destination. HPNSSH supports fixed
TCP and SOCKS5. Rathole supports fixed TCP over mutually authenticated Noise.

For HPNSSH and Rathole, the agent generates transport identities locally and
reports only public keys. The API requires recent keys from both nodes before
activation and pins the peer identity on each side. The HPNSSH listener uses
the bundled daemon with a forwarding-only authorization policy. The root
agent prepares the bundled daemon's unprivileged `hpnsshd` account if needed.
No per-lane systemd unit, uploaded SSH key, or hand-written config is required.

Managed drafts are reserved without starting a listener. Enabling, disabling,
restarting, changing the destination, and removing a lane use paired desired
documents. Port claims cover both the control and loopback ports. Transport
keys are separate from the management-lane credentials. Nodes also need
network reachability to the selected control port, including any provider
firewall rules.

The local listener probe checks reachability, not an application transaction.
Public ingress mapping and switching a public port between lanes are not
part of this contract.

## HPNSSH

`hpnssh.forward.v1` runs an HPNSSH client. `direction` means the SSH forwarding
primitive, not a vague geographic label:

- `remote_socks`: `hpnssh -R bind:port peer`; the peer gets a SOCKS listener.
- `remote_tcp`: `hpnssh -R bind:port:target_host:target_port peer`.
- `local_socks`: `hpnssh -D bind:port peer`; this node gets a local SOCKS
  listener.

Strict host-key checking is enabled by default. A strict contract requires a
specific `known_hosts_file`; it never accepts a changed remote key during a live
lane. `hpnssh.server.v1` is optional for an already bootstrapped HPN SSHD
configuration. Host keys, users, and privilege-separation setup remain a
host-baseline artifact.

```json
{
  "contract": "hpnssh.forward.v1",
  "id": "tr_to_ir_hpn",
  "artifact": "hpnssh",
  "direction": "remote_socks",
  "peer": {
    "host": "ir-1.example",
    "user": "tunnel",
    "port": 2222,
    "identity_file": "/etc/tunnel-agent/keys/tr_to_ir",
    "known_hosts_file": "/etc/tunnel-agent/known_hosts"
  },
  "listener": { "bind": "127.0.0.1", "port": 10810 }
}
```

## WSTunnel

WSTunnel has two roles. The domestic/server role owns its TLS listener; a
foreign/client role opens a reverse SOCKS or fixed TCP mapping through the WSS
endpoint. These roles must not claim the same local port on one host.

```json
{
  "contract": "wstunnel.reverse.v1",
  "id": "tr_to_ir_wstunnel",
  "artifact": "wstunnel",
  "endpoint": {
    "url": "wss://ir-1.example:4444",
    "tls_sni": "tunnel.example",
    "path_prefix": "/control/tunnel",
    "verify_certificate": true
  },
  "remote_listener": {
    "scheme": "socks5",
    "bind": "127.0.0.1",
    "port": 10910
  }
}
```

For a fixed TCP destination, use `scheme: "tcp"` with a `target` object.

## Rathole

Rathole is a service-forwarding transport. Its server has the public control
listener and service listeners. Its client connects to the server and binds each
service to a local target. Every service has its own required token.

The first contract revision intentionally renders only Rathole TCP transport.
Rathole TLS and Noise require complete certificate/key and trust contracts;
accepting partial TLS/Noise settings would generate a configuration that appears
valid but cannot safely establish a lane.

Server:

```json
{
  "contract": "rathole.service.v1",
  "id": "de_rathole_server",
  "artifact": "rathole",
  "role": "server",
  "control": { "bind": "0.0.0.0:2333" },
  "services": [
    { "id": "xhttp", "token": "secret-from-api", "bind": "0.0.0.0:8443" }
  ]
}
```

Client:

```json
{
  "contract": "rathole.service.v1",
  "id": "ir_rathole_client",
  "artifact": "rathole",
  "role": "client",
  "control": { "remote": "de.example:2333" },
  "services": [
    { "id": "xhttp", "token": "secret-from-api", "local": "127.0.0.1:48443" }
  ]
}
```

The agent renders a private TOML file under its state directory and invokes
Rathole in explicit `--server` or `--client` mode.

## TrustTunnel

TrustTunnel is an HTTPS-shaped VPN endpoint, not a generic reverse forwarder.
`trusttunnel.endpoint.v1` starts its endpoint binary with prepared main settings
and TLS host files. Credentials, client rules, certificates, and optional
subscription configuration remain protected inputs supplied by the tunnel-agent
API.
The contract makes endpoint lifecycle declarative without misrepresenting it as
a substitute for HPNSSH, WSTunnel, or Rathole lanes.

```json
{
  "contract": "trusttunnel.endpoint.v1",
  "id": "trusttunnel_de",
  "artifact": "trusttunnel_endpoint",
  "settings_file": "/etc/tunnel-agent/trusttunnel/vpn.toml",
  "tls_hosts_file": "/etc/tunnel-agent/trusttunnel/hosts.toml",
  "jobs": 2,
  "log_level": "info"
}
```

## Rollout model

1. Build domestic and foreign mirror entries for every immutable release bundle
   with an exact version, archive checksum, file inventory checksum, manifest,
   and complete chunk set.
2. Install the static agent bundle from the mirror selected for the node class.
3. Deploy a v3 manifest containing only an additional canary lane to the
   owning node's desired document.
4. The local `agent` fetches the new revision, runs the same validation and
   plan compilation before stopping its existing contracts, then atomically
   applies the new managed manifest.
5. Run the canary beside the legacy tunnel on distinct ports and collect health,
   CPU, memory, reconnect, and throughput measurements.
6. Move one production listener only after the canary meets a rollback threshold.
   Retain a disabled legacy definition until the observation window passes.
7. Enable peer-seeded updates only after management-lane isolation, signed
   document delivery, cache reuse, rollback, and recovery tests pass. Retire
   legacy deployment seeding only after peer and mirror recovery both pass.
