-- Templates may ask only for the fields listed in this JSON array. An empty
-- list with prompt_on_launch off keeps the launch immediate, and a template
-- with prompt_on_launch on keeps asking for every field its tool supports.
ALTER TABLE templates ADD COLUMN prompt_fields TEXT NOT NULL DEFAULT '[]';
