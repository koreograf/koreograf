CREATE TABLE IF NOT EXISTS template_groups (
  id TEXT PRIMARY KEY,
  team_id TEXT NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  position INTEGER NOT NULL DEFAULT 0,
  UNIQUE (team_id, name)
);
CREATE TABLE IF NOT EXISTS template_group_members (
  template_id TEXT NOT NULL REFERENCES templates(id) ON DELETE CASCADE,
  group_id TEXT NOT NULL REFERENCES template_groups(id) ON DELETE CASCADE,
  PRIMARY KEY (template_id, group_id)
);
