import type { Architecture, AuditReferences, BundleRow, ClusterRow, DocumentRow, Env, GrantRow, NodeRow, OwnerRow, RelayDocumentRow, RelayRow, TelegramSessionRow } from "./models";
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

export async function clusterById(env: Env, ownerId: string, clusterId: string): Promise<ClusterRow | null> {
  return env.DB.prepare("SELECT id, name FROM clusters WHERE owner_id = ? AND id = ?")
    .bind(ownerId, clusterId)
    .first<ClusterRow>();
}

export async function clustersForOwner(env: Env, ownerId: string): Promise<ClusterRow[]> {
  const rows = await env.DB.prepare("SELECT id, name FROM clusters WHERE owner_id = ? ORDER BY name")
    .bind(ownerId)
    .all<ClusterRow>();
  return rows.results;
}

export async function createClusterForOwner(env: Env, ownerId: string, name: string): Promise<ClusterRow> {
  const id = crypto.randomUUID();
  await env.DB.prepare("INSERT INTO clusters (id, owner_id, name, created_at) VALUES (?, ?, ?, ?)")
    .bind(id, ownerId, name, now())
    .run();
  return { id, name };
}

export async function nodeByName(env: Env, ownerId: string, name: string): Promise<NodeRow | null> {
  return env.DB.prepare(
    `SELECT n.id, n.name, n.node_class, n.address, n.status, h.observed_at AS last_heartbeat_at
       FROM nodes n LEFT JOIN node_heartbeats h ON h.node_id = n.id
      WHERE n.owner_id = ? AND n.name = ?`,
  )
    .bind(ownerId, name)
    .first<NodeRow>();
}

export async function nodeForOwnerById(env: Env, ownerId: string, nodeId: string): Promise<NodeRow | null> {
  return env.DB.prepare(
    `SELECT n.id, n.name, n.node_class, n.address, n.status, h.observed_at AS last_heartbeat_at
       FROM nodes n LEFT JOIN node_heartbeats h ON h.node_id = n.id
      WHERE n.owner_id = ? AND n.id = ?`,
  ).bind(ownerId, nodeId).first<NodeRow>();
}

export async function createNodeForOwner(
  env: Env,
  ownerId: string,
  name: string,
  nodeClass: NodeRow["node_class"],
  address: string | null,
  manifestJson: string,
): Promise<NodeRow> {
  const id = crypto.randomUUID();
  const createdAt = now();
  await env.DB.batch([
    env.DB.prepare(
      `INSERT INTO nodes (id, owner_id, name, node_class, address, created_at)
       VALUES (?, ?, ?, ?, ?, ?)`,
    ).bind(id, ownerId, name, nodeClass, address, createdAt),
    env.DB.prepare(
      "INSERT INTO node_documents (node_id, revision, manifest_json, updated_at) VALUES (?, ?, ?, ?)",
    ).bind(id, 0, manifestJson, createdAt),
  ]);
  return { id, name, node_class: nodeClass, address, status: "pending", last_heartbeat_at: null };
}

export async function attachNodeToCluster(env: Env, clusterId: string, nodeId: string, role: NodeRow["node_class"]): Promise<void> {
  await env.DB.prepare(
    "INSERT OR IGNORE INTO cluster_nodes (cluster_id, node_id, role, created_at) VALUES (?, ?, ?, ?)",
  ).bind(clusterId, nodeId, role, now()).run();
}

export async function nodesForOwner(env: Env, ownerId: string): Promise<NodeRow[]> {
  const rows = await env.DB.prepare(
    `SELECT n.id, n.name, n.node_class, n.address, n.status, h.observed_at AS last_heartbeat_at
       FROM nodes n LEFT JOIN node_heartbeats h ON h.node_id = n.id
      WHERE n.owner_id = ? ORDER BY n.name`,
  ).bind(ownerId).all<NodeRow>();
  return rows.results;
}

export async function nodesForCluster(env: Env, ownerId: string, clusterId: string): Promise<NodeRow[]> {
  const rows = await env.DB.prepare(
    `SELECT n.id, n.name, n.node_class, n.address, n.status, h.observed_at AS last_heartbeat_at
       FROM nodes n JOIN cluster_nodes cn ON cn.node_id = n.id
       JOIN clusters c ON c.id = cn.cluster_id
       LEFT JOIN node_heartbeats h ON h.node_id = n.id
      WHERE c.owner_id = ? AND c.id = ? ORDER BY n.name`,
  ).bind(ownerId, clusterId).all<NodeRow>();
  return rows.results;
}

export async function unattachedNodesForCluster(env: Env, ownerId: string, clusterId: string): Promise<NodeRow[]> {
  const rows = await env.DB.prepare(
    `SELECT n.id, n.name, n.node_class, n.address, n.status, h.observed_at AS last_heartbeat_at
       FROM nodes n JOIN clusters c ON c.owner_id = n.owner_id
       LEFT JOIN node_heartbeats h ON h.node_id = n.id
      WHERE c.owner_id = ? AND c.id = ?
        AND NOT EXISTS (
          SELECT 1 FROM cluster_nodes cn WHERE cn.cluster_id = c.id AND cn.node_id = n.id
        ) ORDER BY n.name`,
  ).bind(ownerId, clusterId).all<NodeRow>();
  return rows.results;
}

