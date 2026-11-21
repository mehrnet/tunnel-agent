CREATE TABLE release_api_tokens (
  owner_id TEXT PRIMARY KEY REFERENCES owners(id) ON DELETE CASCADE,
  token_id TEXT NOT NULL UNIQUE,
  secret_hash TEXT NOT NULL,
  expires_at INTEGER NOT NULL
);
