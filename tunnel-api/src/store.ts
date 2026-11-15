import type { Architecture, AuditReferences, BundleRow, ClusterRow, DocumentRow, Env, GrantRow, NodeRow, OwnerRow, RelayDocumentRow, RelayRow } from "./models";
import { now, parseGrant, randomToken, sha256 } from "./validation";

export async function audit(
  env: Env,
  action: string,
  detail: unknown,
  references: AuditReferences = {},
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

export async function ownerForTelegram(env: Env, telegramId: number, username?: string): Promise<OwnerRow> {
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

export async function clusterByName(env: Env, ownerId: string, name: string): Promise<ClusterRow | null> {
  return env.DB.prepare("SELECT id, name FROM clusters WHERE owner_id = ? AND name = ?")
    .bind(ownerId, name)
    .first<ClusterRow>();
}

export async function nodeByName(env: Env, ownerId: string, name: string): Promise<NodeRow | null> {
  return env.DB.prepare(
    "SELECT id, name, node_class, address, status FROM nodes WHERE owner_id = ? AND name = ?",
  )
    .bind(ownerId, name)
    .first<NodeRow>();
}

export async function nodeById(env: Env, nodeId: string): Promise<NodeRow | null> {
  return env.DB.prepare(
    "SELECT id, name, node_class, address, status FROM nodes WHERE id = ?",
  ).bind(nodeId).first<NodeRow>();
}

export async function requireGrant(env: Env, rawGrant: unknown): Promise<GrantRow | null> {
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
  return row && row.secret_hash === hash && row.consumed_at === null && row.expires_at > now() ? row : null;
}

export async function activeBundle(env: Env, architecture: Architecture): Promise<BundleRow | null> {
  return env.DB.prepare(
    `SELECT version, architecture, sha256, inventory_sha256, public_url, domestic_url, domestic_urls
       FROM bundle_releases
      WHERE architecture = ? AND active = 1`,
  )
    .bind(architecture)
    .first<BundleRow>();
}

export async function nodeClusters(env: Env, nodeId: string): Promise<ClusterRow[]> {
  const rows = await env.DB.prepare(
    `SELECT c.id, c.name FROM clusters c JOIN cluster_nodes cn ON cn.cluster_id = c.id
      WHERE cn.node_id = ? ORDER BY c.name`,
  ).bind(nodeId).all<ClusterRow>();
  return rows.results;
}

export async function foreignRelayAddresses(env: Env, clusterId: string): Promise<RelayRow[]> {
  const rows = await env.DB.prepare(
    `SELECT n.address FROM nodes n JOIN cluster_nodes cn ON cn.node_id = n.id
      WHERE cn.cluster_id = ? AND n.node_class = 'foreign' AND n.status != 'revoked' AND n.address IS NOT NULL
      ORDER BY n.name`,
  ).bind(clusterId).all<RelayRow>();
  return rows.results;
}

export async function relayDocumentsForForeignNode(env: Env, relayNodeId: string): Promise<RelayDocumentRow[]> {
  const rows = await env.DB.prepare(
    `SELECT c.id AS cluster_id, d.node_id, d.revision, d.manifest_json, d.updated_at
       FROM cluster_nodes relay_membership
       JOIN clusters c ON c.id = relay_membership.cluster_id
       JOIN cluster_nodes domestic_membership ON domestic_membership.cluster_id = c.id
       JOIN nodes domestic_node ON domestic_node.id = domestic_membership.node_id
       JOIN node_documents d ON d.node_id = domestic_node.id
      WHERE relay_membership.node_id = ?
        AND relay_membership.role = 'foreign'
        AND domestic_membership.role = 'domestic'
        AND domestic_node.node_class = 'domestic'
        AND domestic_node.status != 'revoked'
      ORDER BY c.id, d.node_id`,
  ).bind(relayNodeId).all<RelayDocumentRow>();
  return rows.results;
}

export async function nodeDocument(env: Env, nodeId: string): Promise<DocumentRow | null> {
  return env.DB.prepare(
    "SELECT revision, manifest_json, updated_at FROM node_documents WHERE node_id = ?",
  ).bind(nodeId).first<DocumentRow>();
}

export async function createEnrollmentGrant(env: Env, node: NodeRow): Promise<string> {
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
