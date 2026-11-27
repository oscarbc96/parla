-- The header shows a conversation's topic and member count from the cache
-- before the first bootstrap of a session.
ALTER TABLE conversations ADD COLUMN topic TEXT;
ALTER TABLE conversations ADD COLUMN member_count INTEGER;
