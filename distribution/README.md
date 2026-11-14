# tunnel-agent distribution Worker

`wrangler.jsonc` deploys the complete `tunnel-agent.mehrnet.com` origin:

- `/api/*` runs [src/index.ts](src/index.ts) with the D1 `DB` binding.
- every other request is served from `public/` through the `ASSETS` binding.

The Worker source deliberately has no transport binary and no bot token in its
repository configuration.

## Initial deployment

1. Create a D1 database and replace `database_id` in `wrangler.jsonc`.
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
6. Register one active amd64 and arm64 bundle in `bundle_releases`. The
   installer rejects enrollment until an exact release archive and SHA-256 are
   registered.

`DOMESTIC_BOOTSTRAP_URL` is a non-secret Worker variable. Point it to an
Iranian mirror of this exact `install.sh`; that mirror must expose the same
versioned archive as `bundle_releases.domestic_url`.

## Required secret handling

Do not put a Telegram bot token in `wrangler.jsonc` or `.dev.vars.example`.
`vars` are configuration values; `TELEGRAM_BOT_TOKEN` and
`TELEGRAM_WEBHOOK_SECRET` must be configured as encrypted Worker secrets.
