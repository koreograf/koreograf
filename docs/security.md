# Security

The prototype is meant for a trusted network. The Docker socket is root on the host. `data/secret.key` decrypts every credential and the saved directory and proxy passwords. Copying that file, the SQLite database, or `koreograf-backup.zip` exposes them. `KOREOGRAF_LDAP_TLS_INSECURE` and an open port without TLS do not belong on the internet.

How to report a vulnerability is in [SECURITY.md](../SECURITY.md).
