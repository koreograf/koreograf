-- A template belongs to at most one group. Backfill the chosen group from the
-- old membership table, lowest position first, then remove the join table.
ALTER TABLE templates ADD COLUMN group_id TEXT REFERENCES template_groups(id) ON DELETE SET NULL;
UPDATE templates SET group_id = (
  SELECT m.group_id FROM template_group_members m
  JOIN template_groups g ON g.id = m.group_id
  WHERE m.template_id = templates.id
  ORDER BY g.position, g.name, g.id
  LIMIT 1
);
DROP TABLE template_group_members;
