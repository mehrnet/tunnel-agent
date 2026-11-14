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

## Next protocol increment

This bootstrap does not yet start a control tunnel. The next increment adds a
dedicated relay-only contract over a separate HPNSSH or WSTunnel listener:

1. Foreign relays poll and verify API revisions.
2. They retain signed state envelopes and fan them out to connected domestic
   nodes.
3. Domestic nodes verify the API signature and monotonic revision before an
   apply, then acknowledge the revision over the control channel.
4. Customer listeners and customer tunnel lanes never carry this traffic.

The relay remains a cache and transport, never a configuration authority. It
cannot manufacture a desired-state revision because domestic nodes verify the
API signature locally.
