# Koreograf

Koreograf ist ein Prototyp für teamfähige Automation. Eine Rust-Binary stellt die API, die Oberfläche und den Runner. Ansible, OpenTofu und Shell laufen als Aufträge. Collections, Rollen und OpenTofu-Provider bleiben in einem Cache. Workflows sind gerichtete Graphen. Webhooks von Alertmanager oder Grafana starten diese Abläufe.

Das ist kein Ersatz für AWX und kein Nachbau von Semaphore. Es gibt keine Surveys, keine Zeitpläne, kein Receptor und kein Hochverfügbarkeits-Setup. Die Datenbank ist heute eine SQLite-Datei mit einem Schreiber. PostgreSQL ist noch nicht angebunden.

## Was der Cache ändert

AWX installiert Collections bei jedem Lauf neu, weil das Execution Environment flüchtig ist. Koreograf legt den Galaxy- und Rollen-Cache unter `data/cache/<team>/` ab und hängt ihn in jeden Lauf. Ein zweiter Lauf schreibt ins Log, welche Rollen schon im Cache liegen. Schlägt `ansible-galaxy` fehl, läuft der Auftrag mit dem vorhandenen Cache weiter.

OpenTofu nutzt `TF_PLUGIN_CACHE_DIR` auf demselben Cache. `TF_DATA_DIR` bleibt im Auftragsverzeichnis.

## Datenbank und Sicherung

HomeLab und dieser Prototyp bleiben bei SQLite (`data/koreograf.db`, WAL, ein Schreiber). `KOREOGRAF_DATABASE_URL` leer oder `sqlite:` meint diese Datei. Eine `postgres://`-Adresse bricht den Start mit einem klaren Hinweis ab, bis die Firmen-Datenbank angebunden ist. Die Sicherung aus der Oberfläche ist dafür das Übergabeformat: eine SQLite-Datei plus Schlüssel, nicht ein Rohkopie-Verzeichnis.

Ein Administrator öffnet **Sicherung** in der Seitenleiste.

- **Herunterladen** schreibt `koreograf-backup.zip`. Darin liegen `manifest.json` (`format` 1, `engine` sqlite), eine konsistente Datenbank (`koreograf.db`, über die SQLite-Backup-API) und `secret.key`. Ohne den Schlüssel sind die Zugänge nach dem Einspielen unlesbar.
- **Nicht enthalten** sind Cache, Git-Checkouts und Auftragsverzeichnisse. Collections und Rollen holt der nächste Lauf neu, Git-Projekte werden erneut geklont. Projekte mit lokalem Pfad brauchen denselben Pfad auf dem Zielrechner.
- **Einspielen** ersetzt die Installation. Laufende Aufträge müssen vorher beendet sein. Die Anfrage schickt den Zip-Körper und den Kopf `X-Restore-Confirm: ERSETZEN`. Danach ist eine neue Anmeldung nötig. Die vorherige Installation bleibt als `restore-previous.zip` im Datenverzeichnis (Modus `0600`) und kann auf demselben Weg wieder eingespielt werden.
- Die Datei ist geheim: sie enthält Benutzer, Hooks und den Schlüssel für alle Zugänge. Grenze für den Upload sind 256 MiB.

`GET /api/admin/backup` und `POST /api/admin/backup` sind nur für den lokalen Administrator.

## Logs und Prometheus

Koreograf öffnet keinen Syslog-Socket. Jedes Ereignis ist eine Zeile auf stdout. journald, Docker und Promtail lesen diese Ausgabe. `KOREOGRAF_LOG_FORMAT=json` schreibt dieselben Felder als JSON. Ohne die Variable bleibt das Textformat.

Die Zeilen enthalten keine Zugänge, keine Tokens und keine Extra-Variablen.

| Ziel | Wann | Felder |
| --- | --- | --- |
| `koreograf::job` | Auftrag gestartet und beendet | `job_id`, `team_id`, `tool`, `playbook`, `template`, beim Ende `status` und `duration_ms` |
| `koreograf::cache` | beim Start eines Auftrags | Anzahl Collections und Rollen, Bytes für Collections, Rollen und OpenTofu |
| `koreograf::role` | Ansible-Rolle beendet, und sofort bei `failed` oder `unreachable` | `playbook`, `role`, `result` oder die Zähler `ok`, `changed`, `failed`, `unreachable`, `skipped` |

