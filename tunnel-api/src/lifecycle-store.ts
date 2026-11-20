import type { Env, NodeRow, OwnerRow } from "./models";
import { now } from "./validation";

export async function transitionNode(env: Env, owner: OwnerRow, node: NodeRow & { public_key: string | null; enrolled_at: number | null }, action: "reinstall" | "rotate" | "retire"): Promise<void> {
  if (node.status === "revoked") throw new Error("Retired nodes cannot be reactivated; register a new server identity.");
  const timestamp = now();
  const eventId = crypto.randomUUID();
  const statements = [
    env.DB.prepare("UPDATE nodes SET status = ?, public_key = NULL, enrolled_at = NULL WHERE id = ? AND owner_id = ? AND status = ? AND public_key IS ? AND enrolled_at IS ?")
      .bind(action === "retire" ? "revoked" : "pending", node.id, owner.id, node.status, node.public_key, node.enrolled_at),
    // A failed compare must raise inside the batch, not after its writes commit.
    env.DB.prepare("INSERT INTO node_lifecycle_events(id,node_id,owner_id,action,created_at) VALUES (?,?,?,CASE WHEN changes()=1 THEN ? ELSE NULL END,?)")
      .bind(eventId, node.id, owner.id, action, timestamp),
    env.DB.prepare("DELETE FROM node_credentials WHERE node_id = ?").bind(node.id),
    env.DB.prepare("UPDATE enrollment_grants SET consumed_at = ? WHERE node_id = ? AND consumed_at IS NULL").bind(timestamp, node.id),
    env.DB.prepare("DELETE FROM node_heartbeats WHERE node_id = ?").bind(node.id),
    env.DB.prepare("DELETE FROM node_telemetry WHERE node_id = ?").bind(node.id),
    env.DB.prepare("DELETE FROM lane_telemetry WHERE node_id = ?").bind(node.id),
  ];
  if (action === "retire") {
    const peers = await env.DB.prepare(
      `SELECT DISTINCT d.node_id,d.manifest_json,d.revision FROM node_documents d JOIN cluster_nodes p ON p.node_id=d.node_id
       JOIN cluster_nodes target ON target.cluster_id=p.cluster_id WHERE target.node_id=?`,
    ).bind(node.id).all<{ node_id: string; manifest_json: string; revision: number }>();
    statements.push(env.DB.prepare(
      `UPDATE node_lifecycle_events SET action=CASE WHEN NOT EXISTS (
         SELECT 1 FROM cluster_nodes target JOIN cluster_nodes p ON p.cluster_id=target.cluster_id
         JOIN node_documents d ON d.node_id=p.node_id WHERE target.node_id=?
         AND d.node_id NOT IN (SELECT value FROM json_each(?))
       ) THEN action ELSE NULL END WHERE id=?`,
    ).bind(node.id, JSON.stringify(peers.results.map((peer) => peer.node_id)), eventId));
    for (const peer of peers.results) {
      const document = JSON.parse(peer.manifest_json) as { agent?: { lanes?: Array<{ peer?: { node_id?: string } }> }; tunnels?: unknown[] };
      // Customer contracts do not consistently identify peers yet. Fail closed rather than leave active orphan tunnels.
      if (document.tunnels?.length) throw new Error("Retire requires removing customer tunnel contracts from affected cluster members first.");
      if (document.agent?.lanes) document.agent.lanes = document.agent.lanes.filter((lane) => lane.peer?.node_id !== node.id);
      statements.push(env.DB.prepare("UPDATE node_documents SET manifest_json=?, revision=revision+1, updated_at=? WHERE node_id=? AND revision=? AND manifest_json=?")
        .bind(JSON.stringify(document), timestamp, peer.node_id, peer.revision, peer.manifest_json));
      statements.push(env.DB.prepare("UPDATE node_lifecycle_events SET action=CASE WHEN changes()=1 THEN action ELSE NULL END WHERE id=?").bind(eventId));
    }
    statements.push(env.DB.prepare("DELETE FROM cluster_nodes WHERE node_id = ?").bind(node.id));
  }
  await env.DB.batch(statements);
}
