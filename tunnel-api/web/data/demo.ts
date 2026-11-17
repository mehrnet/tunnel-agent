import type { ClusterResponse, ClusterSummary } from "./types";

const at = (hour: number, minute: number) => Date.UTC(2026, 10, 17, hour, minute);
const tehNodeId = "teh-06-b37f";
const overseasToIran = "خارج به ایران";

export const demoClusters: ClusterSummary[] = [
  { id: "w4r-canary", name: "W4R Canary" },
  { id: "east-failover", name: "East Failover" },
];

const canary: ClusterResponse = {
  generated_at: at(8, 42),
  cluster: demoClusters[0],
  nodes: [
    {
      id: "fra-01-09d2",
      name: "fra-01",
      address: "138.199.236.120",
      node_class: "foreign",
      status: "enrolled",
      last_heartbeat_at: at(8, 41),
      telemetry: { sampled_at: at(8, 41), cpu_percent: 31, memory_percent: 48, load_1: 1.18, rx_bytes_per_sec: 2_840_000, tx_bytes_per_sec: 1_920_000, agent_rss_bytes: 132_000_000 },
    },
    {
      id: "hel-02-7ac8",
      name: "hel-02",
      address: "65.21.22.89",
      node_class: "foreign",
      status: "enrolled",
      last_heartbeat_at: at(8, 39),
      telemetry: { sampled_at: at(8, 39), cpu_percent: 57, memory_percent: 62, load_1: 2.43, rx_bytes_per_sec: 1_180_000, tx_bytes_per_sec: 980_000, agent_rss_bytes: 148_000_000 },
    },
    {
      id: tehNodeId,
      name: "teh-06",
      address: "2.144.4.192",
      node_class: "domestic",
      status: "enrolled",
      last_heartbeat_at: at(8, 41),
      telemetry: { sampled_at: at(8, 41), cpu_percent: 83, memory_percent: 88, load_1: 5.12, rx_bytes_per_sec: 6_920_000, tx_bytes_per_sec: 5_740_000, agent_rss_bytes: 192_000_000 },
    },
    {
      id: "mhd-03-cf14",
      name: "mhd-03",
      address: "5.160.39.18",
      node_class: "domestic",
      status: "enrolled",
      last_heartbeat_at: at(8, 37),
      telemetry: null,
    },
    {
      id: "qom-01-20ea",
      name: "qom-01",
      address: null,
      node_class: "domestic",
      status: "pending",
      last_heartbeat_at: null,
      telemetry: null,
    },
  ],
  lanes: [
    { id: "ln-05JwQJwF", transport: "HPN SSH", direction: overseasToIran, priority: 1, selected: true, source_node_id: "fra-01-09d2", peer_node_id: tehNodeId, local_listener: ":42001", state: "healthy", probe_ms: 155, observed_at: at(8, 41) },
    { id: "ln-0Lc92jNe", transport: "TUIC", direction: overseasToIran, priority: 2, selected: false, source_node_id: "hel-02-7ac8", peer_node_id: tehNodeId, local_listener: ":443", state: "degraded", probe_ms: 438, observed_at: at(8, 40) },
    { id: "ln-8Ve70cpQ", transport: "Shadowsocks", direction: overseasToIran, priority: 3, selected: false, source_node_id: "fra-01-09d2", peer_node_id: "mhd-03-cf14", local_listener: ":8443", state: "unknown", probe_ms: null, observed_at: null },
    { id: "ln-b40aD2f9", transport: "HPN SSH", direction: "ایران به خارج", priority: 4, selected: false, source_node_id: tehNodeId, peer_node_id: "hel-02-7ac8", local_listener: ":42022", state: "healthy", probe_ms: 169, observed_at: at(8, 41) },
  ],
};

const failover: ClusterResponse = {
  generated_at: at(8, 42),
  cluster: demoClusters[1],
  nodes: [
    { id: "ist-04-11f3", name: "ist-04", address: "185.219.134.40", node_class: "foreign", status: "enrolled", last_heartbeat_at: at(8, 40), telemetry: { sampled_at: at(8, 40), cpu_percent: 22, memory_percent: 37, load_1: 0.89, rx_bytes_per_sec: 980_000, tx_bytes_per_sec: 810_000, agent_rss_bytes: 101_000_000 } },
    { id: "tbz-02-f2a4", name: "tbz-02", address: "37.152.182.71", node_class: "domestic", status: "enrolled", last_heartbeat_at: at(8, 40), telemetry: { sampled_at: at(8, 40), cpu_percent: 44, memory_percent: 55, load_1: 1.92, rx_bytes_per_sec: 1_240_000, tx_bytes_per_sec: 1_060_000, agent_rss_bytes: 118_000_000 } },
  ],
  lanes: [
    { id: "ln-d34F8eA2", transport: "HPN SSH", direction: overseasToIran, priority: 1, selected: true, source_node_id: "ist-04-11f3", peer_node_id: "tbz-02-f2a4", local_listener: ":42001", state: "healthy", probe_ms: 121, observed_at: at(8, 40) },
  ],
};

const demos: Record<string, ClusterResponse> = { "w4r-canary": canary, "east-failover": failover };

export function demoCluster(id: string): ClusterResponse {
  return demos[id] ?? canary;
}
