"use strict";

require("./dom-stub.js");
const assert = require("node:assert/strict");
const { join } = require("node:path");
const { pathToFileURL } = require("node:url");
const test = require("node:test");

const load = () => import(pathToFileURL(join(__dirname, "..", "js", "features", "credentials.js")).href);

function field(value, checked = false, stored = false) {
  return { value, checked, dataset: stored ? { storedMask: "1" } : {} };
}

function form(kind, values, options = {}) {
  const elements = {};
  Object.entries(values).forEach(([name, value]) => {
    elements[name] = field(value, false, (options.stored || []).includes(name));
  });
  return {
    kind: { value: kind },
    elements,
    env: field(options.env || ""),
    authorize: field("", options.authorize),
    verify_ssl: field("", options.verify_ssl),
    querySelectorAll(selector) {
      if (selector !== "[data-clear]:checked") return [];
      return (options.clear || []).map((name) => ({ getAttribute: () => name }));
    },
  };
}

test("the credential list filter matches name, kind and usage", async () => {
  const { credentialMatches } = await load();
  const item = { name: "Deploy key", kind: "scm", usages: [{ project_name: "Web" }] };
  assert.equal(credentialMatches(item, ""), true);
  assert.equal(credentialMatches(item, "deploy"), true);
  assert.equal(credentialMatches(item, "scm"), true);
  assert.equal(credentialMatches(item, "web"), true);
  assert.equal(credentialMatches(item, "vault"), false);
});

test("a credential secret keeps only filled fields", async () => {
  const { credentialSecret } = await load();
  const values = {
    username: " bob ",
    password: "",
    private_key: "key",
    passphrase: "secret",
    become_method: "",
    become_username: " root ",
    become_password: "",
  };
  assert.deepEqual(credentialSecret(form("ssh", values)), {
    username: "bob",
    private_key: "key",
    passphrase: "secret",
    become_username: "root",
  });
});

test("an environment credential rejects a line without a key", async () => {
  const { credentialSecret } = await load();
  assert.throws(() => credentialSecret(form("env", {}, { env: "NO_EQUALS" })), /an_env_line_needs_key_value/);
});

test("an environment credential splits key and value at the first equals sign", async () => {
  const { credentialSecret } = await load();
  assert.deepEqual(credentialSecret(form("env", {}, { env: "A=1\n B=two=2 \n\n" })), { A: "1", B: "two=2" });
});

test("an update skips stored masks and reports cleared fields", async () => {
  const { credentialUpdateBody } = await load();
  const values = {
    username: " bob ",
    password: "********",
    private_key: "key",
    passphrase: "",
    become_method: "sudo",
    become_username: "",
    become_password: "",
  };
  assert.deepEqual(credentialUpdateBody(form("ssh", values, { stored: ["password"], clear: ["password"] }), "ssh"), {
    secret: {
      username: "bob",
      private_key: "key",
      become_method: "sudo",
      become_username: "",
    },
    clear: ["password"],
  });
});

test("an update keeps the network and Kubernetes switches", async () => {
  const { credentialUpdateBody } = await load();
  assert.deepEqual(credentialUpdateBody(form("net", { username: "admin", password: "secret" }, { authorize: true }), "net"), {
    secret: { username: "admin", password: "secret", authorize: true },
    clear: [],
  });
  assert.deepEqual(credentialUpdateBody(form("kubernetes", { host: "https://cluster", token: "token" }, { verify_ssl: true }), "kubernetes"), {
    secret: { host: "https://cluster", token: "token", verify_ssl: true },
    clear: [],
  });
});
