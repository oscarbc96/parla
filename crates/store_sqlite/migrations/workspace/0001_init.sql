CREATE TABLE conversations (id TEXT PRIMARY KEY, kind INTEGER NOT NULL, name TEXT NOT NULL,
  peer TEXT, is_member INTEGER, is_archived INTEGER, is_muted INTEGER, last_read TEXT,
  latest TEXT, unread INTEGER, mentions INTEGER, last_opened_ms INTEGER, ext BLOB);
CREATE TABLE members (id TEXT PRIMARY KEY, display_name TEXT, real_name TEXT, is_bot INTEGER,
  deleted INTEGER, ext BLOB);
CREATE TABLE messages (conversation_id TEXT NOT NULL, ts TEXT NOT NULL, ts_key INTEGER NOT NULL,
  thread_ts TEXT, broadcast INTEGER NOT NULL DEFAULT 0, author TEXT, text TEXT NOT NULL,
  plain TEXT NOT NULL, mentions BLOB, files BLOB, reactions BLOB, reply_count INTEGER,
  latest_reply TEXT, reply_users BLOB, edited_ts TEXT, client_msg_id TEXT, ext BLOB,
  PRIMARY KEY (conversation_id, ts)) WITHOUT ROWID;
CREATE INDEX messages_window ON messages (conversation_id, ts_key);
CREATE INDEX messages_thread ON messages (conversation_id, thread_ts, ts_key);
CREATE TABLE gaps (conversation_id TEXT NOT NULL, after_ts TEXT NOT NULL, before_ts TEXT NOT NULL);
CREATE TABLE outbox (id TEXT PRIMARY KEY, conversation_id TEXT NOT NULL, thread_ts TEXT,
  broadcast INTEGER, text TEXT NOT NULL, created_ms INTEGER NOT NULL, attempts INTEGER, state BLOB);
CREATE TABLE drafts (conversation_id TEXT NOT NULL, thread_ts TEXT NOT NULL DEFAULT '',
  body TEXT NOT NULL, PRIMARY KEY (conversation_id, thread_ts));
