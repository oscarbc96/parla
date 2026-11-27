-- Local schedules (sent by Parla itself), recent emoji, and an author index
-- for the filtered local search.
CREATE TABLE local_schedules (id TEXT PRIMARY KEY, conversation_id TEXT NOT NULL, thread_ts TEXT,
  text TEXT NOT NULL, files BLOB, post_at INTEGER NOT NULL, created_ms INTEGER NOT NULL, state BLOB);
CREATE TABLE recent_emoji (shortcode TEXT PRIMARY KEY, unicode TEXT, skin_tone INTEGER,
  used_ms INTEGER NOT NULL, uses INTEGER NOT NULL);
CREATE INDEX messages_author ON messages (author, ts_key);
