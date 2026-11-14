CREATE TABLE cluster_control_keys (
  cluster_id TEXT PRIMARY KEY REFERENCES clusters(id) ON DELETE CASCADE,
  ssh_public_key TEXT NOT NULL,
  encrypted_private_key TEXT NOT NULL,
  created_at INTEGER NOT NULL
) STRICT;
