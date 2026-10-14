"use strict";

const LANGUAGES = [
  { id: "de", code: "GER", name: "Deutsch", country: "de" },
  { id: "en", code: "ENG", name: "English", country: "gb" },
];

const FLAG_SVG = {
  de: `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 5 3"><rect width="5" height="3" fill="#000"/><rect width="5" y="1" height="2" fill="#DE0000"/><rect width="5" y="2" height="1" fill="#FFCE00"/></svg>`,
  gb: `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 60 30"><clipPath id="a"><path d="M0 0v30h60V0z"/></clipPath><clipPath id="b"><path d="M30 15h30v15zv15H0zH0V0zV0h30z"/></clipPath><g clip-path="url(#a)"><path d="M0 0v30h60V0z" fill="#012169"/><path d="M0 0l60 30m0-30L0 30" stroke="#fff" stroke-width="6"/><path d="M0 0l60 30m0-30L0 30" clip-path="url(#b)" stroke="#C8102E" stroke-width="4"/><path d="M30 0v30M0 15h60" stroke="#fff" stroke-width="10"/><path d="M30 0v30M0 15h60" stroke="#C8102E" stroke-width="6"/></g></svg>`,
};

function languageId(value) {
  return LANGUAGES.some((item) => item.id === value) ? value : "de";
}

const app = document.querySelector("#app");
let me = null;
let teamId = localStorage.getItem("koreograf.team") || "";
let lang = languageId(localStorage.getItem("koreograf.lang"));
let source = null;
let pollTimer = null;
let generation = 0;
let runKey = null;
document.documentElement.lang = lang;

const EN = {
  "Abmelden": "Sign out",
  "Abbrechen": "Cancel",
  "Abhängigkeits-Cache": "Dependency cache",
  "Administrator": "Administrator",
  "Administrator verwaltet Organisationen, Teams, Benutzer und die Sicherung. Die Teamrolle wird dabei nicht geprüft.": "An administrator manages organizations, teams, users, and backup. The team role is not checked.",
  "Aktion": "Action",
  "Aktualisieren": "Refresh",
  "Alertname": "Alert name",
  "Alle": "All",
  "Alle Quellen aktualisieren": "Refresh all sources",
  "Alle Teams der Organisation": "All teams in the organization",
  "Alle Vorgänger": "All predecessors",
  "Allgemein": "General",
  "Angelegt": "Created",
  "Anlegen": "Create",
  "Anlegen und öffnen": "Create and open",
  "Anmelden": "Sign in",
  "Ansicht": "View",
  "Ansible": "Ansible",
  "Anzeigename": "Display name",
  "Anzeigename fehlt.": "Display name is missing.",
  "API-Adresse": "API address",
  "Art": "Type",
  "Art ist run oder check.": "Type is run or check.",
  "Art ist ssh, vault, env oder netbox.": "Type is ssh, vault, env, or netbox.",
  "Access Key": "Access key",
  "Adresse muss mit https:// beginnen.": "The address must start with https://.",
  "Amazon Web Services": "Amazon Web Services",
  "Ansible Galaxy": "Ansible Galaxy",
  "Ansible Vault": "Ansible Vault",
  "Authorize": "Authorize",
  "Authorize-Passwort": "Authorize password",
  "AWS braucht access_key.": "AWS needs access_key.",
  "AWS braucht secret_key.": "AWS needs secret_key.",
  "Azure braucht client_id.": "Azure needs client_id.",
  "Azure braucht secret.": "Azure needs secret.",
  "Azure braucht subscription_id.": "Azure needs subscription_id.",
  "Azure braucht tenant.": "Azure needs tenant.",
  "Become-Benutzer": "Become user",
  "Become-Methode": "Become method",
  "Become-Methode ist ungültig.": "The become method is invalid.",
  "Become-Passwort": "Become password",
  "Benutzer fehlt.": "Username is missing.",
  "Client": "Client",
  "Container-Registry": "Container registry",
  "Der Auftrag sieht Host, Benutzer und Passwort. Docker selbst wird nicht angemeldet.": "The job sees the host, username, and password. Docker itself is not logged in.",
  "Dienstkonto": "Service account",
  "Dienstkonto fehlt.": "The service account is missing.",
  "Dienstkonto ist kein JSON-Objekt.": "The service account is not a JSON object.",
  "Domain": "Domain",
  "Geheimnis": "Secret",
  "Geheimnis ist zu groß.": "The secret is too large.",
  "GitHub-Token": "GitHub token",
  "GitLab-Token": "GitLab token",
  "Gilt im Auftrag. Der Projekt-Sync nutzt diesen Zugang noch nicht.": "Applies to the job. Project sync does not use this credential yet.",
  "Diese Vault-Id ist schon gewählt.": "This vault id is already selected.",
  "Gilt nur für diesen Knoten. Die Vorlage bleibt unverändert. Ein anderer Zugang ersetzt nur dieselbe Art.": "Applies only to this node. The template stays unchanged. Another credential replaces only the same kind.",
  "Keine Zugänge im Team.": "No credentials in this team.",
  "Ohne Inventar": "No inventory",
  "Google Compute Engine": "Google Compute Engine",
  "Host": "Host",
  "Keine": "None",
  "Kubernetes": "Kubernetes",
  "Kubernetes braucht host.": "Kubernetes needs a host.",
  "Leer lässt https://galaxy.ansible.com.": "Empty keeps https://galaxy.ansible.com.",
  "Leer setzt die Domain auf Default.": "Empty sets the domain to Default.",
  "Maschine (SSH)": "Machine (SSH)",
  "Microsoft Azure": "Microsoft Azure",
  "NetBox": "NetBox",
  "Netzwerk": "Network",
  "Netzwerk braucht einen Benutzer oder ein Passwort.": "Network needs a username or a password.",
  "OpenStack": "OpenStack",
  "OpenStack braucht auth_url.": "OpenStack needs auth_url.",
  "OpenStack braucht project.": "OpenStack needs project.",
  "Passphrase": "Passphrase",
  "Passphrase braucht einen privaten Schlüssel.": "A passphrase needs a private key.",
  "Passwort fehlt.": "Password is missing.",
  "Quellcode": "Source control",
  "Quellcode braucht einen Schlüssel oder ein Passwort.": "Source control needs a key or a password.",
  "Red Hat Insights": "Red Hat Insights",
  "Registry braucht host.": "The registry needs a host.",
  "Schlüssel oder Passwort, auch beides.": "A key or a password, or both.",
  "Secret Key": "Secret key",
  "Session-Token": "Session token",
  "SSH braucht einen Schlüssel oder ein Passwort.": "SSH needs a key or a password.",
  "Subscription": "Subscription",
  "Tenant": "Tenant",
  "Token fehlt.": "Token is missing.",
  "Umgebungsvariablen": "Environment variables",
  "Unbekannte Zugangsart.": "Unknown credential type.",
  "Vault-Id": "Vault id",
  "Vault-Id ist ungültig.": "The vault id is invalid.",
  "VMware braucht host.": "VMware needs a host.",
  "VMware vCenter": "VMware vCenter",
  "Auftrag fehlt.": "Job is missing.",
  "Auftrag ohne Vorlage.": "Job has no template.",
  "Aufträge": "Jobs",
  "Auftragsvorlage fehlt.": "Job template is missing.",
  "Ausführlichkeit": "Verbosity",
  "Ausführlichkeit ist 0 bis 4.": "Verbosity is 0 to 4.",
  "0 (Normal)": "0 (Normal)",
  "1 (Ausführlich)": "1 (Verbose)",
  "2 (Ausführlicher)": "2 (More Verbose)",
  "4 (Verbindungs-Debug)": "4 (Connection Debug)",
  "Ausführen": "Run",
  "Ausgabe": "Output",
  "Ausgabe herunterladen": "Download output",
  "Die Liste der fehlgeschlagenen Hosts ist zu lang.": "The failed-host list is too long.",
  "Die Tags sind zu lang.": "The tags are too long.",
  "Die Vorlage dieses Auftrags fehlt.": "This job's template is missing.",
  "Erneut starten": "Relaunch",
  "Fehlgeschlagene Hosts": "Failed hosts",
  "Für diesen Auftrag sind keine fehlgeschlagenen Hosts gespeichert.": "No failed hosts are stored for this job.",
  "Für diesen Auftrag wurden keine Parameter gespeichert.": "No parameters were stored for this job.",
  "Ja": "Yes",
  "Nein": "No",
  "Nur Ansible-Aufträge haben fehlgeschlagene Hosts.": "Only Ansible jobs have failed hosts.",
  "Nur ein fehlgeschlagener Auftrag kann erneut gestartet werden.": "Only a failed job can be relaunched.",
  "Nur fehlgeschlagene Hosts": "Failed hosts only",
  "Parameter": "Parameters",
  "Parameter sind ungültig.": "Parameters are invalid.",
  "Parameter sind zu groß.": "Parameters are too large.",
  "Auslöser": "Trigger",
  "Automation": "Automation",
  "Benutzer": "User",
  "Benutzer angelegt.": "User created.",
  "Benutzer ist unbekannt. Lege ihn zuerst an.": "Unknown user. Create the account first.",
  "Benutzer wurde nicht eindeutig gefunden.": "The user was not found uniquely.",
  "Benutzerkonten": "Users",
  "Benutzername": "Username",
  "Benutzername ist ungültig.": "Username is invalid.",
  "Benutzername: 2–64 Zeichen, a-z, 0-9, Punkt, Unterstrich, Bindestrich.": "Username: 2–64 characters, a-z, 0-9, dot, underscore, hyphen.",
  "Benutzer suchen": "Search users",
  "Besitzer verwaltet zusätzlich Zugänge, Umgebungen und Mitglieder.": "An owner also manages credentials, environments, and members.",
  "Betrachter sieht Projekte, Inventare, Vorlagen, Aufträge und Workflows.": "A viewer sees projects, inventories, templates, jobs, and workflows.",
  "Beschreibung": "Description",
  "Besitzer": "Owner",
  "Betrachter": "Viewer",
  "Check": "Check",
  "Check (Dry-Run)": "Check (dry run)",
  "Check startet ansible-playbook mit --check. Tags, übersprungene Tags und Limit gelten für jeden Start dieser Vorlage.": "Check starts ansible-playbook with --check. Tags, skipped tags, and limit apply to every launch of this template.",
  "Ein Komma oder ein Leerzeichen macht den getippten Tag zu einer Pille.": "A comma or a space turns the typed tag into a pill.",
  "Checkout: {path}": "Checkout: {path}",
  "Collections": "Collections",
  "Collections im Cache": "Cached collections",
  "Collections, Rollen und OpenTofu-Provider bleiben zwischen den Läufen liegen.": "Collections, roles, and OpenTofu providers stay between runs.",
  "Dafür brauchst du die Rolle Besitzer.": "You need the owner role.",
  "Dafür brauchst du die Rolle Operator.": "You need the operator role.",
  "Dafür brauchst du ein Administratorkonto.": "You need an administrator account.",
  "Deaktiviert": "Disabled",
  "Das Konto ist deaktiviert.": "The account is disabled.",
  "Das Passwort kommt aus LDAP oder Active Directory.": "The password comes from LDAP or Active Directory.",
  "Das Projekt ist für dieses Team nicht sichtbar.": "This project is not visible to this team.",
  "Datei fehlt im Projekt.": "File is missing from the project.",
  "Der Cache liegt unter data/cache und bleibt zwischen den Läufen erhalten.": "The cache lives under data/cache and stays between runs.",
  "Der Eintrag wird noch verwendet.": "The entry is still in use.",
  "Der Einstiegspfad muss relativ sein und darf das Projekt nicht verlassen.": "The entry path must be relative and must stay inside the project.",
  "Der Benutzer ist in jedem Team.": "The user is in every team.",
  "Der Tag „{part}“ ist ungültig. Erlaubt sind Buchstaben, Ziffern und _ . : -.": "The tag “{part}” is invalid. Letters, digits, and _ . : - are allowed.",
  "Der Benutzername ist schon vergeben.": "That username is already used.",
  "Der letzte Administrator bleibt erhalten.": "The last administrator stays.",
  "Der Name ist in diesem Team schon vergeben.": "That name is already used in this team.",
  "Der Workflow enthält einen Zyklus.": "The workflow contains a cycle.",
  "Der Workflow hat noch keine Knoten.": "The workflow has no nodes yet.",
  "Die Anfrage ist fehlgeschlagen.": "The request failed.",
  "Die Datei ist kein Inventar.": "The file is not an inventory.",
  "Die E-Mail kommt aus LDAP oder Active Directory.": "The email comes from LDAP or Active Directory.",
  "Die Mitgliedschaft kommt aus LDAP oder Active Directory.": "The membership comes from LDAP or Active Directory.",
  "Die manuelle Quelle ist leer.": "The manual source is empty.",
  "Die Sitzung ist abgelaufen.": "The session has expired.",
  "Die Sitzung ist lokal beendet.": "The session ended locally.",
  "Dieselbe Kante ist doppelt vorhanden.": "The same edge is present twice.",
  "Diese Quelle kommt aus einem Projekt oder aus NetBox. Aktualisieren holt sie neu.": "This source comes from a project or from NetBox. Refresh loads it again.",
  "Diese Seite gibt es nicht.": "This page does not exist.",
  "Diese Vorlage gibt es nicht.": "This template does not exist.",
  "Diesem Konto ist kein Team zugeordnet. Ein Administrator legt eines unter Team an.": "This account has no team. An administrator creates one under Team.",
  "Dieses Konto wirklich löschen?": "Delete this account?",
  "Dieses Inventar hat keine externe Quelle.": "This inventory has no external source.",
  "Dieses Inventar hat mehrere Quellen. Den Text bearbeitest du an der einzelnen Quelle.": "This inventory has several sources. Edit the text on the single source.",
  "Diff": "Diff",
  "Diff ist ja oder nein.": "Diff is yes or no.",
  "Du kannst dein eigenes Konto nicht deaktivieren.": "You cannot disable your own account.",
  "Du kannst dein eigenes Konto nicht löschen.": "You cannot delete your own account.",
  "Du kannst dich nicht selbst aus dem Team nehmen.": "You cannot remove yourself from the team.",
  "Ein Inventar kann mehrere Quellen haben. Sie werden zusammen an Ansible übergeben.": "An inventory can have several sources. They are passed to Ansible together.",
  "Ein Knoten hat keine ID.": "A node has no id.",
  "Ein leerer Graph lässt sich speichern, aber nicht starten.": "An empty graph can be saved, but not launched.",
  "Ein Projekt bleibt in der Organisation des gewählten Teams.": "A project stays in the organization of the selected team.",
  "Eine Kante darf nicht auf denselben Knoten zeigen.": "An edge cannot point at the same node.",
  "Eine Kante verweist auf einen unbekannten Knoten.": "An edge points at an unknown node.",
  "Eine Vorlage gehört zu einem anderen Team.": "A template belongs to another team.",
  "E-Mail": "Email",
  "E-Mail ist ungültig.": "Email is invalid.",
  "Eingänge": "Deliveries",
  "Einstieg": "Entry",
  "Einstiegspfad": "Entry path",
  "Einstiegspfad fehlt.": "Entry path is missing.",
  "Env-Zugang muss ein JSON-Objekt sein.": "The env credential must be a JSON object.",
  "Env-Zeile braucht KEY=Wert.": "An env line needs KEY=value.",
  "Erfolg": "Success",
  "Erfolgskante: Zielknoten anklicken.": "Success edge: click the target node.",
  "Erfolgskanten und Fehlerkanten starten die nächsten Vorlagen.": "Success edges and failure edges start the next templates.",
  "Execution Environment": "Execution environment",
  "Execution Environment fehlt.": "Execution environment is missing.",
  "Execution Environment gehört zu einem anderen Team.": "The execution environment belongs to another team.",
  "Execution Environment ist gesetzt, aber weder Docker noch Podman ist erreichbar.": "An execution environment is set, but neither Docker nor Podman is reachable.",
  "Execution Environments": "Execution environments",
  "Extra-Variablen": "Extra variables",
  "Extra-Variablen (JSON)": "Extra variables (JSON)",
  "Extra-Variablen ist kein gültiges JSON.": "Extra variables are not valid JSON.",
  "Extra-Variablen muss ein JSON-Objekt sein.": "Extra variables must be a JSON object.",
  "extra_vars muss ein Objekt sein.": "extra_vars must be an object.",
  "Farben zeigen den Lauf. Ein Klick zeigt die Ausgabe.": "Colors show the run. A click shows the output.",
  "Dauer": "Duration",
  "Ende": "End",
  "Start": "Start",
  "Schließen": "Close",
  "Auftrag öffnen": "Open the job",
  "Dieser Knoten hat noch keine Ausgabe.": "This node has no output yet.",
  "Dieser Knoten wurde übersprungen.": "This node was skipped.",
  "Fehler": "Error",
  "Fehlerkante: Zielknoten anklicken.": "Failure edge: click the target node.",
  "Filter, eine Zeile je Schlüssel=Wert": "Filters, one key=value per line",
  "Forks": "Forks",
  "Forks ist eine ganze Zahl.": "Forks is a whole number.",
  "Forks ist eine Zahl von 1 bis 50.": "Forks is a number from 1 to 50.",
  "Geheimnis muss ein JSON-Objekt sein.": "The secret must be a JSON object.",
  "Geheimnisse werden verschlüsselt gespeichert und nicht wieder angezeigt.": "Secrets are stored encrypted and are not shown again.",
  "Gespeichert.": "Saved.",
  "Git-Adresse ist ungültig.": "The Git address is invalid.",
  "Git-Adresse muss http(s), ssh, git@ oder file sein.": "The Git address must be http(s), ssh, git@, or file.",
  "Git-Projekt zuerst synchronisieren.": "Sync the Git project first.",
  "Git hat das Zeitlimit von 120 Sekunden überschritten.": "Git exceeded the 120 second time limit.",
  "Grün ist erfolgreich, Rot ist fehlgeschlagen, Blau läuft.": "Green is successful, red is failed, blue is running.",
  "Gruppierung": "Group by",
  "Gruppierung „{key}“ wird nicht unterstützt. Möglich sind sites, device_roles, role, platform, tenant, tags, status, cluster, region, location, manufacturer, device_type, site_group und is_virtual.": "Grouping “{key}” is not supported. Allowed values are sites, device_roles, role, platform, tenant, tags, status, cluster, region, location, manufacturer, device_type, site_group, and is_virtual.",
  "Herkunft": "Source",
  "Hinzufügen": "Add",
  "Hook": "Hook",
  "Hook hat kein Ziel.": "The hook has no target.",
  "Hosts": "Hosts",
  "Höchstens 8 Quellen.": "At most 8 sources.",
  "Id": "Id",
  "Image": "Image",
  "Image-Name ist ungültig.": "The image name is invalid.",
  "Immer": "Always",
  "In dieser Organisation gibt es noch kein Team.": "This organization has no team yet.",
  "INI oder YAML": "INI or YAML",
  "Interner Fehler.": "Internal error.",
  "Inventar": "Inventory",
  "Inventar anlegen": "New inventory",
  "Inventar fehlt.": "Inventory is missing.",
  "Inventar gehört zu einem anderen Team.": "The inventory belongs to another team.",
  "Inventar löschen": "Delete inventory",
  "Inventar löschen?": "Delete this inventory?",
  "Inventardatei": "Inventory file",
  "Inventardatei ist größer als 1 MB.": "The inventory file is larger than 1 MB.",
  "Inventare": "Inventories",
  "Inventare suchen": "Search inventories",
  "Irgendein Vorgänger": "Any predecessor",
  "Kante": "Edge",
  "Kante gesetzt.": "Edge added.",
  "Kante löschen": "Delete edge",
  "Keine Berechtigung für diese Aktion.": "You are not allowed to do this.",
  "Keine E-Mail hinterlegt.": "No email is stored.",
  "Keine Inventardateien. Bei einem Git-Projekt zuerst synchronisieren.": "No inventory files. Sync the Git project first.",
  "Keine Playbooks gefunden. Bei einem Git-Projekt zuerst synchronisieren. Den Pfad kannst du unten eintragen.": "No playbooks found. Sync the Git project first. You can type the path below.",
  "Kein Team": "No team",
  "Kein Team ausgewählt.": "No team selected.",
  "Kommt aus dem Verzeichnis.": "Comes from the directory.",
  "Konto": "Account",
  "Konto deaktivieren": "Disable account.",
  "Leer lassen, um das Passwort zu behalten.": "Leave empty to keep the password.",
  "Lokal": "Local",
  "Mitgliedschaften": "Memberships",
  "Neues Passwort": "New password",
  "Operator startet Aufträge und pflegt Projekte, Inventare, Vorlagen, Workflows und Webhooks.": "An operator launches jobs and maintains projects, inventories, templates, workflows, and webhooks.",
  "Rechte": "Permissions",
  "Zum Team hinzufügen": "Add to a team",
  "KEY=Wert, eine Zeile je Variable": "KEY=value, one variable per line",
  "Knoten": "Nodes",
  "Knoten hinzufügen": "Add node",
  "Knoten löschen": "Delete node",
  "Knoten ziehen. Weitere Knoten rechts hinzufügen. Verbinden über die Auswahl am Knoten oder über Erfolg und Fehler auf der Karte.": "Drag nodes. Add more nodes on the right. Connect them from the node panel, or click success and failure on the card.",
  "Dieser Knoten": "This node",
  "Die Kante startet bei diesem Knoten.": "The edge starts at this node.",
  "Erfolg führt zu": "On success, go to",
  "Fehler führt zu": "On failure, go to",
  "Immer führt zu": "Always go to",
  "Kein Ziel": "No target",
  "Verbinden": "Connect",
  "Wähle einen Knoten, um ihn zu verbinden.": "Select a node to connect it.",
  "Lauf": "Run",
  "Zurück zum Workflow": "Back to the workflow",
  "Laufzeit": "Runtime",
  "LDAP-Gruppe": "LDAP group",
  "LDAP-Gruppe: {group}": "LDAP group: {group}",
  "LDAP- oder AD-Gruppe": "LDAP or AD group",
  "LDAP hat das Passwort abgelehnt.": "LDAP rejected the password.",
  "LDAP ist konfiguriert.": "LDAP is configured.",
  "LDAP ist unvollständig konfiguriert.": "LDAP is not fully configured.",
  "Leeres Passwort.": "Empty password.",
  "Lege zuerst eine Vorlage oder einen Workflow an.": "Create a template or a workflow first.",
  "Letzte Aufträge": "Recent jobs",
  "Limit": "Limit",
  "Limit enthält ein ungültiges Zeichen.": "Limit contains an invalid character.",
  "Lokale Konten.": "Local accounts.",
  "Lokales Konto": "Local account",
  "Löschen": "Delete",
  "Manuell": "Manual",
  "Mein Konto": "My account",
  "Mehrere": "Several",
  "Mitglieder": "Members",
  "Modulverzeichnis": "Module directory",
  "Name": "Name",
  "Name und E-Mail kommen aus dem Verzeichnis.": "Name and email come from the directory.",
  "Name der Quelle": "Source name",
  "Name fehlt oder ist zu lang.": "Name is missing or too long.",
  "NetBox-Adresse muss http oder https sein.": "The NetBox address must be http or https.",
  "NetBox-Antwort hat keine results.": "The NetBox response has no results.",
  "NetBox-Antwort ist kein JSON.": "The NetBox response is not JSON.",
  "NetBox-Antwort ist zu groß (Grenze 20 Seiten oder 2000 Hosts).": "The NetBox response is too large (limit 20 pages or 2000 hosts).",
  "NetBox-Antwort verweist auf einen anderen Host.": "The NetBox response points at another host.",
  "NetBox-Quelle ist unvollständig.": "The NetBox source is incomplete.",
  "NetBox-Token fehlt.": "The NetBox token is missing.",
  "NetBox-Token gehört zu einem anderen Team.": "The NetBox token belongs to another team.",
  "NetBox-Token ist ungültig.": "The NetBox token is invalid.",
  "NetBox-Zugang ist beschädigt.": "The NetBox credential is damaged.",
  "NetBox braucht token.": "NetBox needs a token.",
  "Neue Organisation mit dem Teamnamen": "New organization named after the team",
  "Neue Vorlage": "New template",
  "Neuer Workflow": "New workflow",
  "Neues Team": "New team",
  "Nicht gefunden.": "Not found.",
  "Nichts vorhanden.": "Nothing here.",
  "Noch keine Quelle.": "No source yet.",
  "ohne": "skip",
  "Ohne angehaktes Team dürfen alle Teams der Organisation das Projekt nutzen.": "With no team checked, every team in the organization can use the project.",
  "Ohne Execution Environment läuft die Vorlage auf dem Controller. Im lokalen Runner-Modus wird ein gesetztes Image ignoriert.": "Without an execution environment the template runs on the controller. In local runner mode a set image is ignored.",
  "OpenTofu": "OpenTofu",
  "OpenTofu braucht ein Modulverzeichnis.": "OpenTofu needs a module directory.",
  "OpenTofu init und plan oder apply. Provider liegen im Plugin-Cache.": "OpenTofu init, then plan or apply. Providers stay in the plugin cache.",
  "Operator": "Operator",
  "Organisation": "Organization",
  "Organisation: {name}": "Organization: {name}",
  "Organisation anlegen": "New organization",
  "Organisation fehlt.": "Organization is missing.",
  "Organisation verschieben": "Move organization",
  "Organisation {name}. Sichtbarkeit: {grant}.": "Organization {name}. Visibility: {grant}.",
  "Organisation · Team": "Organization · team",
  "Organisationen": "Organizations",
  "Organisationen suchen": "Search organizations",
  "Passwort": "Password",
  "Passwort braucht mindestens 8 Zeichen.": "Password needs at least 8 characters.",
  "Pfad": "Path",
  "Pfad fehlt.": "Path is missing.",
  "Pfad ist keine Datei.": "Path is not a file.",
  "Pfad verlässt das Projekt.": "Path leaves the project.",
  "Playbook aus dem Projekt": "Playbook from the project",
  "Playbook-Pfad": "Playbook path",
  "Playbooks lassen sich nicht lesen.": "Playbooks cannot be read.",
  "Plugin-Dateien werden nicht als Text übernommen. NetBox trägst du als eigene Quelle ein.": "Plugin files are not copied as text. Add NetBox as its own source.",
  "Plugin-Dateien werden nicht als Text übernommen. NetBox trägst du als Quelle ein, andere Plugins führt dieser Prototyp nicht aus.": "Plugin files are not copied as text. Add NetBox as a source. This prototype does not run other plugins.",
  "Privater Schlüssel": "Private key",
  "Projekt": "Project",
  "Projekt anlegen": "New project",
  "Projekt enthält zu viele Dateien für einen Scan.": "The project has too many files to scan.",
  "Projekt fehlt.": "Project is missing.",
  "Projekt lässt sich nicht lesen.": "The project cannot be read.",
  "Projekt löschen?": "Delete this project?",
  "Projektpfad fehlt.": "Project path is missing.",
  "Projektpfad ist kein Verzeichnis.": "The project path is not a directory.",
  "Projektquelle ist unvollständig.": "The project source is incomplete.",
  "Projekte": "Projects",
  "Projekte suchen": "Search projects",
  "Pull": "Pull",
  "Pull-Policy ist if_not_present, always oder never.": "Pull policy is if_not_present, always, or never.",
  "Quelle": "Source",
  "Quelle entfernen?": "Remove this source?",
  "Quelle hinzufügen": "Add source",
  "Quelle ist manuell, Projekt oder NetBox.": "A source is manual, project, or NetBox.",
  "Quelle ist path oder git.": "Source is path or git.",
  "Quelle speichern": "Save source",
  "Quellen": "Sources",
  "Ressourcen": "Resources",
  "Revision": "Revision",
  "Revision darf nur Buchstaben, Ziffern, Punkt, Schrägstrich, Unterstrich und Bindestrich enthalten.": "Revision may contain only letters, digits, dot, slash, underscore, and hyphen.",
  "Revision ist ungültig.": "Revision is invalid.",
  "Rolle": "Role",
  "Rolle ist viewer, operator oder owner.": "Role is viewer, operator, or owner.",
  "Rollen": "Roles",
  "Rollen im Cache": "Cached roles",
  "Runner": "Runner",
  "Seite": "Page",
  "Shell": "Shell",
  "Shell-Skript aus dem Projekt oder als Inline-Skript.": "Shell script from the project or as an inline script.",
  "Shell braucht einen Pfad oder spec.script.": "Shell needs a path or spec.script.",
  "Shell-Vorlage braucht ein Skript oder einen Einstiegspfad.": "A shell template needs a script or an entry path.",
  "Sichtbarkeit": "Visibility",
  "Sicherung": "Backup",
  "Sicherung herunterladen": "Download backup",
  "Sicherung einspielen": "Restore backup",
  "Sicherung eingespielt. Die vorherige Installation liegt als restore-previous.zip im Datenverzeichnis. Bitte neu anmelden.": "Backup restored. The previous installation is restore-previous.zip in the data directory. Sign in again.",
  "Koreograf speichert in SQLite. Die Sicherung enthält die Datenbank und den Schlüssel für die Zugänge. Cache, Git-Checkouts und Auftragsverzeichnisse bleiben außen vor.": "Koreograf stores data in SQLite. The backup contains the database and the key for credentials. Cache, git checkouts and job directories stay out.",
  "PostgreSQL ist für eine spätere Firmen-Installation vorgesehen. Dieselbe Sicherung ist die Grundlage für den Umzug.": "PostgreSQL is planned for a later company install. This same backup is the basis for that move.",
  "Das ersetzt Organisationen, Teams, Benutzer, Projekte, Inventare, Zugänge, Vorlagen, Workflows und Webhooks. Laufende Aufträge müssen vorher beendet sein. Danach ist eine neue Anmeldung nötig.": "This replaces organizations, teams, users, projects, inventories, credentials, templates, workflows and webhooks. Finish running jobs first. A new sign-in is required afterwards.",
  "Datei": "File",
  "Die aktuelle Installation wird ersetzt.": "The current installation will be replaced.",
  "Bestätige das Ersetzen mit ERSETZEN.": "Confirm the replacement with ERSETZEN.",
  "Es läuft noch ein Auftrag. Beende ihn vor dem Einspielen.": "A job is still running. Finish it before restoring.",
  "Die Sicherung ist leer oder zu groß.": "The backup is empty or too large.",
  "Die Sicherung lässt sich nicht lesen.": "The backup cannot be read.",
  "Die Sicherung hat ein unbekanntes Format.": "The backup has an unknown format.",
  "Die Sicherung enthält keine Koreograf-Datenbank.": "The backup does not contain a Koreograf database.",
  "Die Sicherung enthält keinen Schlüssel.": "The backup does not contain a key.",
  "Der Schlüssel in der Sicherung ist ungültig.": "The key in the backup is invalid.",
  "Die Datenbank in der Sicherung ist beschädigt.": "The database in the backup is damaged.",
  "Skript": "Script",
  "Skript, wenn kein Einstiegspfad gesetzt ist": "Script, when no entry path is set",
  "Skriptpfad": "Script path",
  "spec muss ein Objekt sein.": "spec must be an object.",
  "Speichern": "Save",
  "Sprache": "Language",
  "Starten": "Launch",
  "SSH braucht private_key.": "SSH needs private_key.",
  "Status": "Status",
  "Status ist firing, resolved oder any.": "Status is firing, resolved, or any.",
  "Sync fehlgeschlagen.": "Sync failed.",
  "Synchron": "Synced",
  "Synchronisieren": "Sync",
  "Tag eingeben": "Enter a tag",
  "Tag {name} entfernen": "Remove tag {name}",
  "Tags": "Tags",
  "Tags überspringen": "Skip tags",
  "Team": "Team",
  "Team anlegen": "New team",
  "Team fehlt.": "Team is missing.",
  "Team speichern": "Save team",
  "Teams": "Teams",
  "Teams werden geladen.": "Loading teams.",
  "Test": "Test",
  "Test-Alert": "Test alert",
  "Text": "Text",
  "Token": "Token",
  "Token nur für Operatoren": "Token is visible to operators",
  "Umgebung anlegen": "New environment",
  "Variablen": "Variables",
  "Vault braucht password.": "Vault needs a password.",
  "Vault-Passwort": "Vault password",
  "Version": "Version",
  "Virtuelle Maschinen": "Virtual machines",
  "Vorlage": "Template",
  "Vorlage bearbeiten": "Edit template",
  "Vorlage fehlt.": "Template is missing.",
  "Vorlage gehört zu einem anderen Team.": "The template belongs to another team.",
  "Vorlage löschen?": "Delete this template?",
  "Vorlagen": "Templates",
  "Vorlagen suchen": "Search templates",
  "Wann": "When",
  "Webhook": "Webhook",
  "Webhook anlegen": "New webhook",
  "Webhook löschen?": "Delete this webhook?",
  "Webhook-Eingänge": "Webhook deliveries",
  "Webhooks": "Webhooks",
  "Webhooks suchen": "Search webhooks",
  "Werkzeug": "Tool",
  "Werkzeug ist ansible, opentofu oder shell.": "Tool is ansible, opentofu, or shell.",
  "Werkzeuge": "Tools",
  "Workflow": "Workflow",
  "Workflow fehlt.": "Workflow is missing.",
  "Workflow gehört zu einem anderen Team.": "The workflow belongs to another team.",
  "Workflow löschen?": "Delete this workflow?",
  "Workflow-Graph ist ungültig.": "The workflow graph is invalid.",
  "Workflows": "Workflows",
  "Workflows suchen": "Search workflows",
  "X-Team-Id fehlt.": "X-Team-Id is missing.",
  "Zeit": "Time",
  "Zertifikat prüfen": "Verify certificate",
  "Ziel": "Target",
  "Zugang": "Credential",
  "Zugang anlegen": "New credential",
  "Zugang fehlt.": "Credential is missing.",
  "Zugang gehört zu einem anderen Team.": "The credential belongs to another team.",
  "Zugang ist doppelt.": "Credential is listed twice.",
  "Zugänge": "Credentials",
  "Zugänge suchen": "Search credentials",
  "Zugangsdaten gehören zu einem anderen Team.": "The credential belongs to another team.",
  "Zugangsdaten lassen sich nicht lesen.": "Credentials cannot be read.",
  "Zugangsdaten sind kein JSON.": "Credentials are not JSON.",
  "Zu viele Zugänge.": "Too many credentials.",
  "Nur ein Vault-Zugang darf ohne Vault-Id sein.": "Only one vault credential may omit the vault id.",
  "Pro Art ein Zugang. Vault-Zugänge wählst du über Auswählen. Sie stehen hier als Pillen.": "One credential of each kind. Choose vault credentials with Select. They show up here as pills.",
  "Auswählen": "Select",
  "Übernehmen": "Apply",
  "Vaults suchen": "Search vaults",
  "Keine Vault-Zugänge im Team.": "No vault credentials in this team.",
  "Vault-Zugang {name} entfernen": "Remove vault credential {name}",
  "Pro Art ist nur ein Zugang erlaubt.": "Only one credential of each kind is allowed.",
  "Zugewiesene Teams müssen zur Organisation des Projekts gehören.": "Assigned teams must belong to the project organization.",
  "Zuweisung": "Assignment",
  "Zuweisung speichern": "Save assignment",
  "Zum Team": "Open team",
  "Zusammenfassung": "Summary",
  "Zusammenführung": "Convergence",
  "{count} Hosts": "{count} hosts",
  "{count} Knoten": "{count} nodes",
  "{count} Teams": "{count} teams",
  "{files} wird hier nicht ausgeführt. NetBox legst du als eigene Quelle an.": "{files} is not executed here. Add NetBox as its own source.",
  "Abgebrochen": "Canceled",
  "Angenommen": "Accepted",
  "Änderungen sind Operatoren vorbehalten.": "Changes are reserved for operators.",
  "Anderer Pfad": "Other path",
  "Anmeldung abgelehnt.": "Sign-in was rejected.",
  "Anmeldung erforderlich.": "Sign-in is required.",
  "ansible-playbook. Collections und Rollen bleiben im Team-Cache.": "ansible-playbook. Collections and roles stay in the team cache.",
  "Aufträge suchen": "Search jobs",
  "Benutzer anlegen": "New user",
  "Branch, Tag oder Commit": "Branch, tag, or commit",
  "CN oder DN, zum Beispiel Plattform": "CN or DN, for example Platform",
  "Entfernen": "Remove",
  "Erfolgreich": "Successful",
  "Fehlgeschlagen": "Failed",
  "Filter „{line}“ braucht Schlüssel=Wert.": "Filter “{line}” needs key=value.",
  "Filter „{line}“ ist ungültig.": "Filter “{line}” is invalid.",
  "Gruppen": "Groups",
  "Ignoriert": "Ignored",
  "Läuft": "Running",
  "Mitglied hinzufügen": "Add member",
  "NetBox ist nicht erreichbar: {err}": "NetBox is not reachable: {err}",
  "Quelle: {source}": "Source: {source}",
  "Revision {revision}": "Revision {revision}",
  "Übersprungen": "Skipped",
  "Umgebungen suchen": "Search environments",
  "Wartend": "Pending",
  "Übersicht": "Overview",
  "+ Benutzer": "+ User",
  "+ Inventar": "+ Inventory",
  "+ Manuell": "+ Manual",
  "+ Mitglied": "+ Member",
  "+ NetBox": "+ NetBox",
  "+ Organisation": "+ Organization",
  "+ Projekt": "+ Project",
  "+ Team": "+ Team",
  "+ Umgebung": "+ Environment",
  "+ Vorlage": "+ Template",
  "+ Webhook": "+ Webhook",
  "+ Workflow": "+ Workflow",
  "+ Zugang": "+ Credential",
  "Adresse": "Address",
  "Alertname fehlt.": "Alert name is missing.",
  "Aktion ist plan oder apply.": "Action is plan or apply.",
  "Ansible braucht ein Playbook als Einstieg.": "Ansible needs a playbook as the entry.",
  "Aufträge, Workflows und Alarme für mehrere Teams. Lokales Konto oder LDAP.": "Jobs, workflows, and alerts for several teams. Local account or LDAP.",
  "Hinweis": "Notice",
  "alle Vorgänger": "all predecessors",
  "ein Vorgänger": "one predecessor",
};

