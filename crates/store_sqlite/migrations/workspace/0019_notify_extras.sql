-- Parla's own per-conversation notification extras: replies in followed
-- threads off, silent notifications, and a timed mute (unix seconds).
CREATE TABLE local_notify_extras (conversation_id TEXT PRIMARY KEY,
  threads_off INTEGER NOT NULL, sound_off INTEGER NOT NULL, mute_until INTEGER);
