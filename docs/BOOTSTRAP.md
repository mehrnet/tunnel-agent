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

A normal tag such as `v0.5.1` produces two immutable GitHub release archives,
marked Latest without making them stable for managed nodes. Use one patch
digit (`v0.5.9` is followed by `v0.6.0`). The same files move from testing to
stable without another build or a `-canary` tag.

Download the GitHub Latest release, then publish it to the testing track:

```sh
version=$(gh release view --json tagName --jq .tagName)
download_dir=$(mktemp -d)
gh release download "$version" --dir "$download_dir"
BUNDLE_VERSION="$version" BUNDLE_SOURCE_DIR="$download_dir" \
  BUNDLE_ACTIVATE=true BUNDLE_TRACK=testing ./tools/publish.sh
```

Export `BUNDLE_PUBLISH_TOKEN` securely before running the publisher. To upload
files without moving a track, omit `BUNDLE_ACTIVATE=true`. The publisher
prepares the global static site:

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

Testing rolls out only to the selected testing cluster. Check both nodes run
the tagged version and verify the signed relay and a real payload end to end.
Leave stable pinned until those checks pass. When the exact tested files are
available from domestic mirrors, move the stable pointer explicitly:

```sh
BUNDLE_VERSION="$version" \
BUNDLE_SOURCE_DIR="$download_dir" \
BUNDLE_ACTIVATE=true BUNDLE_TRACK=stable \
DOMESTIC_BUNDLE_BASE_URL=http://dl.mehr-mir.ir \
DOMESTIC_BUNDLE_EXTRA_BASE_URLS=https://mehrnat.ir,https://mahdi-m.ir \
DOMESTIC_BUNDLE_FORMAT=manifest \
./tools/publish.sh
```

`BUNDLE_PUBLISH_TOKEN` is a Worker secret. The publisher deploys static assets
first, then invokes the narrow `/api/v1/admin/bundles/activate` endpoint. That
endpoint only accepts the two expected architecture manifest paths for one
version and moves the chosen track for both architectures atomically. Nodes
outside the testing cluster remain on stable; publishing to testing never
advances stable. Moving stable requires the same archive and inventory
checksums previously registered for testing.

Domestic mirror URLs are optional for testing existing nodes over a management
lane, but required for fresh domestic enrollment and stable promotion. The
installer tries them in the declared order, validates every chunk and the
reconstructed archive SHA-256, then falls back to the global static-asset URL.
Set `DOMESTIC_BUNDLE_BASE_URL` for the first mirror and provide up to three
more comma-separated HTTPS origins in `DOMESTIC_BUNDLE_EXTRA_BASE_URLS`.
`DOMESTIC_BUNDLE_FORMAT` defaults to `manifest`; use `archive` only when every
mirror serves the full immutable release archive.

## 3. Enable the Telegram bot

After the Worker deploys, register its webhook with the same secret supplied to
`TELEGRAM_WEBHOOK_SECRET`:

```sh
curl -fsS -X POST "https://api.telegram.org/bot${TELEGRAM_BOT_TOKEN}/setWebhook" \
  --data-urlencode "url=https://tunnel-api.mehrnet.com/api/telegram/webhook" \
  --data-urlencode "secret_token=${TELEGRAM_WEBHOOK_SECRET}" \
  --data-urlencode 'allowed_updates=["message","callback_query"]'
```

Verify the bot with `/start` and a menu button. Telegram does not send inline
button presses unless `callback_query` is included in `allowed_updates`. A
Telegram user becomes an owner on their first valid command. Owners can create
clusters and attach a server to multiple clusters, but a server must have at
least one membership before `/install` returns a one-time command.

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
   agents run with `--offline` permanently: their only control source is a
   selected loopback-only management lane to an enrolled foreign relay.

For the first HPNSSH management-lane canary, issue a higher revision to both
node documents. The domestic document declares a restricted `hpnsshd` listener
and its loopback seed port; the foreign document declares the pinned HPNSSH
client and its matching loopback seed server. Reserve distinct high ports such
as `42001` for HPNSSH and `42022` for the mapped seed endpoint. Do not reuse a
legacy data-plane HPN listener, SOCKS port, WSTunnel endpoint, or customer
container port. A fresh `/install ir-6` capsule is required after changing a
domestic manifest so the API-free node receives the signed initial lane.

Once connected, the foreign relay polls the API for signed domestic envelopes
and release metadata. The domestic node polls only `127.0.0.1:42022`, verifies
every envelope and release digest, and applies only a higher validated
revision. Do not manually edit production listeners or stop legacy tunnel
services before this canary demonstrates a stable management connection.

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

`http://dl.mehr-mir.ir/...` is the sole HTTP exception, and it may carry only
immutable bundle manifests and chunks. The installer validates every chunk and
the reconstructed archive against the signed capsule's SHA-256. Bootstrap
scripts remain HTTPS-only and continue to come from `tunnel-files`.

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