function esc(value) {
  return String(value ?? "").replace(/[&<>"']/g, (ch) => ({
    "&": "&amp;",
    "<": "&lt;",
    ">": "&gt;",
    '"': "&quot;",
    "'": "&#39;",
  }[ch]));
}

function t(text, vars) {
  let out = lang === "en" && Object.prototype.hasOwnProperty.call(EN, text) ? EN[text] : text;
  if (vars) {
    Object.entries(vars).forEach(([key, value]) => {
      out = out.replaceAll(`{${key}}`, String(value ?? ""));
    });
  }
  return out;
}

function h(text, vars) {
  return esc(t(text, vars));
}

function setLang(next) {
  lang = languageId(next);
  localStorage.setItem("koreograf.lang", lang);
  document.documentElement.lang = lang;
  if (!me) renderLogin("");
  else route();
}

function flagMarkup(country) {
  const svg = FLAG_SVG[country];
  if (!svg) return `<span class="flag flag-fallback" aria-hidden="true">${esc(String(country || "").slice(0, 2).toUpperCase())}</span>`;
  return `<img class="flag" alt="" src="data:image/svg+xml,${encodeURIComponent(svg)}">`;
}

function langOption(item) {
  const selected = item.id === lang ? "true" : "false";
  return `<li role="option" tabindex="-1" lang="${esc(item.id)}" data-lang-option="${esc(item.id)}" aria-selected="${selected}">${flagMarkup(item.country)}<span class="lang-code">${esc(item.code)}</span><span class="lang-name">${esc(item.name)}</span></li>`;
}

function langSwitch() {
  const current = LANGUAGES.find((item) => item.id === lang) || LANGUAGES[0];
  const options = LANGUAGES.map(langOption).join("");
  return `<div class="lang-switch" data-lang-select>
    <button class="lang-button" type="button" data-lang-button aria-haspopup="listbox" aria-expanded="false" aria-controls="lang-menu" aria-label="${h("Sprache")}">${flagMarkup(current.country)}<span class="lang-code">${esc(current.code)}</span><span class="lang-caret" aria-hidden="true"></span></button>
    <ul class="lang-menu" id="lang-menu" role="listbox" hidden>${options}</ul>
  </div>`;
}

function closeLangMenus() {
  document.querySelectorAll(".lang-menu").forEach((menu) => {
    menu.hidden = true;
    const button = menu.parentElement && menu.parentElement.querySelector("[data-lang-button]");
    if (button) button.setAttribute("aria-expanded", "false");
  });
}

function bindLangSwitch() {
  document.querySelectorAll(".lang-switch").forEach((root) => {
    const button = root.querySelector("[data-lang-button]");
    const menu = root.querySelector(".lang-menu");
    if (!button || !menu) return;
    const options = () => [...menu.querySelectorAll("[data-lang-option]")];
    const openMenu = () => {
      closeLangMenus();
      menu.hidden = false;
      button.setAttribute("aria-expanded", "true");
      const current = options().find((item) => item.dataset.langOption === lang) || options()[0];
      if (current) current.focus();
    };
    button.addEventListener("click", (event) => {
      event.stopPropagation();
      if (menu.hidden) openMenu();
      else closeLangMenus();
    });
    button.addEventListener("keydown", (event) => {
      if (event.key === "ArrowDown" || event.key === "ArrowUp" || event.key === "Enter" || event.key === " ") {
        event.preventDefault();
        openMenu();
      }
    });
    options().forEach((option, index, list) => {
      option.addEventListener("click", () => {
        if (option.dataset.langOption !== lang) setLang(option.dataset.langOption);
        else closeLangMenus();
      });
      option.addEventListener("keydown", (event) => {
        if (event.key === "ArrowDown") {
          event.preventDefault();
          list[(index + 1) % list.length].focus();
        } else if (event.key === "ArrowUp") {
          event.preventDefault();
          list[(index - 1 + list.length) % list.length].focus();
        } else if (event.key === "Home") {
          event.preventDefault();
          list[0].focus();
        } else if (event.key === "End") {
          event.preventDefault();
          list[list.length - 1].focus();
        } else if (event.key === "Enter" || event.key === " ") {
          event.preventDefault();
          if (option.dataset.langOption !== lang) setLang(option.dataset.langOption);
          else closeLangMenus();
        } else if (event.key === "Escape") {
          event.preventDefault();
          closeLangMenus();
          button.focus();
        }
      });
    });
  });
  if (!bindLangSwitch.outside) {
    document.addEventListener("click", (event) => {
      if (!event.target.closest(".lang-switch")) closeLangMenus();
    });
    bindLangSwitch.outside = true;
  }
}

function addLink(href, label) {
  return `<a class="btn pill primary" href="${href}">${h(label)}</a>`;
}

function deny(title, min) {
  const text = min === "admin"
    ? "Dafür brauchst du ein Administratorkonto."
    : min === "owner"
      ? "Dafür brauchst du die Rolle Besitzer."
      : "Dafür brauchst du die Rolle Operator.";
  shell(t(title), `<div class="card">${h(text)}</div>`);
}

function roleOf() {
  if (!me) return "viewer";
  if (me.is_admin) return "owner";
  const team = (me.teams || []).find((item) => item.id === teamId);
  return team ? team.role : "viewer";
}

function rank(role) {
  return { viewer: 1, operator: 2, owner: 3 }[role] || 0;
}

function can(min) {
  return rank(roleOf()) >= rank(min);
}

function roleLabel(role) {
  return {
    viewer: t("Betrachter"),
    operator: t("Operator"),
    owner: t("Besitzer"),
  }[role] || role;
}

function roleOptions(selected) {
  const labels = { viewer: "Betrachter", operator: "Operator", owner: "Besitzer" };
  return ["viewer", "operator", "owner"].map((role) =>
    `<option value="${role}"${role === selected ? " selected" : ""}>${h(labels[role])}</option>`
  ).join("");
}

function sourceLabel(source) {
  if (source === "ldap") return "LDAP";
  if (source === "manual") return t("Manuell");
  if (source === "local") return t("Lokal");
  return source || "";
}

function authSourceLabel(source) {
  return sourceLabel(source === "local" ? "local" : source);
}

function roleLegend() {
  return `<ul class="role-legend">
    <li><strong>${h("Betrachter")}</strong> ${h("Betrachter sieht Projekte, Inventare, Vorlagen, Aufträge und Workflows.")}</li>
    <li><strong>${h("Operator")}</strong> ${h("Operator startet Aufträge und pflegt Projekte, Inventare, Vorlagen, Workflows und Webhooks.")}</li>
    <li><strong>${h("Besitzer")}</strong> ${h("Besitzer verwaltet zusätzlich Zugänge, Umgebungen und Mitglieder.")}</li>
    <li><strong>${h("Administrator")}</strong> ${h("Administrator verwaltet Organisationen, Teams, Benutzer und die Sicherung. Die Teamrolle wird dabei nicht geprüft.")}</li>
  </ul>`;
}

function statusLabel(status) {
  return {
    pending: t("Wartend"),
    running: t("Läuft"),
    successful: t("Erfolgreich"),
    failed: t("Fehlgeschlagen"),
    canceled: t("Abgebrochen"),
    skipped: t("Übersprungen"),
    accepted: t("Angenommen"),
    ignored: t("Ignoriert"),
  }[status] || status;
}

function pill(status) {
  return `<span class="pill ${esc(status)}">${esc(statusLabel(status))}</span>`;
}

function clock(value) {
  if (!value) return "—";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return String(value);
  return date.toLocaleTimeString(lang === "en" ? "en-GB" : "de-DE", {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hourCycle: "h23",
  });
}

function spanText(start, end, phase) {
  if (!start || phase === "pending" || phase === "skipped") return "—";
  const from = Date.parse(start);
  if (Number.isNaN(from)) return "—";
  const to = end ? Date.parse(end) : phase === "running" ? Date.now() : NaN;
  if (Number.isNaN(to)) return "—";
  const secs = Math.max(0, Math.round((to - from) / 1000));
  const mins = Math.floor(secs / 60);
  const rest = secs % 60;
  if (mins <= 0) return `${secs}s`;
  return `${mins}m ${String(rest).padStart(2, "0")}s`;
}

