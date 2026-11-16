CREATE TABLE node_heartbeats (
  node_id TEXT PRIMARY KEY REFERENCES nodes(id) ON DELETE CASCADE,
  observer_node_id TEXT NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
  observed_at INTEGER NOT NULL CHECK(observed_at > 0),
  recorded_at INTEGER NOT NULL
) STRICT;

CREATE INDEX node_heartbeats_observer_node_id ON node_heartbeats(observer_node_id);
CREATE INDEX node_heartbeats_observed_at ON node_heartbeats(observed_at);
