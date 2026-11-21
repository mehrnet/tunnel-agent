import { MAX_MANIFEST_BYTES } from "./documents";
import type { Env } from "./models";
import { getIntent } from "./tunnel-store";
import { validTunnelHost, type Intent } from "./tunnel-spec";
import { now } from "./validation";

interface IngressRoute {
  cluster_id: string;
  lane_id: string;
  fallback_lane_id: string | null;
  address: string;
  port: number;
  primary: number;
  fallback: number | null;
  tls_server_name: string | null;
  forward_udp?: boolean;
}

interface IngressDocument {
  version: number;
  public_ingress?: IngressRoute[];
  agent: { lanes: Array<{
    listener?: { port?: number };
    hpnssh?: { server?: { port?: number }; client?: { target?: { port?: number } } };
  }> };
}

interface ListenerNode {
  id: string;
  address: string | null;
  revision: number;
  manifest_json: string;
  public_forward_v2: number | null;
}

const UUID = /^[a-f0-9]{8}(?:-[a-f0-9]{4}){3}-[a-f0-9]{12}$/;

export function presentIngress(documents: Array<{ node_id: string; manifest_json: string; revision: number }>, cluster: string) {
  return documents.flatMap((row) => {
    const doc = JSON.parse(row.manifest_json) as IngressDocument;
    return (doc.public_ingress ?? []).filter((route) => route.cluster_id === cluster)
      .map(({ port, lane_id, fallback_lane_id, tls_server_name, forward_udp }) =>
        ({ node_id: row.node_id, port, lane_id, fallback_lane_id, tls_server_name, forward_udp: !!forward_udp, revision: row.revision }));
  });
}

async function permittedNode(env: Env, cluster: string, id: string): Promise<ListenerNode> {
  const node = await env.DB.prepare(
    `SELECT n.id,n.address,d.revision,d.manifest_json,
            json_extract(t.transport_keys_json,'$.public_forward_v2') AS public_forward_v2
       FROM cluster_nodes m JOIN nodes n ON n.id=m.node_id
       JOIN node_documents d ON d.node_id=n.id JOIN node_telemetry t ON t.node_id=n.id
      WHERE m.cluster_id=? AND m.node_id=? AND n.status='enrolled'
        AND t.sampled_at>=? AND json_extract(t.transport_keys_json,'$.kernel_ingress')=1`,
  ).bind(cluster, id, now() - 300).first<ListenerNode>();
  if (!node) throw new Error("The listener must report support for kernel ingress");
  return node;
}

function addressOf(node: ListenerNode) {
  const value = node.address;
  if (!value || !/^(?:\d{1,3}\.){3}\d{1,3}$/.test(value)) throw new Error("Listener requires a local IPv4 address");
  const [first] = value.split(".").map(Number);
  if (first === 0 || first === 127 || first >= 224 ||
      !value.split(".").every((part) => String(Number(part)) === part && Number(part) <= 255)) {
    throw new Error("Listener requires a local unicast IPv4 address");
  }
  return value;
}

async function requireLane(env: Env, cluster: string, node: string, value: unknown): Promise<Intent> {
  if (typeof value !== "string" || !UUID.test(value)) throw new Error("Select a valid tunnel lane");
  const lane = await getIntent(env, cluster, value);
  if (!lane?.enabled || lane.listener_type !== "tcp" || lane.listener_node_id !== node) {
    throw new Error("Public ingress requires an enabled TCP lane on the selected listener");
  }
  return lane;
}

function sameDestination(a: Intent, b: Intent) {
  return a.source_node_id === b.source_node_id
    && a.destination_host === b.destination_host && a.destination_port === b.destination_port;
}

async function checkPublicClaim(env: Env, node: ListenerNode, doc: IngressDocument, port: number) {
  const reserved = doc.agent.lanes.some((lane) =>
    [lane.listener?.port, lane.hpnssh?.server?.port, lane.hpnssh?.client?.target?.port].includes(port));
  const tunnel = await env.DB.prepare(
    "SELECT 1 FROM tunnel_intents WHERE listener_node_id=? AND (listener_port=? OR control_port=?) LIMIT 1",
  ).bind(node.id, port, port).first();
  if (reserved || tunnel) throw new Error("Public port is already reserved by a tunnel or management listener");
}

function validateRequest(body: Record<string, unknown>) {
  const { node_id: id, port, revision } = body;
  if (typeof id !== "string" || !UUID.test(id)) throw new Error("Select a valid listening server");
  if (typeof port !== "number" || !Number.isInteger(port) || port <= 0 || port > 65535) throw new Error("Invalid public port");
  if (typeof revision !== "number" || !Number.isSafeInteger(revision) || revision < 0) throw new Error("Missing server document revision");
  return { id, port, revision };
}

