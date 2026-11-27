-- The use counter was written but never read: recent emoji rank by recency only.
ALTER TABLE recent_emoji DROP COLUMN uses;
