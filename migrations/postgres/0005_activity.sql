CREATE TABLE IF NOT EXISTS activity (
  id TEXT PRIMARY KEY,
  team_id TEXT NOT NULL,
  actor TEXT NOT NULL,
  action TEXT NOT NULL,
  object_type TEXT NOT NULL,
  object_id TEXT NOT NULL DEFAULT '',
  object_name TEXT NOT NULL DEFAULT '',
  result TEXT NOT NULL,
  created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_activity_team_created ON activity(team_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_activity_team_action ON activity(team_id, action);
