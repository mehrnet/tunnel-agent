import { error, json } from "./http";
import { presentIngress } from "./ingress-store";
import type { Env } from "./models";
import { listProbes } from "./probe-store";
import { observatoryLanes } from "./observatory-lanes";
import { observatoryDocuments, observatoryLaneMetrics, observatoryNodes } from "./observatory-store";
import { clusterById, clustersForOwner, createClusterForOwner } from "./store";
import { ownerFromTelegram } from "./web-auth";
import { isReleaseOperator } from "./release-auth";
import { managedLanes } from "./tunnel-store";
import { now } from "./validation";

export async function handleWebClusters(request: Request, env: Env): Promise<Response> {
  if (!["GET", "POST"].includes(request.method)) return error("method not allowed", 405);
  const owner = await ownerFromTelegram(request, env);
  if (!owner) return error("Telegram authorization required", 401);
  if (request.method === "POST") {
    const body: unknown = await request.json().catch(() => null);
    const name = record(body) && typeof body.name === "string" ? body.name.trim() : "";
    if (!name || name.length > 80) return error("Cluster name must be 1–80 characters", 400);
    return json({ cluster: await createClusterForOwner(env, owner.id, name) });
  }
  return json({ clusters: await clustersForOwner(env, owner.id), release_operator: isReleaseOperator(env, owner) });
}

export async function handleWebCluster(request: Request, env: Env, clusterId: string): Promise<Response> {
  if (request.method !== "GET") return error("method not allowed", 405);
  const owner = await ownerFromTelegram(request, env);
  if (!owner) return error("Telegram authorization required", 401);
  const cluster = await clusterById(env, owner.id, clusterId);
  if (!cluster) return error("cluster not found", 404);
  const [nodes, documents, metrics, managed, probeJobs] = await Promise.all([
    observatoryNodes(env, owner.id, cluster.id), observatoryDocuments(env, cluster.id), observatoryLaneMetrics(env, cluster.id),
    managedLanes(env, cluster.id), listProbes(env, cluster.id),
  ]);
  const timestamp = now();
  const map = observatoryLanes(documents, metrics, timestamp);
  return json({
    generated_at: timestamp,
    cluster,
    nodes: nodes.map((node) => toNode(node, map.expiresAt(node.last_heartbeat_at, node.id, node.heartbeat_reporter_id))),
    document_revisions: Object.fromEntries(documents.map(({node_id,revision}) => [node_id, revision])),
    public_ingress: presentIngress(documents, cluster.id),
    probe_jobs: probeJobs,
    lanes: await map.lanes(nodes, managed),
  });
}

function toNode(node: Awaited<ReturnType<typeof observatoryNodes>>[number], heartbeatExpiry: number | null) {
  const reported = node.telemetry_sampled_at !== null;
  return {
    id: node.id, name: node.name, address: node.address, node_class: node.node_class,
    status: node.status,
    last_heartbeat_at: node.last_heartbeat_at,
    heartbeat_expires_at: heartbeatExpiry,
    telemetry: !reported ? null : {
      sampled_at: node.telemetry_sampled_at, cpu_percent: node.cpu_milli_percent! / 1000,
      memory_percent: node.memory_milli_percent! / 1000, load_1: node.load_1_milli! / 1000,
      rx_bytes_per_sec: node.rx_bytes_per_sec!, tx_bytes_per_sec: node.tx_bytes_per_sec!, agent_rss_bytes: node.agent_rss_bytes!,
    },
  };
}

function record(value: unknown): value is Record<string, unknown> { return value !== null && typeof value === "object" && !Array.isArray(value); }
