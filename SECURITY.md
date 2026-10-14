# Security

Koreograf is a prototype for a trusted network. Do not put it on the public internet without TLS and an access control layer in front of it.

The Docker socket is root on the host. Leave `runner.dockerSocket.enabled` off unless you accept that.

`data/secret.key` decrypts every credential and the saved directory and proxy passwords. The SQLite database and `koreograf-backup.zip` hold the same material. Do not commit them, paste them into an issue, or attach them to a report.

## Reporting

Use GitHub private vulnerability reporting on this repository. Do not open a public issue for a security report, and do not include passwords, tokens, keys, or extra variables.

Enable private reporting in the repository settings if it is still off. Until then, contact a repository owner directly and wait for a private channel.
