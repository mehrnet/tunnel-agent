export interface Env {
  ASSETS: Fetcher;
  DB: D1Database;
  PUBLIC_ORIGIN: string;
  DOMESTIC_BOOTSTRAP_URL?: string;
  TELEGRAM_BOT_USERNAME: string;
  ENROLLMENT_TTL_SECONDS: string;
  TELEGRAM_BOT_TOKEN?: string;
  TELEGRAM_WEBHOOK_SECRET?: string;
  BUNDLE_PUBLISH_TOKEN?: string;
}

type NodeClass = "domestic" | "foreign";
type Architecture = "amd64" | "arm64";

interface OwnerRow {
  id: string;
  telegram_user_id: string;
}

interface ClusterRow {
  id: string;
  name: string;
}

interface NodeRow {
  id: string;
  name: string;
  node_class: NodeClass;
  address: string | null;
  status: "pending" | "enrolled" | "revoked";
}

interface GrantRow extends NodeRow {
  grant_id: string;
  secret_hash: string;
  expires_at: number;
  consumed_at: number | null;
}

interface BundleRow {
  version: string;
  architecture: Architecture;
  sha256: string;
  public_url: string;
  domestic_url: string | null;
}

interface DocumentRow {
  revision: number;
  manifest_json: string;
  updated_at: number;
}

interface BundleActivationEntry {
  architecture: Architecture;
  sha256: string;
  public_url: string;
  domestic_url: string | null;
}

interface BundleActivation {
  version: string;
  bundles: BundleActivationEntry[];
}

interface TelegramUpdate {
  update_id: number;
  message?: {
    chat: { id: number };
    from?: { id: number; username?: string };
    text?: string;
  };
}

const BUNDLE_ROOT = "/opt/tunnel-agent/current";
const NODE_NAME = /^[a-z0-9][a-z0-9-]{1,62}[a-z0-9]$/;
const HEX_64 = /^[a-f0-9]{64}$/;
const BUNDLE_VERSION = /^[0-9A-Za-z._+-]+$/;

function now(): number {
  return Math.floor(Date.now() / 1000);
}

function json(value: unknown, status = 200): Response {
  return new Response(JSON.stringify(value), {
    status,
    headers: { "content-type": "application/json; charset=utf-8", "cache-control": "no-store" },
  });
}

function message(text: string, status = 200): Response {
  return new Response(text, {
    status,
    headers: { "content-type": "text/plain; charset=utf-8", "cache-control": "no-store" },
  });
}

function error(text: string, status = 400): Response {
  return json({ error: text }, status);
}

function publicOrigin(env: Env): string {
  return env.PUBLIC_ORIGIN.replace(/\/$/, "");
}

function validNodeClass(value: string | undefined): value is NodeClass {
  return value === "domestic" || value === "foreign";
}

function validArchitecture(value: string | undefined): value is Architecture {
  return value === "amd64" || value === "arm64";
}

function bundleTarget(architecture: Architecture): string {
  return architecture === "amd64" ? "x86_64-unknown-linux-musl" : "aarch64-unknown-linux-musl";
}

function bundlePublicPath(version: string, architecture: Architecture): string {
  return `/bundles/${version}/tunnel-agent-${version}-${bundleTarget(architecture)}.tar.gz.parts`;
}

function validDomesticBundleUrl(value: string, expectedPath: string): boolean {
  try {
    const url = new URL(value);
    return url.protocol === "https:" && !url.username && !url.password && !url.search && !url.hash &&
      url.pathname.endsWith(expectedPath) && !/[\s,]/.test(value);
  } catch {
    return false;
  }
}

