# tunnel-api

`tunnel-api/wrangler.jsonc` deploys only the API Worker at
`https://tunnel-api.mehrnet.com`:

- `/api/*` runs [worker.ts](../tunnel-api/src/worker.ts) with the D1 `DB` binding.
- non-API requests receive `404 Not Found`.

The Worker source deliberately has no transport binary and no bot token in its
repository configuration.

Static website, installer, manifests, and bundle chunks are separately served
from `https://tunnel-files.mehrnet.com` by the assets-only
`tunnel-files/wrangler.jsonc` deployment.

## Initial deployment

1. Create a D1 database and place its database ID in `wrangler.jsonc`.
2. Run `npm install` and `npm run db:migrate:remote`.
3. Store the Telegram credentials as Worker secrets:

```sh
wrangler secret put TELEGRAM_BOT_TOKEN
wrangler secret put TELEGRAM_WEBHOOK_SECRET
```

4. Deploy with `npm run deploy`.
5. Deploy the static files separately with `npm run files:deploy`.
6. Configure Telegram to post to
   `https://tunnel-api.mehrnet.com/api/telegram/webhook` with the same
   webhook secret.
7. Set `BUNDLE_PUBLISH_TOKEN` as a Worker secret. The release publisher uses
   the scoped activation endpoint to atomically register one active amd64 and
   arm64 bundle. The installer rejects enrollment until exact release archive
   hashes and static chunk manifests are registered.

The scripted release activation verifies both archives, splits them into
sub-25 MiB static chunks, deploys the manifests and chunks as static assets,
then activates the matching D1 rows:

```sh
BUNDLE_VERSION=vX.Y.Z \
BUNDLE_SOURCE_DIR=/path/to/release \
BUNDLE_PUBLISH_TOKEN=... \
DOMESTIC_BUNDLE_BASE_URL=https://iran-mirror.example/tunnel-agent/vX.Y.Z \
DOMESTIC_BUNDLE_FORMAT=archive \
./tools/publish.sh
```

`DOMESTIC_BOOTSTRAP_URL` is an optional Worker secret used only to change the
installer URL sent to domestic nodes. Point it to an Iranian mirror of this
exact `install.sh`; that mirror may expose either the same versioned `.parts`
manifest and every referenced chunk or one full immutable `.tar.gz` archive
with the registered final SHA-256. Until it is set, domestic nodes safely
receive the official installer URL.

`DOMESTIC_BUNDLE_FORMAT` defaults to `manifest`; set it to `archive` when the
Iranian mirror serves the full release archive instead.

## Domain migration

`tunnel-api` is a new Worker name. Cloudflare does not copy encrypted secrets
between Worker names, so set `TELEGRAM_BOT_TOKEN`,
`TELEGRAM_WEBHOOK_SECRET`, and `BUNDLE_PUBLISH_TOKEN` on `tunnel-api` before
deploying it. Verify `/api/health`, the Telegram webhook, and an installer
response from `tunnel-files.mehrnet.com` before removing the former
`tunnel-agent.mehrnet.com` custom domain.

## Required secret handling

Do not put a Telegram bot token in `wrangler.jsonc` or `.dev.vars.example`.
`vars` are configuration values; `TELEGRAM_BOT_TOKEN` and
`TELEGRAM_WEBHOOK_SECRET` must be configured as encrypted Worker secrets.