`playbook` ist bei Ansible der Playbook-Pfad. Bei Shell und OpenTofu steht dort der Einstiegspfad. Rollen liest Koreograf aus der Ansible-Standardausgabe (`TASK [rolle : aufgabe]`, `RUNNING HANDLER`, `ok:`, `changed:`, `fatal:`, `unreachable:`, `skipping:`). `KOREOGRAF Rolle aus Cache` heißt, dass die Rolle schon im Cache liegt. Ein Aufgabenname mit ` : ` außerhalb einer Rolle kann wie eine Rolle aussehen.

`GET /metrics` ist offen, wie `/healthz`, und liefert Prometheus-Text. Zähler liegen im Speicher und beginnen nach einem Neustart bei null. Prometheus behandelt diesen Sprung als Neustart. Die Cache-Zahlen liest der Abruf vom Datenträger, auch wenn gerade kein Auftrag läuft. Der Zeitpunkt einer Rolle steht in der Logzeile. `koreograf_role_results_total` zählt Ergebnisse und hat keine Uhrzeit.

| Metrik | Typ | Labels |
| --- | --- | --- |
| `koreograf_jobs_started_total` | counter | `tool`, `playbook` |
| `koreograf_jobs_finished_total` | counter | `tool`, `playbook`, `status` (`successful`, `failed`, `canceled`) |
| `koreograf_role_results_total` | counter | `playbook`, `role`, `result` (`ok`, `changed`, `failed`, `unreachable`, `skipped`) |
| `koreograf_cache_collections` | gauge | `team_id`, `team` |
| `koreograf_cache_roles` | gauge | `team_id`, `team` |
| `koreograf_cache_bytes` | gauge | `team_id`, `team`, `kind` (`collections`, `roles`, `tofu`) |

## Start auf der Maschine

Voraussetzungen: Rust 1.85 oder neuer, `ansible-playbook` und `tofu` auf `PATH`, wenn diese Werkzeuge lokal laufen sollen. Der Shell-Healthcheck braucht nur `sh`.

```bash
cargo run
```

Die Oberfläche liegt auf `http://127.0.0.1:8080`. Unten in der Seitenleiste und auf der Anmeldung schaltet DE / ENG die Sprache. Listen zum Anlegen haben denselben Pillen-Button, zum Beispiel `+ Projekt`, `+ Zugang` und `+ Umgebung`. Ohne `KOREOGRAF_ADMIN_PASSWORD` schreibt der erste Start das Passwort nach `data/initial-admin-password` (Modus `0600`) und nennt den Pfad im Log. Das Passwort steht nicht im Log.

Umgebungsvariablen: siehe `.env.example`.

| Variable | Vorgabe |
| --- | --- |
| `KOREOGRAF_BIND` | `0.0.0.0:8080` |
| `KOREOGRAF_DATA` | `./data` |
| `KOREOGRAF_WEB` | `./web` |
| `KOREOGRAF_EXAMPLES` | `./examples` |
| `KOREOGRAF_ADMIN_USER` | `admin` |
| `KOREOGRAF_ADMIN_PASSWORD` | leer, dann einmalig erzeugt |
| `KOREOGRAF_RUNNER` | `auto` (`local` oder `docker`) |
| `KOREOGRAF_JOB_TIMEOUT_SECS` | `1800` (30–86400) |
| `KOREOGRAF_SECRET_KEY` | 64 Hex-Zeichen, sonst `data/secret.key` |
| `KOREOGRAF_LOG_FORMAT` | leer, Text auf stdout. `json` für Promtail |
| `KOREOGRAF_DATABASE_URL` | leer, SQLite. `postgres://` wird abgelehnt |

`auto` prüft `docker` und `podman`. Fehlt die Runtime, laufen Aufträge lokal. `local` ignoriert Execution Environments und schreibt das ins Auftrags-Log. `docker` bricht einen Lauf mit gesetztem Image ab, wenn keine Runtime erreichbar ist. Vorlagen ohne Environment laufen immer lokal.

## Docker Compose

```bash
docker compose up --build
```

Das Image enthält Ansible aus Debian, den Docker-Client und OpenTofu 1.12.1 für `linux/amd64`. Der Prozess läuft als root, weil ein gemounteter Docker-Socket root auf dem Host ist.

Der Socket ist in `docker-compose.yml` auskommentiert. Execution Environments brauchen diese Zeile. Ohne Socket bleibt der Runner lokal, und der Demo-Healthcheck läuft.

Daten liegen im Volume `koreograf-data`. Compose liest `.env` für die Variablenersetzung.

## Helm

```bash
helm upgrade --install koreograf deploy/helm/koreograf
```

