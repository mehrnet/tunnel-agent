import type { Env } from "./models";
import { now } from "./validation";

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
