-- The Files tab lists one conversation's messages with files, newest first.
-- A partial index keeps that read off the messages without files.
CREATE INDEX messages_files ON messages (conversation_id, ts_key) WHERE files IS NOT NULL;
