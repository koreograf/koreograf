# Organizations, teams, LDAP, and Active Directory

An organization owns the projects. Each team belongs to exactly one organization. A project is available to every team in that organization until teams are assigned. Once `team_ids` is set, only those teams see the project. Assigned teams must belong to the project's organization.

An operator cannot move a project to an organization other than the one of the team currently selected. An administrator can. A new team without `organization_id` lands in an organization of the same name, or creates that organization. Inventories and credentials stay on the team.

Roles are `viewer`, `operator`, and `owner`. The local administrator bypasses the role check so an LDAP outage cannot lock everyone out. Sign-in checks the local password first, then LDAP.

## Sign-in throttling

Five failed sign-ins for the same normalized username and source address within one minute lock that pair for one minute. The next `POST /api/login` answers `429` with a `Retry-After` header and does not contact the directory. The body keeps `invalid_username_or_password`, so the browser still shows the generic sign-in failure. A correct password from another source still signs in, and a successful sign-in clears the counter for its pair. The counters live in memory, are bounded, and reset on restart.

The source is the direct peer when the connection address is available, or the first `X-Forwarded-For` address when `KOREOGRAF_TRUSTED_PROXY=true`. Direct clients without a connection address share one bucket. A failure log names the normalized username and the client IP and never contains the password.

Memberships from LDAP have source `ldap` and role `operator`. A viewer or owner role is a manual membership on a team the directory does not fill for that user. `POST /api/teams/{id}/members` rejects a role change on an existing directory membership with `the_membership_comes_from_ldap_or_active_directory`. One user has one membership per team, so the directory row cannot also be a manual row. When the group disappears, only that membership is removed. A manual membership stays, and the next directory sync does not replace it. Removing the member is still allowed, and the directory can add that membership back as `operator`.

A team stores an LDAP group as a CN or a DN. A value that contains `=` is a DN and matches only a directory DN. Anything else is a CN and matches only that CN. Comparison ignores case and spaces around commas, plus signs, and equals signs. Equivalent escapes match, so `\2C` and `\,` are the same comma, and `\3D` and `\=` are the same equals sign. A comma, plus, or equals inside a value stays inside that value. `Plattform` and `cn=Plattform,ou=groups,dc=example,dc=com` are both valid. The CN comes from the first RDN, and only when that RDN is a single `cn` attribute.

| Variable | LDAP | Active Directory |
| --- | --- | --- |
| `KOREOGRAF_LDAP_URL` | `ldap://ldap.example.com` | `ldap://dc.example.com` |
| `KOREOGRAF_LDAP_MODE` | `ldap` | `ad` |
| `KOREOGRAF_LDAP_BIND_DN` | `cn=reader,dc=example,dc=com` | `cn=reader,cn=Users,dc=example,dc=com` |
| `KOREOGRAF_LDAP_BIND_PASSWORD` | service account password | service account password |
| `KOREOGRAF_LDAP_USER_BASE` | `ou=people,dc=example,dc=com` | `cn=Users,dc=example,dc=com` |
| `KOREOGRAF_LDAP_USER_FILTER` | `(uid={username})` | `(sAMAccountName={username})` |
| `KOREOGRAF_LDAP_USERNAME_ATTR` | `uid` | `sAMAccountName` |
| `KOREOGRAF_LDAP_GROUP_ATTR` | `memberOf` | `memberOf` |
| `KOREOGRAF_LDAP_GROUP_BASE` | `ou=groups,dc=example,dc=com` | empty, or the same base as the users |

