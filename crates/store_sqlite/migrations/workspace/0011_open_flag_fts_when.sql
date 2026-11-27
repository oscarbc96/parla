-- Persist `is_open` so a cached start shows the same open DMs as the last
-- session (NULL, from rows written before this migration, reads as open).
ALTER TABLE conversations ADD COLUMN is_open INTEGER;

-- Every upsert rewrites `plain`; only re-index the row when the text changed.
DROP TRIGGER messages_fts_au;
CREATE TRIGGER messages_fts_au AFTER UPDATE OF plain ON messages
WHEN old.plain IS NOT new.plain BEGIN
  DELETE FROM messages_fts WHERE rowid =
    (SELECT rowid FROM messages_fts_map WHERE conversation_id = new.conversation_id AND ts = new.ts);
  INSERT INTO messages_fts (rowid, plain)
    SELECT rowid, new.plain FROM messages_fts_map
    WHERE conversation_id = new.conversation_id AND ts = new.ts;
END;

-- `AddGap` and `CloseGap` read and rewrite one conversation's gaps.
CREATE INDEX gaps_conversation ON gaps (conversation_id);
