import { error, json, message, parseJson } from "./http";
import type { DocumentRow, Env, NodeClass } from "./models";
import { audit, requireGrant } from "./store";
import { BUNDLE_ROOT, DEFAULT_REPORT_INTERVAL_SECONDS, base64Utf8, now, randomToken, sha256 } from "./validation";

export function nodeManifest(nodeClass: NodeClass): string {
  return JSON.stringify({
    version: 3,
    node_scope: nodeClass,
    bundle: { root: BUNDLE_ROOT },
    agent: {
      poll_interval_secs: DEFAULT_REPORT_INTERVAL_SECONDS,
      max_active_lanes: 1,
      lanes: [],
    },
    tunnels: [],
  });
}

export async function handleEnroll(request: Request, env: Env): Promise<Response> {
  if (request.method !== "POST") return error("method not allowed", 405);
  const body = await parseJson(request);
  const grant = await requireGrant(env, body?.grant);
  const hostname = typeof body?.hostname === "string" ? body.hostname.slice(0, 255) : null;
  const publicKey = typeof body?.public_key === "string" ? body.public_key.slice(0, 4096) : null;
  if (!grant || !publicKey) return error("invalid enrollment grant or public key", 401);

  const nodeSecret = randomToken();
  const secretHash = await sha256(nodeSecret);
  const timestamp = now();
  const [consumed] = await env.DB.batch([
    env.DB.prepare(
      `UPDATE enrollment_grants SET consumed_at = ?
        WHERE id = ? AND node_id = ? AND consumed_at IS NULL AND expires_at > ?
          AND EXISTS (SELECT 1 FROM nodes WHERE id = ? AND status = 'pending' AND node_class = 'foreign')`,
    ).bind(timestamp, grant.grant_id, grant.id, timestamp, grant.id),
    // Each write is conditional on the previous write in this transaction.
    env.DB.prepare(
      `INSERT INTO node_credentials (node_id, secret_hash, created_at, rotated_at)
       SELECT ?, ?, ?, ? WHERE changes() = 1
       ON CONFLICT(node_id) DO UPDATE SET secret_hash = excluded.secret_hash, rotated_at = excluded.rotated_at`,
    ).bind(grant.id, secretHash, timestamp, timestamp),
    env.DB.prepare(
      "UPDATE nodes SET public_key = ?, status = 'enrolled', enrolled_at = ? WHERE id = ? AND changes() = 1",
    ).bind(publicKey, timestamp, grant.id),
    env.DB.prepare(
      "UPDATE enrollment_grants SET consumed_at = ? WHERE node_id = ? AND consumed_at IS NULL AND changes() = 1",
    ).bind(timestamp, grant.id),
  ]);
  if (consumed.meta.changes !== 1) return error("enrollment grant expired, was consumed, or the node is not pending foreign enrollment", 409);

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

export async function authorizeNode(request: Request, env: Env, nodeId: string): Promise<boolean> {
  const header = request.headers.get("authorization");
  if (!header?.startsWith("Bearer ")) return false;
  const token = header.slice("Bearer ".length);
  const separator = token.indexOf(".");
  if (separator < 1 || token.slice(0, separator) !== nodeId) return false;
  const credential = await env.DB.prepare(
    `SELECT c.secret_hash FROM node_credentials c
       JOIN nodes n ON n.id = c.node_id
      WHERE c.node_id = ? AND n.status = 'enrolled'`,
  )
    .bind(nodeId)
    .first<{ secret_hash: string }>();
  return Boolean(credential && credential.secret_hash === await sha256(token.slice(separator + 1)));
}

export async function handleNodeDocument(request: Request, env: Env, nodeId: string): Promise<Response> {
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
