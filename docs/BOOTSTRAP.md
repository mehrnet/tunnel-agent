# tunnel-agent Bootstrap Runbook

This runbook publishes static tunnel files, Worker API, complete
transport bundles, Telegram owner bot, and the first W4R canary. It does not
replace a production HPNSSH/Jet lane until the canary has met its health and
throughput thresholds.

## 1. Create the Cloudflare state

From `tunnel-api/`, create a D1 database and place the returned database ID
in `wrangler.jsonc`:

```sh
npx wrangler d1 create tunnel-agent
npm install
npm run db:migrate:remote
```

Set encrypted secrets. Neither value belongs in `wrangler.jsonc`, Git, a
release artifact, or shell history:

```sh
npx wrangler secret put TELEGRAM_BOT_TOKEN
npx wrangler secret put TELEGRAM_WEBHOOK_SECRET
npx wrangler secret put CAPSULE_SIGNING_PRIVATE_KEY
npx wrangler secret put CLUSTER_CONTROL_KEY_ENCRYPTION_KEY
```

Set `DOMESTIC_BOOTSTRAP_URL` as a Worker secret to the Iranian mirror copy of
`install.sh`, then deploy the API Worker and attach
`tunnel-api.mehrnet.com`:

```sh
npx wrangler secret put DOMESTIC_BOOTSTRAP_URL
npm run deploy
npm run files:deploy
```

The API Worker only handles `/api/*`. Deploy `tunnel-files/wrangler.jsonc`
separately to attach static files to `tunnel-files.mehrnet.com`.

The renamed `tunnel-api` Worker needs its own encrypted secrets; Cloudflare
does not transfer secret values from the former Worker name. Retire the former
`tunnel-agent.mehrnet.com` custom domain only after the new API, files domain,
and Telegram webhook have been verified.

## 2. Publish a bundle and its mirrors

Every tag matching `v*` produces two release archives and an artifact named
`tunnel-agent-static-assets-vX.Y.Z`. Download that artifact and deploy its
contents with Wrangler so the global static site serves:

```text
/install.sh
/bundles/vX.Y.Z/tunnel-agent-vX.Y.Z-x86_64-unknown-linux-musl.tar.gz.parts
/bundles/vX.Y.Z/tunnel-agent-vX.Y.Z-x86_64-unknown-linux-musl.tar.gz.part-0000
/bundles/vX.Y.Z/tunnel-agent-vX.Y.Z-aarch64-unknown-linux-musl.tar.gz.parts
/bundles/vX.Y.Z/tunnel-agent-vX.Y.Z-aarch64-unknown-linux-musl.tar.gz.part-0000
```

Cloudflare static assets are limited to 25 MiB, so each `.parts` manifest lists
the SHA-256, byte count, and immutable chunk filenames for one archive. Copy
the exact `install.sh`, each `.parts` manifest, and every chunk named by that
manifest to the Iranian mirror. Alternatively, the Iranian mirror may serve
one full immutable versioned `.tar.gz` release archive. A domestic bundle URL
must identify either form and contain the same final archive SHA-256 as its
global counterpart.

Use the release activation script rather than hand-writing D1 SQL. It validates
both generated `.sha256` files, deploys the manifest and chunk assets first,
and then atomically marks one archive active per architecture:

```sh
BUNDLE_VERSION=vX.Y.Z \
BUNDLE_SOURCE_DIR=/path/to/downloaded-release \
BUNDLE_PUBLISH_TOKEN=... \
DOMESTIC_BUNDLE_BASE_URL=https://iran-mirror.example/tunnel-agent/vX.Y.Z \
DOMESTIC_BUNDLE_FORMAT=archive \
./tools/publish.sh
```

`BUNDLE_PUBLISH_TOKEN` is a Worker secret. The publisher deploys static assets
first, then invokes the narrow `/api/v1/admin/bundles/activate` endpoint. That
endpoint only accepts the two expected architecture manifest paths for one
version and switches their D1 rows in one atomic batch; it is not a generic
database or manifest API.

