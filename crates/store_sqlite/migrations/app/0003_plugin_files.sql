-- The installed files' digest, checked before a plugin loads, and the
-- manifest's authors (one per line), which with `source` say whose data a
-- kept row holds (M8 plan Task B6). On the row, so a grant change never
-- drops them.
ALTER TABLE plugins ADD COLUMN files_digest TEXT;
ALTER TABLE plugins ADD COLUMN authors TEXT;
