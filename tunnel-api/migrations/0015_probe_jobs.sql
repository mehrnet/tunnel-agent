ALTER TABLE node_telemetry ADD COLUMN probe_jobs INTEGER NOT NULL DEFAULT 0 CHECK(probe_jobs IN (0,1));

CREATE TABLE probe_jobs (
  id TEXT PRIMARY KEY,
  cluster_id TEXT NOT NULL REFERENCES clusters(id) ON DELETE CASCADE,
  node_id TEXT NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
  lane_id TEXT NOT NULL,
  starts_at INTEGER NOT NULL,
  ends_at INTEGER NOT NULL,
  spec_json TEXT NOT NULL,
  sequence INTEGER NOT NULL DEFAULT 0,
  report_json TEXT,
  reported_at INTEGER,
  cancelled_at INTEGER,
  CHECK (ends_at-starts_at BETWEEN 600 AND 86400 AND (ends_at-starts_at)%30=0)
) STRICT;

CREATE INDEX probe_jobs_by_cluster ON probe_jobs(cluster_id, starts_at DESC);
CREATE INDEX probe_jobs_by_node ON probe_jobs(node_id, ends_at);
