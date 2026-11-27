-- Plugin state. `app.db` is the source of truth for
-- what is installed and granted; every other table cascades from `plugins`.
CREATE TABLE plugins (id TEXT PRIMARY KEY, version TEXT NOT NULL,
  source TEXT NOT NULL, sha256 TEXT NOT NULL, installed_at INTEGER NOT NULL,
  state TEXT NOT NULL, previous_version TEXT);
-- One row per accepted capability and argument (`value` empty when none).
CREATE TABLE plugin_grants (plugin_id TEXT NOT NULL
    REFERENCES plugins (id) ON DELETE CASCADE,
  capability TEXT NOT NULL, value TEXT NOT NULL, granted_at INTEGER NOT NULL,
  PRIMARY KEY (plugin_id, capability, value));
-- Writes allowed without asking; `conversation` is empty for a whole area.
CREATE TABLE plugin_write_allow (plugin_id TEXT NOT NULL
    REFERENCES plugins (id) ON DELETE CASCADE,
  workspace TEXT NOT NULL, area TEXT NOT NULL, conversation TEXT NOT NULL,
  PRIMARY KEY (plugin_id, workspace, area, conversation));
-- `storage`; `scope` is `global` or a workspace key. Limits
-- are enforced by the store.
CREATE TABLE plugin_kv (plugin_id TEXT NOT NULL
    REFERENCES plugins (id) ON DELETE CASCADE,
  scope TEXT NOT NULL, key TEXT NOT NULL, value BLOB NOT NULL,
  PRIMARY KEY (plugin_id, scope, key));
-- Names of the plugin's keychain secrets, so uninstall can
-- remove them.
CREATE TABLE plugin_secret_keys (plugin_id TEXT NOT NULL
    REFERENCES plugins (id) ON DELETE CASCADE,
  key TEXT NOT NULL, PRIMARY KEY (plugin_id, key));
-- SHA-256 of each precompiled `.cwasm`, checked before it is loaded.
CREATE TABLE plugin_cwasm (plugin_id TEXT NOT NULL
    REFERENCES plugins (id) ON DELETE CASCADE,
  version TEXT NOT NULL, engine TEXT NOT NULL, sha256 BLOB NOT NULL,
  PRIMARY KEY (plugin_id, version, engine));
