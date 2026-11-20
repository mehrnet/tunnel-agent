CREATE TABLE seed_canary_releases (
  node_id TEXT NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
  architecture TEXT NOT NULL CHECK(architecture IN ('amd64','arm64')),
  version TEXT NOT NULL,
  sha256 TEXT NOT NULL,
  inventory_sha256 TEXT NOT NULL,
  PRIMARY KEY(node_id, architecture)
) STRICT;
