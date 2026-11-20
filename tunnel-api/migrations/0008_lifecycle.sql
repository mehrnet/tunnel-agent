CREATE TABLE node_lifecycle_events (
  id TEXT PRIMARY KEY,
  node_id TEXT NOT NULL REFERENCES nodes(id),
  owner_id TEXT NOT NULL REFERENCES owners(id),
  action TEXT NOT NULL CHECK(action IN ('reinstall','rotate','retire')),
  created_at INTEGER NOT NULL
) STRICT;

CREATE TABLE tunnel_intents (
  id TEXT PRIMARY KEY,
  cluster_id TEXT NOT NULL REFERENCES clusters(id),
  source_node_id TEXT NOT NULL REFERENCES nodes(id),
  peer_node_id TEXT NOT NULL REFERENCES nodes(id),
  transport TEXT NOT NULL,
  direction TEXT NOT NULL,
  listener_type TEXT NOT NULL CHECK(listener_type IN ('tcp','socks5')),
  listener_node_id TEXT NOT NULL REFERENCES nodes(id),
  listener_bind TEXT NOT NULL,
  listener_port INTEGER NOT NULL CHECK(listener_port BETWEEN 1 AND 65535),
  destination_host TEXT,
  destination_port INTEGER CHECK(destination_port BETWEEN 1 AND 65535),
  enabled INTEGER NOT NULL DEFAULT 0 CHECK(enabled IN (0,1)),
  revision INTEGER NOT NULL DEFAULT 1,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  CHECK(source_node_id != peer_node_id)
) STRICT;
CREATE UNIQUE INDEX tunnel_listener_claim ON tunnel_intents(listener_node_id, listener_bind, listener_port);