function bytes(value) {
  const n = Number(value) || 0;
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KiB`;
  return `${(n / 1024 / 1024).toFixed(1)} MiB`;
}

function closeLive() {
  generation += 1;
  if (source) {
    source.close();
    source = null;
  }
  if (pollTimer) {
    clearInterval(pollTimer);
    pollTimer = null;
  }
  if (runKey) {
    document.removeEventListener("keydown", runKey);
    runKey = null;
  }
}

async function api(method, path, body) {
  const headers = {};
  if (teamId) headers["X-Team-Id"] = teamId;
  const init = { method, headers, credentials: "same-origin" };
  if (body !== undefined) {
    headers["Content-Type"] = "application/json";
    init.body = JSON.stringify(body);
  }
  const response = await fetch(path, init);
  const text = await response.text();
  let data = null;
  if (text) {
    try {
      data = JSON.parse(text);
    } catch {
      data = { error: text };
    }
  }
  if (!response.ok) {
    const error = new Error((data && data.error) || "Die Anfrage ist fehlgeschlagen.");
    error.status = response.status;
    throw error;
  }
  return data;
}

function handle(error) {
  if (error && error.status === 401) {
    me = null;
    renderLogin("Die Sitzung ist abgelaufen.");
    return;
  }
  if (!document.querySelector(".main")) shell(t("Hinweis"), "");
  const main = document.querySelector(".main");
  main.insertAdjacentHTML("afterbegin", `<div class="banner bad">${h(error.message || "Fehler")}</div>`);
}

function go(hash) {
  if ((location.hash || "#/") === hash) route();
  else location.hash = hash;
}

function navActive(here, href) {
  if (href === "#/") return here === "#/" || here === "";
  if (href === "#/runtime" && here.startsWith("#/environments")) return true;
  return here === href || here.startsWith(`${href}/`);
}

function shell(title, body, actions) {
  const here = (location.hash || "#/").split("?")[0];
  const groups = [
    [t("Automation"), [
      ["#/", t("Übersicht")],
      ["#/templates", t("Vorlagen")],
      ["#/jobs", t("Aufträge")],
      ["#/workflows", t("Workflows")],
      ["#/hooks", t("Webhooks")],
    ]],
    [t("Ressourcen"), [
      ["#/projects", t("Projekte")],
      ["#/inventories", t("Inventare")],
      ["#/credentials", t("Zugänge")],
      ["#/runtime", t("Laufzeit")],
    ]],
    [lang === "en" ? "Access" : "Zugang", [
      ["#/organizations", t("Organisationen")],
      ["#/team", t("Team")],
      ...(me.is_admin ? [["#/users", t("Benutzerkonten")], ["#/backup", t("Sicherung")]] : []),
    ]],
  ];
  const nav = groups.map(([label, links]) => `
    <div class="nav-group">
      <div class="nav-label">${label}</div>
      ${links.map(([href, text]) => `<a href="${href}" class="${navActive(here, href) ? "active" : ""}">${text}</a>`).join("")}
    </div>`).join("");
  const options = (me.teams || []).map((team) => {
    const label = team.organization_name ? `${team.organization_name} · ${team.name}` : team.name;
    return `<option value="${esc(team.id)}"${team.id === teamId ? " selected" : ""}>${esc(label)}</option>`;
  }).join("");
  app.innerHTML = `
    <div class="shell">
      <aside class="side">
        <div class="brand-lockup"><span class="mark" aria-hidden="true"></span><p class="word">Koreograf</p></div>
        <label class="field"><span class="small">${h("Organisation · Team")}</span><select id="team-select">${options || `<option value="">${h("Kein Team")}</option>`}</select></label>
        <nav class="nav">${nav}</nav>
        <div class="side-foot">
          <a class="account-link" href="#/account">${esc(me.display_name)} · ${esc(roleLabel(roleOf()))}${me.auth_source === "ldap" ? " · LDAP" : ""}</a>
          ${langSwitch()}
          <button class="btn ghost" id="logout" type="button">${h("Abmelden")}</button>
        </div>
      </aside>
      <main class="main">
        <div class="top"><div><h1>${esc(title)}</h1></div><div class="row">${actions || ""}</div></div>
        ${body}
      </main>
    </div>`;
  const select = document.querySelector("#team-select");
  select.addEventListener("change", () => {
    teamId = select.value;
    localStorage.setItem("koreograf.team", teamId);
    route();
  });
  bindLangSwitch();
  document.querySelector("#logout").addEventListener("click", async () => {
    try { await api("POST", "/api/logout"); } catch { /* session ended locally */ }
    me = null;
    go("#/login");
  });
}

function needTeam() {
  if (teamId) return true;
  shell(t("Team"), `<div class="card"><p>${h("Diesem Konto ist kein Team zugeordnet. Ein Administrator legt eines unter Team an.")}</p><p><a href="#/team">${h("Zum Team")}</a></p></div>`);
  return false;
}

function table(headers, rows) {
  const body = rows.length
    ? rows.join("")
    : `<tr><td class="muted" colspan="${headers.length}">${h("Nichts vorhanden.")}</td></tr>`;
  return `<table class="table"><thead><tr>${headers.map((item) => `<th>${esc(item)}</th>`).join("")}</tr></thead><tbody>${body}</tbody></table>`;
}

function jsonObject(text, label) {
  let value;
  try {
    value = JSON.parse(text.trim() || "{}");
  } catch {
    throw new Error(`${label} ist kein gültiges JSON.`);
  }
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new Error(`${label} muss ein JSON-Objekt sein.`);
  }
  return value;
}

function renderLogin(message, tone) {
  const kind = tone === "good" ? "good" : "bad";
  app.innerHTML = `
    <div class="login-wrap">
      <form class="login-card" id="login-form">
        <div class="spread">
          <div class="brand-lockup"><span class="mark" aria-hidden="true"></span><h1 class="word">Koreograf</h1></div>
          ${langSwitch()}
        </div>
        <p class="lede">${h("Aufträge, Workflows und Alarme für mehrere Teams. Lokales Konto oder LDAP.")}</p>
        ${message ? `<div class="banner ${kind}">${h(message)}</div>` : ""}
        <div class="form">
          <label class="field"><span>${h("Benutzername")}</span><input name="username" autocomplete="username" required></label>
          <label class="field"><span>${h("Passwort")}</span><input name="password" type="password" autocomplete="current-password" required></label>
          <button class="btn primary" type="submit">${h("Anmelden")}</button>
        </div>
      </form>
    </div>`;
  bindLangSwitch();
  document.querySelector("#login-form").addEventListener("submit", async (event) => {
    event.preventDefault();
    const form = event.currentTarget;
    const button = form.querySelector("button[type=submit]");
    button.disabled = true;
    try {
      me = await api("POST", "/api/login", {
        username: form.username.value,
        password: form.password.value,
      });
      const known = (me.teams || []).some((team) => team.id === teamId);
      if (!known) teamId = (me.teams[0] && me.teams[0].id) || "";
      localStorage.setItem("koreograf.team", teamId);
      go("#/");
    } catch (error) {
      renderLogin(error.message || "Anmeldung abgelehnt.");
    }
  });
}

async function pageDashboard() {
  if (!needTeam()) return;
  const data = await api("GET", "/api/overview");
  const jobs = (data.jobs || []).map((job) =>
    `<tr><td><a href="#/jobs/${esc(job.id)}">${esc(job.name)}</a></td><td class="tool">${esc(job.tool)}</td><td>${pill(job.status)}</td><td class="small muted">${esc(job.created_at)}</td></tr>`
  );
  const deliveries = (data.deliveries || []).map((item) =>
    `<tr><td>${esc(item.hook_name)}</td><td>${pill(item.status)}</td><td>${esc(item.summary)}</td></tr>`
  );
  shell(t("Übersicht"), `
    <div class="grid-3">
      <article class="card stat"><span class="small muted">${h("Abhängigkeits-Cache")}</span><strong>${esc(bytes(data.cache_bytes))}</strong><p class="small muted">${h("Collections, Rollen und OpenTofu-Provider bleiben zwischen den Läufen liegen.")}</p></article>
      <article class="card stat"><span class="small muted">${h("Workflow")}</span><strong>${data.workflow_id ? `<a href="#/workflows/${esc(data.workflow_id)}">${esc(data.workflow_name)}</a>` : "—"}</strong><p class="small muted">${h("Erfolgskanten und Fehlerkanten starten die nächsten Vorlagen.")}</p></article>
      <article class="card stat"><span class="small muted">${h("Webhook")}</span><strong>${data.hook_id ? esc(data.hook_name) : "—"}</strong><p class="small muted">${data.ldap ? h("LDAP ist konfiguriert.") : h("Lokale Konten.")} ${h("Runner")}: ${esc(data.runner_mode)}${data.runtime ? `, ${esc(data.runtime)}` : ""}.</p></article>
    </div>
    <div class="stack" style="margin-top:14px">
      <section class="card">
        <div class="spread"><h2>${h("Letzte Aufträge")}</h2><a href="#/jobs">${h("Alle")}</a></div>
        ${table([t("Name"), t("Werkzeug"), t("Status"), t("Angelegt")], jobs)}
      </section>
      <section class="card">
        <div class="spread">
          <h2>${h("Webhook-Eingänge")}</h2>
          ${data.hook_id && can("operator") ? `<button class="btn primary" id="test-alert" type="button">${h("Test-Alert")}</button>` : ""}
        </div>
        ${table([t("Hook"), t("Status"), t("Zusammenfassung")], deliveries)}
      </section>
    </div>`);
  const test = document.querySelector("#test-alert");
  if (test) {
    test.addEventListener("click", async () => {
      test.disabled = true;
      try {
        const result = await api("POST", `/api/hooks/${encodeURIComponent(data.hook_id)}/test`);
        if (result.workflow_job_id) go(`#/runs/${result.workflow_job_id}`);
        else if (result.job_id) go(`#/jobs/${result.job_id}`);
        else route();
      } catch (error) {
        test.disabled = false;
        handle(error);
      }
    });
  }
}

function selectOptions(items, selected, allowEmpty, label) {
  const empty = allowEmpty ? `<option value="">—</option>` : "";
  const options = items.map((item) =>
    `<option value="${esc(item.id)}"${item.id === selected ? " selected" : ""}>${esc(label(item))}</option>`
  ).join("");
  return empty + options;
}

function verbosityLabel(level) {
  return [
    "0 (Normal)",
    "1 (Ausführlich)",
    "2 (Ausführlicher)",
    "3 (Debug)",
    "4 (Verbindungs-Debug)",
  ][Number(level)] || "";
}

function parseTagList(value) {
  return String(value || "").split(",").map((part) => part.trim()).filter(Boolean);
}

function validTag(part) {
  return part.length > 0
    && part.length <= 80
    && !part.startsWith("-")
    && /^[A-Za-z0-9_.:-]+$/.test(part);
}

function splitDraft(raw, flush) {
  const parts = String(raw || "").split(/[,\s]+/);
  const trailing = /[,\s]$/.test(raw);
  if (flush || trailing) return { tokens: parts.filter(Boolean), rest: "" };
  return { tokens: parts.slice(0, -1).filter(Boolean), rest: parts.at(-1) || "" };
}

function readChips(box) {
  return [...box.querySelectorAll(".chip-text")].map((node) => node.textContent);
}

function renderChips(box, tokens) {
  const entry = box.querySelector(".chip-entry");
  box.querySelectorAll(".chip").forEach((node) => node.remove());
  const html = tokens.map((token) => `<span class="chip"><span class="chip-text">${esc(token)}</span><button class="chip-remove" type="button" aria-label="${h("Tag {name} entfernen", { name: token })}">×</button></span>`).join("");
  entry.insertAdjacentHTML("beforebegin", html);
  entry.placeholder = tokens.length ? "" : t("Tag eingeben");
  const hidden = box.parentElement.querySelector(`input[type="hidden"][name="${CSS.escape(box.dataset.chips)}"]`);
  if (hidden) hidden.value = tokens.join(",");
  box.querySelectorAll(".chip-remove").forEach((button) => {
    button.addEventListener("mousedown", (event) => event.preventDefault());
    button.addEventListener("click", () => {
      const name = button.parentElement.querySelector(".chip-text").textContent;
      renderChips(box, readChips(box).filter((item) => item !== name));
      const error = box.parentElement.querySelector(".chip-error");
      if (error) error.textContent = "";
      entry.focus();
    });
  });
}

function addTokens(box, tokens) {
  const error = box.parentElement.querySelector(".chip-error");
  const chips = readChips(box);
  for (let index = 0; index < tokens.length; index += 1) {
    const token = tokens[index];
    if (chips.includes(token)) continue;
    const tooLong = [...chips, token].join(",").length > 300;
    if (!validTag(token) || tooLong) {
      if (error) {
        error.textContent = tooLong
          ? t("Die Tags sind zu lang.")
          : t("Der Tag „{part}“ ist ungültig. Erlaubt sind Buchstaben, Ziffern und _ . : -.", { part: token });
      }
      renderChips(box, chips);
      return { ok: false, keep: tokens.slice(index) };
    }
    chips.push(token);
  }
  if (error) error.textContent = "";
  renderChips(box, chips);
  return { ok: true, keep: [] };
}

function takeDraft(box, flush) {
  const entry = box.querySelector(".chip-entry");
  const { tokens, rest } = splitDraft(entry.value, flush);
  if (!tokens.length) {
    entry.value = rest;
    return true;
  }
  const result = addTokens(box, tokens);
  const leftover = result.ok ? rest : [...result.keep, rest].filter(Boolean).join(" ");
  entry.value = leftover;
  return result.ok;
}

function bindChips(form) {
  form.querySelectorAll("[data-chips]").forEach((box) => {
    const hidden = box.parentElement.querySelector(`input[type="hidden"][name="${CSS.escape(box.dataset.chips)}"]`);
    renderChips(box, parseTagList(hidden ? hidden.value : ""));
    const entry = box.querySelector(".chip-entry");
    const error = box.parentElement.querySelector(".chip-error");
    entry.addEventListener("keydown", (event) => {
      if (event.key === "Enter" || event.key === "," || event.key === " ") {
        event.preventDefault();
        takeDraft(box, true);
      } else if (event.key === "Backspace" && entry.value === "") {
        const tokens = readChips(box);
        if (!tokens.length) return;
        event.preventDefault();
        renderChips(box, tokens.slice(0, -1));
        if (error) error.textContent = "";
      }
    });
    entry.addEventListener("input", () => {
      if (error) error.textContent = "";
      if (/[,\s]/.test(entry.value)) takeDraft(box, false);
    });
    entry.addEventListener("blur", () => takeDraft(box, true));
    box.addEventListener("click", (event) => {
      if (event.target === box) entry.focus();
    });
  });
}

function flushChips(form) {
  let ok = true;
  form.querySelectorAll("[data-chips]").forEach((box) => {
    if (box.closest("[hidden]")) return;
    if (!takeDraft(box, true)) ok = false;
  });
  return ok;
}

function chipBox(name, label, value) {
  return `<label class="field chips-field"><span>${h(label)}</span>
    <div class="chips" data-chips="${esc(name)}">
      <input class="chip-entry" type="text" autocomplete="off" aria-label="${h(label)}">
    </div>
    <input type="hidden" name="${esc(name)}" value="${esc(parseTagList(value).join(","))}">
    <span class="chip-error" aria-live="polite"></span>
  </label>`;
}

function chipReadout(value) {
  const tokens = parseTagList(value);
  if (!tokens.length) return "";
  return `<span class="chips readonly">${tokens.map((token) => `<span class="chip"><span class="chip-text">${esc(token)}</span></span>`).join("")}</span>`;
}

function templateSpec(form) {
  if (form.tool.value === "ansible") {
    const spec = { job_type: form.job_type.value === "check" ? "check" : "run" };
    const tags = form.tags.value.trim();
    const skip = form.skip_tags.value.trim();
    const limit = form.limit.value.trim();
    const forks = form.forks.value.trim();
    if (tags) spec.tags = tags;
    if (skip) spec.skip_tags = skip;
    if (limit) spec.limit = limit;
    if (form.diff.checked) spec.diff = true;
    const verbosity = Number(form.verbosity.value);
    if (verbosity) spec.verbosity = verbosity;
    if (forks) {
      const parsed = Number(forks);
      if (!Number.isInteger(parsed)) throw new Error("Forks ist eine ganze Zahl.");
      spec.forks = parsed;
    }
    return spec;
  }
  if (form.tool.value === "opentofu") {
    return { action: form.action.value === "apply" ? "apply" : "plan" };
  }
  const script = form.script.value;
  return script.trim() ? { script } : {};
}

function templateOptionLine(item) {
  const spec = item.spec || {};
  if (item.tool === "ansible") {
    const bits = [spec.job_type === "check" ? t("Check") : t("Lauf")];
    if (spec.verbosity) bits.push(t(verbosityLabel(spec.verbosity)));
    if (spec.tags) bits.push(`${t("Tags")} ${spec.tags}`);
    if (spec.skip_tags) bits.push(`${t("ohne")} ${spec.skip_tags}`);
    if (spec.limit) bits.push(`${t("Limit")} ${spec.limit}`);
    if (spec.diff) bits.push(t("Diff"));
    return bits.join(" · ");
  }
  if (item.tool === "opentofu") return spec.action === "apply" ? "apply" : "plan";
  return item.entrypoint || t("Skript");
}

async function pageTemplates() {
  if (!needTeam()) return;
  const items = await api("GET", "/api/templates");
  shell(t("Vorlagen"), `
    <section class="card">
      <div class="spread">
        <input class="search" id="template-search" placeholder="${h("Vorlagen suchen")}" autocomplete="off">
        <div class="choice-row" id="template-filters">
          <button class="btn pill primary" type="button" data-tool="all">${h("Alle")}</button>
          <button class="btn pill" type="button" data-tool="ansible">Ansible</button>
          <button class="btn pill" type="button" data-tool="opentofu">OpenTofu</button>
          <button class="btn pill" type="button" data-tool="shell">Shell</button>
        </div>
      </div>
      <div id="template-table"></div>
    </section>`,
    can("operator") ? addLink("#/templates/new", "+ Vorlage") : "");
  let tool = "all";
  let query = "";
  const draw = () => {
    const needle = query.trim().toLowerCase();
    const shown = items.filter((item) => {
      if (tool !== "all" && item.tool !== tool) return false;
      if (!needle) return true;
      const spec = item.spec || {};
      return [item.name, item.description, item.tool, item.entrypoint, spec.tags, spec.skip_tags, spec.limit]
        .join(" ")
        .toLowerCase()
        .includes(needle);
    });
    const rows = shown.map((item) => `
      <tr>
        <td><a href="#/templates/${esc(item.id)}">${esc(item.name)}</a><div class="small muted">${esc(item.description)}</div></td>
        <td class="tool">${esc(item.tool)}</td>
        <td class="mono">${esc(item.entrypoint || "—")}</td>
        <td class="small">${esc(templateOptionLine(item))}</td>
        <td>${can("operator") ? `<button class="btn primary" type="button" data-launch="${esc(item.id)}">${h("Starten")}</button>` : ""}</td>
      </tr>`);
    document.querySelector("#template-table").innerHTML = table([t("Name"), t("Werkzeug"), t("Einstieg"), t("Lauf"), ""], rows);
    document.querySelectorAll("[data-launch]").forEach((button) => {
      button.addEventListener("click", async () => {
        button.disabled = true;
        try {
          const job = await api("POST", `/api/templates/${encodeURIComponent(button.dataset.launch)}/launch`, { extra_vars: {} });
          go(`#/jobs/${job.id}`);
        } catch (error) {
          button.disabled = false;
          handle(error);
        }
      });
    });
  };
  document.querySelector("#template-search").addEventListener("input", (event) => {
    query = event.target.value;
    draw();
  });
  document.querySelectorAll("#template-filters button").forEach((button) => {
    button.addEventListener("click", () => {
      tool = button.dataset.tool;
      document.querySelectorAll("#template-filters button").forEach((item) => {
        item.classList.toggle("primary", item === button);
      });
      draw();
    });
  });
  draw();
}

async function pageTemplateForm(id) {
  if (!needTeam()) return;
  const [projects, inventories, credentials, environments, templates] = await Promise.all([
    api("GET", "/api/projects"),
    api("GET", "/api/inventories"),
    api("GET", "/api/credentials"),
    api("GET", "/api/environments"),
    api("GET", "/api/templates"),
  ]);
  const existing = id ? templates.find((item) => item.id === id) : null;
  if (id && !existing) {
    shell(t("Vorlage"), `<div class="card">${h("Diese Vorlage gibt es nicht.")}</div>`);
    return;
  }
  const spec = (existing && existing.spec) || {};
  const item = existing || {
    name: "", description: "", tool: "ansible", project_id: "", inventory_id: "",
    credential_id: "", credential_ids: [], environment_id: "", entrypoint: "", extra_vars: {}, spec: {},
  };
  const verbosity = Number(spec.verbosity) || 0;
  shell(id ? t("Vorlage bearbeiten") : t("Neue Vorlage"), `
    <form class="card form" id="template-form">
      <div class="form-section">
        <h2>${h("Allgemein")}</h2>
        <label class="field"><span>${h("Name")}</span><input name="name" required value="${esc(item.name)}"></label>
        <label class="field"><span>${h("Beschreibung")}</span><input name="description" value="${esc(item.description)}"></label>
        <div class="field-row">
          <label class="field"><span>${h("Werkzeug")}</span>
            <select class="fit" name="tool">
              <option value="ansible"${item.tool === "ansible" ? " selected" : ""}>Ansible</option>
              <option value="opentofu"${item.tool === "opentofu" ? " selected" : ""}>OpenTofu</option>
              <option value="shell"${item.tool === "shell" ? " selected" : ""}>Shell</option>
            </select>
          </label>
        </div>
      </div>
      <div class="form-section">
        <h2>${h("Ziel")}</h2>
        <div class="field-grid">
          <label class="field"><span>${h("Projekt")}</span><select name="project_id">${selectOptions(projects, item.project_id, true, (row) => row.name)}</select></label>
          <label class="field" id="inventory-field"${item.tool === "ansible" ? "" : " hidden"}><span>${h("Inventar")}</span><select name="inventory_id">${selectOptions(inventories, item.inventory_id, true, (row) => row.name)}</select></label>
          <label class="field"><span>${h("Execution Environment")}</span><select name="environment_id">${selectOptions(environments, item.environment_id, true, (row) => row.name)}</select></label>
        </div>
        ${templateCredentialFields(credentials, selectedCredentialIds(item))}
        <label class="field" id="playbook-field" hidden><span>${h("Playbook aus dem Projekt")}</span><select name="playbook"><option value="">${h("Anderer Pfad")}</option></select></label>
        <p class="small muted" id="entrypoint-note"></p>
        <label class="field"><span id="entrypoint-label">${h("Einstiegspfad")}</span><input name="entrypoint" value="${esc(item.entrypoint)}" placeholder="site.yml"></label>
        <p class="small muted">${h("Ohne Execution Environment läuft die Vorlage auf dem Controller. Im lokalen Runner-Modus wird ein gesetztes Image ignoriert.")}</p>
      </div>
      <div class="form-section" id="ansible-options"${item.tool === "ansible" ? "" : " hidden"}>
        <h2>Ansible</h2>
        <div class="field-row">
          <label class="field"><span>${h("Art")}</span>
            <select class="fit" name="job_type">
              <option value="run"${spec.job_type === "check" ? "" : " selected"}>${h("Lauf")}</option>
              <option value="check"${spec.job_type === "check" ? " selected" : ""}>${h("Check (Dry-Run)")}</option>
            </select>
          </label>
          <label class="field"><span>${h("Ausführlichkeit")}</span>
            <select class="fit" name="verbosity">
              ${[0, 1, 2, 3, 4].map((level) => `<option value="${level}"${verbosity === level ? " selected" : ""}>${h(verbosityLabel(level))}</option>`).join("")}
            </select>
          </label>
          <label class="field"><span>${h("Forks")}</span><input class="tiny" name="forks" inputmode="numeric" value="${esc(spec.forks || "")}" placeholder="5"></label>
          <label class="field check"><span>${h("Diff")}</span><input name="diff" type="checkbox"${spec.diff ? " checked" : ""}></label>
        </div>
        <div class="field-row">
          ${chipBox("tags", "Tags", spec.tags)}
          ${chipBox("skip_tags", "Tags überspringen", spec.skip_tags)}
        </div>
        <p class="small muted">${h("Ein Komma oder ein Leerzeichen macht den getippten Tag zu einer Pille.")}</p>
        <div class="field-row">
          <label class="field"><span>${h("Limit")}</span><input class="fit" name="limit" value="${esc(spec.limit || "")}" placeholder="web:&db"></label>
        </div>
        <p class="small muted">${h("Check startet ansible-playbook mit --check. Tags, übersprungene Tags und Limit gelten für jeden Start dieser Vorlage.")}</p>
      </div>
      <div class="form-section" id="tofu-options"${item.tool === "opentofu" ? "" : " hidden"}>
        <h2>OpenTofu</h2>
        <div class="field-row">
          <label class="field"><span>${h("Aktion")}</span>
            <select class="fit" name="action">
              <option value="plan"${spec.action === "apply" ? "" : " selected"}>plan</option>
              <option value="apply"${spec.action === "apply" ? " selected" : ""}>apply</option>
            </select>
          </label>
        </div>
      </div>
      <div class="form-section" id="shell-options"${item.tool === "shell" ? "" : " hidden"}>
        <h2>Shell</h2>
        <label class="field"><span>${h("Skript, wenn kein Einstiegspfad gesetzt ist")}</span><textarea name="script">${esc(spec.script || "")}</textarea></label>
      </div>
      <div class="form-section">
        <h2>${h("Variablen")}</h2>
        <label class="field"><span>${h("Extra-Variablen (JSON)")}</span><textarea name="extra_vars">${esc(JSON.stringify(item.extra_vars || {}, null, 2))}</textarea></label>
      </div>
      <div class="row">
        <button class="btn pill primary" type="submit">${id ? h("Speichern") : h("Anlegen")}</button>
        ${id && can("operator") ? `<button class="btn danger" id="delete-template" type="button">${h("Löschen")}</button>` : ""}
      </div>
    </form>`);
  const form = document.querySelector("#template-form");
  bindChips(form);
  bindVaultPicker(form, credentials, selectedCredentialIds(item));
  form.addEventListener("submit", async (event) => {
    event.preventDefault();
    if (!flushChips(form)) return;
    try {
      const body = {
        name: form.name.value,
        description: form.description.value,
        tool: form.tool.value,
        project_id: form.project_id.value || null,
        inventory_id: form.inventory_id.value || null,
        credential_ids: [
          ...[...form.querySelectorAll("[data-credential-kind]")].map((select) => select.value).filter(Boolean),
          ...[...form.querySelectorAll("#vault-pills [data-vault-id]")].map((node) => node.dataset.vaultId),
        ],
        environment_id: form.environment_id.value || null,
        entrypoint: form.tool.value === "ansible" && form.playbook.value ? form.playbook.value : form.entrypoint.value,
        extra_vars: jsonObject(form.extra_vars.value, "Extra-Variablen"),
        spec: templateSpec(form),
      };
      if (id) await api("PUT", `/api/templates/${encodeURIComponent(id)}`, body);
      else await api("POST", "/api/templates", body);
      go("#/templates");
    } catch (error) {
      handle(error);
    }
  });
  const loadPlaybooks = async () => {
    const field = document.querySelector("#playbook-field");
    const note = document.querySelector("#entrypoint-note");
    const select = form.playbook;
    const ansible = form.tool.value === "ansible" && form.project_id.value;
    field.hidden = !ansible;
    note.textContent = "";
    if (!ansible) return;
    const current = form.entrypoint.value.trim();
    try {
      const contents = await api("GET", `/api/projects/${encodeURIComponent(form.project_id.value)}/contents`);
      const books = contents.playbooks || [];
      const known = books.map((path) => `<option value="${esc(path)}">${esc(path)}</option>`).join("");
      const extra = current && !books.includes(current)
        ? `<option value="${esc(current)}">${esc(current)}</option>`
        : "";
      select.innerHTML = `${known}${extra}<option value="">${h("Anderer Pfad")}</option>`;
      select.value = current && (books.includes(current) || extra) ? current : "";
      if (!books.length) {
        note.textContent = t("Keine Playbooks gefunden. Bei einem Git-Projekt zuerst synchronisieren. Den Pfad kannst du unten eintragen.");
      }
    } catch (error) {
      select.innerHTML = `<option value="">${h("Anderer Pfad")}</option>`;
      note.textContent = t(error.message || "Playbooks lassen sich nicht lesen.");
    }
  };
  const showToolSections = () => {
    const tool = form.tool.value;
    document.querySelector("#ansible-options").hidden = tool !== "ansible";
    document.querySelector("#tofu-options").hidden = tool !== "opentofu";
    document.querySelector("#shell-options").hidden = tool !== "shell";
    document.querySelector("#inventory-field").hidden = tool !== "ansible";
    const label = document.querySelector("#entrypoint-label");
    if (tool === "ansible") label.textContent = t("Playbook-Pfad");
    else if (tool === "opentofu") label.textContent = t("Modulverzeichnis");
    else label.textContent = t("Skriptpfad");
  };
  form.tool.addEventListener("change", () => {
    showToolSections();
    loadPlaybooks();
  });
  showToolSections();
  form.project_id.addEventListener("change", loadPlaybooks);
  form.playbook.addEventListener("change", () => {
    if (form.playbook.value) form.entrypoint.value = form.playbook.value;
  });
  loadPlaybooks();
  const remove = document.querySelector("#delete-template");
  if (remove) {
    remove.addEventListener("click", async () => {
      if (!confirm(t("Vorlage löschen?"))) return;
      try {
        await api("DELETE", `/api/templates/${encodeURIComponent(id)}`);
        go("#/templates");
      } catch (error) {
        handle(error);
      }
    });
  }
}

