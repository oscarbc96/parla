-- Reminders kept by Parla itself, for providers that keep none. `due` is unix
-- seconds or NULL; `recurring` and `completed` are 0 or 1.
CREATE TABLE local_reminders (id TEXT PRIMARY KEY, text TEXT NOT NULL, due INTEGER,
  recurring INTEGER NOT NULL, completed INTEGER NOT NULL, for_user TEXT NOT NULL);