function parseBundleActivation(body: Record<string, unknown> | null): BundleActivation | null {
  const version = typeof body?.version === "string" ? body.version : "";
  if (!BUNDLE_VERSION.test(version) || !Array.isArray(body?.bundles) || body.bundles.length !== 2) return null;

  const expectedArchitectures = new Set<Architecture>(["amd64", "arm64"]);
  const bundles: BundleActivationEntry[] = [];
  for (const rawEntry of body.bundles) {
    if (!rawEntry || typeof rawEntry !== "object" || Array.isArray(rawEntry)) return null;
    const entry = rawEntry as Record<string, unknown>;
    const architecture = typeof entry.architecture === "string" ? entry.architecture : undefined;
    const sha256 = typeof entry.sha256 === "string" ? entry.sha256 : "";
    const publicUrl = typeof entry.public_url === "string" ? entry.public_url : "";
    const domesticUrl = entry.domestic_url;
    if (!validArchitecture(architecture) || !expectedArchitectures.delete(architecture) || !HEX_64.test(sha256)) return null;
    const expectedPath = bundlePublicPath(version, architecture);
    if (publicUrl !== expectedPath) return null;
    if (domesticUrl !== null && (typeof domesticUrl !== "string" || !validDomesticBundleUrl(domesticUrl, expectedPath))) {
      return null;
    }
    bundles.push({ architecture, sha256, public_url: publicUrl, domestic_url: domesticUrl });
  }
  return expectedArchitectures.size === 0 ? { version, bundles } : null;
}

function nodeManifest(nodeClass: NodeClass): string {
  return JSON.stringify({
    version: 3,
    node_scope: nodeClass,
    bundle: { root: BUNDLE_ROOT },
    tunnels: [],
  });
}

function randomToken(): string {
  const bytes = new Uint8Array(32);
  crypto.getRandomValues(bytes);
  return Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join("");
}

async function sha256(value: string): Promise<string> {
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(value));
  return Array.from(new Uint8Array(digest), (byte) => byte.toString(16).padStart(2, "0")).join("");
}

function equalSecret(actual: string | null, expected: string | undefined): boolean {
  if (!actual || !expected) return false;
  const length = Math.max(actual.length, expected.length);
  let difference = actual.length ^ expected.length;
  for (let index = 0; index < length; index += 1) {
    difference |= (actual.charCodeAt(index) || 0) ^ (expected.charCodeAt(index) || 0);
  }
  return difference === 0;
}

function parseGrant(value: unknown): { id: string; secret: string } | null {
  if (typeof value !== "string") return null;
  const separator = value.indexOf(".");
  if (separator < 1) return null;
  const id = value.slice(0, separator);
  const secret = value.slice(separator + 1);
  if (!/^[0-9a-f-]{36}$/.test(id) || !/^[0-9a-f]{64}$/.test(secret)) return null;
  return { id, secret };
}

async function parseJson(request: Request): Promise<Record<string, unknown> | null> {
  try {
    const value = await request.json();
    return value !== null && typeof value === "object" && !Array.isArray(value)
      ? (value as Record<string, unknown>)
      : null;
  } catch {
    return null;
  }
}

async function audit(
  env: Env,
  action: string,
  detail: unknown,
  references: { ownerId?: string; nodeId?: string; clusterId?: string } = {},
): Promise<void> {
  await env.DB.prepare(
    `INSERT INTO audit_events (id, owner_id, node_id, cluster_id, action, detail_json, created_at)
     VALUES (?, ?, ?, ?, ?, ?, ?)`,
  )
    .bind(
      crypto.randomUUID(),
      references.ownerId ?? null,
      references.nodeId ?? null,
      references.clusterId ?? null,
      action,
      JSON.stringify(detail),
      now(),
    )
    .run();
}

async function ownerForTelegram(env: Env, telegramId: number, username?: string): Promise<OwnerRow> {
  const existing = await env.DB.prepare(
    "SELECT id, telegram_user_id FROM owners WHERE telegram_user_id = ?",
  )
    .bind(String(telegramId))
    .first<OwnerRow>();
  if (existing) return existing;

  const id = crypto.randomUUID();
  await env.DB.prepare(
    "INSERT INTO owners (id, telegram_user_id, telegram_username, created_at) VALUES (?, ?, ?, ?)",
  )
    .bind(id, String(telegramId), username ?? null, now())
    .run();
  return { id, telegram_user_id: String(telegramId) };
}

