import { randomToken } from "./validation";

type TunnelTransport = "wstunnel" | "hpnssh" | "rathole";

export interface Intent {
  id: string;
  cluster_id: string;
  source_node_id: string;
  listener_node_id: string;
  transport: TunnelTransport;
  listener_type: "tcp" | "socks5";
  listener_port: number;
  control_port: number;
  tls_server_name: string | null;
  destination_host: string | null;
  destination_port: number | null;
  path_key: string;
  enabled: number;
  revision: number;
}

const label = /^[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?$/i;
const uuid = /^[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}$/;

export function validTunnelHost(host: string): boolean {
  return host.length <= 253 && host.split(".").every((part) => label.test(part));
}

function tlsName(value: unknown): string | null {
  if (value === undefined || value === null || value === "") return null;
  if (typeof value !== "string" || /^[\d.]+$/.test(value) || !validTunnelHost(value)) {
    throw new Error("TLS server name must be a DNS name, not an IP address");
  }
  return value;
}

function port(value: unknown, minimum = 1): number {
  if (typeof value !== "number" || !Number.isInteger(value) || value < minimum || value > 65535) {
    throw new Error(`Port must be between ${minimum} and 65535`);
  }
  return value;
}

export function destination(body: Record<string, unknown>) {
  const host = body.destination_host;
  if (typeof host !== "string" || !validTunnelHost(host)) throw new Error("Destination must be a hostname or IPv4 address");
  return { destination_host: host, destination_port: port(body.destination_port) };
}

export function parseIntent(body: Record<string, unknown>, clusterId: string): Intent {
  const { source_node_id: source, listener_node_id: listener, listener_type: type, transport } = body;
  if (typeof source !== "string" || typeof listener !== "string" || !uuid.test(source) || !uuid.test(listener) || source === listener) {
    throw new Error("Select two distinct servers");
  }
  if (type !== "tcp" && type !== "socks5") throw new Error("Select TCP or SOCKS5");
  if (transport !== "wstunnel" && transport !== "hpnssh" && transport !== "rathole") {
    throw new Error("Select WSTunnel, HPNSSH, or Rathole");
  }
  if (transport === "rathole" && type !== "tcp") throw new Error("Rathole requires a fixed TCP destination");
  const listenerPort = port(body.listener_port, 1024);
  const controlPort = port(body.control_port, 1024);
  if (listenerPort === controlPort) throw new Error("Use different listener and control ports");
  return {
    id: crypto.randomUUID(), cluster_id: clusterId, source_node_id: source, listener_node_id: listener,
    transport, listener_type: type, listener_port: listenerPort, control_port: controlPort, path_key: randomToken(),
    tls_server_name: transport === "wstunnel" ? tlsName(body.tls_server_name) : null,
    enabled: 0, revision: 1,
    ...(type === "tcp" ? destination(body) : { destination_host: null, destination_port: null }),
  };
}

export function wstunnelContracts(intent: Intent, serverHost: string) {
  if (!validTunnelHost(serverHost)) throw new Error("The listening server needs a hostname or IPv4 address");
  const serverName = tlsName(intent.tls_server_name ?? serverHost);
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
      endpoint: { url: `wss://${serverHost}:${intent.control_port}`, tls_sni: serverName, path_prefix: prefix, verify_certificate: true },
      remote_listener: { scheme: intent.listener_type, bind: "127.0.0.1", port: intent.listener_port, target },
    },
  };
}
