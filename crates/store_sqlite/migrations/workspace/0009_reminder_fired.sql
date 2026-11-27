-- Local reminders whose desktop alert was already raised (0 or 1), so a
-- restart does not alert twice.
ALTER TABLE local_reminders ADD COLUMN fired INTEGER NOT NULL DEFAULT 0;
