CREATE TABLE IF NOT EXISTS users (
  id TEXT PRIMARY KEY,
  username TEXT NOT NULL UNIQUE COLLATE NOCASE,
  display_name TEXT NOT NULL,
  password_hash TEXT,
  auth_source TEXT NOT NULL,
  is_admin INTEGER NOT NULL DEFAULT 0,
  disabled INTEGER NOT NULL DEFAULT 0,
  email TEXT,
  created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS sessions (
  token TEXT PRIMARY KEY,
  user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  expires_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS api_tokens (
  id TEXT PRIMARY KEY,
  user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  prefix TEXT NOT NULL,
  token_hash TEXT NOT NULL UNIQUE,
  created_at TEXT NOT NULL,
  last_used_at TEXT
);
CREATE INDEX IF NOT EXISTS idx_api_tokens_user ON api_tokens(user_id);
CREATE TABLE IF NOT EXISTS teams (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL UNIQUE,
  description TEXT NOT NULL DEFAULT '',
  ldap_group TEXT,
  organization_id TEXT,
  created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS memberships (
  team_id TEXT NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
  user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  role TEXT NOT NULL,
  source TEXT NOT NULL,
  PRIMARY KEY (team_id, user_id)
);
CREATE TABLE IF NOT EXISTS projects (
  id TEXT PRIMARY KEY,
  team_id TEXT NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  source_type TEXT NOT NULL,
  source TEXT NOT NULL,
  revision TEXT NOT NULL DEFAULT '',
  credential_id TEXT,
  organization_id TEXT,
  created_at TEXT NOT NULL,
  admin_saved INTEGER NOT NULL DEFAULT 1,
  last_sync_at TEXT NOT NULL DEFAULT '',
  last_sync_commit TEXT NOT NULL DEFAULT '',
  last_sync_status TEXT NOT NULL DEFAULT '',
  UNIQUE (team_id, name)
);
CREATE TABLE IF NOT EXISTS inventories (
  id TEXT PRIMARY KEY,
  team_id TEXT NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  body TEXT NOT NULL,
  created_at TEXT NOT NULL,
  source_kind TEXT NOT NULL DEFAULT 'ini',
  source_json TEXT NOT NULL DEFAULT '{}',
  credential_id TEXT,
  admin_saved INTEGER NOT NULL DEFAULT 1,
  UNIQUE (team_id, name)
);
CREATE TABLE IF NOT EXISTS inventory_sources (
  id TEXT PRIMARY KEY,
  inventory_id TEXT NOT NULL REFERENCES inventories(id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  kind TEXT NOT NULL,
  source_json TEXT NOT NULL DEFAULT '{}',
  credential_id TEXT,
  body TEXT NOT NULL DEFAULT '',
  position INTEGER NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS credentials (
  id TEXT PRIMARY KEY,
  team_id TEXT NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  kind TEXT NOT NULL,
  secret_blob TEXT NOT NULL,
  created_at TEXT NOT NULL,
  auto INTEGER NOT NULL DEFAULT 0,
  disabled INTEGER NOT NULL DEFAULT 0,
  UNIQUE (team_id, name)
);
CREATE TABLE IF NOT EXISTS environments (
  id TEXT PRIMARY KEY,
  team_id TEXT REFERENCES teams(id) ON DELETE CASCADE,
  organization_id TEXT,
  name TEXT NOT NULL,
  tool TEXT NOT NULL DEFAULT 'any',
  image TEXT NOT NULL,
  pull_policy TEXT NOT NULL,
  network TEXT NOT NULL DEFAULT '',
  description TEXT NOT NULL DEFAULT '',
  created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS templates (
  id TEXT PRIMARY KEY,
  team_id TEXT NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  description TEXT NOT NULL DEFAULT '',
  tool TEXT NOT NULL,
  project_id TEXT REFERENCES projects(id) ON DELETE SET NULL,
  inventory_id TEXT REFERENCES inventories(id) ON DELETE SET NULL,
  credential_id TEXT REFERENCES credentials(id) ON DELETE SET NULL,
  environment_id TEXT REFERENCES environments(id) ON DELETE SET NULL,
  no_environment INTEGER NOT NULL DEFAULT 0,
  entrypoint TEXT NOT NULL DEFAULT '',
  extra_vars TEXT NOT NULL DEFAULT '{}',
  spec TEXT NOT NULL DEFAULT '{}',
  created_at TEXT NOT NULL,
  update_on_launch INTEGER NOT NULL DEFAULT 1,
  admin_saved INTEGER NOT NULL DEFAULT 1,
  prompt_on_launch INTEGER NOT NULL DEFAULT 0,
  UNIQUE (team_id, name)
);
CREATE TABLE IF NOT EXISTS template_credentials (
  template_id TEXT NOT NULL REFERENCES templates(id) ON DELETE CASCADE,
  credential_id TEXT NOT NULL REFERENCES credentials(id) ON DELETE CASCADE,
  position INTEGER NOT NULL,
  PRIMARY KEY (template_id, credential_id)
);
CREATE TABLE IF NOT EXISTS workflow_jobs (
  id TEXT PRIMARY KEY,
  workflow_id TEXT,
  team_id TEXT NOT NULL,
  name TEXT NOT NULL,
  status TEXT NOT NULL,
  extra_vars TEXT NOT NULL DEFAULT '{}',
  launched_by TEXT NOT NULL,
  trigger TEXT NOT NULL,
  created_at TEXT NOT NULL,
  finished_at TEXT
);
CREATE TABLE IF NOT EXISTS jobs (
  id TEXT PRIMARY KEY,
  team_id TEXT NOT NULL,
  template_id TEXT,
  workflow_job_id TEXT,
  workflow_node_id TEXT,
  name TEXT NOT NULL,
  tool TEXT NOT NULL,
  status TEXT NOT NULL,
  extra_vars TEXT NOT NULL DEFAULT '{}',
  launched_by TEXT NOT NULL,
  trigger TEXT NOT NULL,
  started_at TEXT,
  finished_at TEXT,
  created_at TEXT NOT NULL,
  parameters TEXT NOT NULL DEFAULT '{}',
  failed_hosts TEXT NOT NULL DEFAULT '[]'
);
CREATE TABLE IF NOT EXISTS job_events (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  job_id TEXT NOT NULL,
  ts TEXT NOT NULL,
  stream TEXT NOT NULL,
  line TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS workflows (
  id TEXT PRIMARY KEY,
  team_id TEXT NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  description TEXT NOT NULL DEFAULT '',
  graph TEXT NOT NULL,
  created_at TEXT NOT NULL,
  admin_saved INTEGER NOT NULL DEFAULT 1,
  UNIQUE (team_id, name)
);
CREATE TABLE IF NOT EXISTS workflow_nodes (
  workflow_job_id TEXT NOT NULL,
  node_id TEXT NOT NULL,
  phase TEXT NOT NULL,
  job_id TEXT,
  PRIMARY KEY (workflow_job_id, node_id)
);
CREATE TABLE IF NOT EXISTS hooks (
  id TEXT PRIMARY KEY,
  team_id TEXT NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  token TEXT NOT NULL UNIQUE,
  match_alertname TEXT NOT NULL,
  match_status TEXT NOT NULL DEFAULT 'firing',
  template_id TEXT,
  workflow_id TEXT,
  extra_vars TEXT NOT NULL DEFAULT '{}',
  enabled INTEGER NOT NULL DEFAULT 1,
  created_at TEXT NOT NULL,
  UNIQUE (team_id, name)
);
CREATE TABLE IF NOT EXISTS hook_deliveries (
  id TEXT PRIMARY KEY,
  hook_id TEXT NOT NULL,
  status TEXT NOT NULL,
  summary TEXT NOT NULL,
  payload TEXT NOT NULL,
  job_id TEXT,
  workflow_job_id TEXT,
  created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS organizations (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL UNIQUE,
  description TEXT NOT NULL DEFAULT '',
  created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS project_teams (
  project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  team_id TEXT NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
  PRIMARY KEY (project_id, team_id)
);
CREATE TABLE IF NOT EXISTS settings (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS seed_state (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS schedules (
  id TEXT PRIMARY KEY,
  team_id TEXT NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  target_kind TEXT NOT NULL,
  target_id TEXT NOT NULL,
  timezone TEXT NOT NULL,
  start_date TEXT NOT NULL,
  start_time TEXT NOT NULL,
  frequency TEXT NOT NULL,
  weekdays TEXT NOT NULL DEFAULT '',
  end_kind TEXT NOT NULL,
  end_count INTEGER,
  end_date TEXT,
  enabled INTEGER NOT NULL DEFAULT 1,
  next_run_at TEXT,
  missed_at TEXT,
  occurrence_count INTEGER NOT NULL DEFAULT 0,
  created_by TEXT NOT NULL,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  updated_by TEXT NOT NULL DEFAULT '',
  launch_block TEXT,
  UNIQUE (target_kind, target_id, name)
);
CREATE INDEX IF NOT EXISTS idx_jobs_team ON jobs(team_id, created_at);
CREATE INDEX IF NOT EXISTS idx_jobs_team_created ON jobs(team_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_jobs_team_status ON jobs(team_id, status, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_events_job ON job_events(job_id, id);
CREATE INDEX IF NOT EXISTS idx_members_user ON memberships(user_id);
CREATE INDEX IF NOT EXISTS idx_schedules_due ON schedules(enabled, next_run_at);
CREATE INDEX IF NOT EXISTS idx_schedules_target ON schedules(target_kind, target_id);