async function clusterByName(env: Env, ownerId: string, name: string): Promise<ClusterRow | null> {
  return env.DB.prepare("SELECT id, name FROM clusters WHERE owner_id = ? AND name = ?")
    .bind(ownerId, name)
    .first<ClusterRow>();
}

async function nodeByName(env: Env, ownerId: string, name: string): Promise<NodeRow | null> {
  return env.DB.prepare(
    "SELECT id, name, node_class, address, status FROM nodes WHERE owner_id = ? AND name = ?",
  )
    .bind(ownerId, name)
    .first<NodeRow>();
}

async function requireGrant(env: Env, rawGrant: unknown): Promise<GrantRow | null> {
  const grant = parseGrant(rawGrant);
  if (!grant) return null;
  const hash = await sha256(grant.secret);
  const row = await env.DB.prepare(
    `SELECT g.id AS grant_id, g.secret_hash, g.expires_at, g.consumed_at,
            n.id, n.name, n.node_class, n.address, n.status
       FROM enrollment_grants g
       JOIN nodes n ON n.id = g.node_id
      WHERE g.id = ?`,
  )
    .bind(grant.id)
    .first<GrantRow>();
  if (!row || row.secret_hash !== hash || row.consumed_at !== null || row.expires_at <= now()) {
    return null;
  }
  return row;
}

async function activeBundle(env: Env, architecture: Architecture): Promise<BundleRow | null> {
  return env.DB.prepare(
    `SELECT version, architecture, sha256, public_url, domestic_url
       FROM bundle_releases
      WHERE architecture = ? AND active = 1`,
  )
    .bind(architecture)
    .first<BundleRow>();
}

function absoluteUrl(origin: string, path: string): string {
  return new URL(path, origin).toString();
}

function bootstrapEnv(bundle: BundleRow, nodeClass: NodeClass, origin: string): string {
  if (!HEX_64.test(bundle.sha256)) throw new Error("bundle registry has invalid SHA-256");
  const publicUrl = absoluteUrl(origin, bundle.public_url);
  const sources = nodeClass === "domestic" && bundle.domestic_url
    ? [bundle.domestic_url, publicUrl]
    : [publicUrl];
  if (sources.some((source) => !source.startsWith("https://") || /[\s,]/.test(source))) {
    throw new Error("bundle registry has invalid source URL");
  }
  if (!BUNDLE_VERSION.test(bundle.version)) {
    throw new Error("bundle registry has invalid version");
  }
  return [
    `TUNNEL_AGENT_BUNDLE_VERSION=${bundle.version}`,
    `TUNNEL_AGENT_BUNDLE_SHA256=${bundle.sha256}`,
    `TUNNEL_AGENT_BUNDLE_URLS=${sources.join(",")}`,
  ].join("\n") + "\n";
}

function base64Utf8(value: string): string {
  const bytes = new TextEncoder().encode(value);
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary);
}

async function handleBootstrap(request: Request, env: Env): Promise<Response> {
  if (request.method !== "POST") return error("method not allowed", 405);
  const body = await parseJson(request);
  const grant = await requireGrant(env, body?.grant);
  const requestedArchitecture = typeof body?.architecture === "string" ? body.architecture : undefined;
  if (!grant || !validArchitecture(requestedArchitecture)) {
    return error("invalid enrollment grant or architecture", 401);
  }
  const bundle = await activeBundle(env, requestedArchitecture);
  if (!bundle) return error(`no active ${requestedArchitecture} tunnel-agent bundle`, 503);
  try {
    return message(bootstrapEnv(bundle, grant.node_class, publicOrigin(env)));
  } catch (reason) {
    return error(reason instanceof Error ? reason.message : "invalid bundle registry", 503);
  }
}

