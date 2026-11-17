import { error, json } from "./http";
import type { Env } from "./models";
import { observatoryDocuments, observatoryLaneMetrics, observatoryNodes } from "./observatory-store";
import { clusterById, clustersForOwner } from "./store";
import { ownerFromTelegram } from "./web-auth";

type LaneState = "healthy" | "degraded" | "unknown";

export async function handleWebClusters(request: Request, env: Env): Promise<Response> {
  if (request.method !== "GET") return error("method not allowed", 405);
  const owner = await ownerFromTelegram(request, env);
  if (!owner) return error("Telegram authorization required", 401);
  return json({ clusters: await clustersForOwner(env, owner.id) });
}

export async function handleWebCluster(request: Request, env: Env, clusterId: string): Promise<Response> {
  if (request.method !== "GET") return error("method not allowed", 405);
  const owner = await ownerFromTelegram(request, env);
  if (!owner) return error("Telegram authorization required", 401);
  const cluster = await clusterById(env, owner.id, clusterId);
  if (!cluster) return error("cluster not found", 404);
  const [nodes, documents, metrics] = await Promise.all([
    observatoryNodes(env, owner.id, cluster.id), observatoryDocuments(env, cluster.id), observatoryLaneMetrics(env, cluster.id),
  ]);
  const metricByLane = new Map(metrics.map((metric) => [`${metric.node_id}:${metric.lane_id}`, metric]));
  return json({
    generated_at: Math.floor(Date.now() / 1000),
    cluster,
    nodes: nodes.map(toNode),
    lanes: documents.flatMap((document) => lanesFromDocument(document.node_id, document.manifest_json, metricByLane)),
  });
}

function toNode(node: Awaited<ReturnType<typeof observatoryNodes>>[number]) {
  const reported = node.telemetry_sampled_at !== null;
  return {
    id: node.id, name: node.name, address: node.address, node_class: node.node_class, status: node.status,
    last_heartbeat_at: node.last_heartbeat_at,
    telemetry: !reported ? null : {
      sampled_at: node.telemetry_sampled_at, cpu_percent: node.cpu_milli_percent! / 1000,
      memory_percent: node.memory_milli_percent! / 1000, load_1: node.load_1_milli! / 1000,
      rx_bytes_per_sec: node.rx_bytes_per_sec!, tx_bytes_per_sec: node.tx_bytes_per_sec!, agent_rss_bytes: node.agent_rss_bytes!,
    },
  };
}

function lanesFromDocument(sourceNodeId: string, raw: string, metrics: Map<string, { state: LaneState; probe_ms: number | null; sampled_at: number }>) {
  const agent = parseAgent(raw);
  if (!agent) return [];
  const selected = new Set(agent.lanes.filter((lane) => lane.enabled).sort((left, right) => left.priority - right.priority).slice(0, agent.maxActive).map((lane) => lane.id));
  return agent.lanes.map((lane) => {
    const metric = metrics.get(`${sourceNodeId}:${lane.id}`);
    return {
      id: lane.id, transport: lane.transport, direction: lane.direction, priority: lane.priority, selected: selected.has(lane.id),
      source_node_id: sourceNodeId, peer_node_id: lane.peerNodeId, local_listener: lane.listener,
      state: metric?.state ?? "unknown", probe_ms: metric?.probe_ms ?? null, observed_at: metric?.sampled_at ?? null,
    };
  });
}

function parseAgent(raw: string): { maxActive: number; lanes: ParsedLane[] } | null {
  try {
    const document: unknown = JSON.parse(raw);
    if (!record(document) || !record(document.agent) || !Array.isArray(document.agent.lanes)) return null;
    const maxActive = bounded(document.agent.max_active_lanes, 1, 16) ?? 1;
    const lanes = document.agent.lanes.map(parseLane).filter((lane): lane is ParsedLane => lane !== null);
    return { maxActive, lanes };
  } catch {
    return null;
  }
}

interface ParsedLane { id: string; enabled: boolean; transport: string; direction: string; priority: number; peerNodeId: string; listener: string }

function parseLane(value: unknown): ParsedLane | null {
  if (!record(value) || !record(value.peer) || !record(value.listener)) return null;
  const id = text(value.id, 128); const transport = text(value.transport, 32); const direction = text(value.direction, 32); const peerNodeId = text(value.peer.node_id, 36);
  const bind = text(value.listener.bind, 64); const port = bounded(value.listener.port, 1, 65535); const priority = bounded(value.priority, 0, 65535);
  if (!id || !transport || !direction || !peerNodeId || !bind || port === null || priority === null) return null;
  return { id, enabled: value.enabled !== false, transport, direction, priority, peerNodeId, listener: `${bind}:${port}` };
}

function record(value: unknown): value is Record<string, unknown> { return value !== null && typeof value === "object" && !Array.isArray(value); }
function text(value: unknown, maximum: number): string | null { return typeof value === "string" && value.length > 0 && value.length <= maximum ? value : null; }
function bounded(value: unknown, minimum: number, maximum: number): number | null { return typeof value === "number" && Number.isInteger(value) && value >= minimum && value <= maximum ? value : null; }
