import test from "node:test";
import assert from "node:assert/strict";

class Event {
  listeners = [];
  addListener(listener) { this.listeners.push(listener); }
  fire(...args) { for (const listener of this.listeners) listener(...args); }
}

let index = 0;
async function fixture(options = {}) {
  const requests = [];
  const injections = [];
  const connections = [];
  const runtime = {
    id: "fictional-extension",
    getURL: (file) => `chrome-extension://fictional-extension/${file}`,
    onMessage: new Event(), onSuspend: new Event(),
    connectNative() {
      const port = {
        onMessage: new Event(), onDisconnect: new Event(),
        disconnect() { this.onDisconnect.fire(); },
        postMessage(message) {
          requests.push(message.request);
          if (options.post) return options.post(message, port);
          queueMicrotask(() => port.onMessage.fire({ id: message.id, status: "ok", data:
            message.request.action === "credential" ? { username: "fictional-user", password: "fictional-password" } :
            message.request.action === "save_generated" ? { entry_id: "fictional-id" } : { unlocked: true } }));
        },
      };
      connections.push(port);
      return port;
    },
  };
  const idle = { setDetectionInterval() {}, onStateChanged: new Event() };
  globalThis.browser = {
    runtime, idle,
    tabs: { get: async () => ({ id: 1, active: true, url: options.url || "https://accounts.example/login" }) },
    scripting: { executeScript: async (injection) => {
      injections.push(injection);
      if (options.inject) return options.inject(injection);
      return [{ result: injection.func.name === "inspectPage" ? { token: "document-token" } : { filled: true } }];
    } },
  };
  await import(`../src/background.js?test=${index++}`);
  const sender = { id: runtime.id, url: runtime.getURL("popup.html") };
  const message = (body, from = sender) => new Promise((resolve) => {
    const accepted = runtime.onMessage.listeners[0](body, from, resolve);
    if (!accepted) resolve(undefined);
  });
  return { requests, injections, connections, idle, message, close: () => runtime.onSuspend.fire() };
}

test("untrusted web page messages cannot invoke native host", async () => {
  const f = await fixture();
  assert.equal(await f.message({ action: "unlock", path: "/secret", password: "secret" },
    { id: "fictional-extension", url: "https://attacker.test", tab: { id: 1 } }), undefined);
  assert.equal(f.connections.length, 0);
  f.close();
});

test("credential fetched only after successful form inspection, never returned to popup", async () => {
  const f = await fixture();
  await f.message({ action: "status" });
  const reply = await f.message({ action: "fill", tabId: 1, origin: "https://accounts.example", entryId: "id" });
  assert.deepEqual(reply, { ok: true, data: { filled: true } });
  assert.deepEqual(f.injections.map((item) => item.func.name), ["inspectPage", "fillPage"]);
  for (const injection of f.injections) {
    assert.deepEqual(injection.target, { tabId: 1, frameIds: [0] });
    assert.equal(injection.world, "ISOLATED");
  }
  assert.equal(f.injections[1].args[1], "document-token");
  assert.equal(f.requests.at(-1).action, "credential");
  assert.ok(!JSON.stringify(reply).includes("fictional-password"));
  f.close();
});

test("cross-origin tab and unsafe forms block credential retrieval", async () => {
  for (const options of [{ url: "https://attacker.test" }, { inject: () => [{ result: { error: "Unsafe form" } }] }]) {
    const f = await fixture(options);
    const reply = await f.message({ action: "fill", tabId: 1, origin: "https://accounts.example", entryId: "id" });
    assert.equal(reply.ok, false);
    assert.equal(f.requests.length, 0);
    f.close();
  }
});

test("generated credential saved before retrieval and page delivery", async () => {
  const f = await fixture();
  await f.message({ action: "status" });
  const reply = await f.message({ action: "save_generated", tabId: 1, origin: "https://accounts.example",
    title: "Fictional", username: "fictional-user", entryId: null });
  assert.equal(reply.ok, true);
  assert.deepEqual(f.requests.map((item) => item.action), ["status", "save_generated", "credential"]);
  assert.equal(f.injections[0].args[1], "new");
  f.close();
});

test("locking while form inspection is pending cancels fill without reopening native host", async () => {
  let release;
  const paused = new Promise((resolve) => { release = resolve; });
  const f = await fixture({ inject: () => paused });
  await f.message({ action: "status" });
  const filling = f.message({ action: "fill", tabId: 1, origin: "https://accounts.example", entryId: "id" });
  await new Promise((resolve) => setImmediate(resolve));
  await f.message({ action: "lock" });
  release([{ result: { token: "document-token" } }]);
  assert.equal((await filling).ok, false);
  assert.deepEqual(f.requests.map((item) => item.action), ["status"]);
  assert.equal(f.connections.length, 1);
  f.close();
});

test("browser idle and screen lock disconnect native session", async () => {
  for (const state of ["idle", "locked"]) {
    const f = await fixture();
    await f.message({ action: "status" });
    let closed = false;
    f.connections[0].onDisconnect.addListener(() => { closed = true; });
    f.idle.onStateChanged.fire(state);
    assert.equal(closed, true);
    f.close();
  }
});

test("unmatched replies are ignored and native failures preserve the connection", async () => {
  const posted = [];
  const f = await fixture({ post: (message) => posted.push(message) });
  const body = { action: "unlock", path: "/fictional-vault", password: "fictional-password" };
  let settled = false;
  const opening = f.message(body).then((reply) => { settled = true; return reply; });
  await new Promise((resolve) => setImmediate(resolve));
  const connection = f.connections[0];
  connection.onMessage.fire({ id: posted[0].id + 1, status: "ok", data: { unlocked: true } });
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(settled, false);
  connection.onMessage.fire({ id: posted[0].id, status: "error", error: "Fictional denial" });
  assert.deepEqual(await opening, { ok: false, error: "Fictional denial" });
  assert.equal(body.password, "");

  const status = f.message({ action: "status" });
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(f.connections.length, 1);
  connection.onMessage.fire({ id: posted[1].id, status: "ok", data: { unlocked: false } });
  assert.deepEqual(await status, { ok: true, data: { unlocked: false } });
  f.close();
});

test("disconnection rejects pending work and old ports cannot reply to new requests", async () => {
  const posted = [];
  const f = await fixture({ post: (message) => posted.push(message) });
  const first = f.message({ action: "status" });
  await new Promise((resolve) => setImmediate(resolve));
  const old = f.connections[0];
  f.close();
  assert.deepEqual(await first, { ok: false, error: "Vault disconnected or locked. Unlock again." });

  let settled = false;
  const next = f.message({ action: "status" }).then((reply) => { settled = true; return reply; });
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(f.connections.length, 2);
  assert.ok(posted[1].id > posted[0].id);
  old.onMessage.fire({ id: posted[1].id, status: "ok", data: { unlocked: true } });
  old.onDisconnect.fire();
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(settled, false);
  f.connections[1].onMessage.fire({ id: posted[1].id, status: "ok", data: { unlocked: false } });
  assert.deepEqual(await next, { ok: true, data: { unlocked: false } });
  f.close();
});
