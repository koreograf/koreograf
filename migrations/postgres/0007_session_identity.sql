ALTER TABLE sessions ADD COLUMN id TEXT;
ALTER TABLE sessions ADD COLUMN created_at TEXT;
ALTER TABLE sessions ADD COLUMN last_seen_at TEXT;
UPDATE sessions SET id = gen_random_uuid()::text WHERE id IS NULL;
UPDATE sessions SET created_at = to_char(now() AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS"Z"')
  WHERE created_at IS NULL;
CREATE UNIQUE INDEX IF NOT EXISTS idx_sessions_id ON sessions(id);
CREATE INDEX IF NOT EXISTS idx_sessions_user ON sessions(user_id);