`replicaCount` ist 1. Ein zweites Pod mit derselben SQLite-Datei verliert Schreibvorgänge. Das Image muss im Cluster erreichbar sein (`image.repository`). Der Docker-Socket ist standardmäßig aus (`runner.dockerSocket.enabled`).

Probes rufen `/healthz` auf. Prometheus kann auf demselben Port `GET /metrics` scrapen. Die Zähler gehören zu dieser einen Replica und liegen nur im Speicher. Ist `admin.password` leer, liegt das erste Passwort im Volume unter `/var/lib/koreograf/initial-admin-password`.

## Organisationen, Teams, LDAP und Active Directory

Eine Organisation besitzt die Projekte. Jedes Team gehört zu genau einer Organisation. Ein Projekt ist für jedes Team dieser Organisation nutzbar, solange keine Teams zugewiesen sind. Sind `team_ids` gesetzt, sehen nur diese Teams das Projekt. Die zugewiesenen Teams müssen zur Organisation des Projekts gehören.

Ein Operator kann das Projekt nicht in eine andere Organisation verschieben als die des gerade gewählten Teams. Ein Administrator kann das. Ein neues Team ohne `organization_id` landet in einer Organisation mit demselben Namen, oder legt sie an. Inventare und Zugänge bleiben am Team.

Rollen sind `viewer`, `operator` und `owner`. Der lokale Administrator umgeht die Rollenprüfung, damit ein LDAP-Ausfall nicht aussperrt. Die Anmeldung prüft zuerst das lokale Passwort, danach LDAP.

Mitgliedschaften aus LDAP haben die Quelle `ldap` und die Rolle `operator`. Verschwindet die Gruppe, wird nur diese Mitgliedschaft entfernt. Manuelle Mitgliedschaften bleiben.

Am Team steht eine LDAP-Gruppe als CN oder DN. Der Vergleich ignoriert Großschreibung.

| Variable | LDAP | Active Directory |
| --- | --- | --- |
| `KOREOGRAF_LDAP_URL` | `ldap://ldap.example.com` | `ldap://dc.example.com` |
| `KOREOGRAF_LDAP_MODE` | `ldap` | `ad` |
| `KOREOGRAF_LDAP_USER_FILTER` | `(uid={username})` | `(sAMAccountName={username})` |
| `KOREOGRAF_LDAP_USERNAME_ATTR` | `uid` | `sAMAccountName` |
| `KOREOGRAF_LDAP_GROUP_ATTR` | `memberOf` | `memberOf` |

Leere Filter und Attribute nutzen die Vorgabe der gewählten Art. `KOREOGRAF_LDAP_STARTTLS=true` schaltet StartTLS ein. `KOREOGRAF_LDAP_TLS_INSECURE=true` prüft das Zertifikat nicht. Der Dienst bindet mit `KOREOGRAF_LDAP_BIND_DN`, sucht den Benutzer und bindet danach mit dessen Passwort. `{username}` im Filter wird maskiert.

## Execution Environments

Ein Environment ist ein Image plus Pull-Policy (`if_not_present`, `always`, `never`). Die Demo legt `quay.io/ansible/awx-ee:latest` an und hängt es an keine Vorlage, damit der erste Lauf ohne Image-Pull funktioniert.

Im Container mountet der Runner das Projekt nur lesend nach `/runner/project`, das Arbeitsverzeichnis nach `/runner/work` und den Cache nach `/cache`. `--env-file` zeigt auf eine Datei auf dem Host. Das Image bekommt eine bereinigte Umgebung: `PATH` nur lokal, `HOME` im Arbeitsverzeichnis, `LANG=C.UTF-8` und die `KOREOGRAF_*`-Variablen. Die Umgebung des Servers wird nicht vererbt.

## Workflows

Der Editor zeichnet Knoten und Kanten. Kanten gelten bei `success`, `failure` oder `always`. Ein Knoten startet bei `any`, sobald eine passende Kante feuerbereit ist, oder bei `all`, wenn jede passende Vorgängerkante erfüllt ist. Ein Fehlschlag gilt als behandelt, wenn eine `failure`- oder `always`-Kante davon ausgeht. Zyklen lehnt die API ab. Ein leerer Graph lässt sich speichern und nicht starten.

Abbrechen setzt den Auftrag auf `canceled`. Der Workflow behandelt das wie einen Fehlschlag, damit eine Fehlerkante weiterlaufen kann.

## Webhooks

