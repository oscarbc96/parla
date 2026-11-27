-- Seconds between occurrences of a recurring local reminder (0: none stored;
-- an older recurring row then repeats weekly).
ALTER TABLE local_reminders ADD COLUMN interval_secs INTEGER NOT NULL DEFAULT 0;