async function handleEnroll(request: Request, env: Env): Promise<Response> {
  if (request.method !== "POST") return error("method not allowed", 405);
  const body = await parseJson(request);
  const grant = await requireGrant(env, body?.grant);
  const hostname = typeof body?.hostname === "string" ? body.hostname.slice(0, 255) : null;
  const publicKey = typeof body?.public_key === "string" ? body.public_key.slice(0, 4096) : null;
  if (!grant || !publicKey) return error("invalid enrollment grant or public key", 401);

  const consumed = await env.DB.prepare(
    "UPDATE enrollment_grants SET consumed_at = ? WHERE id = ? AND consumed_at IS NULL AND expires_at > ?",
  )
    .bind(now(), grant.grant_id, now())
    .run();
  if (consumed.meta.changes !== 1) return error("enrollment grant was already consumed or expired", 409);

  const nodeSecret = randomToken();
  await env.DB.batch([
    env.DB.prepare(
      "UPDATE nodes SET public_key = ?, status = 'enrolled', enrolled_at = ? WHERE id = ?",
    ).bind(publicKey, now(), grant.id),
    env.DB.prepare(
      `INSERT INTO node_credentials (node_id, secret_hash, created_at, rotated_at)
       VALUES (?, ?, ?, ?)
       ON CONFLICT(node_id) DO UPDATE SET secret_hash = excluded.secret_hash, rotated_at = excluded.rotated_at`,
    ).bind(grant.id, await sha256(nodeSecret), now(), now()),
  ]);

  const document = await env.DB.prepare(
    "SELECT revision, manifest_json, updated_at FROM node_documents WHERE node_id = ?",
  )
    .bind(grant.id)
    .first<DocumentRow>();
  await audit(env, "node.enrolled", { hostname }, { nodeId: grant.id });
  const payload = {
    node_id: grant.id,
    node_token: `${grant.id}.${nodeSecret}`,
    document: document ?? { revision: 0, manifest_json: nodeManifest(grant.node_class), updated_at: now() },
  };
  if (request.headers.get("accept") === "text/x-tunnel-agent-env") {
    return message([
      `TUNNEL_AGENT_NODE_ID=${payload.node_id}`,
      `TUNNEL_AGENT_NODE_TOKEN=${payload.node_token}`,
      `TUNNEL_AGENT_DOCUMENT_REVISION=${payload.document.revision}`,
      `TUNNEL_AGENT_MANIFEST_B64=${base64Utf8(payload.document.manifest_json)}`,
    ].join("\n") + "\n");
  }
  return json(payload);
}

async function authorizeNode(request: Request, env: Env, nodeId: string): Promise<boolean> {
  const header = request.headers.get("authorization");
  if (!header?.startsWith("Bearer ")) return false;
  const token = header.slice("Bearer ".length);
  const separator = token.indexOf(".");
  if (separator < 1 || token.slice(0, separator) !== nodeId) return false;
  const credential = await env.DB.prepare("SELECT secret_hash FROM node_credentials WHERE node_id = ?")
    .bind(nodeId)
    .first<{ secret_hash: string }>();
  return Boolean(credential && credential.secret_hash === await sha256(token.slice(separator + 1)));
}

async function handleNodeDocument(request: Request, env: Env, nodeId: string): Promise<Response> {
  if (request.method !== "GET") return error("method not allowed", 405);
  if (!await authorizeNode(request, env, nodeId)) return error("unauthorized", 401);
  const document = await env.DB.prepare(
    "SELECT revision, manifest_json, updated_at FROM node_documents WHERE node_id = ?",
  )
    .bind(nodeId)
    .first<DocumentRow>();
  if (!document) return error("node has no desired document", 404);
  const etag = `"${document.revision}"`;
  if (request.headers.get("if-none-match") === etag) {
    return new Response(null, { status: 304, headers: { etag, "cache-control": "no-store" } });
  }
  return new Response(JSON.stringify(document), {
    headers: {
      "content-type": "application/json; charset=utf-8",
      "cache-control": "no-store",
      etag,
    },
  });
}