export async function clusterMembershipCount(env: Env, nodeId: string): Promise<number> {
  const row = await env.DB.prepare("SELECT COUNT(*) AS count FROM cluster_nodes WHERE node_id = ?")
    .bind(nodeId)
    .first<{ count: number }>();
  return row?.count ?? 0;
}

export async function telegramSession(env: Env, ownerId: string, chatId: number): Promise<TelegramSessionRow | null> {
  return env.DB.prepare(
    `SELECT owner_id, chat_id, mode, payload_json, menu_message_id, menu_created_at, updated_at
       FROM telegram_sessions WHERE owner_id = ? AND chat_id = ?`,
  ).bind(ownerId, String(chatId)).first<TelegramSessionRow>();
}

export async function setTelegramSessionState(
  env: Env,
  ownerId: string,
  chatId: number,
  mode: string,
  payload: unknown,
): Promise<TelegramSessionRow> {
  await env.DB.prepare(
    `INSERT INTO telegram_sessions (owner_id, chat_id, mode, payload_json, updated_at)
     VALUES (?, ?, ?, ?, ?)
     ON CONFLICT(owner_id, chat_id) DO UPDATE SET
       mode = excluded.mode, payload_json = excluded.payload_json, updated_at = excluded.updated_at`,
  ).bind(ownerId, String(chatId), mode, JSON.stringify(payload), now()).run();
  const session = await telegramSession(env, ownerId, chatId);
  if (!session) throw new Error("cannot read Telegram session");
  return session;
}

export async function setTelegramMenu(
  env: Env,
  ownerId: string,
  chatId: number,
  messageId: number,
  createdAt: number,
): Promise<void> {
  await env.DB.prepare(
    `UPDATE telegram_sessions SET menu_message_id = ?, menu_created_at = ?, updated_at = ?
      WHERE owner_id = ? AND chat_id = ?`,
  ).bind(messageId, createdAt, now(), ownerId, String(chatId)).run();
}

export async function nodeById(env: Env, nodeId: string): Promise<NodeRow | null> {
  return env.DB.prepare(
    `SELECT n.id, n.name, n.node_class, n.address, n.status, h.observed_at AS last_heartbeat_at
       FROM nodes n LEFT JOIN node_heartbeats h ON h.node_id = n.id
      WHERE n.id = ?`,
  ).bind(nodeId).first<NodeRow>();
}

export async function requireGrant(env: Env, rawGrant: unknown): Promise<GrantRow | null> {
  const grant = parseGrant(rawGrant);
  if (!grant) return null;
  const hash = await sha256(grant.secret);
  const row = await env.DB.prepare(
    `SELECT g.id AS grant_id, g.secret_hash, g.expires_at, g.consumed_at,
            n.id, n.name, n.node_class, n.address, n.status, h.observed_at AS last_heartbeat_at
       FROM enrollment_grants g
       JOIN nodes n ON n.id = g.node_id
       LEFT JOIN node_heartbeats h ON h.node_id = n.id
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

export async function foreignClusterMember(env: Env, nodeId: string, clusterId: string): Promise<boolean> {
  const row = await env.DB.prepare(
    "SELECT 1 AS present FROM cluster_nodes WHERE node_id = ? AND cluster_id = ? AND role = 'foreign'",
  ).bind(nodeId, clusterId).first<{ present: number }>();
  return row?.present === 1;
}

export async function foreignCanObserveDomestic(
  env: Env,
  foreignNodeId: string,
  domesticNodeId: string,
): Promise<boolean> {
  const row = await env.DB.prepare(
    `SELECT 1 AS present
       FROM cluster_nodes foreign_membership
       JOIN cluster_nodes domestic_membership
         ON domestic_membership.cluster_id = foreign_membership.cluster_id
       JOIN nodes domestic_node ON domestic_node.id = domestic_membership.node_id
      WHERE foreign_membership.node_id = ?
        AND foreign_membership.role = 'foreign'
        AND domestic_membership.node_id = ?
        AND domestic_membership.role = 'domestic'
        AND domestic_node.node_class = 'domestic'
        AND domestic_node.status != 'revoked'
      LIMIT 1`,
  ).bind(foreignNodeId, domesticNodeId).first<{ present: number }>();
  return row?.present === 1;
}

export async function recordNodeHeartbeat(
  env: Env,
  domesticNodeId: string,
  foreignNodeId: string,
  observedAt: number,
): Promise<void> {
  await env.DB.prepare(
    `INSERT INTO node_heartbeats (node_id, observer_node_id, observed_at, recorded_at)
     VALUES (?, ?, ?, ?)
     ON CONFLICT(node_id) DO UPDATE SET
       observer_node_id = excluded.observer_node_id,
       observed_at = excluded.observed_at,
       recorded_at = excluded.recorded_at
     WHERE excluded.observed_at > node_heartbeats.observed_at`,
  ).bind(domesticNodeId, foreignNodeId, observedAt, now()).run();
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