async function pageJobs() {
  if (!needTeam()) return;
  const jobs = await api("GET", "/api/jobs");
  shell(t("Aufträge"), `
    <section class="card">
      <input class="search" id="job-search" placeholder="${h("Aufträge suchen")}" autocomplete="off">
      <div id="job-table"></div>
    </section>`);
  const drawJobs = () => {
    const needle = document.querySelector("#job-search").value.trim().toLowerCase();
    const shown = jobs.filter((job) => !needle || [job.name, job.tool, job.status, job.trigger, statusLabel(job.status)].join(" ").toLowerCase().includes(needle));
    const filtered = shown.map((job) =>
      `<tr><td><a href="#/jobs/${esc(job.id)}">${esc(job.name)}</a></td><td class="tool">${esc(job.tool)}</td><td>${pill(job.status)}</td><td class="small">${esc(job.trigger)}</td><td class="small muted">${esc(job.created_at)}</td><td>${jobListActions(job)}</td></tr>`
    );
    document.querySelector("#job-table").innerHTML = table([t("Name"), t("Werkzeug"), t("Status"), t("Auslöser"), t("Angelegt"), ""], filtered);
    document.querySelectorAll("[data-relaunch]").forEach((button) => {
      button.addEventListener("click", async () => {
        button.disabled = true;
        try {
          const copy = await api("POST", `/api/jobs/${encodeURIComponent(button.dataset.relaunch)}/relaunch`, {
            only_failed_hosts: button.dataset.failed === "1",
          });
          go(`#/jobs/${copy.id}`);
        } catch (error) {
          button.disabled = false;
          handle(error);
        }
      });
    });
  };
  document.querySelector("#job-search").addEventListener("input", drawJobs);
  drawJobs();
}

function appendLog(row) {
  const log = document.querySelector("#log");
  if (!log) return;
  const line = document.createElement("div");
  if (row.stream === "system" || row.stream === "stderr") line.className = row.stream;
  line.textContent = row.line;
  log.appendChild(line);
  log.scrollTop = log.scrollHeight;
}

function paramRow(label, value) {
  if (value === undefined || value === null || value === "") return "";
  return `<dt>${h(label)}</dt><dd>${esc(value)}</dd>`;
}

function paramRowHtml(label, html) {
  if (!html) return "";
  return `<dt>${h(label)}</dt><dd>${html}</dd>`;
}

function credentialSummary(params) {
  if (Array.isArray(params.credentials)) {
    return params.credentials.map((item) => {
      const name = item && item.name;
      if (!name) return "";
      const kind = item.kind ? credentialKindLabel(item.kind) : "";
      return [name, kind].filter(Boolean).join(" · ");
    }).filter(Boolean).join(", ");
  }
  const kind = params.credential_kind ? credentialKindLabel(params.credential_kind) : "";
  return [params.credential_name, kind].filter(Boolean).join(" · ");
}

function jobParameters(job) {
  const params = job.parameters && job.parameters.tool ? job.parameters : null;
  const hosts = Array.isArray(job.failed_hosts) ? job.failed_hosts : [];
  if (!params) {
    const extra = job.extra_vars && Object.keys(job.extra_vars).length
      ? `<pre class="vars">${esc(JSON.stringify(job.extra_vars, null, 2))}</pre>`
      : "";
    return `<p class="muted">${h("Für diesen Auftrag wurden keine Parameter gespeichert.")}</p>${extra}`;
  }
  const spec = params.spec || {};
  const templateId = params.template_id || job.template_id;
  const templateName = params.template_name || job.name;
  const template = templateId
    ? `<a href="#/templates/${esc(templateId)}">${esc(templateName)}</a>`
    : esc(templateName);
  const credential = credentialSummary(params);
  const environment = [params.environment_name, params.environment_image].filter(Boolean).join(" · ");
  const kind = spec.job_type === "check" ? t("Check") : spec.job_type === "run" ? t("Ausführen") : "";
  const diff = spec.diff === true ? t("Ja") : spec.diff === false ? t("Nein") : "";
  const hostLine = hosts.length
    ? `<dt>${h("Fehlgeschlagene Hosts")}</dt><dd>${hosts.map((host) => esc(host)).join(", ")}</dd>`
    : "";
  return `<dl class="kv">
    <dt>${h("Vorlage")}</dt><dd>${template}</dd>
    ${paramRow("Werkzeug", params.tool)}
    ${paramRow("Einstieg", params.entrypoint)}
    ${paramRow("Projekt", params.project_name)}
    ${paramRow("Inventar", params.inventory_name)}
    ${paramRow("Zugang", credential)}
    ${paramRow("Execution Environment", environment)}
    ${paramRow("Art", kind)}
    ${paramRowHtml("Tags", chipReadout(spec.tags))}
    ${paramRowHtml("Tags überspringen", chipReadout(spec.skip_tags))}
    ${paramRow("Limit", spec.limit)}
    ${paramRow("Diff", diff)}
    ${paramRow("Ausführlichkeit", spec.verbosity === undefined || spec.verbosity === null || spec.verbosity === "" ? "" : t(verbosityLabel(spec.verbosity)))}
    ${paramRow("Forks", spec.forks)}
    ${paramRow("Aktion", spec.action)}
    ${hostLine}
  </dl>
  ${spec.script ? `<h3>${h("Skript")}</h3><pre class="vars">${esc(spec.script)}</pre>` : ""}
  <h3>${h("Extra-Variablen")}</h3>
  <pre class="vars">${esc(JSON.stringify(params.extra_vars || {}, null, 2))}</pre>`;
}

function jobListActions(job) {
  if (!can("operator") || job.status !== "failed") return "";
  const hosts = Array.isArray(job.failed_hosts) ? job.failed_hosts : [];
  const failedHosts = job.tool === "ansible" && hosts.length
    ? `<button class="btn" type="button" data-relaunch="${esc(job.id)}" data-failed="1">${h("Nur fehlgeschlagene Hosts")}</button>`
    : "";
  return `<div class="row"><button class="btn" type="button" data-relaunch="${esc(job.id)}" data-failed="0">${h("Erneut starten")}</button>${failedHosts}</div>`;
}

function jobActions(job) {
  const running = job.status === "pending" || job.status === "running";
  const hosts = Array.isArray(job.failed_hosts) ? job.failed_hosts : [];
  if (!can("operator")) return "";
  if (running) return `<button class="btn danger" type="button" data-action="cancel">${h("Abbrechen")}</button>`;
  if (job.status !== "failed") return "";
  const failedHosts = job.tool === "ansible" && hosts.length
    ? `<button class="btn" type="button" data-action="relaunch-failed">${h("Nur fehlgeschlagene Hosts")}</button>`
    : "";
  return `<button class="btn primary" type="button" data-action="relaunch">${h("Erneut starten")}</button>${failedHosts}`;
}

async function pageJob(id) {
  const live = generation;
  const job = await api("GET", `/api/jobs/${encodeURIComponent(id)}`);
  const running = job.status === "pending" || job.status === "running";
  const actions = jobActions(job);
  shell(job.name, `
    <div class="spread" style="margin-bottom:12px">
      <div class="row">${pill(job.status)} <span class="tool">${esc(job.tool)}</span> <span class="small muted">${esc(job.trigger)} · ${esc(job.launched_by)}</span></div>
    </div>
    <div class="spread tab-bar">
      <div class="tabs">
        <button class="btn pill primary" type="button" data-job-tab="output">${h("Ausgabe")}</button>
        <button class="btn pill" type="button" data-job-tab="parameters">${h("Parameter")}</button>
      </div>
      <div class="row">
        ${actions}
        <button class="btn" type="button" id="download-output">${h("Ausgabe herunterladen")}</button>
      </div>
    </div>
    <div id="panel-output"><div class="log" id="log"></div></div>
    <div id="panel-parameters" hidden>${jobParameters(job)}</div>`);
  const badge = document.querySelector(".pill");
  (job.events || []).forEach(appendLog);
  const relaunchButtons = () => document.querySelectorAll("[data-action=relaunch], [data-action=relaunch-failed]");
  const startAgain = async (onlyFailed) => {
    relaunchButtons().forEach((button) => {
      button.disabled = true;
    });
    try {
      const copy = await api("POST", `/api/jobs/${encodeURIComponent(id)}/relaunch`, { only_failed_hosts: onlyFailed });
      go(`#/jobs/${copy.id}`);
    } catch (error) {
      relaunchButtons().forEach((button) => {
        button.disabled = false;
      });
      handle(error);
    }
  };
  document.querySelectorAll("[data-action=cancel]").forEach((cancel) => {
    cancel.addEventListener("click", async () => {
      try {
        await api("POST", `/api/jobs/${encodeURIComponent(id)}/cancel`, {});
        if (badge) {
          badge.className = "pill canceled";
          badge.textContent = statusLabel("canceled");
        }
        document.querySelectorAll("[data-action=cancel]").forEach((button) => button.remove());
      } catch (error) {
        handle(error);
      }
    });
  });
  document.querySelectorAll("[data-action=relaunch]").forEach((button) => {
    button.addEventListener("click", () => startAgain(false));
  });
  document.querySelectorAll("[data-action=relaunch-failed]").forEach((button) => {
    button.addEventListener("click", () => startAgain(true));
  });
  document.querySelectorAll("[data-job-tab]").forEach((button) => {
    button.addEventListener("click", () => {
      const tab = button.dataset.jobTab;
      document.querySelectorAll("[data-job-tab]").forEach((item) => {
        item.classList.toggle("primary", item === button);
      });
      document.querySelector("#panel-output").hidden = tab !== "output";
      document.querySelector("#panel-parameters").hidden = tab !== "parameters";
    });
  });
  document.querySelector("#download-output").addEventListener("click", async () => {
    const button = document.querySelector("#download-output");
    button.disabled = true;
    try {
      const headers = {};
      if (teamId) headers["X-Team-Id"] = teamId;
      const response = await fetch(`/api/jobs/${encodeURIComponent(id)}/output`, { headers, credentials: "same-origin" });
      const blob = await response.blob();
      if (!response.ok) {
        const text = await blob.text();
        let message = "Die Anfrage ist fehlgeschlagen.";
        try { message = JSON.parse(text).error || message; } catch { /* keep */ }
        const error = new Error(message);
        error.status = response.status;
        throw error;
      }
      const match = /filename="([^"]+)"/.exec(response.headers.get("content-disposition") || "");
      const url = URL.createObjectURL(blob);
      const link = document.createElement("a");
      link.href = url;
      link.download = match ? match[1] : "job.txt";
      document.body.appendChild(link);
      link.click();
      link.remove();
      URL.revokeObjectURL(url);
    } catch (error) {
      handle(error);
    } finally {
      button.disabled = false;
    }
  });
  if (!running || live !== generation) return;
  const last = (job.events || []).reduce((max, event) => Math.max(max, event.id), 0);
  source = new EventSource(`/api/jobs/${encodeURIComponent(id)}/events?after=${last}`);
  source.addEventListener("log", (event) => {
    if (live !== generation) return;
    appendLog(JSON.parse(event.data));
  });
  source.addEventListener("end", (event) => {
    if (live !== generation) return;
    const payload = JSON.parse(event.data);
    if (badge) {
      badge.className = `pill ${payload.status}`;
      badge.textContent = statusLabel(payload.status);
    }
    document.querySelectorAll("[data-action=cancel]").forEach((button) => button.remove());
    source.close();
    source = null;
    if (payload.status === "failed") pageJob(id);
  });
}

async function pageWorkflows() {
  if (!needTeam()) return;
  const items = await api("GET", "/api/workflows");
  shell(t("Workflows"), `
    <section class="card">
      <input class="search" id="workflow-search" placeholder="${h("Workflows suchen")}" autocomplete="off">
      <div id="workflow-table"></div>
    </section>`,
    can("operator") ? addLink("#/workflows/new", "+ Workflow") : "");
  const drawWorkflows = () => {
    const needle = document.querySelector("#workflow-search").value.trim().toLowerCase();
    const shown = items.filter((item) => !needle || `${item.name} ${item.description}`.toLowerCase().includes(needle));
    const filtered = shown.map((item) => `
      <tr>
        <td><a href="#/workflows/${esc(item.id)}">${esc(item.name)}</a><div class="small muted">${esc(item.description)}</div></td>
        <td>${h("{count} Knoten", { count: (item.graph.nodes || []).length })}</td>
        <td>${can("operator") ? `<button class="btn primary" type="button" data-launch="${esc(item.id)}">${h("Starten")}</button>` : ""}</td>
      </tr>`);
    document.querySelector("#workflow-table").innerHTML = table([t("Name"), t("Knoten"), ""], filtered);
    document.querySelectorAll("[data-launch]").forEach((button) => {
      button.addEventListener("click", async () => {
        button.disabled = true;
        try {
          const run = await api("POST", `/api/workflows/${encodeURIComponent(button.dataset.launch)}/launch`, { extra_vars: {} });
          go(`#/runs/${run.id}`);
        } catch (error) {
          button.disabled = false;
          handle(error);
        }
      });
    });
  };
  document.querySelector("#workflow-search").addEventListener("input", drawWorkflows);
  drawWorkflows();
}

async function pageWorkflowNew() {
  if (!needTeam()) return;
  if (!can("operator")) {
    deny("Workflow", "operator");
    return;
  }
  shell(t("Neuer Workflow"), `
    <form class="card form" id="workflow-new">
      <label class="field"><span>${h("Name")}</span><input name="name" required></label>
      <label class="field"><span>${h("Beschreibung")}</span><input name="description"></label>
      <button class="btn pill primary" type="submit">${h("Anlegen und öffnen")}</button>
    </form>`);
  document.querySelector("#workflow-new").addEventListener("submit", async (event) => {
    event.preventDefault();
    const form = event.currentTarget;
    try {
      const created = await api("POST", "/api/workflows", {
        name: form.name.value,
        description: form.description.value,
        graph: { nodes: [], edges: [] },
      });
      go(`#/workflows/${created.id}`);
    } catch (error) {
      handle(error);
    }
  });
}

function edgePoint(node, when) {
  const y = when === "failure" ? 54 : when === "always" ? 39 : 24;
  return { x: node.x + 200, y: node.y + y };
}

async function pageWorkflow(id) {
  if (!needTeam()) return;
  const [workflow, templates, inventories, credentials] = await Promise.all([
    api("GET", `/api/workflows/${encodeURIComponent(id)}`),
    api("GET", "/api/templates"),
    api("GET", "/api/inventories"),
    api("GET", "/api/credentials"),
  ]);
  mountEditor(workflow, templates, false, {}, { inventories, credentials });
}