async function handleBundleActivation(request: Request, env: Env): Promise<Response> {
  if (request.method !== "POST") return error("method not allowed", 405);
  const authorization = request.headers.get("authorization");
  const token = authorization?.startsWith("Bearer ") ? authorization.slice("Bearer ".length) : null;
  if (!equalSecret(token, env.BUNDLE_PUBLISH_TOKEN)) return error("unauthorized", 401);

  const activation = parseBundleActivation(await parseJson(request));
  if (!activation) return error("invalid bundle activation payload", 400);

  const timestamp = now();
  const statements: D1PreparedStatement[] = [];
  for (const bundle of activation.bundles) {
    statements.push(
      env.DB.prepare("UPDATE bundle_releases SET active = 0 WHERE architecture = ? AND active = 1")
        .bind(bundle.architecture),
      env.DB.prepare(
        `INSERT INTO bundle_releases
          (version, architecture, sha256, public_url, domestic_url, active, created_at)
         VALUES (?, ?, ?, ?, ?, 1, ?)
         ON CONFLICT(version, architecture) DO UPDATE SET
           sha256 = excluded.sha256,
           public_url = excluded.public_url,
           domestic_url = excluded.domestic_url,
           active = 1,
           created_at = excluded.created_at`,
      ).bind(
        activation.version,
        bundle.architecture,
        bundle.sha256,
        bundle.public_url,
        bundle.domestic_url,
        timestamp,
      ),
    );
  }
  statements.push(
    env.DB.prepare(
      `INSERT INTO audit_events (id, owner_id, node_id, cluster_id, action, detail_json, created_at)
       VALUES (?, NULL, NULL, NULL, ?, ?, ?)`,
    ).bind(
      crypto.randomUUID(),
      "bundle.activated",
      JSON.stringify({ version: activation.version, architectures: activation.bundles.map((bundle) => bundle.architecture) }),
      timestamp,
    ),
  );
  await env.DB.batch(statements);
  return json({ ok: true, version: activation.version });
}

async function createEnrollmentGrant(env: Env, node: NodeRow): Promise<string> {
  const id = crypto.randomUUID();
  const secret = randomToken();
  const ttl = Number.parseInt(env.ENROLLMENT_TTL_SECONDS, 10);
  const expiresAt = now() + (Number.isSafeInteger(ttl) && ttl >= 60 ? ttl : 900);
  await env.DB.prepare(
    `INSERT INTO enrollment_grants (id, node_id, secret_hash, expires_at, created_at)
     VALUES (?, ?, ?, ?, ?)`,
  )
    .bind(id, node.id, await sha256(secret), expiresAt, now())
    .run();
  return `${id}.${secret}`;
}

function installCommand(env: Env, node: NodeRow, grant: string): string {
  const publicInstaller = `${publicOrigin(env)}/install.sh`;
  const domesticInstaller = env.DOMESTIC_BOOTSTRAP_URL?.trim();
  const installer = node.node_class === "domestic" && domesticInstaller?.startsWith("https://")
    ? domesticInstaller
    : publicInstaller;
  const shared = `TUNNEL_AGENT_API=${publicOrigin(env)}/api TUNNEL_AGENT_ENROLLMENT=${grant}`;
  return [
    `curl -fsSL ${installer} | sudo env ${shared} sh`,
    `wget -qO- ${installer} | sudo env ${shared} sh`,
  ].join("\n\n");
}

