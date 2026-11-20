import { validTunnelHost, type Intent } from "./tunnel-spec";
import { parseTransportKeys } from "./transport-keys";
import { now } from "./validation";

interface EndpointIdentity {
  id: string;
  address: string | null;
  node_class: string;
  sampled_at: number | null;
  reporter_node_id: string | null;
  transport_keys_json: string | null;
  manifest_json: string;
}

interface ManagementLane {
  enabled?: boolean;
  role?: string;
  peer?: { node_id?: string };
}

const KEY_FRESHNESS_SECONDS = 600;

function trustedReporter(node: EndpointIdentity): boolean {
  if (node.node_class === "foreign") return node.reporter_node_id === node.id;
  const agent = (JSON.parse(node.manifest_json) as { agent?: { lanes?: ManagementLane[] } }).agent;
  return !!agent?.lanes?.some((lane) =>
    lane.enabled !== false && lane.role === "server" && lane.peer?.node_id === node.reporter_node_id);
}

function identity(node: EndpointIdentity, transport: "hpnssh" | "rathole") {
  if (!node.address || !validTunnelHost(node.address)) {
    throw new Error("Both endpoints need a valid hostname or IPv4 address");
  }
  const keys = node.transport_keys_json ? parseTransportKeys(JSON.parse(node.transport_keys_json)) : null;
  const key = keys?.[transport];
  if (!key || !node.sampled_at || now() - node.sampled_at > KEY_FRESHNESS_SECONDS || !trustedReporter(node)) {
    throw new Error(`Both nodes need a recent ${transport} identity from a compatible agent`);
  }
  return { host: node.address, key };
}

export function managedContracts(intent: Intent, connector: EndpointIdentity, listener: EndpointIdentity) {
  const transport = intent.transport;
  if (transport === "wstunnel") throw new Error("WSTunnel uses a separate contract");
  const source = identity(connector, transport);
  const peer = identity(listener, transport);
  if (source.key === peer.key) throw new Error("The endpoints reported the same transport identity");
  const bind = { bind: "127.0.0.1", port: intent.listener_port };
  const target = intent.listener_type === "tcp" ? { host: intent.destination_host, port: intent.destination_port } : undefined;
  const common = {
    contract: "managed.reverse.v1", id: `owned-${intent.id}`, enabled: true,
    transport, scheme: intent.listener_type, control_port: intent.control_port,
    generation: intent.path_key,
  };
  return {
    server: { ...common, role: "server", listener: bind, peer_host: source.host, peer_key: source.key, local_key: peer.key },
    client: { ...common, role: "client", listener: { ...bind, target }, peer_host: peer.host, peer_key: peer.key, local_key: source.key },
  };
}
