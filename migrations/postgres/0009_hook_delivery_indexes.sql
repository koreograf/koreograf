CREATE INDEX IF NOT EXISTS idx_hook_deliveries_created ON hook_deliveries(created_at);
CREATE INDEX IF NOT EXISTS idx_hook_deliveries_hook ON hook_deliveries(hook_id);