The domestic base URL is optional. When specified, it is returned only to
domestic nodes, before the global static-asset URL as a checksum-verified
fallback. `DOMESTIC_BUNDLE_FORMAT` defaults to `manifest`; use `archive` when
the Iranian mirror serves the full immutable release archive.

## 3. Enable the Telegram bot

After the Worker deploys, register its webhook with the same secret supplied to
`TELEGRAM_WEBHOOK_SECRET`:

```sh
curl -fsS -X POST "https://api.telegram.org/bot${TELEGRAM_BOT_TOKEN}/setWebhook" \
  --data-urlencode "url=https://tunnel-api.mehrnet.com/api/telegram/webhook" \
  --data-urlencode "secret_token=${TELEGRAM_WEBHOOK_SECRET}" \
  --data-urlencode 'allowed_updates=["message"]'
```

Verify the bot with `/start`. A Telegram user becomes an owner on their first
valid command. Owners can create clusters and attach a server to multiple
clusters, but a server must have at least one membership before `/install`
returns a one-time command.

## 4. W4R canary enrollment

Use non-production ports and leave the legacy hpn/jet services untouched:

```text
/cluster_add w4r-canary
/server_add w4r-canary foreign de-1 138.199.236.120
/server_add w4r-canary domestic ir-6 2.144.4.192
/install de-1
/install ir-6
```

Foreign nodes use a one-time API enrollment grant. Domestic nodes instead
receive one signed base62 capsule that includes the immutable bundle metadata,
the initial domestic manifest, a cluster SSH control public key, and foreign
relay addresses. A domestic command does not contact the API.

The installer:

1. verifies a domestic capsule with the pinned Ed25519 public key, or calls
   `/api/v1/bootstrap` for a foreign enrollment grant;
2. downloads and validates a complete architecture bundle from its prescribed
   mirror order, chunk by chunk when given a `.parts` manifest;
3. validates the final SHA-256 before atomically publishing `/opt/tunnel-agent/current`;
4. creates an HPNSSH identity; and
5. calls `/api/v1/enroll` once for a foreign node, or persists the verified
   domestic capsule without an API call; and
6. starts `tunnel-agent agent` through `tunnel-agent.service`. Domestic
   agents run with `--offline` until the relay synchronization protocol ships.

At this point the node has the runtime but no active lane. The owner-side
desired-state compiler will write an enabled, typed WSTunnel (or HPNSSH)
canary contract to both node documents. The agent applies only a higher,
validated revision. Do not manually edit production listeners or stop legacy
tunnel services before that compiler validates a unique canary port.

## 5. Canary promotion criteria

The canary should report, for at least seven days:

- no listener collision or restart storm;
- reconnect and availability at least equal to the incumbent lane;
- throughput and latency within the agreed budget for the path;
- measured CPU and RSS lower than or equal to the incumbent role; and
- successful mirror fallback during a domestic-mirror failure test.

Only then move one customer listener at a time. Keep the old lane disabled,
not deleted, through the rollback observation window.
## Domestic Mirrors

Domestic bundle mirrors are optional HTTPS sources registered by an operator.
The installer always verifies each archive against the SHA-256 returned by the
API and falls back to the Cloudflare files origin when a domestic mirror fails.

A mirror may expose a complete archive or a chunk manifest. A v1 chunk
manifest uses filenames relative to its own URL. A v2 manifest carries direct
HTTPS URLs for each part, which supports providers that only expose opaque
per-file public links. Generate a v2 manifest from the canonical v1 manifest:

```sh
SOURCE_MANIFEST=./bundles/v0.4.6/tunnel-agent-v0.4.6-x86_64-unknown-linux-musl.tar.gz.parts \
PART_URLS='https://mirror.example/part-a,https://mirror.example/part-b' \
./tools/mirror-manifest.sh > iran-amd64.parts
```

Register opaque per-architecture mirror manifests during publishing with
`DOMESTIC_BUNDLE_URL_AMD64` and `DOMESTIC_BUNDLE_URL_ARM64`. This is mutually
exclusive with `DOMESTIC_BUNDLE_BASE_URL`.
