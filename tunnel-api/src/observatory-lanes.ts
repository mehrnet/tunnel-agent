import type { NodeDocumentRow, LaneMetricRow } from "./observatory-store";
import type { ManagedLane } from "./tunnel-store";
import { DEFAULT_REPORT_INTERVAL_SECONDS, sha256 } from "./validation";

interface ParsedLane {
  id: string;
  enabled: boolean;
  transport: string;
  direction: string;
  priority: number;
  peerNodeId: string;
  listener: string;
  client: boolean;
}

interface Snapshot {
  nodeId: string;
  updatedAt: number;
  interval: number;
  maxActive: number;
  lanes: ParsedLane[];
  tunnels: Record<string, unknown>[];
}

interface Peer { id: string; address: string | null }
type ProbeScope = "peer_control" | "local_control" | "local_listener";

function record(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function bounded(value: unknown, min: number, max: number): number | null {
  return typeof value === "number" && Number.isInteger(value) && value >= min && value <= max ? value : null;
}

function listenerAddress(value: unknown): string | null {
  if (!record(value) || typeof value.bind !== "string" || bounded(value.port, 1, 65535) === null) return null;
  const bind = value.bind.includes(":") ? `[${value.bind}]` : value.bind;
  const address = `${bind}:${value.port}`;
  return typeof value.scheme === "string" ? `${value.scheme} ${address}` : address;
}

function parseLane(value: unknown): ParsedLane | null {
  if (!record(value) || !record(value.peer)) return null;
  const { id, transport, direction } = value;
  const peerNodeId = value.peer.node_id;
  const listener = listenerAddress(value.listener);
  const priority = bounded(value.priority, 0, 65535);
  if (typeof id !== "string" || typeof transport !== "string" || typeof direction !== "string" ||
      typeof peerNodeId !== "string" || !listener || priority === null) return null;
  return { id, transport, direction, priority, peerNodeId, listener, enabled: value.enabled !== false, client: value.role === "client" };
}

function parseSnapshot(row: NodeDocumentRow): Snapshot {
  let value: unknown;
  try { value = JSON.parse(row.manifest_json); } catch { value = null; }
  const manifest = record(value) ? value : {};
  const agent = record(manifest.agent) ? manifest.agent : {};
  return {
    nodeId: row.node_id, updatedAt: row.updated_at,
    interval: bounded(agent.poll_interval_secs, 5, 3600) ?? DEFAULT_REPORT_INTERVAL_SECONDS,
    maxActive: bounded(agent.max_active_lanes, 1, 16) ?? 1,
    lanes: Array.isArray(agent.lanes) ? agent.lanes.map(parseLane).filter((lane) => lane !== null) : [],
    tunnels: Array.isArray(manifest.tunnels) ? manifest.tunnels.filter(record) : [],
  };
}

export function observatoryLanes(documents: NodeDocumentRow[], samples: LaneMetricRow[], now: number) {
  const snapshots = documents.map(parseSnapshot);
  const intervals = new Map(snapshots.map((doc) => [doc.nodeId, doc.interval]));
  const metrics = new Map(samples.map((metric) => [`${metric.node_id}:${metric.lane_id}`, metric]));
  function expiresAt(timestamp: number | null, nodeId: string, reporterId: string | null) {
    if (timestamp === null) return null;
    const interval = intervals.get(nodeId) ?? DEFAULT_REPORT_INTERVAL_SECONDS;
    const relayInterval = reporterId ? intervals.get(reporterId) ?? DEFAULT_REPORT_INTERVAL_SECONDS : interval;
    return timestamp + 3 * Math.max(interval, relayInterval);
  }
  function observation(nodeId: string, id: string, updatedAt: number, selected: boolean, scope: ProbeScope | null) {
    const metric = selected && scope ? metrics.get(`${nodeId}:${id}`) : undefined;
    const observed = metric && metric.sampled_at >= updatedAt ? metric : undefined;
    const until = expiresAt(observed?.sampled_at ?? null, nodeId, observed?.reporter_node_id ?? null);
    return {
      state: observed && until !== null && until >= now ? observed.state : "unknown" as const,
      probe_ms: observed?.probe_ms ?? null, observed_at: observed?.sampled_at ?? null,
      fresh_until: until, probe_scope: scope,
    };
  }
  return {
    expiresAt,
    async lanes(nodes: Peer[], managed: ManagedLane[]) {
      const edges = mergeManagementEnds(snapshots);
      const management = edges.map(({ doc, lane, selected }) => ({
        id: lane.id, kind: "management", transport: lane.transport, direction: lane.direction,
        priority: lane.priority, selected, source_node_id: doc.nodeId, peer_node_id: lane.peerNodeId,
        local_listener: lane.listener,
        ...observation(doc.nodeId, lane.id, doc.updatedAt, selected, lane.client ? "peer_control" : "local_control"),
      }));
      const customerLanes = [...snapshots.flatMap((doc) => tunnelEdges(doc, nodes)), ...managed.map(managedEdge)];
      const customer = await Promise.all(customerLanes.map(async ({ observedNode, contract, updatedAt, scope, edge }) => ({
        ...edge,
        ...observation(observedNode, scope ? "tunnel" + await sha256(contract) : "", updatedAt, edge.selected, scope),
      })));
      return [...management, ...customer];
    },
  };
}

function mergeManagementEnds(snapshots: Snapshot[]) {
  const edges = new Map<string, { doc: Snapshot; lane: ParsedLane; selected: boolean }>();
  for (const doc of snapshots) {
    const active = doc.lanes.filter((lane) => lane.enabled).sort((left, right) => left.priority - right.priority).slice(0, doc.maxActive);
    for (const lane of doc.lanes) {
      const endpoints = [doc.nodeId, lane.peerNodeId].sort();
      const key = JSON.stringify([...endpoints, lane.transport, lane.direction, lane.listener, lane.priority]);
      if (!edges.has(key) || lane.client) edges.set(key, { doc, lane, selected: active.includes(lane) });
    }
  }
  return [...edges.values()];
}

function managedEdge(lane: ManagedLane) {
  return {
    observedNode: lane.listener_node_id, contract: `owned-${lane.id}`, updatedAt: lane.updated_at,
    scope: lane.transport === "wstunnel" ? "local_control" as const : "local_listener" as const,
    edge: {
      id: lane.id, kind: "tunnel", transport: lane.transport, direction: lane.direction, priority: null,
      selected: lane.enabled === 1, source_node_id: lane.source_node_id, peer_node_id: lane.listener_node_id,
      local_listener: listenerAddress({ scheme: lane.listener_type, bind: lane.listener_bind, port: lane.listener_port }),
      managed: { ...lane, enabled: lane.enabled === 1 },
    },
  };
}

function tunnelPeer(value: Record<string, unknown>, sourceId: string, nodes: Peer[]) {
  const peer = record(value.peer) ? value.peer : record(value.endpoint) ? value.endpoint : {};
  let address = typeof peer.host === "string" ? peer.host : null;
  try { if (typeof peer.url === "string") address = new URL(peer.url).hostname; } catch { return undefined; }
  return nodes.find((node) => node.id !== sourceId && (node.id === peer.node_id || (address !== null && node.address === address)));
}

function tunnelEdges(doc: Snapshot, nodes: Peer[]) {
  return doc.tunnels.flatMap((value) => {
    if (typeof value.id !== "string" || value.id.startsWith("owned-") || typeof value.contract !== "string") return [];
    const target = tunnelPeer(value, doc.nodeId, nodes);
    if (!target) return [];
    const direction = typeof value.direction === "string" ? value.direction : value.contract === "wstunnel.reverse.v1" ? "reverse" : null;
    const scope: ProbeScope | null = value.contract === "hpnssh.forward.v1" && direction === "local_socks" ? "local_listener" : null;
    return [{
      observedNode: doc.nodeId, contract: value.id, updatedAt: doc.updatedAt, scope,
      edge: {
        id: value.id, kind: "tunnel", transport: value.contract.split(".")[0], direction, priority: null,
        selected: value.enabled !== false, source_node_id: doc.nodeId, peer_node_id: target.id,
        local_listener: listenerAddress(value.remote_listener ?? value.listener),
      },
    }];
  });
}
