CREATE TABLE IF NOT EXISTS host_results (
  id BIGSERIAL PRIMARY KEY,
  job_id TEXT NOT NULL,
  team_id TEXT NOT NULL,
  inventory_id TEXT,
  host TEXT NOT NULL,
  playbook TEXT NOT NULL,
  role TEXT NOT NULL,
  result TEXT NOT NULL,
  ok BIGINT NOT NULL DEFAULT 0,
  changed BIGINT NOT NULL DEFAULT 0,
  failed BIGINT NOT NULL DEFAULT 0,
  unreachable BIGINT NOT NULL DEFAULT 0,
  skipped BIGINT NOT NULL DEFAULT 0,
  ts TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_host_results_team_host
  ON host_results(team_id, inventory_id, host, ts DESC);
CREATE INDEX IF NOT EXISTS idx_host_results_job ON host_results(job_id);
