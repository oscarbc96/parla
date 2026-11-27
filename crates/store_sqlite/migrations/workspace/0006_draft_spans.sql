-- A draft keeps the mentions, emoji and links the user picked, as JSON over the
-- byte ranges of `body` (provider-neutral; NULL for a plain-text draft).
ALTER TABLE drafts ADD COLUMN spans TEXT;
