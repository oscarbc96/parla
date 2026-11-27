-- Notification level and mute flag kept by Parla itself, for providers that
-- cannot store them. `level`: 0 default, 1 all, 2 mentions, 3 nothing.
CREATE TABLE local_notify_prefs (conversation_id TEXT PRIMARY KEY, level INTEGER NOT NULL,
  muted INTEGER NOT NULL);
