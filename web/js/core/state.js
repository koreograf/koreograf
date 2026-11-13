export const app = document.querySelector("#app");

export let me = null;
export let teamId = localStorage.getItem("koreograf.team") || "";
export let source = null;
export let pollTimer = null;
export let generation = 0;
export let navOpen = false;
export let navKey = null;
export let navToken = null;
export let leaveGuard = null;
export let shownHash = null;
export const routeScroll = new Map();
export let historyIndex = 0;
export let buildVersion = "";
export let buildCommit = "";
export let versionRequested = false;
export let assistantEnabled = false;
export let assistantModel = "";
export let maintenanceActive = false;
export let maintenanceSince = "";
export let maintenanceTimer = null;
export let flashKey = "";
export let notedInvalid = false;
export let editSerial = 0;

export function setMe(value) {
  return (me = value);
}

export function setTeamId(value) {
  return (teamId = value);
}

export function setSource(value) {
  return (source = value);
}

export function setPollTimer(value) {
  return (pollTimer = value);
}

export function setGeneration(value) {
  return (generation = value);
}

export function setNavOpen(value) {
  return (navOpen = value);
}

export function setNavKey(value) {
  return (navKey = value);
}

export function setNavToken(value) {
  return (navToken = value);
}

export function setLeaveGuard(value) {
  return (leaveGuard = value);
}

export function setShownHash(value) {
  return (shownHash = value);
}

export function setHistoryIndex(value) {
  return (historyIndex = value);
}

export function setBuildVersion(value) {
  return (buildVersion = value);
}

export function setBuildCommit(value) {
  return (buildCommit = value);
}

export function setVersionRequested(value) {
  return (versionRequested = value);
}

export function setAssistantEnabled(value) {
  return (assistantEnabled = value);
}

export function setAssistantModel(value) {
  return (assistantModel = value);
}

export function setMaintenanceActive(value) {
  return (maintenanceActive = value);
}

export function setMaintenanceSince(value) {
  return (maintenanceSince = value);
}

export function setMaintenanceTimer(value) {
  return (maintenanceTimer = value);
}

export function setFlashKey(value) {
  return (flashKey = value);
}

export function setNotedInvalid(value) {
  return (notedInvalid = value);
}

export function setEditSerial(value) {
  return (editSerial = value);
}