function mountEditor(workflow, templates, readonly, phases, catalogs, hooks) {
  const onNode = hooks && hooks.onNode;
  const lists = catalogs || { inventories: [], credentials: [] };
  const editable = !readonly && can("operator");
  const graph = {
    nodes: (workflow.graph.nodes || []).map((node) => ({ ...node })),
    edges: (workflow.graph.edges || []).map((edge) => ({ ...edge })),
  };
  let selected = null;
  let selectedEdge = null;
  let link = null;
  let drag = null;
  let flushNode = () => true;
  const back = readonly && workflow.workflow_id
    ? `<a class="btn" href="#/workflows/${esc(workflow.workflow_id)}">${h("Zurück zum Workflow")}</a>`
    : "";
  shell(readonly ? workflow.name : t("Workflow"), `
    <div class="editor-stack">
    <div class="editor-body">
      <div>
        ${readonly ? "" : `
          <div class="row" style="margin-bottom:10px">
            <input id="wf-name" value="${esc(workflow.name)}" ${can("operator") ? "" : "disabled"}>
            <input id="wf-description" value="${esc(workflow.description)}" placeholder="${h("Beschreibung")}" ${can("operator") ? "" : "disabled"}>
          </div>`}
        <p class="small muted" id="wf-hint">${readonly ? h("Farben zeigen den Lauf. Ein Klick zeigt die Ausgabe.") : h("Knoten ziehen. Weitere Knoten rechts hinzufügen. Verbinden über die Auswahl am Knoten oder über Erfolg und Fehler auf der Karte.")}</p>
        <div class="canvas" id="canvas"><div id="board" style="position:relative;min-width:100%;min-height:560px"><svg class="edges" id="edges"></svg><div id="nodes"></div></div></div>
      </div>
      <aside class="card inspector" id="inspector"></aside>
    </div>
    ${readonly ? `<section class="card run-log" id="run-log" aria-live="polite"></section>` : ""}
    </div>`, readonly ? back : `
      ${can("operator") ? `<button class="btn" id="wf-save" type="button">${h("Speichern")}</button><button class="btn primary" id="wf-launch" type="button">${h("Starten")}</button>` : ""}
      ${can("operator") ? `<button class="btn danger" id="wf-delete" type="button">${h("Löschen")}</button>` : ""}`);

  const board = document.querySelector("#board");
  const svg = document.querySelector("#edges");
  const nodes = document.querySelector("#nodes");

  function localPoint(event) {
    const rect = board.getBoundingClientRect();
    return { x: event.clientX - rect.left, y: event.clientY - rect.top };
  }

  function sameJson(left, right) {
    const norm = (value) => {
      if (Array.isArray(value)) return value.map(norm);
      if (value && typeof value === "object") {
        return Object.keys(value).sort().reduce((acc, key) => {
          acc[key] = norm(value[key]);
          return acc;
        }, {});
      }
      return value;
    };
    return JSON.stringify(norm(left)) === JSON.stringify(norm(right));
  }

  function nodeCaption(node) {
    const join = node.convergence === "all" ? t("alle Vorgänger") : t("ein Vorgänger");
    const inventoryId = node.overrides && Object.prototype.hasOwnProperty.call(node.overrides, "inventory_id")
      ? node.overrides.inventory_id
      : undefined;
    if (inventoryId === undefined) return join;
    if (!inventoryId) return `${join} · ${t("Ohne Inventar")}`;
    const inventory = lists.inventories.find((item) => item.id === inventoryId);
    return inventory ? `${join} · ${inventory.name}` : join;
  }

  function paintCaption(node) {
    const label = nodes.querySelector(`[data-id="${CSS.escape(node.id)}"] .muted`);
    if (label) label.textContent = nodeCaption(node);
  }

  function nodeParams(node, template) {
    if (!template) return "";
    const over = node.overrides || {};
    const spec = { ...(template.spec || {}), ...(over.spec || {}) };
    const inventoryId = over.inventory_id !== undefined ? over.inventory_id : (template.inventory_id || "");
    const credentialId = over.credential_id !== undefined ? over.credential_id : (template.credential_id || "");
    const extra = over.extra_vars || template.extra_vars || {};
    const verbosity = Number(spec.verbosity) || 0;
    const jobType = spec.job_type === "check" ? "check" : "run";
    let toolFields = "";
    if (template.tool === "ansible") {
      toolFields = `
        <label class="field"><span>${h("Inventar")}</span><select class="fit" id="node-inventory">${selectOptions(lists.inventories, inventoryId, true, (row) => row.name)}</select></label>
        <label class="field"><span>${h("Art")}</span>
          <select class="fit" id="node-job-type">
            <option value="run"${jobType === "check" ? "" : " selected"}>${h("Lauf")}</option>
            <option value="check"${jobType === "check" ? " selected" : ""}>${h("Check (Dry-Run)")}</option>
          </select>
        </label>
        <label class="field"><span>${h("Ausführlichkeit")}</span>
          <select class="fit" id="node-verbosity">
            ${[0, 1, 2, 3, 4].map((level) => `<option value="${level}"${verbosity === level ? " selected" : ""}>${h(verbosityLabel(level))}</option>`).join("")}
          </select>
        </label>
        <label class="field"><span>${h("Forks")}</span><input class="tiny" id="node-forks" inputmode="numeric" value="${esc(spec.forks ?? "")}"></label>
        <label class="field check"><span>${h("Diff")}</span><input id="node-diff" type="checkbox"${spec.diff ? " checked" : ""}></label>
        <label class="field"><span>${h("Tags")}</span><input id="node-tags" value="${esc(spec.tags || "")}"></label>
        <label class="field"><span>${h("Tags überspringen")}</span><input id="node-skip-tags" value="${esc(spec.skip_tags || "")}"></label>
        <label class="field"><span>${h("Limit")}</span><input id="node-limit" value="${esc(spec.limit || "")}"></label>`;
    } else if (template.tool === "opentofu") {
      const action = spec.action === "apply" ? "apply" : "plan";
      toolFields = `
        <label class="field"><span>${h("Aktion")}</span>
          <select class="fit" id="node-action">
            <option value="plan"${action === "apply" ? "" : " selected"}>plan</option>
            <option value="apply"${action === "apply" ? " selected" : ""}>apply</option>
          </select>
        </label>`;
    }
    return `
      <details class="node-params" open>
        <summary>${h("Parameter")}</summary>
        <p class="small muted">${h("Gilt nur für diesen Knoten. Die Vorlage bleibt unverändert. Ein anderer Zugang ersetzt nur dieselbe Art.")}</p>
        ${toolFields}
        <label class="field"><span>${h("Zugang")}</span><select class="fit" id="node-credential">${selectOptions(lists.credentials, credentialId, true, (row) => `${row.name} (${credentialKindLabel(row.kind)})`)}</select></label>
        <label class="field"><span>${h("Extra-Variablen (JSON)")}</span><textarea id="node-extra">${esc(JSON.stringify(extra, null, 2))}</textarea></label>
      </details>`;
  }

  function drawEdges() {
    const width = Math.max(board.parentElement.clientWidth, ...graph.nodes.map((node) => node.x + 260), 640);
    const height = Math.max(560, ...graph.nodes.map((node) => node.y + 140));
    board.style.width = `${width}px`;
    board.style.height = `${height}px`;
    svg.setAttribute("viewBox", `0 0 ${width} ${height}`);
    svg.innerHTML = graph.edges.map((edge, index) => {
      const from = graph.nodes.find((node) => node.id === edge.from);
      const to = graph.nodes.find((node) => node.id === edge.to);
      if (!from || !to) return "";
      const start = edgePoint(from, edge.when);
      const end = { x: to.x, y: to.y + 36 };
      const dx = Math.max(48, Math.abs(end.x - start.x) / 2);
      const klass = ["success", "failure", "always"].includes(edge.when) ? edge.when : "always";
      return `<path class="${klass}${selectedEdge === index ? " selected" : ""}" data-edge="${index}" d="M ${start.x} ${start.y} C ${start.x + dx} ${start.y}, ${end.x - dx} ${end.y}, ${end.x} ${end.y}"></path>`;
    }).join("");
  }

  function renderNodes() {
    nodes.innerHTML = graph.nodes.map((node) => {
      const phase = phases[node.id];
      const phaseClass = phase ? ` phase-${esc(phase.phase)}` : "";
      const selectedClass = selected === node.id ? " selected" : "";
      const runClass = readonly ? " run-node" : "";
      const armed = link && link.from === node.id ? link.when : "";
      const ports = editable ? `<div class="ports"><button type="button" class="port success${armed === "success" ? " armed" : ""}" data-port="success">${h("Erfolg")}</button><button type="button" class="port failure${armed === "failure" ? " armed" : ""}" data-port="failure">${h("Fehler")}</button></div>` : "";
      return `<div class="wf-node${selectedClass}${phaseClass}${runClass}" data-id="${esc(node.id)}" style="left:${Number(node.x) || 0}px;top:${Number(node.y) || 0}px"><strong>${esc(node.name)}</strong><span class="small muted">${esc(nodeCaption(node))}</span>${ports}</div>`;
    }).join("");
    nodes.querySelectorAll(".wf-node").forEach((element) => {
      const node = graph.nodes.find((item) => item.id === element.dataset.id);
      element.addEventListener("pointerdown", (event) => {
        if (event.target.closest(".port")) return;
        if (readonly) {
          if (onNode) onNode(node, phases[node.id] || null);
          return;
        }
        if (!editable) {
          selected = node.id;
          selectedEdge = null;
          renderInspector();
          nodes.querySelectorAll(".wf-node").forEach((item) => item.classList.toggle("selected", item.dataset.id === selected));
          return;
        }
        if (link && link.from !== node.id) {
          if (!flushNode()) return;
          const fromId = link.from;
          const exists = graph.edges.some((edge) => edge.from === fromId && edge.to === node.id && edge.when === link.when);
          if (!exists) graph.edges.push({ from: fromId, to: node.id, when: link.when });
          link = null;
          selected = fromId;
          selectedEdge = null;
          document.querySelector("#wf-hint").textContent = t("Kante gesetzt.");
          renderNodes();
          renderInspector();
          return;
        }
        if (!flushNode()) return;
        selected = node.id;
        selectedEdge = null;
        const point = localPoint(event);
        drag = { id: node.id, ox: point.x - node.x, oy: point.y - node.y };
        element.setPointerCapture(event.pointerId);
        renderInspector();
        nodes.querySelectorAll(".wf-node").forEach((item) => item.classList.toggle("selected", item.dataset.id === selected));
      });
      element.addEventListener("pointermove", (event) => {
        if (!drag || drag.id !== node.id) return;
        const point = localPoint(event);
        node.x = Math.max(0, Math.round(point.x - drag.ox));
        node.y = Math.max(0, Math.round(point.y - drag.oy));
        element.style.left = `${node.x}px`;
        element.style.top = `${node.y}px`;
        drawEdges();
      });
      element.addEventListener("pointerup", () => { drag = null; });
      element.querySelectorAll(".port").forEach((port) => {
        port.addEventListener("pointerdown", (event) => event.stopPropagation());
        port.addEventListener("click", (event) => {
          event.stopPropagation();
          link = { from: node.id, when: port.dataset.port };
          nodes.querySelectorAll(".port").forEach((item) => item.classList.toggle("armed", item === port));
          document.querySelector("#wf-hint").textContent = port.dataset.port === "failure"
            ? t("Fehlerkante: Zielknoten anklicken.")
            : t("Erfolgskante: Zielknoten anklicken.");
        });
      });
    });
    drawEdges();
  }

  function outgoingTarget(fromId, when) {
    const edge = graph.edges.find((item) => item.from === fromId && item.when === when);
    return edge ? edge.to : "";
  }

  function setOutgoing(fromId, when, toId) {
    graph.edges = graph.edges.filter((edge) => !(edge.from === fromId && edge.when === when));
    if (toId && toId !== fromId && graph.nodes.some((item) => item.id === toId)) {
      graph.edges.push({ from: fromId, to: toId, when });
    }
    link = null;
    drawEdges();
  }

  function targetOptions(fromId, when) {
    const current = outgoingTarget(fromId, when);
    const options = [`<option value="">${h("Kein Ziel")}</option>`];
    graph.nodes.forEach((item) => {
      if (item.id === fromId) return;
      options.push(`<option value="${esc(item.id)}"${item.id === current ? " selected" : ""}>${esc(item.name)}</option>`);
    });
    return options.join("");
  }

  function addNodeControls() {
    return `<div class="inspector-block">
      <h2>${h("Knoten")}</h2>
      <label class="field"><span>${h("Vorlage")}</span><select class="fit" id="add-template">${selectOptions(templates, "", false, (row) => `${row.name} · ${row.tool}`)}</select></label>
      <button class="btn pill" id="add-node" type="button">${h("Knoten hinzufügen")}</button>
    </div>`;
  }

  function bindAddNode(host) {
    host.querySelector("#add-node").addEventListener("click", () => {
      if (!flushNode()) return;
      const templateId = host.querySelector("#add-template").value;
      const template = templates.find((item) => item.id === templateId);
      if (!template) {
        const hint = document.querySelector("#wf-hint");
        if (hint) hint.textContent = t("Lege zuerst eine Vorlage oder einen Workflow an.");
        return;
      }
      const index = graph.nodes.length;
      const id = `n${Math.random().toString(16).slice(2, 10)}`;
      graph.nodes.push({
        id,
        template_id: template.id,
        name: template.name,
        x: 40 + (index % 3) * 240,
        y: 40 + Math.floor(index / 3) * 140,
        convergence: "any",
      });
      selected = id;
      selectedEdge = null;
      link = null;
      renderNodes();
      renderInspector();
    });
  }

  function fieldValue(id) {
    const element = document.querySelector(id);
    if (!element) return "";
    if (element.tagName === "SELECT") {
      const index = element.selectedIndex;
      if (typeof index === "number" && index >= 0 && element.options && element.options[index]) {
        return element.options[index].value;
      }
      const chosen = element.querySelector("option[selected]");
      if (chosen) return chosen.value;
    }
    return element.value == null ? "" : String(element.value);
  }

  function note(message) {
    const hint = document.querySelector("#wf-hint");
    if (hint) hint.textContent = message;
  }

  function readOverrides(node) {
    const template = templates.find((item) => item.id === node.template_id);
    const extraField = document.querySelector("#node-extra");
    if (!template || !extraField) return true;
    const over = {};
    const templateSpec = template.spec || {};
    if (template.tool === "ansible") {
      const inventoryId = fieldValue("#node-inventory");
      if (inventoryId !== (template.inventory_id || "")) over.inventory_id = inventoryId;
      const spec = {};
      const jobType = fieldValue("#node-job-type") === "check" ? "check" : "run";
      const templateJob = templateSpec.job_type === "check" ? "check" : "run";
      if (jobType !== templateJob) spec.job_type = jobType;
      const verbosity = Number(fieldValue("#node-verbosity")) || 0;
      const templateVerbosity = Number(templateSpec.verbosity) || 0;
      if (verbosity !== templateVerbosity) spec.verbosity = verbosity;
      const forksRaw = fieldValue("#node-forks").trim();
      const templateForks = templateSpec.forks;
      const templateHasForks = templateForks !== undefined && templateForks !== null && templateForks !== "";
      if (forksRaw === "") {
        if (templateHasForks) spec.forks = null;
      } else {
        const forks = Number(forksRaw);
        if (!Number.isInteger(forks)) {
          note(t("Forks ist eine ganze Zahl."));
          return false;
        }
        if (forks < 1 || forks > 50) {
          note(t("Forks ist eine Zahl von 1 bis 50."));
          return false;
        }
        if (forks !== templateForks) spec.forks = forks;
      }
      const diff = Boolean(document.querySelector("#node-diff") && document.querySelector("#node-diff").checked);
      if (diff !== Boolean(templateSpec.diff)) spec.diff = diff;
      const tags = fieldValue("#node-tags");
      if (tags !== (templateSpec.tags || "")) spec.tags = tags;
      const skip = fieldValue("#node-skip-tags");
      if (skip !== (templateSpec.skip_tags || "")) spec.skip_tags = skip;
      const limit = fieldValue("#node-limit");
      if (limit !== (templateSpec.limit || "")) spec.limit = limit;
      if (Object.keys(spec).length) over.spec = spec;
    } else if (template.tool === "opentofu") {
      const action = fieldValue("#node-action") === "apply" ? "apply" : "plan";
      const templateAction = templateSpec.action === "apply" ? "apply" : "plan";
      if (action !== templateAction) over.spec = { action };
    }
    const credentialId = fieldValue("#node-credential");
    if (credentialId !== (template.credential_id || "")) over.credential_id = credentialId;
    let extra;
    try {
      extra = jsonObject(extraField.value, "Extra-Variablen");
    } catch (error) {
      note(t(error.message));
      return false;
    }
    if (!sameJson(extra, template.extra_vars || {})) over.extra_vars = extra;
    if (Object.keys(over).length) node.overrides = over;
    else delete node.overrides;
    paintCaption(node);
    const hint = document.querySelector("#wf-hint");
    if (hint && (hint.textContent === t("Forks ist eine ganze Zahl.") || hint.textContent === t("Forks ist eine Zahl von 1 bis 50.") || hint.textContent === t("Extra-Variablen ist kein gültiges JSON.") || hint.textContent === t("Extra-Variablen muss ein JSON-Objekt sein."))) {
      hint.textContent = "";
    }
    return true;
  }

  function renderInspector() {
    if (!flushNode()) return;
    flushNode = () => true;
    const host = document.querySelector("#inspector");
    if (readonly) {
      host.innerHTML = `<h2>${h("Lauf")}</h2><p class="small"><span id="run-state" class="pill ${esc(workflow.status || "pending")}">${esc(statusLabel(workflow.status || "pending"))}</span></p>${back}<p class="small muted">${h("Grün ist erfolgreich, Rot ist fehlgeschlagen, Blau läuft.")}</p>`;
      return;
    }
    if (!editable) {
      host.innerHTML = `<h2>${h("Ansicht")}</h2><p class="small muted">${h("Änderungen sind Operatoren vorbehalten.")}</p>`;
      return;
    }
    const node = graph.nodes.find((item) => item.id === selected);
    if (selectedEdge !== null && graph.edges[selectedEdge]) {
      const edge = graph.edges[selectedEdge];
      host.innerHTML = `${addNodeControls()}
        <div class="inspector-block">
          <h2>${h("Kante")}</h2>
          <label class="field"><span>${h("Wann")}</span>
            <select class="fit" id="edge-when">
              <option value="success"${edge.when === "success" ? " selected" : ""}>${h("Erfolg")}</option>
              <option value="failure"${edge.when === "failure" ? " selected" : ""}>${h("Fehler")}</option>
              <option value="always"${edge.when === "always" ? " selected" : ""}>${h("Immer")}</option>
            </select>
          </label>
          <button class="btn danger" id="edge-delete" type="button">${h("Kante löschen")}</button>
        </div>`;
      bindAddNode(host);
      host.querySelector("#edge-when").addEventListener("change", (event) => {
        edge.when = event.target.value;
        drawEdges();
      });
      host.querySelector("#edge-delete").addEventListener("click", () => {
        graph.edges.splice(selectedEdge, 1);
        selectedEdge = null;
        renderInspector();
        drawEdges();
      });
      return;
    }
    if (!node) {
      host.innerHTML = `${addNodeControls()}
        <p class="small muted">${h("Ein leerer Graph lässt sich speichern, aber nicht starten.")}</p>
        <p class="small muted">${h("Wähle einen Knoten, um ihn zu verbinden.")}</p>`;
      bindAddNode(host);
      return;
    }
    host.innerHTML = `${addNodeControls()}
      <div class="inspector-block">
        <h2>${h("Dieser Knoten")}</h2>
        <label class="field"><span>${h("Name")}</span><input id="node-name" value="${esc(node.name)}"></label>
        <label class="field"><span>${h("Vorlage")}</span><select class="fit" id="node-template">${selectOptions(templates, node.template_id, false, (row) => row.name)}</select></label>
        <label class="field"><span>${h("Zusammenführung")}</span>
          <select class="fit" id="node-join">
            <option value="any"${node.convergence !== "all" ? " selected" : ""}>${h("Irgendein Vorgänger")}</option>
            <option value="all"${node.convergence === "all" ? " selected" : ""}>${h("Alle Vorgänger")}</option>
          </select>
        </label>
        ${nodeParams(node, templates.find((item) => item.id === node.template_id))}
        <h2>${h("Verbinden")}</h2>
        <label class="field"><span>${h("Erfolg führt zu")}</span><select class="fit" id="link-success">${targetOptions(node.id, "success")}</select></label>
        <label class="field"><span>${h("Fehler führt zu")}</span><select class="fit" id="link-failure">${targetOptions(node.id, "failure")}</select></label>
        <label class="field"><span>${h("Immer führt zu")}</span><select class="fit" id="link-always">${targetOptions(node.id, "always")}</select></label>
        <p class="small muted">${h("Die Kante startet bei diesem Knoten.")}</p>
        <button class="btn danger" id="node-delete" type="button">${h("Knoten löschen")}</button>
      </div>`;
    bindAddNode(host);
    host.querySelector("#node-name").addEventListener("input", (event) => {
      node.name = event.target.value;
      const label = nodes.querySelector(`[data-id="${CSS.escape(node.id)}"] strong`);
      if (label) label.textContent = node.name;
    });
    host.querySelector("#node-template").addEventListener("change", (event) => {
      const next = event.target.value;
      if (!readOverrides(node)) {
        Array.from(event.target.options).forEach((option) => {
          if (option.value === node.template_id) option.setAttribute("selected", "");
          else option.removeAttribute("selected");
        });
        return;
      }
      if (node.overrides) delete node.overrides.spec;
      if (node.overrides && node.overrides.inventory_id === undefined && node.overrides.credential_id === undefined && node.overrides.extra_vars === undefined) {
        delete node.overrides;
      }
      node.template_id = next;
      flushNode = () => true;
      renderInspector();
    });
    host.querySelector("#node-join").addEventListener("change", (event) => {
      node.convergence = event.target.value;
      paintCaption(node);
    });
    flushNode = () => readOverrides(node);
    ["#node-inventory", "#node-credential", "#node-job-type", "#node-verbosity", "#node-diff", "#node-action", "#node-forks", "#node-tags", "#node-skip-tags", "#node-limit", "#node-extra"].forEach((id) => {
      const element = host.querySelector(id);
      if (!element) return;
      element.addEventListener("change", () => { flushNode(); });
    });
    ["success", "failure", "always"].forEach((when) => {
      host.querySelector(`#link-${when}`).addEventListener("change", (event) => {
        setOutgoing(node.id, when, event.target.value);
        renderNodes();
      });
    });
    host.querySelector("#node-delete").addEventListener("click", () => {
      flushNode = () => true;
      graph.nodes = graph.nodes.filter((item) => item.id !== node.id);
      graph.edges = graph.edges.filter((edge) => edge.from !== node.id && edge.to !== node.id);
      selected = null;
      link = null;
      renderNodes();
      renderInspector();
    });
  }

  svg.addEventListener("click", (event) => {
    const path = event.target.closest("path");
    if (!path || readonly) return;
    if (!flushNode()) return;
    selectedEdge = Number(path.dataset.edge);
    selected = null;
    link = null;
    drawEdges();
    renderInspector();
  });
  board.addEventListener("pointerdown", (event) => {
    if (readonly || !editable) return;
    if (event.target.closest(".wf-node, path")) return;
    if (!flushNode()) return;
    selected = null;
    selectedEdge = null;
    link = null;
    renderNodes();
    renderInspector();
  });

  renderNodes();
  renderInspector();

  const save = document.querySelector("#wf-save");
  if (save) {
    save.addEventListener("click", async () => {
      if (!flushNode()) return;
      try {
        await api("PUT", `/api/workflows/${encodeURIComponent(workflow.id)}`, {
          name: document.querySelector("#wf-name").value,
          description: document.querySelector("#wf-description").value,
          graph,
        });
        document.querySelector("#wf-hint").textContent = t("Gespeichert.");
      } catch (error) {
        handle(error);
      }
    });
  }
  const launch = document.querySelector("#wf-launch");
  if (launch) {
    launch.addEventListener("click", async () => {
      if (!flushNode()) return;
      try {
        await api("PUT", `/api/workflows/${encodeURIComponent(workflow.id)}`, {
          name: document.querySelector("#wf-name").value,
          description: document.querySelector("#wf-description").value,
          graph,
        });
        const run = await api("POST", `/api/workflows/${encodeURIComponent(workflow.id)}/launch`, { extra_vars: {} });
        go(`#/runs/${run.id}`);
      } catch (error) {
        handle(error);
      }
    });
  }
  const remove = document.querySelector("#wf-delete");
  if (remove) {
    remove.addEventListener("click", async () => {
      if (!confirm(t("Workflow löschen?"))) return;
      try {
        await api("DELETE", `/api/workflows/${encodeURIComponent(workflow.id)}`);
        go("#/workflows");
      } catch (error) {
        handle(error);
      }
    });
  }

  function applyPhases() {
    nodes.querySelectorAll(".wf-node").forEach((element) => {
      const phase = phases[element.dataset.id];
      const phaseClass = phase ? ` phase-${phase.phase}` : "";
      element.className = `wf-node run-node${phaseClass}`;
    });
  }

  return { applyPhases };
}

async function pageRun(id) {
  const live = generation;
  const phases = {};
  let editor = null;
  let openNodeId = null;
  let shownJob = "";
  let outputToken = 0;

  function adopt(run) {
    Object.keys(phases).forEach((key) => delete phases[key]);
    (run.nodes || []).forEach((node) => { phases[node.node_id] = node; });
    return {
      id: run.id,
      workflow_id: run.workflow_id || "",
      name: run.name,
      description: "",
      status: run.status,
      graph: run.graph || { nodes: [], edges: [] },
    };
  }

  function nodeStart(phase) {
    if (!phase) return "";
    return phase.started_at || (phase.phase === "running" ? phase.job_created_at : "") || "";
  }

  function renderLog(run) {
    const host = document.querySelector("#run-log");
    if (!host) return;
    const graphNodes = (run.graph && run.graph.nodes) || [];
    const ranked = graphNodes.map((node, index) => ({ node, index, phase: phases[node.id] }));
    ranked.sort((left, right) => {
      const a = nodeStart(left.phase);
      const b = nodeStart(right.phase);
      if (a && b && a !== b) return a < b ? -1 : 1;
      if (a && !b) return -1;
      if (!a && b) return 1;
      return left.index - right.index;
    });
    const rows = ranked.map(({ node, phase }) => {
      const status = phase ? phase.phase : "pending";
      const start = nodeStart(phase);
      const end = phase && phase.finished_at ? phase.finished_at : "";
      return `<button class="run-line ${esc(status)}" type="button" data-run-node="${esc(node.id)}">
        <span class="mono">${esc(clock(start))}</span>
        <span>${esc(node.name)}</span>
        <span>${esc(statusLabel(status))}</span>
        <span class="mono">${esc(clock(end))}</span>
        <span class="mono">${esc(spanText(start, end, status))}</span>
      </button>`;
    }).join("");
    host.innerHTML = `
      <h2>${h("Status")}</h2>
      <div class="run-lines">
        <div class="run-line head">
          <span>${h("Start")}</span>
          <span>${h("Name")}</span>
          <span>${h("Status")}</span>
          <span>${h("Ende")}</span>
          <span>${h("Dauer")}</span>
        </div>
        ${rows}
      </div>`;
    host.querySelectorAll("[data-run-node]").forEach((row) => {
      row.addEventListener("click", () => {
        const node = graphNodes.find((item) => item.id === row.dataset.runNode);
        if (node) openOutput(node, phases[node.id] || null);
      });
    });
  }

  function overlayMeta(phase) {
    const status = phase ? phase.phase : "pending";
    const start = nodeStart(phase);
    const end = phase && phase.finished_at ? phase.finished_at : "";
    const tool = phase && phase.tool ? `<span class="tool">${esc(phase.tool)}</span>` : "";
    const link = phase && phase.job_id
      ? `<a href="#/jobs/${esc(phase.job_id)}">${h("Auftrag öffnen")}</a>`
      : "";
    return `${pill(status)} ${tool} <span class="small muted">${esc(clock(start))} – ${esc(clock(end))} · ${esc(spanText(start, end, status))}</span> ${link}`;
  }

  function closeOutput() {
    outputToken += 1;
    openNodeId = null;
    shownJob = "";
    const overlay = document.querySelector("#node-output");
    if (overlay) overlay.remove();
    if (source) {
      source.close();
      source = null;
    }
  }

  async function openOutput(node, phase) {
    if (live !== generation) return;
    const token = ++outputToken;
    openNodeId = node.id;
    const jobId = phase && phase.job_id ? phase.job_id : "";
    shownJob = jobId;
    if (source) {
      source.close();
      source = null;
    }
    let overlay = document.querySelector("#node-output");
    if (!overlay) {
      document.querySelector(".main").insertAdjacentHTML("beforeend", `
        <div class="overlay" id="node-output">
          <div class="overlay-card" role="dialog" aria-modal="true" aria-labelledby="overlay-title">
            <div class="spread">
              <h2 id="overlay-title"></h2>
              <button class="btn" type="button" id="overlay-close">${h("Schließen")}</button>
            </div>
            <p class="small row" id="overlay-meta"></p>
            <div class="log" id="log"></div>
          </div>
        </div>`);
      overlay = document.querySelector("#node-output");
      overlay.addEventListener("click", (event) => {
        if (event.target === overlay) closeOutput();
      });
      document.querySelector("#overlay-close").addEventListener("click", closeOutput);
    }
    document.querySelector("#overlay-title").textContent = node.name;
    document.querySelector("#overlay-meta").innerHTML = overlayMeta(phase);
    const log = document.querySelector("#log");
    log.replaceChildren();
    const note = (text) => {
      const line = document.createElement("div");
      line.className = "system";
      line.textContent = text;
      log.appendChild(line);
    };
    if (!jobId) {
      note(phase && phase.phase === "skipped"
        ? t("Dieser Knoten wurde übersprungen.")
        : t("Dieser Knoten hat noch keine Ausgabe."));
      return;
    }
    try {
      const job = await api("GET", `/api/jobs/${encodeURIComponent(jobId)}`);
      if (token !== outputToken || live !== generation) return;
      (job.events || []).forEach(appendLog);
      if (!log.childNodes.length) note(t("Dieser Knoten hat noch keine Ausgabe."));
      const running = job.status === "pending" || job.status === "running";
      if (!running) return;
      const last = (job.events || []).reduce((max, event) => Math.max(max, event.id), 0);
      source = new EventSource(`/api/jobs/${encodeURIComponent(jobId)}/events?after=${last}`);
      source.addEventListener("log", (event) => {
        if (token !== outputToken || live !== generation) return;
        appendLog(JSON.parse(event.data));
      });
      source.addEventListener("end", (event) => {
        if (token !== outputToken || live !== generation) return;
        const payload = JSON.parse(event.data);
        const badge = document.querySelector("#node-output .pill");
        if (badge) {
          badge.className = `pill ${payload.status}`;
          badge.textContent = statusLabel(payload.status);
        }
        source.close();
        source = null;
      });
    } catch (error) {
      if (token === outputToken && live === generation) handle(error);
    }
  }

  function paint(run) {
    const view = adopt(run);
    if (!editor) {
      editor = mountEditor(view, [], true, phases, null, { onNode: openOutput });
    } else if (editor) {
      editor.applyPhases();
      const badge = document.querySelector("#run-state");
      if (badge) {
        badge.className = `pill ${run.status || "pending"}`;
        badge.textContent = statusLabel(run.status || "pending");
      }
    }
    renderLog(run);
    if (!openNodeId || !document.querySelector("#node-output")) return;
    const node = ((run.graph && run.graph.nodes) || []).find((item) => item.id === openNodeId);
    const phase = phases[openNodeId] || null;
    if (!node) return;
    const jobId = phase && phase.job_id ? phase.job_id : "";
    if (jobId && jobId !== shownJob) openOutput(node, phase);
    else {
      const meta = document.querySelector("#overlay-meta");
      if (meta) meta.innerHTML = overlayMeta(phase);
    }
  }

  runKey = (event) => {
    if (event.key === "Escape") closeOutput();
  };
  document.addEventListener("keydown", runKey);

  const run = await api("GET", `/api/workflow-jobs/${encodeURIComponent(id)}`);
  if (live !== generation) return;
  paint(run);
  if (["successful", "failed", "canceled"].includes(run.status)) return;
  pollTimer = setInterval(async () => {
    if (live !== generation) return;
    try {
      const next = await api("GET", `/api/workflow-jobs/${encodeURIComponent(id)}`);
      if (live !== generation) return;
      paint(next);
      if (["successful", "failed", "canceled"].includes(next.status)) {
        clearInterval(pollTimer);
        pollTimer = null;
      }
    } catch (error) {
      if (live === generation) handle(error);
    }
  }, 1000);
}

