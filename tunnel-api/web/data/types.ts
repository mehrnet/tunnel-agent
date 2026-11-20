type NodeClass = "domestic" | "foreign";
type EnrollmentStatus = "pending" | "enrolled" | "revoked";
type LaneState = "healthy" | "degraded" | "unknown";

interface NodeTelemetry {
  sampled_at: number;
  cpu_percent: number;
  memory_percent: number;
  load_1: number;
  rx_bytes_per_sec: number;
  tx_bytes_per_sec: number;
  agent_rss_bytes: number;
}

export interface ClusterNode {
  id: string;
  name: string;
  address: string | null;
  node_class: NodeClass;
  status: EnrollmentStatus;
  last_heartbeat_at: number | null;
  heartbeat_expires_at: number | null;
  telemetry: NodeTelemetry | null;
}

export interface TunnelLane {
  kind?: "management" | "tunnel";
  id: string;
  transport: string;
  direction: string | null;
  priority: number | null;
  selected: boolean;
  source_node_id: string;
  peer_node_id: string;
  local_listener: string | null;
  state: LaneState;
  probe_ms: number | null;
  observed_at: number | null;
  fresh_until: number | null;
  probe_scope: "peer_control" | "local_control" | "local_listener" | null;
  managed?: {
    enabled: boolean;
    revision: number;
    listener_type: "tcp" | "socks5";
    control_port: number;
    destination_host: string | null;
    destination_port: number | null;
  };
}

export interface ClusterSummary {
  id: string;
  name: string;
  release_track?: "stable" | "testing";
}

export interface ClusterResponse {
  generated_at: number;
  cluster: ClusterSummary;
  nodes: ClusterNode[];
  lanes: TunnelLane[];
}

export type Selection =
  | { kind: "lane"; id: string }
  | { kind: "node"; id: string }
  | undefined;
