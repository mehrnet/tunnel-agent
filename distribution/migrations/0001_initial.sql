PRAGMA foreign_keys = ON;

CREATE TABLE owners (
  id TEXT PRIMARY KEY,
  telegram_user_id TEXT NOT NULL UNIQUE,
  telegram_username TEXT,
  created_at INTEGER NOT NULL
) STRICT;

CREATE TABLE clusters (
  id TEXT PRIMARY KEY,
  owner_id TEXT NOT NULL REFERENCES owners(id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  UNIQUE(owner_id, name)
) STRICT;

CREATE TABLE nodes (
  id TEXT PRIMARY KEY,
  owner_id TEXT NOT NULL REFERENCES owners(id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  node_class TEXT NOT NULL CHECK(node_class IN ('domestic', 'foreign')),
  address TEXT,
  status TEXT NOT NULL DEFAULT 'pending' CHECK(status IN ('pending', 'enrolled', 'revoked')),
  public_key TEXT,
  created_at INTEGER NOT NULL,
  enrolled_at INTEGER,
  UNIQUE(owner_id, name)
) STRICT;

CREATE TABLE cluster_nodes (
  cluster_id TEXT NOT NULL REFERENCES clusters(id) ON DELETE CASCADE,
  node_id TEXT NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
  role TEXT NOT NULL CHECK(role IN ('domestic', 'foreign')),
  created_at INTEGER NOT NULL,
  PRIMARY KEY(cluster_id, node_id)
) STRICT;

CREATE TABLE enrollment_grants (
  id TEXT PRIMARY KEY,
  node_id TEXT NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
  secret_hash TEXT NOT NULL,
  expires_at INTEGER NOT NULL,
  created_at INTEGER NOT NULL,
  consumed_at INTEGER
) STRICT;

CREATE INDEX enrollment_grants_node_id ON enrollment_grants(node_id);

CREATE TABLE node_credentials (
  node_id TEXT PRIMARY KEY REFERENCES nodes(id) ON DELETE CASCADE,
  secret_hash TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  rotated_at INTEGER NOT NULL
) STRICT;

CREATE TABLE bundle_releases (
  version TEXT NOT NULL,
  architecture TEXT NOT NULL CHECK(architecture IN ('amd64', 'arm64')),
  sha256 TEXT NOT NULL,
  public_url TEXT NOT NULL,
  domestic_url TEXT,
  active INTEGER NOT NULL DEFAULT 0 CHECK(active IN (0, 1)),
  created_at INTEGER NOT NULL,
  PRIMARY KEY(version, architecture)
) STRICT;

CREATE UNIQUE INDEX one_active_bundle_per_architecture
  ON bundle_releases(architecture) WHERE active = 1;

CREATE TABLE node_documents (
  node_id TEXT PRIMARY KEY REFERENCES nodes(id) ON DELETE CASCADE,
  revision INTEGER NOT NULL,
  manifest_json TEXT NOT NULL,
  updated_at INTEGER NOT NULL
) STRICT;

CREATE TABLE audit_events (
  id TEXT PRIMARY KEY,
  owner_id TEXT REFERENCES owners(id) ON DELETE SET NULL,
  node_id TEXT REFERENCES nodes(id) ON DELETE SET NULL,
  cluster_id TEXT REFERENCES clusters(id) ON DELETE SET NULL,
  action TEXT NOT NULL,
  detail_json TEXT NOT NULL,
  created_at INTEGER NOT NULL
) STRICT;

CREATE TABLE telegram_updates (
  update_id INTEGER PRIMARY KEY,
  received_at INTEGER NOT NULL
) STRICT;
