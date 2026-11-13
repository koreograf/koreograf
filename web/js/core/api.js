import * as appState from "./state.js";

export async function api(method, path, body) {
  const headers = {};
  if (appState.teamId) headers["X-Team-Id"] = appState.teamId;
  const init = { method, headers, credentials: "same-origin" };
  if (method === "GET" && appState.navToken) init.signal = appState.navToken.signal;
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
    const error = new Error((data && data.error) || "request.failed");
    error.status = response.status;
    error.vars = (data && data.vars) || null;
    throw error;
  }
  return data;
}