async function pageHooks() {
  if (!needTeam()) return;
  const hooks = await api("GET", "/api/hooks");
  const deliveryRows = hooks.length
    ? await api("GET", `/api/hooks/${encodeURIComponent(hooks[0].id)}/deliveries`)
    : [];
  const origin = location.origin;
  const incoming = deliveryRows.map((item) =>
    `<tr><td>${esc(item.hook_name)}</td><td>${pill(item.status)}</td><td>${esc(item.summary)}</td><td class="small muted">${esc(item.created_at)}</td></tr>`
  );
  shell(t("Webhooks"), `
    <section class="card" style="margin-bottom:14px">
      <input class="search" id="hook-search" placeholder="${h("Webhooks suchen")}" autocomplete="off">
      <div id="hook-table"></div>
    </section>
    <section class="card"><h2>${h("Eingänge")}</h2>${table([t("Hook"), t("Status"), t("Zusammenfassung"), t("Zeit")], incoming)}</section>`,
    can("operator") ? addLink("#/hooks/new", "+ Webhook") : "");
  const drawHooks = () => {
    const needle = document.querySelector("#hook-search").value.trim().toLowerCase();
    const rows = hooks.filter((hook) => !needle || `${hook.name} ${hook.match_alertname} ${hook.match_status}`.toLowerCase().includes(needle)).map((hook) => {
      const url = hook.token ? `${origin}/hooks/${hook.id}/${hook.token}` : "";
      return `<tr>
        <td>${esc(hook.name)}<div class="small muted">${esc(hook.match_alertname)} · ${esc(hook.match_status)}</div></td>
        <td class="mono">${url ? esc(url) : h("Token nur für Operatoren")}</td>
        <td>${can("operator") ? `<button class="btn" type="button" data-test="${esc(hook.id)}">${h("Test")}</button> <button class="btn danger" type="button" data-delete="${esc(hook.id)}">${h("Löschen")}</button>` : ""}</td>
      </tr>`;
    });
    document.querySelector("#hook-table").innerHTML = table([t("Hook"), t("Adresse"), ""], rows);
    document.querySelectorAll("[data-test]").forEach((button) => {
      button.addEventListener("click", async () => {
        try {
          const result = await api("POST", `/api/hooks/${encodeURIComponent(button.dataset.test)}/test`);
          if (result.workflow_job_id) go(`#/runs/${result.workflow_job_id}`);
          else if (result.job_id) go(`#/jobs/${result.job_id}`);
          else route();
        } catch (error) {
          handle(error);
        }
      });
    });
    document.querySelectorAll("[data-delete]").forEach((button) => {
      button.addEventListener("click", async () => {
        if (!confirm(t("Webhook löschen?"))) return;
        try {
          await api("DELETE", `/api/hooks/${encodeURIComponent(button.dataset.delete)}`);
          route();
        } catch (error) {
          handle(error);
        }
      });
    });
  };
  document.querySelector("#hook-search").addEventListener("input", drawHooks);
  drawHooks();
}

async function pageHookNew() {
  if (!needTeam()) return;
  if (!can("operator")) {
    deny("Webhook", "operator");
    return;
  }
  const [templates, workflows] = await Promise.all([
    api("GET", "/api/templates"),
    api("GET", "/api/workflows"),
  ]);
  shell(t("Webhook anlegen"), `
    <form class="card form" id="hook-form">
      <label class="field"><span>${h("Name")}</span><input name="name" required></label>
      <div class="field-row">
        <label class="field"><span>${h("Alertname")}</span><input class="fit" name="match_alertname" required placeholder="DiskSpaceLow"></label>
        <label class="field"><span>${h("Status")}</span>
          <select class="fit" name="match_status"><option value="firing">firing</option><option value="resolved">resolved</option><option value="any">any</option></select>
        </label>
      </div>
      <label class="field"><span>${h("Ziel")}</span>
        <select name="target">
          <optgroup label="${h("Workflow")}">${workflows.map((item) => `<option value="workflow:${esc(item.id)}">${esc(item.name)}</option>`).join("")}</optgroup>
          <optgroup label="${h("Vorlage")}">${templates.map((item) => `<option value="template:${esc(item.id)}">${esc(item.name)}</option>`).join("")}</optgroup>
        </select>
      </label>
      <label class="field"><span>${h("Extra-Variablen (JSON)")}</span><textarea name="extra_vars">{}</textarea></label>
      <button class="btn pill primary" type="submit">${h("Anlegen")}</button>
    </form>`);
  document.querySelector("#hook-form").addEventListener("submit", async (event) => {
    event.preventDefault();
    const form = event.currentTarget;
    try {
      const rawTarget = form.target.value;
      const splitAt = rawTarget.indexOf(":");
      if (splitAt < 1) throw new Error("Lege zuerst eine Vorlage oder einen Workflow an.");
      const kind = rawTarget.slice(0, splitAt);
      const targetId = rawTarget.slice(splitAt + 1);
      await api("POST", "/api/hooks", {
        name: form.name.value,
        match_alertname: form.match_alertname.value,
        match_status: form.match_status.value,
        extra_vars: jsonObject(form.extra_vars.value, "Extra-Variablen"),
        template_id: kind === "template" ? targetId : null,
        workflow_id: kind === "workflow" ? targetId : null,
      });
      go("#/hooks");
    } catch (error) {
      handle(error);
    }
  });
}

async function pageRuntime() {
  if (!needTeam()) return;
  const [cache, plugins, environments] = await Promise.all([
    api("GET", "/api/cache"),
    api("GET", "/api/plugins"),
    api("GET", "/api/environments"),
  ]);
  const collectionRows = (cache.collections || []).map((item) => `<tr><td>${esc(item.name)}</td><td>${esc(item.version)}</td></tr>`);
  const roleRows = (cache.roles || []).map((item) => `<tr><td>${esc(item.name)}</td><td>${esc(item.version)}</td></tr>`);
  const pluginRows = plugins.map((item) => `<tr><td class="tool">${esc(item.id)}</td><td>${esc(t(item.summary))}</td></tr>`);
  shell(t("Laufzeit"), `
    <section class="card" style="margin-bottom:14px">
      <h2>${h("Execution Environments")}</h2>
      <input class="search" id="environment-search" placeholder="${h("Umgebungen suchen")}" autocomplete="off">
      <div id="environment-table"></div>
    </section>
    <div class="grid-3" style="margin-bottom:14px">
      <article class="card stat"><span class="small muted">${h("Collections")}</span><strong>${esc(bytes(cache.collections_bytes))}</strong></article>
      <article class="card stat"><span class="small muted">${h("Rollen")}</span><strong>${esc(bytes(cache.roles_bytes))}</strong></article>
      <article class="card stat"><span class="small muted">${h("OpenTofu")}</span><strong>${esc(bytes(cache.tofu_bytes))}</strong></article>
    </div>
    <p class="small muted" style="margin-top:0">${esc(t(cache.note || ""))}</p>
    <section class="card" style="margin-bottom:14px"><h2>${h("Werkzeuge")}</h2>${table([t("Id"), t("Beschreibung")], pluginRows)}</section>
    <section class="card" style="margin-bottom:14px"><h2>${h("Collections im Cache")}</h2>${table([t("Name"), t("Version")], collectionRows)}</section>
    <section class="card" style="margin-bottom:14px"><h2>${h("Rollen im Cache")}</h2>${table([t("Name"), t("Version")], roleRows)}</section>`,
    can("owner") ? addLink("#/environments/new", "+ Umgebung") : "");
  const drawEnvironments = () => {
    const needle = document.querySelector("#environment-search").value.trim().toLowerCase();
    const rows = environments.filter((item) => !needle || `${item.name} ${item.image} ${item.pull_policy}`.toLowerCase().includes(needle)).map((item) =>
      `<tr><td><a href="#/environments/${esc(item.id)}">${esc(item.name)}</a></td><td class="mono">${esc(item.image)}</td><td>${esc(item.pull_policy)}</td><td>${can("owner") ? `<button class="btn danger" type="button" data-env="${esc(item.id)}">${h("Löschen")}</button>` : ""}</td></tr>`
    );
    document.querySelector("#environment-table").innerHTML = table([t("Name"), t("Image"), t("Pull"), ""], rows);
    document.querySelectorAll("[data-env]").forEach((button) => {
      button.addEventListener("click", async () => {
        try {
          await api("DELETE", `/api/environments/${encodeURIComponent(button.dataset.env)}`);
          route();
        } catch (error) {
          handle(error);
        }
      });
    });
  };
  document.querySelector("#environment-search").addEventListener("input", drawEnvironments);
  drawEnvironments();
}

function environmentForm(item) {
  const policy = item ? item.pull_policy : "if_not_present";
  const option = (value) => `<option value="${value}"${policy === value ? " selected" : ""}>${value}</option>`;
  return `<form class="card form" id="env-form">
    <label class="field"><span>${h("Name")}</span><input name="name" required value="${item ? esc(item.name) : ""}"></label>
    <label class="field"><span>${h("Image")}</span><input name="image" required value="${item ? esc(item.image) : ""}" placeholder="quay.io/ansible/awx-ee:latest"></label>
    <div class="field-row">
      <label class="field"><span>${h("Pull")}</span><select class="fit" name="pull_policy">${option("if_not_present")}${option("always")}${option("never")}</select></label>
    </div>
    <label class="field"><span>${h("Beschreibung")}</span><input name="description" value="${item ? esc(item.description) : ""}"></label>
    <button class="btn pill primary" type="submit">${item ? h("Speichern") : h("Anlegen")}</button>
  </form>`;
}

function bindEnvironmentForm(id) {
  document.querySelector("#env-form").addEventListener("submit", async (event) => {
    event.preventDefault();
    const form = event.currentTarget;
    const payload = {
      name: form.name.value,
      image: form.image.value,
      pull_policy: form.pull_policy.value,
      description: form.description.value,
    };
    try {
      if (id) await api("PUT", `/api/environments/${encodeURIComponent(id)}`, payload);
      else await api("POST", "/api/environments", payload);
      go("#/runtime");
    } catch (error) {
      handle(error);
    }
  });
}

async function pageEnvironmentNew() {
  if (!needTeam()) return;
  if (!can("owner")) {
    deny("Execution Environment", "owner");
    return;
  }
  shell(t("Umgebung anlegen"), environmentForm(null));
  bindEnvironmentForm(null);
}

async function pageEnvironment(id) {
  if (!needTeam()) return;
  const item = await api("GET", `/api/environments/${encodeURIComponent(id)}`);
  if (!can("owner")) {
    shell(item.name, `<section class="card"><dl class="kv">
      ${paramRow("Name", item.name)}
      ${paramRow("Image", item.image)}
      ${paramRow("Pull", item.pull_policy)}
      ${paramRow("Beschreibung", item.description)}
    </dl></section>`);
    return;
  }
  shell(item.name, environmentForm(item), `<button class="btn danger" id="delete-env" type="button">${h("Löschen")}</button>`);
  bindEnvironmentForm(id);
  document.querySelector("#delete-env").addEventListener("click", async () => {
    try {
      await api("DELETE", `/api/environments/${encodeURIComponent(id)}`);
      go("#/runtime");
    } catch (error) {
      handle(error);
    }
  });
}

function grantLabel(item) {
  return item.team_ids && item.team_ids.length
    ? t("{count} Teams", { count: item.team_ids.length })
    : t("Alle Teams der Organisation");
}

function orgOptions(organizations, selected) {
  return organizations.map((item) =>
    `<option value="${esc(item.id)}"${item.id === selected ? " selected" : ""}>${esc(item.name)}</option>`
  ).join("");
}

function teamBoxes(teams, selected) {
  const chosen = new Set(selected || []);
  if (!teams.length) return `<p class="small muted">${h("In dieser Organisation gibt es noch kein Team.")}</p>`;
  return teams.map((team) =>
    `<label class="row"><input type="checkbox" name="team_ids" value="${esc(team.id)}"${chosen.has(team.id) ? " checked" : ""}> ${esc(team.name)}</label>`
  ).join("");
}

async function pageProjects() {
  if (!needTeam()) return;
  const projects = await api("GET", "/api/projects");
  shell(t("Projekte"), `
    <section class="card">
      <input class="search" id="project-search" placeholder="${h("Projekte suchen")}" autocomplete="off">
      <div id="project-table"></div>
    </section>`,
    can("operator") ? addLink("#/projects/new", "+ Projekt") : "");
  const drawProjects = () => {
    const needle = document.querySelector("#project-search").value.trim().toLowerCase();
    const rows = projects.filter((item) => !needle || `${item.name} ${item.organization_name || ""} ${item.source_type} ${grantLabel(item)}`.toLowerCase().includes(needle)).map((item) =>
      `<tr><td><a href="#/projects/${esc(item.id)}">${esc(item.name)}</a></td><td>${esc(item.organization_name || "")}</td><td>${esc(item.source_type)}</td><td class="small muted">${esc(grantLabel(item))}</td></tr>`
    );
    document.querySelector("#project-table").innerHTML = table([t("Name"), t("Organisation"), t("Art"), t("Sichtbarkeit")], rows);
  };
  document.querySelector("#project-search").addEventListener("input", drawProjects);
  drawProjects();
}

async function pageProjectNew() {
  if (!needTeam()) return;
  if (!can("operator")) {
    deny("Projekt", "operator");
    return;
  }
  const organizations = me.is_admin ? await api("GET", "/api/organizations") : [];
  const current = (me.teams || []).find((team) => team.id === teamId);
  shell(t("Projekt anlegen"), `
    <form class="card form" id="project-form">
      <label class="field"><span>${h("Name")}</span><input name="name" required></label>
      <div class="field-row">
        <label class="field"><span>${h("Art")}</span><select class="fit" name="source_type"><option value="path">${h("Pfad")}</option><option value="git">Git</option></select></label>
        <label class="field"><span>${h("Revision")}</span><input class="fit" name="revision" placeholder="${h("Branch, Tag oder Commit")}"></label>
      </div>
      <label class="field"><span>${h("Quelle")}</span><input name="source" required placeholder="https://… oder file://…"></label>
      ${me.is_admin ? `<label class="field"><span>${h("Organisation")}</span><select name="organization_id">${orgOptions(organizations, current && current.organization_id)}</select></label>` : ""}
      <button class="btn pill primary" type="submit">${h("Anlegen")}</button>
    </form>`);
  document.querySelector("#project-form").addEventListener("submit", async (event) => {
    event.preventDefault();
    const form = event.currentTarget;
    try {
      const payload = {
        name: form.name.value,
        source_type: form.source_type.value,
        source: form.source.value,
        revision: form.revision.value,
        team_ids: [],
      };
      if (form.organization_id) payload.organization_id = form.organization_id.value;
      const created = await api("POST", "/api/projects", payload);
      go(`#/projects/${created.id}`);
    } catch (error) { handle(error); }
  });
}

async function pageProject(id) {
  if (!needTeam()) return;
  const project = await api("GET", `/api/projects/${encodeURIComponent(id)}`);
  const organizations = await api("GET", "/api/organizations");
  const org = await api("GET", `/api/organizations/${encodeURIComponent(project.organization_id)}`);
  shell(project.name, `
    <section class="card" style="margin-bottom:14px">
      <p class="muted">${h("Quelle: {source}", { source: `${project.source_type} · ${project.source}` })}</p>
      ${project.revision ? `<p class="small">${h("Revision {revision}", { revision: project.revision })}</p>` : ""}
      ${can("operator") && project.source_type === "git" ? `<div class="row"><button class="btn" id="sync" type="button">${h("Synchronisieren")}</button></div><p class="small" id="sync-note"></p>` : ""}
    </section>
    <section class="card">
      <h2>${h("Zuweisung")}</h2>
      <p class="small muted">${h("Ohne angehaktes Team dürfen alle Teams der Organisation das Projekt nutzen.")}</p>
      ${can("operator") ? `<form class="form" id="assign-form">
        <label class="field"><span>${h("Organisation")}</span><select name="organization_id"${me.is_admin ? "" : " disabled"}>${orgOptions(organizations, project.organization_id)}</select></label>
        <div class="stack" id="team-boxes">${teamBoxes(org.teams || [], project.team_ids)}</div>
        <button class="btn pill primary" type="submit">${h("Zuweisung speichern")}</button>
      </form>` : `<p>${h("Organisation {name}. Sichtbarkeit: {grant}.", { name: project.organization_name || "", grant: grantLabel(project) })}</p>`}
    </section>`,
    can("operator") ? `<button class="btn danger" id="delete-project" type="button">${h("Löschen")}</button>` : "");
  const sync = document.querySelector("#sync");
  if (sync) {
    sync.addEventListener("click", async () => {
      sync.disabled = true;
      const note = document.querySelector("#sync-note");
      try {
        const result = await api("POST", `/api/projects/${encodeURIComponent(id)}/sync`, {});
        note.textContent = t("Checkout: {path}", { path: result.path });
        sync.textContent = t("Synchron");
      } catch (error) {
        sync.disabled = false;
        if (note) note.textContent = t(error.message || "Sync fehlgeschlagen.");
        handle(error);
      }
    });
  }
  const form = document.querySelector("#assign-form");
  if (form) {
    form.organization_id.addEventListener("change", async () => {
      const box = document.querySelector("#team-boxes");
      box.innerHTML = `<p class="small muted">${h("Teams werden geladen.")}</p>`;
      try {
        const next = await api("GET", `/api/organizations/${encodeURIComponent(form.organization_id.value)}`);
        box.innerHTML = teamBoxes(next.teams || [], []);
      } catch (error) { handle(error); }
    });
    form.addEventListener("submit", async (event) => {
      event.preventDefault();
      try {
        const teamIds = [...form.querySelectorAll("input[name=team_ids]:checked")].map((input) => input.value);
        await api("PUT", `/api/projects/${encodeURIComponent(id)}`, {
          organization_id: form.organization_id.value,
          team_ids: teamIds,
        });
        route();
      } catch (error) { handle(error); }
    });
  }
  const remove = document.querySelector("#delete-project");
  if (remove) {
    remove.addEventListener("click", async () => {
      if (!confirm(t("Projekt löschen?"))) return;
      try {
        await api("DELETE", `/api/projects/${encodeURIComponent(id)}`);
        go("#/projects");
      } catch (error) { handle(error); }
    });
  }
}

function sourceKindLabel(kind) {
  return { ini: "INI", project: t("Projekt"), netbox: "NetBox", mixed: t("Mehrere") }[kind] || kind || "INI";
}

function sourceSummary(item) {
  const data = item.source || {};
  if (item.kind === "netbox") return data.api_endpoint || "NetBox";
  if (item.kind === "project") return data.path || t("Projekt");
  return t("Text");
}

function mountSourceDraft(container, projects, kind, preset) {
  const data = preset || {};
  const card = document.createElement("section");
  card.className = "card";
  card.dataset.kind = kind;
  card.style.marginTop = "10px";
  const projectOptions = projects.map((item) => `<option value="${esc(item.id)}">${esc(item.name)}</option>`).join("");
  const title = kind === "ini" ? t("Manuell") : kind === "project" ? t("Projekt") : "NetBox";
  let fields = "";
  if (kind === "ini") {
    fields = `<label class="field"><span>${h("INI oder YAML")}</span><textarea name="body" required>${esc(data.body || "[all]\nlocalhost ansible_connection=local")}</textarea></label>`;
  } else if (kind === "project") {
    fields = `<label class="field"><span>${h("Projekt")}</span><select name="project_id"><option value="">—</option>${projectOptions}</select></label>
      <label class="field"><span>${h("Inventardatei")}</span><select name="path"><option value="">—</option></select></label>
      <p class="small muted" data-plugin-note></p>`;
  } else {
    const groupBy = Array.isArray(data.group_by) ? data.group_by.join(", ") : (data.group_by || "");
    const filters = Array.isArray(data.query_filters) ? data.query_filters.join("\n") : (data.query || "");
    fields = `<label class="field"><span>${h("API-Adresse")}</span><input name="api_endpoint" required value="${esc(data.api_endpoint || "")}" placeholder="https://netbox.example.com"></label>
      <label class="field"><span>${h("Token")}</span><input name="token" type="password" required autocomplete="off"></label>
      <label class="field"><span>${h("Gruppierung")}</span><input name="group_by" value="${esc(groupBy)}" placeholder="sites, device_roles"></label>
      <label class="field"><span>${h("Filter, eine Zeile je Schlüssel=Wert")}</span><textarea name="query">${esc(filters)}</textarea></label>
      <label class="field"><span>${h("Zertifikat prüfen")}</span><input name="validate_certs" type="checkbox"${data.validate_certs === false ? "" : " checked"}></label>
      <label class="field"><span>${h("Virtuelle Maschinen")}</span><input name="virtual_machines" type="checkbox"${data.virtual_machines === false ? "" : " checked"}></label>`;
  }
  card.innerHTML = `<div class="spread"><h2>${esc(title)}</h2><button class="btn ghost" type="button" data-remove-draft>${h("Entfernen")}</button></div>
    <div class="form">${fields}
      <label class="field"><span>${h("Name der Quelle")}</span><input name="source_name" value="${esc(data.name || "")}" placeholder="${esc(title)}"></label>
    </div>`;
  container.appendChild(card);
  card.querySelector("[data-remove-draft]").addEventListener("click", () => card.remove());
  if (kind === "project") {
    const select = card.querySelector("[name=project_id]");
    const files = card.querySelector("[name=path]");
    const note = card.querySelector("[data-plugin-note]");
    select.addEventListener("change", async () => {
      files.innerHTML = `<option value="">—</option>`;
      note.textContent = "";
      if (!select.value) return;
      try {
        const contents = await api("GET", `/api/projects/${encodeURIComponent(select.value)}/contents`);
        const inventories = contents.inventories || [];
        const staticFiles = inventories.filter((file) => file.format !== "plugin");
        files.innerHTML = `<option value="">—</option>${staticFiles.map((file) => `<option value="${esc(file.path)}">${esc(file.path)}</option>`).join("")}`;
        const plugins = inventories.filter((file) => file.format === "plugin");
        if (plugins.length) {
          note.textContent = t("{files} wird hier nicht ausgeführt. NetBox legst du als eigene Quelle an.", {
            files: plugins.map((file) => `${file.path} (${file.plugin || "Plugin"})`).join(", "),
          });
        } else if (!staticFiles.length) {
          note.textContent = t("Keine Inventardateien. Bei einem Git-Projekt zuerst synchronisieren.");
        }
      } catch (error) {
        note.textContent = t(error.message || "Projekt lässt sich nicht lesen.");
      }
    });
  }
  return card;
}

function readSourceDraft(card) {
  const kind = card.dataset.kind;
  const name = card.querySelector("[name=source_name]").value.trim();
  if (kind === "ini") return { kind, name, body: card.querySelector("[name=body]").value };
  if (kind === "project") {
    return {
      kind,
      name,
      project_id: card.querySelector("[name=project_id]").value,
      path: card.querySelector("[name=path]").value,
    };
  }
  return {
    kind,
    name,
    api_endpoint: card.querySelector("[name=api_endpoint]").value,
    token: card.querySelector("[name=token]").value,
    group_by: card.querySelector("[name=group_by]").value.split(",").map((item) => item.trim()).filter(Boolean),
    query_filters: card.querySelector("[name=query]").value.split("\n").map((item) => item.trim()).filter(Boolean),
    validate_certs: card.querySelector("[name=validate_certs]").checked,
    virtual_machines: card.querySelector("[name=virtual_machines]").checked,
  };
}

function bindSourcePicks(projects) {
  document.querySelectorAll("[data-add-kind]").forEach((button) => {
    button.addEventListener("click", () => {
      mountSourceDraft(document.querySelector("#source-drafts"), projects, button.dataset.addKind);
    });
  });
}

async function pageInventories() {
  if (!needTeam()) return;
  const items = await api("GET", "/api/inventories");
  shell(t("Inventare"), `
    <section class="card">
      <input class="search" id="inventory-search" placeholder="${h("Inventare suchen")}" autocomplete="off">
      <div id="inventory-table"></div>
    </section>`,
    can("operator") ? addLink("#/inventories/new", "+ Inventar") : "");
  const draw = () => {
    const needle = document.querySelector("#inventory-search").value.trim().toLowerCase();
    const shown = items.filter((item) => {
      if (!needle) return true;
      const kinds = (item.sources || []).map((source) => `${source.name} ${sourceKindLabel(source.kind)} ${sourceSummary(source)}`).join(" ");
      return `${item.name} ${kinds}`.toLowerCase().includes(needle);
    });
    const rows = shown.map((item) => {
      const kinds = (item.sources || []).map((source) => sourceKindLabel(source.kind));
      const label = kinds.length ? [...new Set(kinds)].join(", ") : sourceKindLabel(item.source_kind);
      const count = item.source_count != null ? item.source_count : kinds.length;
      return `<tr><td><a href="#/inventories/${esc(item.id)}">${esc(item.name)}</a></td><td>${esc(count)}</td><td>${esc(label)}</td><td>${esc(item.host_count)}</td></tr>`;
    });
    document.querySelector("#inventory-table").innerHTML = table([t("Name"), t("Quellen"), t("Art"), t("Hosts")], rows);
  };
  document.querySelector("#inventory-search").addEventListener("input", draw);
  draw();
}

async function pageInventoryNew() {
  if (!needTeam()) return;
  if (!can("operator")) {
    deny("Inventar", "operator");
    return;
  }
  const projects = await api("GET", "/api/projects");
  shell(t("Inventar anlegen"), `
    <form class="card form" id="inventory-create">
      <label class="field"><span>${h("Name")}</span><input name="name" required></label>
      <div>
        <h2>${h("Quellen")}</h2>
        <p class="small muted">${h("Ein Inventar kann mehrere Quellen haben. Sie werden zusammen an Ansible übergeben.")}</p>
        <div class="choice-row">
          <button class="btn pill" type="button" data-add-kind="ini">${h("+ Manuell")}</button>
          <button class="btn pill" type="button" data-add-kind="project">${h("+ Projekt")}</button>
          <button class="btn pill" type="button" data-add-kind="netbox">${h("+ NetBox")}</button>
        </div>
        <div id="source-drafts"></div>
      </div>
      <button class="btn pill primary" type="submit">${h("Anlegen")}</button>
    </form>`);
  bindSourcePicks(projects);
  document.querySelector("#inventory-create").addEventListener("submit", async (event) => {
    event.preventDefault();
    const form = event.currentTarget;
    try {
      const sources = [...document.querySelectorAll("#source-drafts [data-kind]")].map(readSourceDraft);
      const created = await api("POST", "/api/inventories", { name: form.name.value, sources });
      go(`#/inventories/${created.id}`);
    } catch (error) { handle(error); }
  });
}

