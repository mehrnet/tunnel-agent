import { MAX_MANIFEST_BYTES } from "./documents";
import type { Env } from "./models";
import { managedContracts } from "./managed-transport";
import { wstunnelContracts, type Intent } from "./tunnel-spec";
import { now } from "./validation";

interface Endpoint {
  id: string;
  address: string | null;
  status: string;
  node_class: string;
  sampled_at: number | null;
  reporter_node_id: string | null;
  transport_keys_json: string | null;
  revision: number;
  manifest_json: string;
}

interface Contract {
  id: string;
  contract: string;
  listener?: { port: number };
  role?: string;
  control_port?: number;
}

interface Manifest {
  version: number;
  tunnels: Contract[];
  agent: { lanes: Array<{ listener?: { port: number }; hpnssh?: { server?: { port: number } } }> };
  public_ingress?: Array<{ port: number; lane_id: string; fallback_lane_id: string | null }>;
}

export async function getIntent(env: Env, cluster: string, id: string) {
  return env.DB.prepare("SELECT * FROM tunnel_intents WHERE cluster_id=? AND id=? AND path_key IS NOT NULL")
    .bind(cluster, id).first<Intent>();
}

export type ManagedLane = Omit<Intent, "path_key" | "cluster_id"> & {
  direction: string; listener_bind: string; updated_at: number;
};

export async function managedLanes(env: Env, clusterId: string): Promise<ManagedLane[]> {
  const rows = await env.DB.prepare(
    `SELECT id,source_node_id,listener_node_id,transport,direction,listener_type,listener_bind,listener_port,control_port,tls_server_name,
     destination_host,destination_port,enabled,revision,updated_at FROM tunnel_intents
     WHERE cluster_id=? AND path_key IS NOT NULL ORDER BY created_at,id`,
  ).bind(clusterId).all<ManagedLane>();
  return rows.results;
}

async function endpoints(env: Env, intent: Intent) {
  const { results } = await env.DB.prepare(
    `SELECT n.id,n.address,n.status,n.node_class,d.revision,d.manifest_json,
            t.sampled_at,t.reporter_node_id,t.transport_keys_json FROM nodes n
     JOIN cluster_nodes m ON m.node_id=n.id JOIN node_documents d ON d.node_id=n.id
     LEFT JOIN node_telemetry t ON t.node_id=n.id
     WHERE m.cluster_id=? AND n.id IN (?,?) AND n.status!='revoked'`,
  ).bind(intent.cluster_id, intent.source_node_id, intent.listener_node_id).all<Endpoint>();
  const source = results.find((row) => row.id === intent.source_node_id);
  const listener = results.find((row) => row.id === intent.listener_node_id);
  if (!source || !listener || !listener.address) throw new Error("Both servers must belong to this cluster and the listener needs an address");
  if (intent.enabled && results.some((row) => row.status !== "enrolled")) throw new Error("Both servers must be enrolled before enabling");
  return { source, listener, host: listener.address };
}

function manifest(row: Endpoint, id: string): Manifest {
  const value = JSON.parse(row.manifest_json) as Manifest;
  if (value.version !== 3 || !Array.isArray(value.tunnels) || !Array.isArray(value.agent?.lanes)) {
    throw new Error("Both servers need version 3 manifests");
  }
  value.tunnels = value.tunnels.filter((entry) => entry.id !== `owned-${id}`);
  return value;
}

function checkPorts(doc: Manifest, intent: Intent) {
  const ports = new Set([intent.control_port, intent.listener_port]);
  if (doc.public_ingress?.some((route) => ports.has(route.port))) throw new Error("Port reserved for public ingress");
  for (const lane of doc.agent.lanes) {
    if (ports.has(lane.listener?.port ?? 0) || ports.has(lane.hpnssh?.server?.port ?? 0)) throw new Error("Port reserved by a management lane");
  }
  for (const entry of doc.tunnels) {
    if (conflicts(entry, ports)) throw new Error("Existing listener conflict or unmanaged contract with unknown port claims");
  }
}

function conflicts(entry: Contract, ports: Set<number>) {
  if (entry.contract === "wstunnel.server.v1") return ports.has(entry.listener?.port ?? 0);
  if (!entry.id.startsWith("owned-")) return true;
  if (entry.contract === "wstunnel.reverse.v1") return false;
  if (entry.contract === "managed.reverse.v1") {
    return entry.role !== "client" && (ports.has(entry.listener?.port ?? 0) || ports.has(entry.control_port ?? 0));
  }
  return true;
}

async function reservePorts(env: Env, intent: Intent) {
  const found = await env.DB.prepare(
    `SELECT 1 FROM tunnel_intents WHERE listener_node_id=? AND id!=?
     AND (listener_port IN (?,?) OR control_port IN (?,?)) LIMIT 1`,
  ).bind(intent.listener_node_id, intent.id, intent.listener_port, intent.control_port, intent.listener_port, intent.control_port).first();
  if (found) throw new Error("Listener or control port already reserved");
}

