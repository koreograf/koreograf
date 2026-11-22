ALTER TABLE inventories ADD COLUMN vars_json TEXT NOT NULL DEFAULT '[]';
ALTER TABLE inventory_sources ADD COLUMN vars_json TEXT NOT NULL DEFAULT '[]';
