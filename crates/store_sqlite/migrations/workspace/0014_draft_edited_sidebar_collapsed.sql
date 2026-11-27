-- When each draft was last edited (Unix seconds; 0 for drafts saved before).
ALTER TABLE drafts ADD COLUMN edited_at INTEGER NOT NULL DEFAULT 0;
-- Sidebar sections the user collapsed; kept by Parla itself.
CREATE TABLE local_sidebar_collapsed (section TEXT PRIMARY KEY);
