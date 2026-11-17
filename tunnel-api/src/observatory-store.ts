import type { Env, NodeRow } from "./models";

export interface ObservatoryNode extends NodeRow {
  cpu_milli_percent: number | null;
  memory_milli_percent: number | null;
  load_1_milli: number | null;
  rx_bytes_per_sec: number | null;
  tx_bytes_per_sec: number | null;
  agent_rss_bytes: number | null;
  telemetry_sampled_at: number | null;
}

export interface NodeDocumentRow {
  node_id: string;
  manifest_json: string;
}

export interface LaneMetricRow {
  node_id: string;
  lane_id: string;
  state: "healthy" | "degraded" | "unknown";
  probe_ms: number | null;
  sampled_at: number;
}

export async function observatoryNodes(env: Env, ownerId: string, clusterId: string): Promise<ObservatoryNode[]> {
  const rows = await env.DB.prepare(
    `SELECT n.id, n.name, n.node_class, n.address, n.status, h.observed_at AS last_heartbeat_at,
            t.cpu_milli_percent, t.memory_milli_percent, t.load_1_milli, t.rx_bytes_per_sec,
            t.tx_bytes_per_sec, t.agent_rss_bytes, t.sampled_at AS telemetry_sampled_at
       FROM nodes n JOIN cluster_nodes cn ON cn.node_id = n.id
       JOIN clusters c ON c.id = cn.cluster_id
       LEFT JOIN node_heartbeats h ON h.node_id = n.id
       LEFT JOIN node_telemetry t ON t.node_id = n.id
      WHERE c.owner_id = ? AND c.id = ? ORDER BY n.node_class DESC, n.name`,
  ).bind(ownerId, clusterId).all<ObservatoryNode>();
  return rows.results;
}

export async function observatoryDocuments(env: Env, clusterId: string): Promise<NodeDocumentRow[]> {
  const rows = await env.DB.prepare(
    `SELECT d.node_id, d.manifest_json FROM node_documents d
       JOIN cluster_nodes cn ON cn.node_id = d.node_id
      WHERE cn.cluster_id = ?`,
  ).bind(clusterId).all<NodeDocumentRow>();
  return rows.results;
}

export async function observatoryLaneMetrics(env: Env, clusterId: string): Promise<LaneMetricRow[]> {
  const rows = await env.DB.prepare(
    `SELECT t.node_id, t.lane_id, t.state, t.probe_ms, t.sampled_at
       FROM lane_telemetry t JOIN cluster_nodes cn ON cn.node_id = t.node_id
      WHERE cn.cluster_id = ?`,
  ).bind(clusterId).all<LaneMetricRow>();
  return rows.results;
}
