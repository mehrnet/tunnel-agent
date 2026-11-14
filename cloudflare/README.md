# tunnel-agent Cloudflare service

`wrangler.jsonc` deploys the complete `tunnel-agent.mehrnet.com` origin:

- `/api/*` runs [src/worker.ts](src/worker.ts) with the D1 `DB` binding.
- every other request is served from `public/` through the `ASSETS` binding.

The Worker source deliberately has no transport binary and no bot token in its
repository configuration.

## Initial deployment

1. Create a D1 database and place its database ID in `wrangler.jsonc`.
2. Run `npm install` and `npm run db:migrate:remote`.
3. Store the Telegram credentials as Worker secrets:

```sh
wrangler secret put TELEGRAM_BOT_TOKEN
wrangler secret put TELEGRAM_WEBHOOK_SECRET
```

4. Deploy with `npm run deploy`.
5. Configure Telegram to post to
   `https://tunnel-agent.mehrnet.com/api/telegram/webhook` with the same
   webhook secret.
6. Set `BUNDLE_PUBLISH_TOKEN` as a Worker secret. The release publisher uses
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
./scripts/publish-bundle.sh
```

`DOMESTIC_BOOTSTRAP_URL` is an optional Worker secret used only to change the
installer URL sent to domestic nodes. Point it to an Iranian mirror of this
exact `install.sh`; that mirror may expose either the same versioned `.parts`
manifest and every referenced chunk or one full immutable `.tar.gz` archive
with the registered final SHA-256. Until it is set, domestic nodes safely
receive the official installer URL.

`DOMESTIC_BUNDLE_FORMAT` defaults to `manifest`; set it to `archive` when the
Iranian mirror serves the full release archive instead.

## Required secret handling

Do not put a Telegram bot token in `wrangler.jsonc` or `.dev.vars.example`.
`vars` are configuration values; `TELEGRAM_BOT_TOKEN` and
`TELEGRAM_WEBHOOK_SECRET` must be configured as encrypted Worker secrets.