async function desiredRoute(env: Env, cluster: string, node: ListenerNode, port: number,
  body: Record<string, unknown>, original?: IngressRoute): Promise<IngressRoute> {
  const primary = await requireLane(env, cluster, node.id, body.lane_id);
  const route = { cluster_id: cluster, address: addressOf(node), port, lane_id: primary.id, primary: primary.listener_port };
  if (primary.forward_udp) {
    if (!node.public_forward_v2 || primary.transport !== "wstunnel" || original && !original.forward_udp) {
      throw new Error("Fixed TCP+UDP ingress requires a compatible agent and WSTunnel lane");
    }
    if (body.fallback_lane_id || body.tls_server_name) throw new Error("Fixed TCP+UDP ingress does not use TLS health checks or fallback");
    return { ...route, fallback_lane_id: null, fallback: null, tls_server_name: null, forward_udp: true };
  }
  if (original?.forward_udp) throw new Error("Remove this TCP+UDP mapping before replacing it with a TCP-only route");
  const requested = selectFallback(body, primary.id, original);
  const fallback = await requireLane(env, cluster, node.id, requested);
  if (fallback.id === primary.id || fallback.forward_udp || !sameDestination(primary, fallback)) {
    throw new Error("Fallback must be a separate lane reaching the same destination from the same connector");
  }
  const hostname = body.tls_server_name;
  if (typeof hostname !== "string" || !hostname.includes(".") || !/[a-z]/i.test(hostname) || !validTunnelHost(hostname)) {
    throw new Error("Provide the destination's TLS server name");
  }
  return { ...route, fallback_lane_id: fallback.id, fallback: fallback.listener_port, tls_server_name: hostname };
}

function selectFallback(body: Record<string, unknown>, primary: string, original?: IngressRoute) {
  if (!original) return body.fallback_lane_id;
  if (primary === original.lane_id) return original.fallback_lane_id;
  if (primary === original.fallback_lane_id) return original.lane_id;
  throw new Error("This public port can switch only between its reserved pair of lanes");
}

export async function saveIngress(env: Env, cluster: string, body: Record<string, unknown>) {
  const { id, port, revision } = validateRequest(body);
  const node = await permittedNode(env, cluster, id);
  if (node.revision !== revision) throw new Error("Server configuration changed; reload before updating public ingress");
  const document = JSON.parse(node.manifest_json) as IngressDocument;
  const routes = document.public_ingress ?? [];
  if (document.version !== 3 || !Array.isArray(routes) || !Array.isArray(document.agent?.lanes)) {
    throw new Error("Public ingress requires a version 3 server document");
  }
  const original = routes.find((route) => route.port === port);
  if (original && original.cluster_id !== cluster) throw new Error("Public port belongs to a different cluster");
  await changeIngress(env, cluster, node, document, port, body, original);
  const encoded = JSON.stringify(document);
  if (new TextEncoder().encode(encoded).byteLength > MAX_MANIFEST_BYTES) {
    throw new Error("Server document size limit exceeded");
  }
  const result = await env.DB.prepare(
    `UPDATE node_documents SET manifest_json=?, revision=revision+1, updated_at=?
      WHERE node_id=? AND revision=? AND EXISTS (
        SELECT 1 FROM cluster_nodes m JOIN nodes n ON n.id=m.node_id
        WHERE m.node_id=node_documents.node_id AND m.cluster_id=? AND n.status='enrolled')`,
  ).bind(encoded, now(), node.id, revision, cluster).run();
  if (result.meta.changes !== 1) throw new Error("Server configuration changed; reload before updating public ingress");
  return { revision: revision + 1 };
}

async function changeIngress(env: Env, cluster: string, node: ListenerNode, document: IngressDocument,
  port: number, body: Record<string, unknown>, original?: IngressRoute) {
  const routes = document.public_ingress ?? [];
  document.public_ingress = routes.filter((route) => route.port !== port);
  if (body.action === "remove") {
    if (!original) throw new Error("Public port mapping not found");
    return;
  }
  if (body.action !== "set") throw new Error("Unknown public ingress action");
  if (!node.public_forward_v2 && !original && routes.length >= 16) {
    throw new Error("Upgrade this agent before adding more than 16 public ports");
  }
  await checkPublicClaim(env, node, document, port);
  document.public_ingress.push(await desiredRoute(env, cluster, node, port, body, original));
}
