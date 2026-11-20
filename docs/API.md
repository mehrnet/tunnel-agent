# tunnel-api

`tunnel-api/wrangler.jsonc` deploys only the API Worker at
`https://tunnel-api.mehrnet.com`:

- `/api/*` runs [worker.ts](../tunnel-api/src/worker.ts) with the D1 `DB` binding.
- non-API requests receive `404 Not Found`.

The Worker source deliberately has no transport binary and no bot token in its
repository configuration.

## Relay documents

An enrolled foreign node can fetch the signed desired-state envelopes for the
domestic members of its own clusters:

```text
GET /api/v1/nodes/<foreign-node-uuid>/relay-documents
Authorization: Bearer <foreign-node-uuid>.<node-token>
```

The endpoint returns no foreign documents and no documents from unrelated
clusters. Each envelope is signed by the Worker key pinned into
`tunnel-agent`; domestic nodes verify the cluster ID, node ID, revision, and
manifest before a future management ingress is allowed to apply it. The
endpoint only issues state: it does not itself establish a tunnel or connect to
a domestic node. Current relay envelopes are not encrypted, so relayed
manifests must not contain inline secret values.

## Domestic relay heartbeats

A domestic node does not call the API. Instead, it sends a loopback-only
`/v1/ping/<domestic-node-uuid>` request through its selected agent lane. The
foreign seed receives that request, records the timestamp locally, then reports
only that timestamp to the API during its regular authenticated refresh:

```text
POST /api/v1/nodes/<foreign-node-uuid>/heartbeats
Authorization: Bearer <foreign-node-uuid>.<node-token>
{"observations":[{"node_id":"<domestic-node-uuid>","observed_at":1730000000}]}
```

The API accepts an observation only when the authenticated foreign node and the
domestic node share a cluster. The timestamp is monotonic per domestic node, so
retries cannot make a stale relay look fresh. Telegram displays this as the
domestic node's last relay ping; it is not an enrollment event.

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
   the scoped activation endpoint to atomically set the testing or stable
   pointer for both architectures. The installer rejects enrollment until
   exact release archive hashes and static chunk manifests are registered.

See [release publishing](BOOTSTRAP.md#2-publish-a-bundle-and-its-mirrors) for
the GitHub Latest, live testing, and stable promotion sequence. Changing
GitHub's Latest label does not by itself change a fleet's installed version.

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
