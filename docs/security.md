# Security

The prototype is meant for a trusted network. The Docker socket is root on the host. `data/secret.key` decrypts every credential and the saved directory and proxy passwords. Copying that file, the SQLite database, or `koreograf-backup.zip` exposes them. `KOREOGRAF_LDAP_TLS_INSECURE` and an open port without TLS do not belong on the internet.

Local jobs run as the server user. `KOREOGRAF_LOCAL_RUNNER_FOR` defaults to `admins`, so an operator or an anonymous webhook cannot start one. The job process does not inherit the server environment. Ansible extra vars are written as unsafe YAML, so a webhook value is not evaluated as a template. Decrypted credential files are removed when the job ends and again on the next start. A container cannot mount the data directory.

Path projects and `file://` clones cannot use the data directory or a system directory such as `/etc`. See [Configuration](configuration.md) for `KOREOGRAF_PROJECT_ROOTS`.

LDAP sign-in does not take over a local account that already uses the same username. A team bound to a group DN matches that directory entry. A CN binding matches the CN under the search base.

If an operator who should not have been trusted could start local jobs, create new credentials and retire the old ones. `secret.key` decrypts every stored secret, and a backup zip from that period contains the same key.

How to report a vulnerability is in [SECURITY.md](../SECURITY.md).
