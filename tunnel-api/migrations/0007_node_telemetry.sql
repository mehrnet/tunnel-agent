CREATE TABLE node_telemetry (
  node_id TEXT PRIMARY KEY REFERENCES nodes(id) ON DELETE CASCADE,
  reporter_node_id TEXT NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
  sampled_at INTEGER NOT NULL CHECK(sampled_at > 0),
  recorded_at INTEGER NOT NULL CHECK(recorded_at > 0),
  cpu_milli_percent INTEGER NOT NULL CHECK(cpu_milli_percent BETWEEN 0 AND 100000),
  memory_milli_percent INTEGER NOT NULL CHECK(memory_milli_percent BETWEEN 0 AND 100000),
  load_1_milli INTEGER NOT NULL CHECK(load_1_milli >= 0),
  rx_bytes_per_sec INTEGER NOT NULL CHECK(rx_bytes_per_sec >= 0),
  tx_bytes_per_sec INTEGER NOT NULL CHECK(tx_bytes_per_sec >= 0),
  agent_rss_bytes INTEGER NOT NULL CHECK(agent_rss_bytes >= 0)
) STRICT;

CREATE TABLE lane_telemetry (
  node_id TEXT NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
  lane_id TEXT NOT NULL,
  reporter_node_id TEXT NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
  sampled_at INTEGER NOT NULL CHECK(sampled_at > 0),
  recorded_at INTEGER NOT NULL CHECK(recorded_at > 0),
  state TEXT NOT NULL CHECK(state IN ('healthy', 'degraded', 'unknown')),
  probe_ms INTEGER CHECK(probe_ms BETWEEN 0 AND 60000),
  PRIMARY KEY(node_id, lane_id)
) STRICT;

CREATE INDEX lane_telemetry_node_id ON lane_telemetry(node_id);
