-- The last open of each conversation, kept apart from its `conversations` row:
-- the app can open a conversation before the bootstrap has stored that row,
-- and a page fetched for it can reach the store first. Retention must still
-- see the open, or it deletes the page as if nobody had ever opened it.
CREATE TABLE opened (conversation_id TEXT PRIMARY KEY, at_ms INTEGER NOT NULL);
INSERT INTO opened (conversation_id, at_ms)
  SELECT id, last_opened_ms FROM conversations WHERE last_opened_ms IS NOT NULL;
