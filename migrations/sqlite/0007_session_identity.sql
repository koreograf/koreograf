ALTER TABLE sessions ADD COLUMN id TEXT;
ALTER TABLE sessions ADD COLUMN created_at TEXT;
ALTER TABLE sessions ADD COLUMN last_seen_at TEXT;
UPDATE sessions SET id = lower(hex(randomblob(4))) || '-' ||
        lower(hex(randomblob(2))) || '-4' ||
        substr(lower(hex(randomblob(2))), 2) || '-' ||
        substr('89ab', (random() & 3) + 1, 1) ||
        substr(lower(hex(randomblob(2))), 2) || '-' ||
        lower(hex(randomblob(6)))
  WHERE id IS NULL;
UPDATE sessions SET created_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
  WHERE created_at IS NULL;
CREATE UNIQUE INDEX IF NOT EXISTS idx_sessions_id ON sessions(id);
CREATE INDEX IF NOT EXISTS idx_sessions_user ON sessions(user_id);
