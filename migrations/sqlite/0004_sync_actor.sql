ALTER TABLE projects ADD COLUMN last_sync_by TEXT NOT NULL DEFAULT '';
ALTER TABLE inventories ADD COLUMN last_sync_at TEXT NOT NULL DEFAULT '';
ALTER TABLE inventories ADD COLUMN last_sync_status TEXT NOT NULL DEFAULT '';
ALTER TABLE inventories ADD COLUMN last_sync_by TEXT NOT NULL DEFAULT '';
