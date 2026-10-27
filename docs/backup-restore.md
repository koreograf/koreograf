# Database and backup

Home labs and this prototype stay on SQLite (`data/koreograf.db`, WAL, one writer). An empty `KOREOGRAF_DATABASE_URL`, or a value of `sqlite:`, means that file. A `postgres://` URL stops startup with a clear message until a company database exists. The backup from the UI is the hand-off format for that later move: one SQLite file plus the key, not a raw copy of the data directory.

An administrator opens **Backup** in the sidebar. In the German UI the link is **Sicherung**.

- **Download** writes `koreograf-backup.zip`. Inside are `manifest.json` (`format` 1, `engine` sqlite), a consistent database (`koreograf.db`, copied through the SQLite backup API), and `secret.key`. Without the key, credentials cannot be read after a restore.
- **Left out** are the cache, git checkouts, and job directories. The next run fetches collections and roles again, and git projects are cloned again. OpenTofu state under `data/cache/<team>/tofu-state/` is part of that cache, so a restore does not bring it back. A remote backend is the setup to use when that state has to survive a new data directory. A template that does not update on launch needs a project sync after a restore. Projects that use a local path need that same path on the target machine.
- **Restore** replaces the installation. Finish running jobs first. The request sends the zip body and the header `X-Restore-Confirm: ERSETZEN`. A new sign-in is required afterwards. The previous installation stays as `restore-previous.zip` in the data directory (mode `0600`) and can be restored the same way.
- The file is secret: it contains users, hooks, and the key for every credential. The upload limit is 256 MiB.

`GET /api/admin/backup` and `POST /api/admin/backup` are limited to the local administrator.
