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

module.exports = { origin, post };
