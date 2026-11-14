# tunnel-agent Bootstrap Runbook

This runbook publishes the static `tunnel-agent` website, Worker API, complete
transport bundles, Telegram owner bot, and the first W4R canary. It does not
replace a production HPNSSH/Jet lane until the canary has met its health and
throughput thresholds.

## 1. Create the Cloudflare state

From `distribution/`, create a D1 database and place the returned database ID
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
```

Set `DOMESTIC_BOOTSTRAP_URL` in `wrangler.jsonc` to the Iranian mirror copy of
`install.sh`, then deploy the Worker and attach the custom domain
`tunnel-agent.mehrnet.com`:

```sh
npm run deploy
```

The Worker only handles `/api/*`; the custom domain's remaining paths are
served by the `ASSETS` binding.

## 2. Publish a bundle and its mirrors

Every tag matching `v*` produces two release archives and an artifact named
`tunnel-agent-static-assets-vX.Y.Z`. Download that artifact and deploy its
contents with Wrangler so the global static site serves:

```text
/install.sh
/bundles/vX.Y.Z/tunnel-agent-vX.Y.Z-x86_64-unknown-linux-musl.tar.gz
/bundles/vX.Y.Z/tunnel-agent-vX.Y.Z-aarch64-unknown-linux-musl.tar.gz
```

Copy the exact `install.sh` and archive files to the Iranian mirror. A domestic
bundle URL must be a versioned immutable URL containing the same bytes and
SHA-256 as its global counterpart.

Use the generated `.sha256` files to activate one bundle per architecture in
D1. Do this as a single SQL batch so nodes never observe two active bundles:

```sql
BEGIN;
UPDATE bundle_releases SET active = 0 WHERE architecture = 'amd64';
INSERT INTO bundle_releases
  (version, architecture, sha256, public_url, domestic_url, active, created_at)
VALUES
  ('vX.Y.Z', 'amd64', '<64-lowercase-hex>',
   '/bundles/vX.Y.Z/tunnel-agent-vX.Y.Z-x86_64-unknown-linux-musl.tar.gz',
   'https://iran-mirror.example/tunnel-agent/vX.Y.Z/tunnel-agent-vX.Y.Z-x86_64-unknown-linux-musl.tar.gz',
   1, unixepoch());
COMMIT;
```

Repeat for `arm64`. The `public_url` must be a relative static-asset path or a
trusted HTTPS URL. `domestic_url` is only returned to domestic nodes, before
the global bundle URL as a checksum-verified fallback.

## 3. Enable the Telegram bot

After the Worker deploys, register its webhook with the same secret supplied to
`TELEGRAM_WEBHOOK_SECRET`:

```sh
curl -fsS -X POST "https://api.telegram.org/bot${TELEGRAM_BOT_TOKEN}/setWebhook" \
  --data-urlencode "url=https://tunnel-agent.mehrnet.com/api/telegram/webhook" \
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

Run one returned command on each matching host. The installer:

1. calls `/api/v1/bootstrap` with its short-lived enrollment grant;
2. downloads a complete architecture bundle from its prescribed mirror order;
3. validates the SHA-256 before atomically publishing `/opt/tunnel-agent/current`;
4. creates an HPNSSH identity; and
5. calls `/api/v1/enroll` once, writes a v3 manifest, and starts
   `tunnel-agent.service`.

At this point the node has the runtime but no active lane. The next implementation
step is the desired-state compiler and authenticated document reconciliation:
it will write an enabled, typed WSTunnel (or HPNSSH) canary contract to both
node documents. Do not manually edit production listeners or stop legacy
tunnel services before that compiler applies and validates a unique canary port.

## 5. Canary promotion criteria

The canary should report, for at least seven days:

- no listener collision or restart storm;
- reconnect and availability at least equal to the incumbent lane;
- throughput and latency within the agreed budget for the path;
- measured CPU and RSS lower than or equal to the incumbent role; and
- successful mirror fallback during a domestic-mirror failure test.

Only then move one customer listener at a time. Keep the old lane disabled,
not deleted, through the rollback observation window.
