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

## Next transport increment

This bootstrap does not yet start a control tunnel. The next increment adds a
dedicated relay-only contract over a separate HPNSSH or WSTunnel listener:

1. Foreign relays poll `/v1/nodes/<id>/relay-documents` and retain signed
   state envelopes for their cluster's domestic members.
2. They fan envelopes out to connected domestic nodes over selected management
   lanes.
3. Domestic nodes verify the API signature and monotonic revision before an
   apply, then acknowledge the revision over the control channel.
4. Customer listeners and customer tunnel lanes never carry this traffic.

The relay remains a cache and transport, never a configuration authority. It
cannot manufacture a desired-state revision because domestic nodes verify the
API signature locally.