async function pageInventory(id) {
  if (!needTeam()) return;
  const [item, projects] = await Promise.all([
    api("GET", `/api/inventories/${encodeURIComponent(id)}`),
    can("operator") ? api("GET", "/api/projects") : Promise.resolve([]),
  ]);
  const rows = (item.hosts || []).map((host) => {
    const vars = Object.entries(host.vars || {}).map(([key, value]) => `${key}=${value}`).join(" ");
    return `<tr><td class="mono">${esc(host.name)}</td><td>${esc((host.groups || []).join(", "))}</td><td class="mono">${esc(vars)}</td></tr>`;
  });
  const external = (item.sources || []).some((source) => source.kind === "project" || source.kind === "netbox");
  const sourceCards = (item.sources || []).map((source) => `
    <section class="card" style="margin-top:10px">
      <div class="spread">
        <div><strong>${esc(source.name)}</strong> <span class="small muted">${esc(sourceKindLabel(source.kind))} · ${esc(sourceSummary(source))}</span></div>
        <div class="row">
          ${can("operator") && source.kind !== "ini" ? `<button class="btn" type="button" data-sync-source="${esc(source.id)}">${h("Aktualisieren")}</button>` : ""}
          ${can("operator") ? `<button class="btn danger" type="button" data-drop-source="${esc(source.id)}">${h("Entfernen")}</button>` : ""}
        </div>
      </div>
      <p class="small muted">${h("{count} Hosts", { count: source.host_count })}</p>
      ${can("operator") && source.kind === "ini" ? `<form class="form" data-ini-source="${esc(source.id)}">
        <label class="field"><span>${h("Text")}</span><textarea name="body" required>${esc(source.body || "")}</textarea></label>
        <button class="btn" type="submit">${h("Quelle speichern")}</button>
      </form>` : `<pre class="mono">${esc(source.body || "")}</pre>`}
    </section>`).join("");
  shell(item.name, `
    <section class="card" style="margin-bottom:14px">
      <div class="spread">
        <h2>${h("Hosts")}</h2>
        ${external && can("operator") ? `<button class="btn" id="sync-inventory" type="button">${h("Alle Quellen aktualisieren")}</button>` : ""}
      </div>
      ${table([t("Name"), t("Gruppen"), t("Variablen")], rows)}
    </section>
    <section>
      <div class="spread">
        <h2>${h("Quellen")}</h2>
        ${can("operator") ? `<button class="btn danger" id="delete-inventory" type="button">${h("Inventar löschen")}</button>` : ""}
      </div>
      ${sourceCards || `<p class="small muted">${h("Noch keine Quelle.")}</p>`}
      ${can("operator") ? `<div class="card" style="margin-top:14px">
        <h2>${h("Quelle hinzufügen")}</h2>
        <div class="choice-row">
          <button class="btn pill" type="button" data-add-kind="ini">${h("+ Manuell")}</button>
          <button class="btn pill" type="button" data-add-kind="project">${h("+ Projekt")}</button>
          <button class="btn pill" type="button" data-add-kind="netbox">${h("+ NetBox")}</button>
        </div>
        <div id="source-drafts"></div>
        <button class="btn pill primary" id="add-sources" type="button" style="margin-top:12px">${h("Hinzufügen")}</button>
      </div>` : ""}
    </section>`,
    can("operator") ? `<a class="btn pill" href="#/inventories/new">${h("+ Inventar")}</a>` : "");
  const sync = document.querySelector("#sync-inventory");
  if (sync) {
    sync.addEventListener("click", async () => {
      sync.disabled = true;
      try {
        await api("POST", `/api/inventories/${encodeURIComponent(id)}/sync`);
        route();
      } catch (error) {
        sync.disabled = false;
        handle(error);
      }
    });
  }
  document.querySelectorAll("[data-sync-source]").forEach((button) => {
    button.addEventListener("click", async () => {
      button.disabled = true;
      try {
        await api("POST", `/api/inventories/${encodeURIComponent(id)}/sources/${encodeURIComponent(button.dataset.syncSource)}/sync`);
        route();
      } catch (error) {
        button.disabled = false;
        handle(error);
      }
    });
  });
  document.querySelectorAll("[data-drop-source]").forEach((button) => {
    button.addEventListener("click", async () => {
      if (!confirm(t("Quelle entfernen?"))) return;
      try {
        await api("DELETE", `/api/inventories/${encodeURIComponent(id)}/sources/${encodeURIComponent(button.dataset.dropSource)}`);
        route();
      } catch (error) { handle(error); }
    });
  });
  document.querySelectorAll("[data-ini-source]").forEach((form) => {
    form.addEventListener("submit", async (event) => {
      event.preventDefault();
      try {
        await api("PUT", `/api/inventories/${encodeURIComponent(id)}/sources/${encodeURIComponent(form.dataset.iniSource)}`, {
          body: form.body.value,
        });
        route();
      } catch (error) { handle(error); }
    });
  });
  if (can("operator")) {
    bindSourcePicks(projects);
    document.querySelector("#add-sources").addEventListener("click", async () => {
      const drafts = [...document.querySelectorAll("#source-drafts [data-kind]")].map(readSourceDraft);
      if (!drafts.length) return;
      try {
        for (const source of drafts) {
          await api("POST", `/api/inventories/${encodeURIComponent(id)}/sources`, source);
        }
        route();
      } catch (error) { handle(error); }
    });
    document.querySelector("#delete-inventory").addEventListener("click", async () => {
      if (!confirm(t("Inventar löschen?"))) return;
      try {
        await api("DELETE", `/api/inventories/${encodeURIComponent(id)}`);
        go("#/inventories");
      } catch (error) { handle(error); }
    });
  }
}
const CREDENTIAL_KINDS = [
  ["ssh", "Maschine (SSH)", "Schlüssel oder Passwort, auch beides."],
  ["vault", "Ansible Vault", ""],
  ["scm", "Quellcode", "Gilt im Auftrag. Der Projekt-Sync nutzt diesen Zugang noch nicht."],
  ["net", "Netzwerk", ""],
  ["aws", "Amazon Web Services", ""],
  ["azure", "Microsoft Azure", ""],
  ["gce", "Google Compute Engine", ""],
  ["vmware", "VMware vCenter", ""],
  ["openstack", "OpenStack", "Leer setzt die Domain auf Default."],
  ["registry", "Container-Registry", "Der Auftrag sieht Host, Benutzer und Passwort. Docker selbst wird nicht angemeldet."],
  ["github", "GitHub-Token", ""],
  ["gitlab", "GitLab-Token", ""],
  ["galaxy", "Ansible Galaxy", "Leer lässt https://galaxy.ansible.com."],
  ["kubernetes", "Kubernetes", ""],
  ["insights", "Red Hat Insights", ""],
  ["env", "Umgebungsvariablen", ""],
  ["netbox", "NetBox", ""],
];

function credentialKindLabel(kind) {
  const row = CREDENTIAL_KINDS.find((item) => item[0] === kind);
  return row ? t(row[1]) : kind;
}

function selectedCredentialIds(item) {
  if (Array.isArray(item.credential_ids)) return item.credential_ids;
  return item.credential_id ? [item.credential_id] : [];
}

function templateCredentialFields(credentials, selectedIds) {
  const selected = new Set(selectedIds || []);
  const present = new Set(credentials.map((row) => row.kind));
  const selects = CREDENTIAL_KINDS
    .map((row) => row[0])
    .filter((kind) => kind !== "vault" && present.has(kind))
    .map((kind) => {
      const rows = credentials.filter((row) => row.kind === kind);
      const current = rows.find((row) => selected.has(row.id));
      return `<label class="field"><span>${esc(credentialKindLabel(kind))}</span><select class="fit" data-credential-kind="${esc(kind)}">${selectOptions(rows, current ? current.id : "", true, (row) => row.name)}</select></label>`;
    })
    .join("");
  const grid = selects ? `<div class="field-grid">${selects}</div>` : "";
  const empty = credentials.length ? "" : `<p class="small muted">${h("Keine Zugänge im Team.")}</p>`;
  return `${empty}<p class="small muted">${h("Pro Art ein Zugang. Vault-Zugänge wählst du über Auswählen. Sie stehen hier als Pillen.")}</p>${grid}
    <div class="field">
      <span>${esc(credentialKindLabel("vault"))}</span>
      <div class="vault-row">
        <div class="chips" id="vault-pills"></div>
        <button class="btn pill" id="vault-add" type="button">${h("Auswählen")}</button>
      </div>
    </div>`;
}

function renderVaultPills(box, credentials, ids) {
  const byId = new Map(credentials.map((row) => [row.id, row]));
  box.innerHTML = ids.map((id) => {
    const row = byId.get(id);
    if (!row) return "";
    return `<span class="chip" data-vault-id="${esc(id)}"><span class="chip-text">${esc(row.name)}</span><button class="chip-remove" type="button" aria-label="${h("Vault-Zugang {name} entfernen", { name: row.name })}">×</button></span>`;
  }).join("");
  box.querySelectorAll(".chip-remove").forEach((button) => {
    button.addEventListener("click", () => {
      const chip = button.closest(".chip");
      const id = chip ? chip.dataset.vaultId : "";
      const next = [...box.querySelectorAll("[data-vault-id]")]
        .map((node) => node.dataset.vaultId)
        .filter((item) => item !== id);
      renderVaultPills(box, credentials, next);
    });
  });
}

function closeVaultPicker() {
  const overlay = document.querySelector("#vault-picker");
  if (overlay) overlay.remove();
  if (closeVaultPicker.onKey) {
    document.removeEventListener("keydown", closeVaultPicker.onKey);
    closeVaultPicker.onKey = null;
  }
}

function openVaultPicker(credentials, selected, onApply) {
  closeVaultPicker();
  const chosen = new Set(selected);
  const vaults = credentials
    .filter((row) => row.kind === "vault")
    .slice()
    .sort((left, right) => left.name.localeCompare(right.name, "de"));
  const rows = vaults.map((row) => `
    <label class="vault-choice${chosen.has(row.id) ? " selected" : ""}">
      <input type="checkbox" value="${esc(row.id)}"${chosen.has(row.id) ? " checked" : ""}>
      <span>${esc(row.name)}</span>
    </label>`).join("");
  const body = vaults.length
    ? `<input class="search" id="vault-search" placeholder="${h("Vaults suchen")}" autocomplete="off">
       <div class="vault-choices" id="vault-choices">${rows}</div>
       <div class="row">
         <button class="btn pill primary" type="button" id="vault-apply">${h("Übernehmen")}</button>
         <button class="btn" type="button" id="vault-cancel">${h("Abbrechen")}</button>
       </div>`
    : `<p class="muted">${h("Keine Vault-Zugänge im Team.")}</p>`;
  document.querySelector(".main").insertAdjacentHTML("beforeend", `
    <div class="overlay" id="vault-picker">
      <div class="overlay-card vault-picker" role="dialog" aria-modal="true" aria-labelledby="vault-picker-title">
        <div class="spread">
          <h2 id="vault-picker-title">${esc(credentialKindLabel("vault"))}</h2>
          <button class="btn" type="button" id="vault-picker-close">${h("Schließen")}</button>
        </div>
        ${body}
      </div>
    </div>`);
  const overlay = document.querySelector("#vault-picker");
  const dismiss = () => closeVaultPicker();
  overlay.addEventListener("click", (event) => {
    if (event.target === overlay) dismiss();
  });
  document.querySelector("#vault-picker-close").addEventListener("click", dismiss);
  const cancel = document.querySelector("#vault-cancel");
  if (cancel) cancel.addEventListener("click", dismiss);
  closeVaultPicker.onKey = (event) => {
    if (event.key === "Escape") dismiss();
  };
  document.addEventListener("keydown", closeVaultPicker.onKey);
  document.querySelectorAll(".vault-choice").forEach((label) => {
    const input = label.querySelector("input");
    input.addEventListener("change", () => label.classList.toggle("selected", input.checked));
  });
  const search = document.querySelector("#vault-search");
  if (search) {
    search.addEventListener("input", () => {
      const query = search.value.trim().toLowerCase();
      document.querySelectorAll(".vault-choice").forEach((label) => {
        const name = label.querySelector("span").textContent.toLowerCase();
        label.hidden = query !== "" && !name.includes(query);
      });
    });
    if (typeof search.focus === "function") search.focus();
  }
  const apply = document.querySelector("#vault-apply");
  if (!apply) return;
  apply.addEventListener("click", () => {
    const checked = [...document.querySelectorAll("#vault-choices input")]
      .filter((input) => input.checked)
      .map((input) => input.value);
    const kept = selected.filter((id) => checked.includes(id));
    const added = checked.filter((id) => !selected.includes(id));
    onApply([...kept, ...added]);
    closeVaultPicker();
  });
}

function bindVaultPicker(form, credentials, selectedIds) {
  const box = form.querySelector("#vault-pills");
  const open = form.querySelector("#vault-add");
  if (!box || !open) return;
  const vaultIds = new Set(credentials.filter((row) => row.kind === "vault").map((row) => row.id));
  renderVaultPills(box, credentials, (selectedIds || []).filter((id) => vaultIds.has(id)));
  open.addEventListener("click", () => {
    const current = [...box.querySelectorAll("[data-vault-id]")].map((node) => node.dataset.vaultId);
    openVaultPicker(credentials, current, (ids) => renderVaultPills(box, credentials, ids));
  });
}

function credText(name, label, type) {
  const inputType = type || "text";
  const complete = inputType === "password" ? "new-password" : "off";
  return `<label class="field"><span>${h(label)}</span><input name="${name}" type="${inputType}" autocomplete="${complete}"></label>`;
}

function credArea(name, label) {
  return `<label class="field"><span>${h(label)}</span><textarea name="${name}" autocomplete="off" spellcheck="false"></textarea></label>`;
}

function credCheck(name, label, checked) {
  return `<label class="field check"><span>${h(label)}</span><input name="${name}" type="checkbox"${checked ? " checked" : ""}></label>`;
}

function credHint(text) {
  return text ? `<p class="small muted">${h(text)}</p>` : "";
}

function becomeSelect() {
  const methods = ["sudo", "su", "pbrun", "pfexec", "doas", "dzdo", "ksu", "runas"];
  const options = [`<option value="">${h("Keine")}</option>`].concat(
    methods.map((method) => `<option value="${method}">${esc(method)}</option>`)
  );
  return `<label class="field"><span>${h("Become-Methode")}</span><select class="fit" name="become_method">${options.join("")}</select></label>`;
}

function credentialFields(kind) {
  const row = CREDENTIAL_KINDS.find((item) => item[0] === kind);
  const hint = row ? row[2] : "";
  if (kind === "ssh") {
    return [
      credHint(hint),
      credText("username", "Benutzer"),
      credText("password", "Passwort", "password"),
      credArea("private_key", "Privater Schlüssel"),
      credText("passphrase", "Passphrase", "password"),
      becomeSelect(),
      credText("become_username", "Become-Benutzer"),
      credText("become_password", "Become-Passwort", "password"),
    ].join("");
  }
  if (kind === "scm") {
    return [
      credHint(hint),
      credText("username", "Benutzer"),
      credText("password", "Passwort", "password"),
      credArea("private_key", "Privater Schlüssel"),
      credText("passphrase", "Passphrase", "password"),
    ].join("");
  }
  if (kind === "vault") {
    return [credText("password", "Vault-Passwort", "password"), credText("vault_id", "Vault-Id")].join("");
  }
  if (kind === "net") {
    return [
      credText("username", "Benutzer"),
      credText("password", "Passwort", "password"),
      credArea("private_key", "Privater Schlüssel"),
      credCheck("authorize", "Authorize", false),
      credText("authorize_password", "Authorize-Passwort", "password"),
    ].join("");
  }
  if (kind === "aws") {
    return [
      credText("access_key", "Access Key"),
      credText("secret_key", "Secret Key", "password"),
      credText("security_token", "Session-Token", "password"),
    ].join("");
  }
  if (kind === "azure") {
    return [
      credText("subscription_id", "Subscription"),
      credText("client_id", "Client"),
      credText("secret", "Geheimnis", "password"),
      credText("tenant", "Tenant"),
    ].join("");
  }
  if (kind === "gce") return credArea("service_account", "Dienstkonto");
  if (kind === "vmware") {
    return [
      credText("host", "Host"),
      credText("username", "Benutzer"),
      credText("password", "Passwort", "password"),
    ].join("");
  }
  if (kind === "openstack") {
    return [
      credHint(hint),
      credText("auth_url", "API-Adresse"),
      credText("username", "Benutzer"),
      credText("password", "Passwort", "password"),
      credText("project", "Projekt"),
      credText("domain", "Domain"),
    ].join("");
  }
  if (kind === "registry") {
    return [
      credHint(hint),
      credText("host", "Host"),
      credText("username", "Benutzer"),
      credText("password", "Passwort", "password"),
    ].join("");
  }
  if (kind === "github" || kind === "gitlab") return credText("token", "Token", "password");
  if (kind === "galaxy") {
    return [credHint(hint), credText("url", "API-Adresse"), credText("token", "Token", "password")].join("");
  }
  if (kind === "kubernetes") {
    return [
      credText("host", "API-Adresse"),
      credText("token", "Token", "password"),
      credCheck("verify_ssl", "Zertifikat prüfen", true),
    ].join("");
  }
  if (kind === "insights") {
    return [credText("username", "Benutzer"), credText("password", "Passwort", "password")].join("");
  }
  if (kind === "netbox") {
    return [credText("url", "API-Adresse"), credText("token", "Token", "password")].join("");
  }
  return credArea("env", "KEY=Wert, eine Zeile je Variable");
}

function inputText(form, name) {
  const field = form.elements[name];
  return field && "value" in field ? field.value.trim() : "";
}

function credentialSecret(form) {
  const kind = form.kind.value;
  if (kind === "env") {
    const secret = {};
    form.env.value.split("\n").forEach((line) => {
      const trimmed = line.trim();
      if (!trimmed) return;
      const eq = trimmed.indexOf("=");
      if (eq < 1) throw new Error("Env-Zeile braucht KEY=Wert.");
      secret[trimmed.slice(0, eq)] = trimmed.slice(eq + 1);
    });
    return secret;
  }
  const names = {
    ssh: ["username", "password", "private_key", "passphrase", "become_method", "become_username", "become_password"],
    vault: ["password", "vault_id"],
    scm: ["username", "password", "private_key", "passphrase"],
    net: ["username", "password", "private_key", "authorize_password"],
    aws: ["access_key", "secret_key", "security_token"],
    azure: ["subscription_id", "client_id", "secret", "tenant"],
    gce: ["service_account"],
    vmware: ["host", "username", "password"],
    openstack: ["auth_url", "username", "password", "project", "domain"],
    registry: ["host", "username", "password"],
    github: ["token"],
    gitlab: ["token"],
    galaxy: ["url", "token"],
    kubernetes: ["host", "token"],
    insights: ["username", "password"],
    netbox: ["token", "url"],
  };
  const secret = {};
  (names[kind] || []).forEach((name) => {
    const value = inputText(form, name);
    if (value) secret[name] = value;
  });
  if (kind === "net" && form.authorize && form.authorize.checked) secret.authorize = true;
  if (kind === "kubernetes") secret.verify_ssl = Boolean(form.verify_ssl && form.verify_ssl.checked);
  return secret;
}

function bindCredentialKind(form) {
  const box = document.querySelector("#credential-fields");
  const draw = () => {
    box.innerHTML = credentialFields(form.kind.value);
  };
  form.kind.addEventListener("change", draw);
  draw();
}

async function pageCredentials() {
  if (!needTeam()) return;
  const credentials = await api("GET", "/api/credentials");
  shell(t("Zugänge"), `
    <section class="card">
      <p class="small muted">${h("Geheimnisse werden verschlüsselt gespeichert und nicht wieder angezeigt.")}</p>
      <input class="search" id="credential-search" placeholder="${h("Zugänge suchen")}" autocomplete="off">
      <div id="credential-table"></div>
    </section>`,
    can("owner") ? addLink("#/credentials/new", "+ Zugang") : "");
  const drawCredentials = () => {
    const needle = document.querySelector("#credential-search").value.trim().toLowerCase();
    const rows = credentials.filter((item) => !needle || `${item.name} ${item.kind} ${credentialKindLabel(item.kind)}`.toLowerCase().includes(needle)).map((item) =>
      `<tr><td>${esc(item.name)}</td><td>${esc(credentialKindLabel(item.kind))}</td><td>${can("owner") ? `<button class="btn danger" type="button" data-credential="${esc(item.id)}">${h("Löschen")}</button>` : ""}</td></tr>`
    );
    document.querySelector("#credential-table").innerHTML = table([t("Name"), t("Art"), ""], rows);
    document.querySelectorAll("[data-credential]").forEach((button) => {
      button.addEventListener("click", async () => {
        try {
          await api("DELETE", `/api/credentials/${encodeURIComponent(button.dataset.credential)}`);
          route();
        } catch (error) { handle(error); }
      });
    });
  };
  document.querySelector("#credential-search").addEventListener("input", drawCredentials);
  drawCredentials();
}

async function pageCredentialNew() {
  if (!needTeam()) return;
  if (!can("owner")) {
    deny("Zugang", "owner");
    return;
  }
  const kindOptions = CREDENTIAL_KINDS.map(([id, label]) => `<option value="${esc(id)}">${h(label)}</option>`).join("");
  shell(t("Zugang anlegen"), `
    <form class="card form" id="credential-form">
      <label class="field"><span>${h("Name")}</span><input name="name" required></label>
      <div class="field-row">
        <label class="field"><span>${h("Art")}</span><select class="fit" name="kind">${kindOptions}</select></label>
      </div>
      <div id="credential-fields"></div>
      <button class="btn pill primary" type="submit">${h("Anlegen")}</button>
    </form>`);
  const form = document.querySelector("#credential-form");
  bindCredentialKind(form);
  form.addEventListener("submit", async (event) => {
    event.preventDefault();
    try {
      await api("POST", "/api/credentials", {
        name: form.name.value,
        kind: form.kind.value,
        secret: credentialSecret(form),
      });
      go("#/credentials");
    } catch (error) { handle(error); }
  });
}

async function pageOrganizations() {
  const items = await api("GET", "/api/organizations");
  shell(t("Organisationen"), `
    <section class="card">
      <input class="search" id="organization-search" placeholder="${h("Organisationen suchen")}" autocomplete="off">
      <div id="organization-table"></div>
    </section>`,
    me.is_admin ? addLink("#/organizations/new", "+ Organisation") : "");
  const drawOrganizations = () => {
    const needle = document.querySelector("#organization-search").value.trim().toLowerCase();
    const rows = items.filter((item) => !needle || `${item.name} ${item.description || ""}`.toLowerCase().includes(needle)).map((item) =>
      `<tr><td><a href="#/organizations/${esc(item.id)}">${esc(item.name)}</a></td><td>${esc(item.description || "")}</td></tr>`
    );
    document.querySelector("#organization-table").innerHTML = table([t("Name"), t("Beschreibung")], rows);
  };
  document.querySelector("#organization-search").addEventListener("input", drawOrganizations);
  drawOrganizations();
}

async function pageOrganizationNew() {
  if (!me.is_admin) {
    deny("Organisation", "admin");
    return;
  }
  shell(t("Organisation anlegen"), `
    <form class="card form" id="org-form">
      <label class="field"><span>${h("Name")}</span><input name="name" required></label>
      <label class="field"><span>${h("Beschreibung")}</span><input name="description"></label>
      <button class="btn pill primary" type="submit">${h("Anlegen")}</button>
    </form>`);
  document.querySelector("#org-form").addEventListener("submit", async (event) => {
    event.preventDefault();
    const form = event.currentTarget;
    try {
      await api("POST", "/api/organizations", { name: form.name.value, description: form.description.value });
      go("#/organizations");
    } catch (error) { handle(error); }
  });
}

async function pageOrganization(id) {
  const item = await api("GET", `/api/organizations/${encodeURIComponent(id)}`);
  const rows = (item.teams || []).map((team) =>
    `<tr><td>${esc(team.name)}</td><td>${esc(team.description || "")}</td><td class="small muted">${esc(team.ldap_group || "")}</td></tr>`
  );
  shell(item.name, `
    <section class="card" style="margin-bottom:14px">
      ${me.is_admin ? `<form class="form" id="org-edit">
        <label class="field"><span>${h("Name")}</span><input name="name" value="${esc(item.name)}" required></label>
        <label class="field"><span>${h("Beschreibung")}</span><input name="description" value="${esc(item.description || "")}"></label>
        <button class="btn pill primary" type="submit">${h("Speichern")}</button>
      </form>` : `<p class="muted">${esc(item.description || "")}</p>`}
    </section>
    <section class="card">
      <h2>${h("Teams")}</h2>
      ${table([t("Name"), t("Beschreibung"), t("LDAP-Gruppe")], rows)}
    </section>`);
  const form = document.querySelector("#org-edit");
  if (form) {
    form.addEventListener("submit", async (event) => {
      event.preventDefault();
      try {
        await api("PUT", `/api/organizations/${encodeURIComponent(id)}`, {
          name: form.name.value,
          description: form.description.value,
        });
        me = await api("GET", "/api/me");
        route();
      } catch (error) { handle(error); }
    });
  }
}