async function telegramReply(env: Env, chatId: number, text: string): Promise<void> {
  if (!env.TELEGRAM_BOT_TOKEN) return;
  await fetch(`https://api.telegram.org/bot${env.TELEGRAM_BOT_TOKEN}/sendMessage`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ chat_id: chatId, text, disable_web_page_preview: true }),
  });
}

const helpText = [
  "TunnelOwnersBot commands",
  "/cluster_add <name>",
  "/cluster_list",
  "/server_add <cluster> <domestic|foreign> <server> [address]",
  "/server_attach <cluster> <domestic|foreign> <server>",
  "/server_list",
  "/install <server>",
  "Server names and cluster names use lowercase letters, numbers, and hyphens.",
].join("\n");

async function handleBotCommand(env: Env, update: TelegramUpdate): Promise<void> {
  const incoming = update.message;
  if (!incoming?.from || !incoming.text) return;
  const parts = incoming.text.trim().split(/\s+/);
  const command = parts.shift()?.split("@", 1)[0].toLowerCase();
  const owner = await ownerForTelegram(env, incoming.from.id, incoming.from.username);
  let response = helpText;

  if (command === "/start" || command === "/help") {
    response = helpText;
  } else if (command === "/cluster_add") {
    const name = parts[0]?.toLowerCase();
    if (!name || !NODE_NAME.test(name)) {
      response = "Usage: /cluster_add <lowercase-name>";
    } else {
      const existing = await clusterByName(env, owner.id, name);
      if (existing) {
        response = `Cluster ${name} already exists.`;
      } else {
        const id = crypto.randomUUID();
        await env.DB.prepare("INSERT INTO clusters (id, owner_id, name, created_at) VALUES (?, ?, ?, ?)")
          .bind(id, owner.id, name, now())
          .run();
        await audit(env, "cluster.created", { name }, { ownerId: owner.id, clusterId: id });
        response = `Created cluster ${name}.`;
      }
    }
  } else if (command === "/cluster_list") {
    const rows = await env.DB.prepare("SELECT name FROM clusters WHERE owner_id = ? ORDER BY name")
      .bind(owner.id)
      .all<{ name: string }>();
    response = rows.results.length ? `Clusters:\n${rows.results.map((row) => `- ${row.name}`).join("\n")}` : "No clusters yet.";
  } else if (command === "/server_add" || command === "/server_attach") {
    const [clusterName, className, nodeName, address] = parts;
    const cluster = clusterName ? await clusterByName(env, owner.id, clusterName.toLowerCase()) : null;
    const nodeClass = className?.toLowerCase();
    const name = nodeName?.toLowerCase();
    if (!cluster || !validNodeClass(nodeClass) || !name || !NODE_NAME.test(name)) {
      response = `Usage: ${command} <cluster> <domestic|foreign> <server> ${command === "/server_add" ? "[address]" : ""}`;
    } else {
      let node = await nodeByName(env, owner.id, name);
      if (!node && command === "/server_attach") {
        response = `Server ${name} does not exist; add it first.`;
      } else {
        if (!node) {
          const id = crypto.randomUUID();
          await env.DB.prepare(
            `INSERT INTO nodes (id, owner_id, name, node_class, address, created_at)
             VALUES (?, ?, ?, ?, ?, ?)`,
          )
            .bind(id, owner.id, name, nodeClass, address ?? null, now())
            .run();
          await env.DB.prepare(
            "INSERT INTO node_documents (node_id, revision, manifest_json, updated_at) VALUES (?, ?, ?, ?)",
          )
            .bind(id, 0, nodeManifest(nodeClass), now())
            .run();
          node = { id, name, node_class: nodeClass, address: address ?? null, status: "pending" };
        }
        if (node && node.node_class !== nodeClass) {
          response = `Server ${name} is already ${node.node_class}; node class cannot be changed through attach.`;
        } else if (node) {
          await env.DB.prepare(
            "INSERT OR IGNORE INTO cluster_nodes (cluster_id, node_id, role, created_at) VALUES (?, ?, ?, ?)",
          )
            .bind(cluster.id, node.id, nodeClass, now())
            .run();
          await audit(env, "cluster.node_attached", { node: name, role: nodeClass }, {
            ownerId: owner.id,
            nodeId: node.id,
            clusterId: cluster.id,
          });
          response = `Server ${name} is a ${nodeClass} member of ${cluster.name}. Run /install ${name} on the server.`;
        }
      }
    }
  } else if (command === "/server_list") {
    const rows = await env.DB.prepare(
      "SELECT name, node_class, address, status FROM nodes WHERE owner_id = ? ORDER BY name",
    )
      .bind(owner.id)
      .all<NodeRow>();
    response = rows.results.length
      ? `Servers:\n${rows.results.map((row) => `- ${row.name} (${row.node_class}, ${row.status}${row.address ? `, ${row.address}` : ""})`).join("\n")}`
      : "No servers yet.";
  } else if (command === "/install") {
    const name = parts[0]?.toLowerCase();
    const node = name ? await nodeByName(env, owner.id, name) : null;
    const memberships = node
      ? await env.DB.prepare("SELECT COUNT(*) AS count FROM cluster_nodes WHERE node_id = ?").bind(node.id).first<{ count: number }>()
      : null;
    if (!node || !memberships || memberships.count < 1) {
      response = "Usage: /install <server>. The server must belong to at least one cluster.";
    } else {
      const grant = await createEnrollmentGrant(env, node);
      await audit(env, "node.enrollment_grant_created", { expires_in_seconds: env.ENROLLMENT_TTL_SECONDS }, {
        ownerId: owner.id,
        nodeId: node.id,
      });
      response = `Run one command on ${node.name}. This grant is single-use and short-lived.\n\n${installCommand(env, node, grant)}`;
    }
  }

  await telegramReply(env, incoming.chat.id, response);
}

