-- A member's profile picture (`p:<url>` fetched through the provider,
-- `w:<url>` fetched without credentials; null for none), so rows show it
-- from the cache before the provider refills it.
ALTER TABLE members ADD COLUMN avatar TEXT;
ALTER TABLE members ADD COLUMN avatar_lg TEXT;
