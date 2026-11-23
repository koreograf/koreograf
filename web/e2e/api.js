"use strict";

const { port } = require("./env");

const origin = `http://127.0.0.1:${port}`;

// The request fixture does not send Origin, so mutating API calls supply the
// same origin the browser would send for the running server.
function post(request, url, options = {}) {
  return request.post(url, {
    ...options,
    headers: { Origin: origin, ...(options.headers || {}) },
  });
}

function put(request, url, options = {}) {
  return request.put(url, {
    ...options,
    headers: { Origin: origin, ...(options.headers || {}) },
  });
}

function storeAssistantKey(page) {
  return put(page.request, "/api/v1/settings", {
    data: {
      ldap: {},
      proxy: {},
      runner: "local",
      job_timeout_secs: 60,
      job_output_lines: 8000,
      llm: {
        enabled: false,
        provider: "ollama",
        base_url: "",
        model: "",
        api_key: "e2e-assistant-key",
      },
    },
  });
}

module.exports = { origin, post, put, storeAssistantKey };
