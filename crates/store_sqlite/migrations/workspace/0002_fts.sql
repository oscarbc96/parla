-- `messages` is WITHOUT ROWID, so a map table gives each message the integer
-- rowid the contentless FTS5 index needs. Only the provider-produced `plain`
-- text is indexed; the database file is encrypted, so the index is too.
CREATE TABLE messages_fts_map (rowid INTEGER PRIMARY KEY, conversation_id TEXT NOT NULL,
  ts TEXT NOT NULL, UNIQUE (conversation_id, ts));
CREATE VIRTUAL TABLE messages_fts USING fts5(plain, content='', contentless_delete=1,
  tokenize='unicode61', prefix='2 3');

INSERT INTO messages_fts_map (conversation_id, ts) SELECT conversation_id, ts FROM messages;
INSERT INTO messages_fts (rowid, plain)
  SELECT f.rowid, m.plain FROM messages_fts_map f
  JOIN messages m ON m.conversation_id = f.conversation_id AND m.ts = f.ts;

CREATE TRIGGER messages_fts_ai AFTER INSERT ON messages BEGIN
  INSERT INTO messages_fts_map (conversation_id, ts) VALUES (new.conversation_id, new.ts);
  INSERT INTO messages_fts (rowid, plain) VALUES (last_insert_rowid(), new.plain);
END;
CREATE TRIGGER messages_fts_au AFTER UPDATE OF plain ON messages BEGIN
  DELETE FROM messages_fts WHERE rowid =
    (SELECT rowid FROM messages_fts_map WHERE conversation_id = new.conversation_id AND ts = new.ts);
  INSERT INTO messages_fts (rowid, plain)
    SELECT rowid, new.plain FROM messages_fts_map
    WHERE conversation_id = new.conversation_id AND ts = new.ts;
END;
CREATE TRIGGER messages_fts_ad AFTER DELETE ON messages BEGIN
  DELETE FROM messages_fts WHERE rowid =
    (SELECT rowid FROM messages_fts_map WHERE conversation_id = old.conversation_id AND ts = old.ts);
  DELETE FROM messages_fts_map WHERE conversation_id = old.conversation_id AND ts = old.ts;
END;
