-- Cross-workspace data. First tenant: the MCP audit log (one row per call;
-- `content` is empty unless the owner turned content logging on).
CREATE TABLE mcp_audit (id INTEGER PRIMARY KEY, at_ms INTEGER NOT NULL,
  client TEXT NOT NULL, peer_pid INTEGER, tool TEXT NOT NULL,
  workspace TEXT, target TEXT, outcome INTEGER NOT NULL, detail TEXT,
  decision INTEGER, duration_ms INTEGER NOT NULL, content TEXT);
CREATE INDEX mcp_audit_at ON mcp_audit (at_ms);
CREATE INDEX mcp_audit_workspace ON mcp_audit (workspace, at_ms);