`POST /hooks/{id}/{token}` ist öffentlich. Der Token ist die Anmeldung. Alertmanager liest `alerts[].labels.alertname` und `status`. Grafana liest das ältere Format mit `state` und `ruleName`. Sonst reicht ein Feld `alertname`. `alerting` wird zu `firing`, `ok` zu `resolved`.

Ein Treffer aus Alertname und Status (`firing`, `resolved`, `any`) startet die Vorlage oder den Workflow. Pro Aufruf höchstens 10 Starts. Ein gültiger Token antwortet mit 200, auch wenn kein Alert passt. Ein falscher Token ist 401, eine unbekannte Id 404.

Variablen entstehen in dieser Reihenfolge: Extra-Variablen der Vorlage, dann die des Hooks, dann das Ereignis (`alertname`, `status`, `instance`, `summary`, `event`). `heal_action` aus dem Hook überschreibt ein gleichnamiges Label nicht.

Alertmanager:

```yaml
receivers:
  - name: koreograf
    webhook_configs:
      - url: http://koreograf.example.com/hooks/HOOK_ID/TOKEN
        send_resolved: true
```

Grafana, Kontaktpunkt Webhook, URL dieselbe Adresse. In der Oberfläche legt ein Operator den Hook an und sieht die fertige URL. `Test` schickt einen Alertmanager-Körper mit dem hinterlegten Alertnamen.

## Werkzeuge

Die Registry steht in `runner::tools()`. Ein weiteres CLI-Werkzeug braucht einen Eintrag dort und einen Zweig in `src/wrapper.sh`. Die drei eingebauten Werkzeuge:

| Id | Lauf |
| --- | --- |
| `ansible` | `ansible-playbook`, Galaxy in den Team-Cache |
| `opentofu` | `tofu init`, danach `plan` oder `apply` |
| `shell` | Skript aus dem Projekt oder `spec.script` als `inline.sh` |

Zugänge sind `ssh`, `vault`, `env` oder `netbox`. Sie werden mit AES-GCM gespeichert und von der API nie zurückgegeben. `ssh` und `vault` liegen im Auftragsverzeichnis mit Modus `0600`. Ein NetBox-Token bleibt im Zugang.

## Playbooks und Inventarquellen

`GET /api/projects/{id}/contents` liest ein Ansible-Projekt und listet Playbooks. Eine Datei zählt, wenn sie auf `.yml` oder `.yaml` endet, die erste Inhaltszeile eine Liste ist und irgendwo `hosts:` steht. `requirements.yml`, `galaxy.yml` und Rollenverzeichnisse wie `tasks`, `handlers` und `group_vars` bleiben außen vor. Ein Git-Projekt muss vorher synchronisiert sein. Die Vorlage bietet die gefundenen Dateien an. Ein anderer Pfad lässt sich weiter von Hand eintragen.

Inventardateien heißen `hosts` oder `inventory`, oder sie liegen in `inventory/` und `inventories/`. Ein relativer `inventory=`-Eintrag unter `[defaults]` in `ansible.cfg` kommt dazu. `POST /api/inventories/import` speichert den Dateitext. INI und das übliche YAML-Inventar erscheinen in der Hostliste. Ansible bekommt denselben Text.

Eine Datei mit `plugin:` wird nicht ausgeführt. Für `plugin: netbox.netbox.nb_inventory` (auch `plugin: netbox`) gibt es eine NetBox-Quelle. Der Controller holt Geräte und, wenn gewünscht, virtuelle Maschinen, gruppiert sie wie das Plugin (`sites_berlin`, `device_roles_web`) und speichert INI. Der Token steht im Zugang, nicht in der Quelle. Ein Inventar kann mehrere Quellen haben, zum Beispiel eine Datei aus dem Projekt und NetBox. Ansible bekommt jede Quelle als eigene Datei. `POST /api/inventories/{id}/sync` holt alle externen Quellen neu. Andere Inventory-Plugins stehen in der Liste und lassen sich hier nicht ausführen.

Eine Ansible-Vorlage speichert den Lauf in `spec`: `job_type` `run` oder `check`, `tags`, `skip_tags`, `limit`, `diff`, `verbosity` von 0 bis 4 und `forks` von 1 bis 50. Check startet `ansible-playbook --check`.

## API

Sitzung per Cookie `koreograf_session`, HttpOnly, SameSite=Lax, sieben Tage. Team-Routen brauchen `X-Team-Id`. Auftrags-Events lesen die Team-Mitgliedschaft am Auftrag, weil `EventSource` keine eigenen Header setzt. Der Strom ist `GET /api/jobs/{id}/events?after=`.

