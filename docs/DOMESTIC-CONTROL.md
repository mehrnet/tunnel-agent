# Domestic Control Bootstrap

Domestic nodes must not depend on `tunnel-api.mehrnet.com` after a bootstrap
command has been issued. The command carries a signed, opaque `ta1` base62
capsule. Its payload is minified JSON but the installer and runtime validate it
before using any mirror, manifest, or relay value.

## Current implementation

- The Worker signs each domestic capsule with an Ed25519 private key stored as
  `CAPSULE_SIGNING_PRIVATE_KEY`.
- The installer embeds only the corresponding public key, verifies the
  signature before downloading a bundle, then checks every archive hash.
- `tunnel-agent bootstrap-capsule` validates the same capsule again and writes
  the domestic manifest, node ID, revision, relay inventory, and a restricted
  `cluster-control-authorized_keys` file.
- The cluster key is an Ed25519 SSH public-key body, without the OpenSSH
  `ssh-ed25519` prefix. Its private key is encrypted with
  `CLUSTER_CONTROL_KEY_ENCRYPTION_KEY` before D1 storage.
- The domestic agent starts with `--offline`; it has no node token or API URL.

The first relay inventory is built from foreign nodes in the domestic node's
single cluster membership. A domestic offline bootstrap currently rejects zero
or multiple clusters because a single local control trust set must be
unambiguous.

## Signed relay documents

The API now issues a signed relay envelope only to an enrolled foreign member
of the same cluster. An envelope binds one domestic node's cluster UUID, node
UUID, monotonic document revision, and base64url manifest. It is signed by the
same pinned API Ed25519 key used by the bootstrap capsule.

The `tunnel-agent verify-relay-document` command verifies the envelope against
the local `relay-state.json` and node ID without calling the API or applying a
change. This makes the state format independently testable before it is sent
over a management lane. It intentionally has no write or restart behavior.

The first envelope format is signed, not end-to-end encrypted. A foreign relay
is therefore a trusted member of the cluster's control plane and may only carry
declarative manifests with no inline secret values. The future document
compiler must reject inline secrets for relayed domestic documents. Per-node
sealed envelopes are required before the control plane carries secret material.

## Management lane

The first live management transport is a dedicated HPNSSH reverse forward. It
does not reuse legacy HPN listeners, SOCKS ports, WSTunnel endpoints, or any
customer tunnel process.

1. A domestic `agent` lane starts a dedicated `hpnsshd` listener, currently on
   an explicitly declared high port such as `42001`. It accepts only the
   cluster control key, disables shells/passwords/PTYs, and permits remote TCP
   forwarding only.
2. A selected foreign lane obtains the encrypted cluster private key through
   its authenticated API credential, pins the domestic listener host key, and
   opens an HPNSSH `-R` forward. The domestic loopback seed port, for example
   `127.0.0.1:42022`, then reaches the foreign loopback seed service.
3. The foreign seed service exposes only signed release metadata, verified
   indexed bundle blobs, and the signed relay document for the requesting
   domestic node. It is never public.
4. The domestic offline agent polls `127.0.0.1:42022` only. It verifies the
   API signature, node binding, and monotonic revision before applying a
   manifest. It verifies every bundle digest before atomically activating a
   self-update and restarting itself.

The foreign relay remains a cache and transport, never a configuration
authority. It cannot manufacture a desired-state revision because domestic
nodes verify the API signature locally. The current cluster-wide SSH key is a
bootstrap compatibility boundary; replace it with rotated per-foreign-node
keys before allowing untrusted foreign membership.

### Relay-authorized renewal

An enrolled foreign member may request `POST
/api/v1/nodes/<foreign-node-id>/clusters/<cluster-id>/domestic-nodes/<domestic-node-id>/capsule`
using its node bearer credential. The API permits this only when the caller is
a foreign member and the active domestic node is the sole domestic member of
that exact cluster. It returns a short-lived, API-signed bootstrap capsule and
records an audit event. This endpoint exists for operator-driven recovery of a
domestic node without putting an API credential on that node; it cannot modify
the domestic desired document.
