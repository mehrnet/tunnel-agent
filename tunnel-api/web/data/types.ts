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
    forward_udp: number;
    control_port: number;
    tls_server_name: string | null;
    destination_host: string | null;
    destination_port: number | null;
  };
}

export interface ClusterSummary {
  id: string;
  name: string;
  release_track?: "stable" | "testing";
}

export interface ProbeJob {
  id: string;
  node_id: string;
  lane_id: string;
  starts_at: number;
  ends_at: number;
  reported_at: number | null;
  cancelled_at: number | null;
  target: { hostname: string; path: string; expected_status: number; download: boolean };
  report: {
    sequence: number;
    passed: number;
    failed: number;
    longest_streak: number;
    latency_total_ms: number;
    latency_max_ms: number;
    bytes: number;
    download_ms: number;
    last_error: string | null;
  } | null;
}

export interface ClusterResponse {
  generated_at: number;
  cluster: ClusterSummary;
  nodes: ClusterNode[];
  lanes: TunnelLane[];
  document_revisions?: Record<string, number>;
  probe_jobs?: ProbeJob[];
  public_ingress?: Array<{
    node_id: string;
    port: number;
    lane_id: string;
    fallback_lane_id: string | null;
    tls_server_name: string | null;
    forward_udp: boolean;
  }>;
}

export type Selection =
  | { kind: "lane"; id: string }
  | { kind: "node"; id: string }
  | undefined;
