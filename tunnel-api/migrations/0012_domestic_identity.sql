CREATE TABLE domestic_identities (
  public_key TEXT PRIMARY KEY,
  node_id TEXT NOT NULL REFERENCES nodes(id),
  relay_node_id TEXT NOT NULL REFERENCES nodes(id),
  confirmed_at INTEGER NOT NULL
) STRICT;

CREATE TABLE domestic_enrollment_keys (
  grant_id TEXT PRIMARY KEY REFERENCES enrollment_grants(id) ON DELETE CASCADE,
  public_key TEXT NOT NULL
) STRICT;