async function requireUnprobedLane(env: Env, id: string) {
  const active = await env.DB.prepare(
    "SELECT 1 FROM probe_jobs WHERE lane_id=? AND ends_at>? AND cancelled_at IS NULL LIMIT 1",
  ).bind(id, now()).first();
  if (active) throw new Error("Cancel the running probe job before changing this lane");
}

function pairWrite(env: Env, intent: Intent, source: Endpoint, listener: Endpoint, documents: [Manifest, Manifest], time: number, creating: boolean) {
  const encoded = documents.map((doc) => JSON.stringify(doc));
  if (encoded.some((doc) => new TextEncoder().encode(doc).byteLength > MAX_MANIFEST_BYTES)) {
    throw new Error("Generated server document exceeds the size limit");
  }
  return env.DB.prepare(
    `UPDATE node_documents SET manifest_json=CASE WHEN node_id=? THEN ? ELSE ? END,
     revision=revision+1, updated_at=?
     WHERE (node_id=? AND revision=? OR node_id=? AND revision=?)
     AND (?=1 OR EXISTS (SELECT 1 FROM tunnel_intents WHERE id=? AND revision=?))
     AND EXISTS (SELECT 1 FROM cluster_nodes m JOIN nodes n ON n.id=m.node_id
                 WHERE m.node_id=node_documents.node_id AND m.cluster_id=?
                   AND n.status!='revoked' AND (?=0 OR n.status='enrolled'))`,
  ).bind(source.id, encoded[0], encoded[1], time, source.id, source.revision, listener.id, listener.revision,
    Number(creating), intent.id, intent.revision,
    intent.cluster_id, intent.enabled);
}

function insertIntent(env: Env, intent: Intent, time: number) {
  return env.DB.prepare(
    `INSERT INTO tunnel_intents(id,cluster_id,source_node_id,peer_node_id,transport,direction,listener_type,
      listener_node_id,listener_bind,listener_port,control_port,tls_server_name,path_key,destination_host,destination_port,enabled,created_at,updated_at)
     VALUES(?,?,?,?,?,'reverse',?,?,'127.0.0.1',?,?,?,?,?,?,CASE WHEN changes()=2 THEN 0 ELSE NULL END,?,?)`,
  ).bind(intent.id, intent.cluster_id, intent.source_node_id, intent.listener_node_id, intent.transport, intent.listener_type, intent.listener_node_id,
    intent.listener_port, intent.control_port, intent.tls_server_name, intent.path_key, intent.destination_host, intent.destination_port, time, time);
}

function updateIntent(env: Env, intent: Intent, time: number) {
  return env.DB.prepare(
    `UPDATE tunnel_intents SET path_key=?,destination_host=?,destination_port=?,revision=revision+1,updated_at=?,
     enabled=CASE WHEN changes()=2 AND revision=? THEN ? ELSE NULL END WHERE id=?`,
  ).bind(intent.path_key, intent.destination_host, intent.destination_port, time, intent.revision, intent.enabled, intent.id);
}

export async function reconcileTunnel(env: Env, intent: Intent, operation: "create" | "update" | "remove") {
  const { source, listener, host } = await endpoints(env, intent);
  const sourceDoc = manifest(source, intent.id);
  const listenerDoc = manifest(listener, intent.id);
  if (operation !== "create" && listenerDoc.public_ingress?.some(
    (route) => route.lane_id === intent.id || route.fallback_lane_id === intent.id,
  )) throw new Error("Remove this lane from public ingress before changing it");
  if (operation !== "create") await requireUnprobedLane(env, intent.id);
  if (operation !== "remove") {
    await reservePorts(env, intent);
    checkPorts(listenerDoc, intent);
    const pair = intent.transport === "wstunnel" ? wstunnelContracts(intent, host)
      : intent.enabled ? managedContracts(intent, source, listener) : null;
    if (pair) {
      sourceDoc.tunnels.push(pair.client);
      listenerDoc.tunnels.push(pair.server);
    }
  }
  const timestamp = now();
  const statements = [
    pairWrite(env, intent, source, listener, [sourceDoc, listenerDoc], timestamp, operation === "create"),
    operation === "create" ? insertIntent(env, intent, timestamp) : updateIntent(env, intent, timestamp),
  ];
  if (operation === "remove") statements.push(env.DB.prepare("DELETE FROM tunnel_intents WHERE id=?").bind(intent.id));
  const [written] = await env.DB.batch(statements);
  if (written.meta.changes !== 2) throw new Error("Tunnel changed concurrently; reload and retry");
}