| Methode | Pfad |
| --- | --- |
| POST | `/api/login`, `/api/logout`, `/api/users` |
| GET | `/api/me`, `/api/overview`, `/api/teams` |
| POST | `/api/teams` |
| PUT | `/api/teams/{id}` |
| GET, POST | `/api/teams/{id}/members` |
| DELETE | `/api/teams/{id}/members/{user_id}` |
| GET, POST | `/api/organizations` |
| GET, PUT | `/api/organizations/{id}` |
| GET, POST | `/api/projects`, `/api/inventories`, `/api/credentials`, `/api/environments` |
| GET | `/api/projects/{id}`, `/api/inventories/{id}` |
| PUT | `/api/projects/{id}`, `/api/inventories/{id}` |
| DELETE | `/api/projects/{id}`, `/api/inventories/{id}`, `/api/credentials/{id}`, `/api/environments/{id}` |
| GET | `/api/projects/{id}/contents` |
| POST | `/api/projects/{id}/sync` |
| POST | `/api/inventories/import`, `/api/inventories/netbox` |
| POST | `/api/inventories/{id}/sync` |
| POST | `/api/inventories/{id}/sources` |
| PUT, DELETE | `/api/inventories/{id}/sources/{source_id}` |
| POST | `/api/inventories/{id}/sources/{source_id}/sync` |
| GET | `/api/cache`, `/api/plugins` |
| GET, POST | `/api/templates`, `/api/workflows`, `/api/hooks`, `/api/jobs` |
| PUT, DELETE | `/api/templates/{id}`, `/api/workflows/{id}` |
| POST | `/api/templates/{id}/launch`, `/api/workflows/{id}/launch` |
| GET | `/api/jobs/{id}`, `/api/workflows/{id}`, `/api/workflow-jobs/{id}` |
| POST | `/api/jobs/{id}/cancel`, `/api/hooks/{id}/test` |
| GET | `/api/hooks/{id}/deliveries` |
| DELETE | `/api/hooks/{id}` |
| POST | `/hooks/{id}/{token}` |
| GET | `/healthz`, `/metrics` |
| GET, POST | `/api/admin/backup` |

`PUT /api/projects/{id}` setzt `organization_id` und `team_ids`. Eine leere Liste gibt das Projekt an jedes Team der Organisation. `GET /api/inventories/{id}` liefert `hosts`, `host_count`, `source_kind` und `source`. Gruppen mit `:vars` oder `:children` sind keine Hosts. Ein Git-Sync antwortet bei einem Fehler mit den letzten Zeilen von Git. Erlaubte Adressen sind `http(s)`, `ssh`, `git@` und `file`. `contents` antwortet mit `playbooks` und `inventories`. `import` übernimmt eine statische Datei. `netbox` erwartet `name`, `api_endpoint`, `token`, optional `group_by`, `query_filters`, `validate_certs` und `virtual_machines`. Der Abruf passiert vor dem Speichern.

Starts antworten mit 202 und brauchen einen JSON-Körper, mindestens `{}`. `extra_vars` ist ein Objekt.

## Sicherheit

Der Prototyp ist für ein vertrauenswürdiges Netz gedacht. Der Docker-Socket ist root auf dem Host. `data/secret.key` entschlüsselt alle Zugänge. Wer die Datei, die SQLite-Datenbank oder `koreograf-backup.zip` kopiert, liest die Zugänge. `KOREOGRAF_LDAP_TLS_INSECURE` und ein offener Port ohne TLS gehören nicht ins Internet.

## Demo

1. Anmelden als `admin`. Passwort aus der Umgebung oder aus `data/initial-admin-password`.
2. Team `Plattform` wählen.
3. Auf der Übersicht `Test-Alert` drücken.
4. Der Workflow `Self-Heal Ablauf` startet. Der Healthcheck ist Shell. Bei Erfolg läuft die Ansible-Rolle `incident_report` auf dem Controller und schreibt einen Bericht ins Auftragsverzeichnis. Bei Fehler läuft `escalate.sh`.
5. Die Rolle löscht nur Dateinamen, die der Hook in `incident_report_cleanup_files` nennt, und nur unter dem Arbeitsverzeichnis des Auftrags. Der Demo-Hook setzt `heal_action` auf `cleanup-tmp` und die Datei `stale.tmp`.

Ein zweiter Ansible-Lauf zeigt im Log die Rolle aus dem Cache.
