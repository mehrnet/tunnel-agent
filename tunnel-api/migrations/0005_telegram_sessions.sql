CREATE TABLE telegram_sessions (
  owner_id TEXT NOT NULL REFERENCES owners(id) ON DELETE CASCADE,
  chat_id TEXT NOT NULL,
  mode TEXT NOT NULL,
  payload_json TEXT NOT NULL,
  menu_message_id INTEGER,
  menu_created_at INTEGER,
  updated_at INTEGER NOT NULL,
  PRIMARY KEY(owner_id, chat_id)
) STRICT;

CREATE INDEX telegram_sessions_updated_at ON telegram_sessions(updated_at);
