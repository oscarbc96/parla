-- The message a local reminder is about: its conversation and ts, author and
-- the start of its text (all null for a reminder about no message).
ALTER TABLE local_reminders ADD COLUMN link_conversation TEXT;
ALTER TABLE local_reminders ADD COLUMN link_ts TEXT;
ALTER TABLE local_reminders ADD COLUMN link_author TEXT;
ALTER TABLE local_reminders ADD COLUMN link_excerpt TEXT;
-- A snoozed occurrence of a recurring local reminder (unix seconds); the
-- series keeps its due time.
ALTER TABLE local_reminders ADD COLUMN snoozed_until INTEGER;
