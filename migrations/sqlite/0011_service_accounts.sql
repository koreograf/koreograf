-- Service accounts are users with auth_source = 'service'. The column records
-- the human who created the account. Deleting that human keeps the account and
-- its tokens, and only clears this reference.
ALTER TABLE users ADD COLUMN created_by TEXT REFERENCES users(id) ON DELETE SET NULL;
