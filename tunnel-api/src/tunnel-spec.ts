import { randomToken } from "./validation";

export interface Intent {
  id: string;
  cluster_id: string;
  source_node_id: string;
  listener_node_id: string;
  listener_type: "tcp" | "socks5";
  listener_port: number;
  control_port: number;
  destination_host: string | null;
  destination_port: number | null;
  path_key: string;
  enabled: number;
  revision: number;
}

const hostname = /^(?=.{1,253}$)[a-z0-9](?:[a-z0-9.-]*[a-z0-9])?$/i;
const uuid = /^[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}$/;

function port(value: unknown, minimum = 1): number {
  if (typeof value !== "number" || !Number.isInteger(value) || value < minimum || value > 65535) {
    throw new Error(`Port must be between ${minimum} and 65535`);
  }
  return value;
}

export function destination(body: Record<string, unknown>) {
  const host = body.destination_host;
  if (typeof host !== "string" || !hostname.test(host)) throw new Error("Destination must be a hostname or IPv4 address");
  return { destination_host: host, destination_port: port(body.destination_port) };
}

export function parseIntent(body: Record<string, unknown>, clusterId: string): Intent {
  const { source_node_id: source, listener_node_id: listener, listener_type: type } = body;
  if (typeof source !== "string" || typeof listener !== "string" || !uuid.test(source) || !uuid.test(listener) || source === listener) {
    throw new Error("Select two distinct servers");
  }
  if (type !== "tcp" && type !== "socks5") throw new Error("Select TCP or SOCKS5");
  if (body.transport !== "wstunnel") throw new Error("Only wstunnel is supported by the owner editor");
  const listenerPort = port(body.listener_port, 1024);
  const controlPort = port(body.control_port, 1024);
  if (listenerPort === controlPort) throw new Error("Use different listener and control ports");
  return {
    id: crypto.randomUUID(), cluster_id: clusterId, source_node_id: source, listener_node_id: listener,
    listener_type: type, listener_port: listenerPort, control_port: controlPort, path_key: randomToken(),
    enabled: 0, revision: 1,
    ...(type === "tcp" ? destination(body) : { destination_host: null, destination_port: null }),
  };
}

export function contracts(intent: Intent, serverHost: string) {
  if (!hostname.test(serverHost)) throw new Error("The listening server needs a hostname or IPv4 address matching its TLS certificate");
  const common = { id: `owned-${intent.id}`, enabled: intent.enabled === 1, artifact: "wstunnel" };
  const prefix = `/${intent.path_key}`;
  const target = intent.listener_type === "tcp" ? { host: intent.destination_host, port: intent.destination_port } : undefined;
  return {
    server: {
      ...common, contract: "wstunnel.server.v1", listener: { bind: "0.0.0.0", port: intent.control_port },
      path_prefix: prefix, tls_certificate: "/etc/tunnel-agent/tls/fullchain.pem",
      tls_private_key: "/etc/tunnel-agent/tls/privkey.pem",
    },
    client: {
      ...common, contract: "wstunnel.reverse.v1",
      endpoint: { url: `wss://${serverHost}:${intent.control_port}`, tls_sni: serverHost, path_prefix: prefix, verify_certificate: true },
      remote_listener: { scheme: intent.listener_type, bind: "127.0.0.1", port: intent.listener_port, target },
    },
  };
}