`KOREOGRAF_LDAP_BIND_DN`, `KOREOGRAF_LDAP_BIND_PASSWORD`, and `KOREOGRAF_LDAP_USER_BASE` are required once `KOREOGRAF_LDAP_URL` is set. If any of them is empty, sign-in stops with `ldap_is_not_fully_configured` before any search. An empty bind password is an unauthenticated bind. Some directories accept it and continue as anonymous, so Koreograf rejects it instead. The password can also be stored under [Saved settings](#saved-settings). It is encrypted, and the API never returns it. Saving a directory with a bind DN and an empty password is rejected the same way. `KOREOGRAF_LDAP_GROUP_BASE` has no default.

Empty filters and attributes use the default for the selected mode. `KOREOGRAF_LDAP_STARTTLS=true` turns on StartTLS. `KOREOGRAF_LDAP_TLS_INSECURE=true` skips certificate checks only while it is set. Koreograf writes a warning at startup and again when that setting is saved. Company CAs come from the mounted `/etc/koreograf/ca` directory described in [Installation](installation.md#company-certificate-authorities); they are added to the public roots. The service account binds with `KOREOGRAF_LDAP_BIND_DN` and searches under `KOREOGRAF_LDAP_USER_BASE`. Koreograf then binds with that user's password. `{username}` in the filter is escaped.

`KOREOGRAF_LDAP_USERNAME_ATTR` is the Koreograf username and the `memberUid` value in a `posixGroup` search. Sign-in reads the first value of that attribute on the entry, trims it, and lowercases it. The filter can accept another name. `(|(uid={username})(mail={username}))` lets the same person sign in as `alice` or `alice@example.com`, and both use the `uid` on the entry. If the attribute is missing, the entered login is the account name and the log has one warning.

The service account needs read access to the user base and to the group base. Users do not. Put the group rule before a general `to *` rule. A base the service account cannot read comes back as `noSuchObject` (LDAP result 32), which looks like a wrong base. Active Directory needs no extra grant when the service account is a normal domain user. Add these rules before your existing rules. They are not a complete server policy by themselves. The first rule lets the service account bind when it lives outside `ou=people`:

```text
olcAccess: {0}to attrs=userPassword
  by anonymous auth
  by self write
  by * none
olcAccess: {1}to dn.subtree="ou=groups,dc=example,dc=com"
  by dn.exact="cn=reader,dc=example,dc=com" read
  by * none
olcAccess: {2}to dn.subtree="ou=people,dc=example,dc=com" attrs=userPassword
  by anonymous auth
  by self write
  by * none
olcAccess: {3}to dn.subtree="ou=people,dc=example,dc=com"
  by dn.exact="cn=reader,dc=example,dc=com" read
  by self read
  by * none
```

Groups come from `KOREOGRAF_LDAP_GROUP_ATTR` on the user entry. When that value is empty and the attribute is not `memberOf`, `memberOf` is read as well. A non-empty list is used as it is. Koreograf does not also search the group base in that case. `memberOf` values are DNs, and a team matches either the DN or the CN inside it.

When the user entry has no groups, Koreograf binds again as the service account and searches. The search base is `KOREOGRAF_LDAP_GROUP_BASE`, or `KOREOGRAF_LDAP_USER_BASE` when the group base is empty. The search matches `groupOfNames` whose `member` is the user DN, `posixGroup` whose `memberUid` is the directory account name (`KOREOGRAF_LDAP_USERNAME_ATTR`), `group` whose `member` is the user DN, and `groupOfUniqueNames` whose `uniqueMember` is the user DN. Each group found by that search is the entry's DN. Attributes on the group entry are not the group name. Set a team to the CN of that DN or to the full DN. The CN is taken from the first RDN only when that RDN is a single `cn`. A later `cn`, or a multi-valued first RDN, does not match a CN team. A `uniqueMember` with a trailing nameAndOptionalUID (`#'0101'B`) is not matched. A failed search, or an empty result while `KOREOGRAF_LDAP_GROUP_BASE` is set, is a warning in the log. The password is not in that warning. Sign-in still succeeds, and the user has no directory teams until a later sign-in can read the groups.

## Saved settings

An administrator can change the directory, the HTTP proxy, the runner, the job timeout, and the output line limit under **Settings** (German: **Einstellungen**). `GET` and `PUT /api/settings` are administrator-only. The response never includes the bind password or the proxy password, only whether one is stored. An empty password keeps the stored value. Removing a password uses `clear` with `ldap_bind_password` or `proxy_password`. `PUT` with `{"reset": true}` drops the saved values and returns to the environment variables.

The environment variables apply until the first save. After that, the saved values apply immediately: the next sign-in, the next git sync, the next NetBox fetch, and the next job. A restart is not required. One proxy address is used for both HTTP and HTTPS. It covers git over HTTP and HTTPS, NetBox, and the cleaned job environment, so Galaxy and git inside an execution environment can use it. SSH git does not. An image pull uses the proxy of the container daemon, not this setting. The proxy password is not written to the log. A saved empty proxy address turns the proxy off for those calls, including a proxy that was set on the process. Before the first save, git and NetBox still follow `http_proxy`, `https_proxy`, and `no_proxy` from the process, and a job copies those variables into its environment.

The job timeout is 30 to 86400 seconds. The runner mode is `auto`, `docker`, or `local`. The output line limit is 100 to 100000 lines for each of stdout and stderr. The default is 8000. The job keeps the first three quarters and the last quarter. A save that leaves the limit out keeps the current value. `GET /api/jobs/{id}/output` downloads that kept log and stops at 8 MiB.

## API tokens

A signed-in user creates and revokes personal API tokens under **My account** (German: **Mein Konto**). `GET /api/me/tokens` lists the caller's tokens with the name, prefix, `created_at`, and `last_used_at`. `POST /api/me/tokens` takes a `name` and returns the secret once. `DELETE /api/me/tokens/{id}` revokes one token owned by the caller. Another user's id is 404. An administrator cannot read or mint another user's tokens.

The secret starts with `koro_` and is stored only as a hash. Send it as `Authorization: Bearer <token>`. The token authenticates the same user as the session cookie, and the role checks stay the same. When both a bearer token and a cookie are present, the bearer token wins; a bad token is 401 and does not fall back to the cookie. Team routes still need `X-Team-Id`. A disabled or deleted user cannot use a token. Deleting a user deletes the tokens, and revoking a token does not delete the sessions.

## Users and permissions

An administrator sees every account under **Users** (German: **Benutzerkonten**). `GET /api/users` lists them. `GET /api/users/{id}` returns the account and its memberships. The password hash is never returned.

Every signed-in user opens **My account** (German: **Mein Konto**) from their name in the sidebar. `GET /api/me` returns the account, the real team memberships with organization and role, and the email when one is stored. A local user can change the display name, the email, and the password there. An LDAP email is taken from the directory attribute `mail` at sign-in and is not edited here.

`PUT /api/users/{id}` changes the display name, the administrator flag, and whether the account is disabled. The username stays as it is, because LDAP matches on it. A local account can take a new password in the same request. An empty password leaves the current one. An LDAP account keeps its password in the directory.

Disabling an account ends that user's sessions. The next sign-in, after a correct password, answers `Das Konto ist deaktiviert.` You cannot disable or delete your own account. The last active administrator cannot be demoted, disabled, or deleted.

There is no per-object role. A viewer sees projects, inventories, templates, jobs, and workflows. An operator launches jobs and edits those objects, plus workflows and webhooks. A local job on the controller stays with an administrator. The default refuses the local job. A local job also requires an administrator to have last saved the template, its project, its inventory, and, when the launch comes from a workflow, that workflow. A node override, including a shell script, is part of the workflow. Relaunch checks the stored parameters. A stored command that is not the current template runs only when the workflow that stored it was last saved by an administrator. A project path outside the allowed roots does not run, including one an administrator saved. The examples directory is always allowed. Set `KOREOGRAF_LOCAL_RUNNER_FOR=all` only on a trusted network. An owner also manages credentials, environments, and team members. The administrator flag skips the team-role check and can manage organizations, teams, users, settings, and backup.

`POST /api/teams/{id}/members` sets a manual role. The same call adds a user to a team they are not in yet.
