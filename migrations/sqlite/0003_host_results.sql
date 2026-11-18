CREATE TABLE IF NOT EXISTS host_results (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  job_id TEXT NOT NULL,
  team_id TEXT NOT NULL,
  inventory_id TEXT,
  host TEXT NOT NULL,
  playbook TEXT NOT NULL,
  role TEXT NOT NULL,
  result TEXT NOT NULL,
  ok INTEGER NOT NULL DEFAULT 0,
  changed INTEGER NOT NULL DEFAULT 0,
  failed INTEGER NOT NULL DEFAULT 0,
  unreachable INTEGER NOT NULL DEFAULT 0,
  skipped INTEGER NOT NULL DEFAULT 0,
  ts TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_host_results_team_host
  ON host_results(team_id, inventory_id, host, ts DESC);
CREATE INDEX IF NOT EXISTS idx_host_results_job ON host_results(job_id);