async function pageTeam() {
  const teams = me.teams || [];
  const current = teams.find((team) => team.id === teamId);
  const organizations = me.is_admin ? await api("GET", "/api/organizations") : [];
  let members = [];
  if (teamId) members = await api("GET", `/api/teams/${encodeURIComponent(teamId)}/members`);
  const memberRows = members.map((member) => {
    const name = me.is_admin
      ? `<a href="#/users/${esc(member.user_id)}">${esc(member.display_name)}</a>`
      : esc(member.display_name);
    const roleCell = member.source === "manual" && can("owner")
      ? `<select data-member-role="${esc(member.username)}" data-current="${esc(member.role)}">${roleOptions(member.role)}</select>`
      : esc(roleLabel(member.role));
    const sourceCell = member.source === "ldap"
      ? `${esc(sourceLabel(member.source))} <span class="small muted">${h("Kommt aus dem Verzeichnis.")}</span>`
      : esc(sourceLabel(member.source));
    const remove = can("owner")
      ? `<button class="btn danger" type="button" data-member="${esc(member.user_id)}">${h("Entfernen")}</button>`
      : "";
    return `<tr><td>${name} <span class="small muted">${esc(member.username)}</span></td><td>${roleCell}</td><td>${sourceCell}</td><td>${remove}</td></tr>`;
  });
  const actions = [
    teamId && can("owner") ? addLink("#/team/members/new", "+ Mitglied") : "",
    me.is_admin ? addLink("#/team/new", "+ Team") : "",
    me.is_admin ? addLink("#/users/new", "+ Benutzer") : "",
  ].join("");
  shell(t("Team"), `
    ${current ? `<section class="card" style="margin-bottom:14px">
      <h2>${esc(current.name)}</h2>
      <p class="muted">${esc(current.description || "")}</p>
      <p class="small">${h("Organisation: {name}", { name: current.organization_name || "—" })}</p>
      ${can("owner") ? `<form class="form" id="team-form">
        <label class="field"><span>${h("Name")}</span><input name="name" value="${esc(current.name)}" required></label>
        <label class="field"><span>${h("Beschreibung")}</span><input name="description" value="${esc(current.description || "")}"></label>
        <label class="field"><span>${h("LDAP- oder AD-Gruppe")}</span><input name="ldap_group" value="${esc(current.ldap_group || "")}" placeholder="${h("CN oder DN, zum Beispiel Plattform")}"></label>
        ${me.is_admin ? `<label class="field"><span>${h("Organisation verschieben")}</span><select name="organization_id">${orgOptions(organizations, current.organization_id)}</select></label>` : ""}
        <button class="btn pill primary" type="submit">${h("Team speichern")}</button>
      </form>` : `<p class="small">${h("LDAP-Gruppe: {group}", { group: current.ldap_group || "—" })}</p>`}
    </section>
    <section class="card">
      <h2>${h("Mitglieder")}</h2>
      ${roleLegend()}
      ${table([t("Name"), t("Rolle"), t("Herkunft"), ""], memberRows)}
    </section>` : `<div class="card">${h("Kein Team ausgewählt.")}</div>`}`, actions);
  const teamForm = document.querySelector("#team-form");
  if (teamForm) {
    teamForm.addEventListener("submit", async (event) => {
      event.preventDefault();
      try {
        const payload = {
          name: teamForm.name.value,
          description: teamForm.description.value,
          ldap_group: teamForm.ldap_group.value,
        };
        if (teamForm.organization_id) payload.organization_id = teamForm.organization_id.value;
        await api("PUT", `/api/teams/${encodeURIComponent(teamId)}`, payload);
        me = await api("GET", "/api/me");
        route();
      } catch (error) { handle(error); }
    });
  }
  document.querySelectorAll("[data-member-role]").forEach((select) => {
    select.addEventListener("change", async () => {
      const next = select.value;
      try {
        await api("POST", `/api/teams/${encodeURIComponent(teamId)}/members`, {
          username: select.dataset.memberRole,
          role: next,
        });
        select.dataset.current = next;
      } catch (error) {
        select.value = select.dataset.current;
        handle(error);
      }
    });
  });
  document.querySelectorAll("[data-member]").forEach((button) => {
    button.addEventListener("click", async () => {
      try {
        await api("DELETE", `/api/teams/${encodeURIComponent(teamId)}/members/${encodeURIComponent(button.dataset.member)}`);
        route();
      } catch (error) { handle(error); }
    });
  });
}

async function pageMemberNew() {
  if (!teamId) {
    shell(t("Team"), `<div class="card">${h("Kein Team ausgewählt.")}</div>`);
    return;
  }
  if (!can("owner")) {
    deny("Mitglieder", "owner");
    return;
  }
  shell(t("Mitglied hinzufügen"), `
    <form class="card form" id="member-form">
      <label class="field"><span>${h("Benutzername")}</span><input name="username" required></label>
      <div class="field-row">
        <label class="field"><span>${h("Rolle")}</span>
          <select class="fit" name="role"><option value="viewer">${h("Betrachter")}</option><option value="operator" selected>${h("Operator")}</option><option value="owner">${h("Besitzer")}</option></select>
        </label>
      </div>
      <button class="btn pill primary" type="submit">${h("Hinzufügen")}</button>
    </form>`);
  document.querySelector("#member-form").addEventListener("submit", async (event) => {
    event.preventDefault();
    const form = event.currentTarget;
    try {
      await api("POST", `/api/teams/${encodeURIComponent(teamId)}/members`, {
        username: form.username.value,
        role: form.role.value,
      });
      go("#/team");
    } catch (error) { handle(error); }
  });
}

async function pageTeamNew() {
  if (!me.is_admin) {
    deny("Team", "admin");
    return;
  }
  const organizations = await api("GET", "/api/organizations");
  const current = (me.teams || []).find((team) => team.id === teamId);
  shell(t("Team anlegen"), `
    <form class="card form" id="create-team">
      <label class="field"><span>${h("Name")}</span><input name="name" required></label>
      <label class="field"><span>${h("Beschreibung")}</span><input name="description"></label>
      <label class="field"><span>${h("Organisation")}</span><select name="organization_id"><option value="">${h("Neue Organisation mit dem Teamnamen")}</option>${orgOptions(organizations, current && current.organization_id)}</select></label>
      <label class="field"><span>${h("LDAP-Gruppe")}</span><input name="ldap_group"></label>
      <button class="btn pill primary" type="submit">${h("Anlegen")}</button>
    </form>`);
  document.querySelector("#create-team").addEventListener("submit", async (event) => {
    event.preventDefault();
    const form = event.currentTarget;
    try {
      const payload = {
        name: form.name.value,
        description: form.description.value,
        ldap_group: form.ldap_group.value,
      };
      if (form.organization_id && form.organization_id.value) payload.organization_id = form.organization_id.value;
      const created = await api("POST", "/api/teams", payload);
      me = await api("GET", "/api/me");
      teamId = created.id;
      localStorage.setItem("koreograf.team", teamId);
      go("#/team");
    } catch (error) { handle(error); }
  });
}

function emailLine(email) {
  return email ? esc(email) : `<span class="muted">${h("Keine E-Mail hinterlegt.")}</span>`;
}

async function pageAccount() {
  me = await api("GET", "/api/me");
  const rows = (me.memberships || []).map((row) => {
    const org = row.organization_name || "—";
    return `<tr><td>${esc(org)}</td><td>${esc(row.team_name)}</td><td>${esc(roleLabel(row.role))}</td><td>${esc(sourceLabel(row.source))}</td></tr>`;
  });
  const local = me.auth_source === "local";
  const identity = local ? `
    <form class="card form" id="account-form">
      <h2>${h("Konto")}</h2>
      <p class="small muted">${h("Benutzername")}: ${esc(me.username)} · ${esc(authSourceLabel(me.auth_source))}${me.is_admin ? ` · ${h("Administrator")}` : ""}</p>
      <label class="field"><span>${h("Anzeigename")}</span><input name="display_name" value="${esc(me.display_name)}" required></label>
      <label class="field"><span>${h("E-Mail")}</span><input name="email" type="email" value="${esc(me.email || "")}" autocomplete="email"></label>
      <label class="field"><span>${h("Neues Passwort")}</span><input name="password" type="password" minlength="8" autocomplete="new-password" placeholder="${h("Leer lassen, um das Passwort zu behalten.")}"></label>
      <button class="btn pill primary" type="submit">${h("Speichern")}</button>
    </form>` : `
    <section class="card">
      <h2>${h("Konto")}</h2>
      <p class="small muted">${h("Benutzername")}: ${esc(me.username)} · ${esc(authSourceLabel(me.auth_source))}${me.is_admin ? ` · ${h("Administrator")}` : ""}</p>
      <p>${h("Anzeigename")}: ${esc(me.display_name)}</p>
      <p>${h("E-Mail")}: ${emailLine(me.email)}</p>
      <p class="muted">${h("Name und E-Mail kommen aus dem Verzeichnis.")}</p>
    </section>`;
  shell(t("Mein Konto"), `
    <div class="stack">
      ${identity}
      <section class="card">
        <h2>${h("Mitgliedschaften")}</h2>
        ${table([t("Organisation"), t("Team"), t("Rolle"), t("Herkunft")], rows)}
      </section>
    </div>`);
  const form = document.querySelector("#account-form");
  if (form) {
    form.addEventListener("submit", async (event) => {
      event.preventDefault();
      try {
        const payload = {
          display_name: form.display_name.value,
          email: form.email.value,
        };
        if (form.password.value) payload.password = form.password.value;
        me = await api("PUT", "/api/me", payload);
        route();
      } catch (error) { handle(error); }
    });
  }
}

async function pageUsers() {
  if (!me.is_admin) {
    deny("Benutzerkonten", "admin");
    return;
  }
  const items = await api("GET", "/api/users");
  shell(t("Benutzerkonten"), `
    <section class="card">
      <input class="search" id="user-search" placeholder="${h("Benutzer suchen")}" autocomplete="off">
      <div id="user-table"></div>
    </section>`,
    addLink("#/users/new", "+ Benutzer"));
  const draw = () => {
    const needle = document.querySelector("#user-search").value.trim().toLowerCase();
    const shown = items.filter((item) => {
      if (!needle) return true;
      return `${item.display_name} ${item.username} ${item.email || ""} ${item.auth_source}`.toLowerCase().includes(needle);
    });
    const rows = shown.map((item) => {
      const flags = [
        item.is_admin ? `<span class="pill">${h("Administrator")}</span>` : "",
        item.disabled ? `<span class="pill failed">${h("Deaktiviert")}</span>` : "",
      ].filter(Boolean).join(" ");
      const when = String(item.created_at || "").slice(0, 10);
      return `<tr>
        <td><a href="#/users/${esc(item.id)}">${esc(item.display_name)}</a> ${flags}</td>
        <td>${esc(item.username)}</td>
        <td>${esc(authSourceLabel(item.auth_source))}</td>
        <td>${esc(item.team_count)}</td>
        <td class="small muted">${esc(when)}</td>
      </tr>`;
    });
    document.querySelector("#user-table").innerHTML = table(
      [t("Name"), t("Benutzername"), t("Herkunft"), t("Teams"), t("Angelegt")],
      rows
    );
  };
  document.querySelector("#user-search").addEventListener("input", draw);
  draw();
}

async function pageUser(id) {
  if (!me.is_admin) {
    deny("Benutzerkonten", "admin");
    return;
  }
  const item = await api("GET", `/api/users/${encodeURIComponent(id)}`);
  const teams = await api("GET", "/api/teams");
  const memberIds = new Set((item.memberships || []).map((row) => row.team_id));
  const openTeams = teams.filter((team) => !memberIds.has(team.id));
  const self = item.id === me.id;
  const rows = (item.memberships || []).map((row) => {
    const org = row.organization_name ? `${row.organization_name} · ` : "";
    const roleCell = row.source === "manual"
      ? `<select data-role="${esc(row.team_id)}" data-current="${esc(row.role)}">${roleOptions(row.role)}</select>`
      : `${esc(roleLabel(row.role))} <span class="small muted">${h("Kommt aus dem Verzeichnis.")}</span>`;
    const remove = row.source === "manual"
      ? `<button class="btn danger" type="button" data-leave="${esc(row.team_id)}">${h("Entfernen")}</button>`
      : "";
    return `<tr><td>${esc(org + row.team_name)}</td><td>${roleCell}</td><td>${esc(sourceLabel(row.source))}</td><td>${remove}</td></tr>`;
  });
  const passwordField = item.auth_source === "local"
    ? `<label class="field"><span>${h("Neues Passwort")}</span><input name="password" type="password" minlength="8" autocomplete="new-password" placeholder="${h("Leer lassen, um das Passwort zu behalten.")}"></label>`
    : `<p class="small muted">${h("Das Passwort kommt aus LDAP oder Active Directory.")}</p>`;
  const teamChoices = openTeams.map((team) => {
    const label = team.organization_name ? `${team.organization_name} · ${team.name}` : team.name;
    return `<option value="${esc(team.id)}">${esc(label)}</option>`;
  }).join("");
  shell(item.display_name, `
    <div class="stack">
      <form class="card form" id="user-form">
        <h2>${h("Konto")}</h2>
        <p class="small muted">${h("Benutzername")}: ${esc(item.username)} · ${esc(authSourceLabel(item.auth_source))}</p>
        <p class="small">${h("E-Mail")}: ${emailLine(item.email)}</p>
        <label class="field"><span>${h("Anzeigename")}</span><input name="display_name" value="${esc(item.display_name)}" required></label>
        <label class="row"><input name="is_admin" type="checkbox"${item.is_admin ? " checked" : ""}> ${h("Administrator")}</label>
        <label class="row"><input name="account_disabled" type="checkbox"${item.disabled ? " checked" : ""}${self ? " disabled" : ""}> ${h("Konto deaktivieren")}</label>
        ${passwordField}
        <div class="row">
          <button class="btn pill primary" type="submit">${h("Speichern")}</button>
          ${self ? "" : `<button class="btn danger" id="delete-user" type="button">${h("Löschen")}</button>`}
        </div>
      </form>
      <section class="card">
        <h2>${h("Rechte")}</h2>
        ${roleLegend()}
      </section>
      <section class="card">
        <h2>${h("Mitgliedschaften")}</h2>
        ${table([t("Team"), t("Rolle"), t("Herkunft"), ""], rows)}
        ${openTeams.length ? `<form class="form" id="grant-form">
          <h2>${h("Zum Team hinzufügen")}</h2>
          <label class="field"><span>${h("Team")}</span><select name="team_id">${teamChoices}</select></label>
          <div class="field-row">
            <label class="field"><span>${h("Rolle")}</span><select class="fit" name="role">${roleOptions("operator")}</select></label>
          </div>
          <button class="btn pill primary" type="submit">${h("Hinzufügen")}</button>
        </form>` : `<p class="muted">${h("Der Benutzer ist in jedem Team.")}</p>`}
      </section>
    </div>`);
  document.querySelector("#user-form").addEventListener("submit", async (event) => {
    event.preventDefault();
    const form = event.currentTarget;
    try {
      const payload = {
        display_name: form.display_name.value,
        is_admin: form.is_admin.checked,
        disabled: form.account_disabled.checked,
      };
      if (form.password && form.password.value) payload.password = form.password.value;
      await api("PUT", `/api/users/${encodeURIComponent(id)}`, payload);
      if (self) me = await api("GET", "/api/me");
      route();
    } catch (error) { handle(error); }
  });
  const deleteButton = document.querySelector("#delete-user");
  if (deleteButton) {
    deleteButton.addEventListener("click", async () => {
      if (!window.confirm(t("Dieses Konto wirklich löschen?"))) return;
      try {
        await api("DELETE", `/api/users/${encodeURIComponent(id)}`);
        go("#/users");
      } catch (error) { handle(error); }
    });
  }
  document.querySelectorAll("[data-role]").forEach((select) => {
    select.addEventListener("change", async () => {
      const next = select.value;
      try {
        await api("POST", `/api/teams/${encodeURIComponent(select.dataset.role)}/members`, {
          username: item.username,
          role: next,
        });
        select.dataset.current = next;
      } catch (error) {
        select.value = select.dataset.current;
        handle(error);
      }
    });
  });
  document.querySelectorAll("[data-leave]").forEach((button) => {
    button.addEventListener("click", async () => {
      try {
        await api("DELETE", `/api/teams/${encodeURIComponent(button.dataset.leave)}/members/${encodeURIComponent(item.id)}`);
        route();
      } catch (error) { handle(error); }
    });
  });
  const grant = document.querySelector("#grant-form");
  if (grant) {
    grant.addEventListener("submit", async (event) => {
      event.preventDefault();
      try {
        await api("POST", `/api/teams/${encodeURIComponent(grant.team_id.value)}/members`, {
          username: item.username,
          role: grant.role.value,
        });
        route();
      } catch (error) { handle(error); }
    });
  }
}

async function pageUserNew() {
  if (!me.is_admin) {
    deny("Benutzer", "admin");
    return;
  }
  shell(t("Benutzer anlegen"), `
    <form class="card form" id="create-user">
      <label class="field"><span>${h("Benutzername")}</span><input name="username" required autocomplete="off"></label>
      <p class="small muted">${h("Benutzername: 2–64 Zeichen, a-z, 0-9, Punkt, Unterstrich, Bindestrich.")}</p>
      <label class="field"><span>${h("Anzeigename")}</span><input name="display_name" required></label>
      <label class="field"><span>${h("Passwort")}</span><input name="password" type="password" required minlength="8" autocomplete="new-password"></label>
      <label class="row"><input name="is_admin" type="checkbox"> ${h("Administrator")}</label>
      <button class="btn pill primary" type="submit">${h("Anlegen")}</button>
    </form>`);
  document.querySelector("#create-user").addEventListener("submit", async (event) => {
    event.preventDefault();
    const form = event.currentTarget;
    try {
      const created = await api("POST", "/api/users", {
        username: form.username.value,
        display_name: form.display_name.value,
        password: form.password.value,
        is_admin: form.is_admin.checked,
      });
      go(`#/users/${created.id}`);
    } catch (error) { handle(error); }
  });
}

async function pageBackup() {
  if (!me.is_admin) {
    deny("Sicherung", "admin");
    return;
  }
  shell(t("Sicherung"), `<div class="stack">
    <div class="card">
      <p>${h("Koreograf speichert in SQLite. Die Sicherung enthält die Datenbank und den Schlüssel für die Zugänge. Cache, Git-Checkouts und Auftragsverzeichnisse bleiben außen vor.")}</p>
      <p class="muted">${h("PostgreSQL ist für eine spätere Firmen-Installation vorgesehen. Dieselbe Sicherung ist die Grundlage für den Umzug.")}</p>
      <div><button class="btn pill primary" id="backup-download" type="button">${h("Sicherung herunterladen")}</button></div>
    </div>
    <form class="card form" id="backup-restore">
      <h2>${h("Sicherung einspielen")}</h2>
      <p>${h("Das ersetzt Organisationen, Teams, Benutzer, Projekte, Inventare, Zugänge, Vorlagen, Workflows und Webhooks. Laufende Aufträge müssen vorher beendet sein. Danach ist eine neue Anmeldung nötig.")}</p>
      <label class="field"><span>${h("Datei")}</span><input name="file" type="file" accept=".zip,application/zip" required></label>
      <label class="row"><input name="confirm" type="checkbox" required> ${h("Die aktuelle Installation wird ersetzt.")}</label>
      <button class="btn pill primary" type="submit">${h("Sicherung einspielen")}</button>
    </form>
  </div>`);
  document.querySelector("#backup-download").addEventListener("click", async () => {
    const button = document.querySelector("#backup-download");
    button.disabled = true;
    try {
      const headers = {};
      if (teamId) headers["X-Team-Id"] = teamId;
      const response = await fetch("/api/admin/backup", { headers, credentials: "same-origin" });
      const blob = await response.blob();
      if (!response.ok) {
        const text = await blob.text();
        let message = "Die Anfrage ist fehlgeschlagen.";
        try { message = JSON.parse(text).error || message; } catch { /* keep */ }
        throw new Error(message);
      }
      const url = URL.createObjectURL(blob);
      const link = document.createElement("a");
      link.href = url;
      link.download = "koreograf-backup.zip";
      link.click();
      URL.revokeObjectURL(url);
    } catch (error) {
      handle(error);
    } finally {
      button.disabled = false;
    }
  });
  document.querySelector("#backup-restore").addEventListener("submit", async (event) => {
    event.preventDefault();
    const form = event.currentTarget;
    const file = form.file.files[0];
    if (!file || !form.confirm.checked) return;
    const button = form.querySelector("button[type=submit]");
    button.disabled = true;
    try {
      const headers = { "X-Restore-Confirm": "ERSETZEN" };
      if (teamId) headers["X-Team-Id"] = teamId;
      const response = await fetch("/api/admin/backup", {
        method: "POST",
        headers,
        body: file,
        credentials: "same-origin",
      });
      const data = await response.json().catch(() => ({}));
      if (!response.ok) throw new Error(data.error || "Die Anfrage ist fehlgeschlagen.");
      me = null;
      renderLogin("Sicherung eingespielt. Die vorherige Installation liegt als restore-previous.zip im Datenverzeichnis. Bitte neu anmelden.", "good");
    } catch (error) {
      handle(error);
      button.disabled = false;
    }
  });
}

async function route() {
  closeLive();
  const hash = location.hash || "#/";
  if (!me) {
    if (hash !== "#/login") location.hash = "#/login";
    renderLogin("");
    return;
  }
  if (hash === "#/login") {
    location.hash = "#/";
    return;
  }
  const parts = hash.slice(1).split("/").filter(Boolean);
  try {
    if (parts.length === 0) await pageDashboard();
    else if (parts[0] === "templates" && parts[1] === "new") await pageTemplateForm(null);
    else if (parts[0] === "templates" && parts[1]) await pageTemplateForm(parts[1]);
    else if (parts[0] === "templates") await pageTemplates();
    else if (parts[0] === "jobs" && parts[1]) await pageJob(parts[1]);
    else if (parts[0] === "jobs") await pageJobs();
    else if (parts[0] === "workflows" && parts[1] === "new") await pageWorkflowNew();
    else if (parts[0] === "workflows" && parts[1]) await pageWorkflow(parts[1]);
    else if (parts[0] === "workflows") await pageWorkflows();
    else if (parts[0] === "runs" && parts[1]) await pageRun(parts[1]);
    else if (parts[0] === "hooks" && parts[1] === "new") await pageHookNew();
    else if (parts[0] === "hooks") await pageHooks();
    else if (parts[0] === "environments" && parts[1] === "new") await pageEnvironmentNew();
    else if (parts[0] === "environments" && parts[1]) await pageEnvironment(parts[1]);
    else if (parts[0] === "runtime") await pageRuntime();
    else if (parts[0] === "projects" && parts[1] === "new") await pageProjectNew();
    else if (parts[0] === "projects" && parts[1]) await pageProject(parts[1]);
    else if (parts[0] === "projects") await pageProjects();
    else if (parts[0] === "inventories" && parts[1] === "new") await pageInventoryNew();
    else if (parts[0] === "inventories" && parts[1]) await pageInventory(parts[1]);
    else if (parts[0] === "inventories") await pageInventories();
    else if (parts[0] === "credentials" && parts[1] === "new") await pageCredentialNew();
    else if (parts[0] === "credentials") await pageCredentials();
    else if (parts[0] === "organizations" && parts[1] === "new") await pageOrganizationNew();
    else if (parts[0] === "organizations" && parts[1]) await pageOrganization(parts[1]);
    else if (parts[0] === "organizations") await pageOrganizations();
    else if (parts[0] === "catalog") location.hash = "#/projects";
    else if (parts[0] === "team" && parts[1] === "members" && parts[2] === "new") await pageMemberNew();
    else if (parts[0] === "team" && parts[1] === "new") await pageTeamNew();
    else if (parts[0] === "account") await pageAccount();
    else if (parts[0] === "users" && parts[1] === "new") await pageUserNew();
    else if (parts[0] === "users" && parts[1]) await pageUser(parts[1]);
    else if (parts[0] === "users") await pageUsers();
    else if (parts[0] === "backup") await pageBackup();
    else if (parts[0] === "team") await pageTeam();
    else shell(t("Seite"), `<div class="card">${h("Diese Seite gibt es nicht.")}</div>`);
  } catch (error) {
    handle(error);
  }
}

async function boot() {
  try {
    me = await api("GET", "/api/me");
    const known = (me.teams || []).some((team) => team.id === teamId);
    if (!known) {
      teamId = (me.teams[0] && me.teams[0].id) || "";
      localStorage.setItem("koreograf.team", teamId);
    }
  } catch {
    me = null;
  }
  window.addEventListener("hashchange", route);
  route();
}

boot();