async function handleTelegram(request: Request, env: Env, ctx: ExecutionContext): Promise<Response> {
  if (request.method !== "POST") return error("method not allowed", 405);
  if (!equalSecret(request.headers.get("X-Telegram-Bot-Api-Secret-Token"), env.TELEGRAM_WEBHOOK_SECRET)) {
    return error("unauthorized", 401);
  }
  let update: TelegramUpdate;
  try {
    update = await request.json<TelegramUpdate>();
  } catch {
    return error("invalid Telegram update");
  }
  if (!Number.isInteger(update.update_id)) return error("invalid Telegram update");
  const recorded = await env.DB.prepare("INSERT OR IGNORE INTO telegram_updates (update_id, received_at) VALUES (?, ?)")
    .bind(update.update_id, now())
    .run();
  if (recorded.meta.changes === 0) return json({ ok: true });
  ctx.waitUntil(handleBotCommand(env, update));
  return json({ ok: true });
}

async function handleApi(request: Request, env: Env, ctx: ExecutionContext): Promise<Response> {
  const url = new URL(request.url);
  if (url.pathname === "/api/health") return json({ ok: true, service: "tunnel-agent" });
  if (url.pathname === "/api/telegram/webhook") return handleTelegram(request, env, ctx);
  if (url.pathname === "/api/v1/bootstrap") return handleBootstrap(request, env);
  if (url.pathname === "/api/v1/enroll") return handleEnroll(request, env);
  if (url.pathname === "/api/v1/admin/bundles/activate") return handleBundleActivation(request, env);
  const document = url.pathname.match(/^\/api\/v1\/nodes\/([0-9a-f-]{36})\/document$/);
  if (document) return handleNodeDocument(request, env, document[1]);
  return error("not found", 404);
}

export default {
  async fetch(request: Request, env: Env, ctx: ExecutionContext): Promise<Response> {
    const url = new URL(request.url);
    if (url.pathname.startsWith("/api/")) return handleApi(request, env, ctx);
    return env.ASSETS.fetch(request);
  },
};
