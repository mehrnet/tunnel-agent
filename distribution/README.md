# Static distribution assets

Publish this directory to the Cloudflare ASSETS site behind
`tunnel-agent.mehrnet.com`. Keep assets immutable by release path:

```text
/install.sh
/releases/v0.2.0/tunnel-agent-x86_64-unknown-linux-musl.tar.gz
/releases/v0.2.0/tunnel-agent-aarch64-unknown-linux-musl.tar.gz
```

The public site is a transport mirror, not the source of truth. MehrControl
pins artifact URLs and SHA-256 values in each desired-state document. A domestic
provider mirror may be listed first for `node_scope: domestic`, with this
Cloudflare path as a lower-priority `any` fallback.

Do not publish a mutable `latest` binary for automated installations. A future
`latest.json` can be an advisory catalog only; every node still receives an
exact version and checksum from desired state.
