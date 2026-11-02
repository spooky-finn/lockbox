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
            message.request.action === "fill" ? { username: "fictional-user", password: "fictional-password" } :
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
    windows: { getLastFocused: async () => ({ focused: false }), onFocusChanged: new Event() },
    action: {},
    tabs: { onUpdated: new Event(), onActivated: new Event(), onRemoved: new Event(), get: async () => ({ id: 1, active: true, url: options.url || "https://accounts.example/login" }) },
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

test("untrusted page cannot invoke desktop", async () => {
  const f = await fixture();
  assert.equal(await f.message({ action: "fill", tabId: 1, origin: "https://accounts.example" },
    { id: "fictional-extension", url: "https://attacker.test", tab: { id: 1 } }), undefined);
  assert.equal(f.connections.length, 0);
  f.close();
});

test("desktop approval follows inspection; only credential is filled, never sent to popup", async () => {
  const f = await fixture();
  const reply = await f.message({ action: "fill", tabId: 1, origin: "https://accounts.example" });
  assert.deepEqual(reply, { ok: true, data: { filled: true } });
  assert.deepEqual(f.requests, [{ action: "fill", origin: "https://accounts.example" }]);
  assert.deepEqual(f.injections.map(item => item.func.name), ["inspectPage", "fillPage"]);
  assert.deepEqual(f.injections[1].args, ["https://accounts.example", "document-token", "fictional-user", "fictional-password", "login"]);
  assert.ok(!JSON.stringify(reply).includes("fictional-password"));
  f.close();
});

test("unsafe origin and form never open desktop", async () => {
  for (const options of [{ url: "https://attacker.test" }, { inject: () => [{ result: { error: "Unsafe form" } }] }]) {
    const f = await fixture(options);
    assert.equal((await f.message({ action: "fill", tabId: 1, origin: "https://accounts.example" })).ok, false);
    assert.equal(f.requests.length, 0);
    f.close();
  }
});

test("cancel during inspection prevents desktop launch", async () => {
  let release;
  const paused = new Promise(resolve => { release = resolve; });
  const f = await fixture({ inject: () => paused });
  const filling = f.message({ action: "fill", tabId: 1, origin: "https://accounts.example" });
  await new Promise(resolve => setImmediate(resolve));
  await f.message({ action: "cancel" });
  release([{ result: { token: "document-token" } }]);
  assert.equal((await filling).ok, false);
  assert.equal(f.requests.length, 0);
  f.close();
});

test("navigation while desktop consent is pending prevents fill", async () => {
  let posted;
  const f = await fixture({ post: (message, port) => { posted = { message, port }; } });
  const filling = f.message({ action: "fill", tabId: 1, origin: "https://accounts.example" });
  await new Promise(resolve => setImmediate(resolve));
  globalThis.browser.tabs.get = async () => ({ active: true, url: "https://attacker.test" });
  posted.port.onMessage.fire({ id: posted.message.id, status: "ok", data: { username: "fictional-user", password: "fictional-password" } });
  assert.equal((await filling).ok, false);
  assert.equal(f.injections.length, 1);
  f.close();
});

test("cancel, idle, screen lock and suspension reject pending consent", async () => {
  for (const reason of ["cancel", "idle", "locked", "suspend"]) {
    const f = await fixture({ post() {} });
    const filling = f.message({ action: "fill", tabId: 1, origin: "https://accounts.example" });
    await new Promise(resolve => setImmediate(resolve));
    if (reason === "cancel") await f.message({ action: "cancel" });
    else if (reason === "suspend") f.close();
    else f.idle.onStateChanged.fire(reason);
    assert.equal((await filling).ok, false);
    assert.equal(f.injections.length, 1);
    f.close();
  }
});

test("legacy vault operations rejected without native connection", async () => {
  const f = await fixture();
  for (const action of ["unlock", "credential", "generate", "save_generated", "suggestions"]) {
    assert.equal((await f.message({ action, password: "secret" })).ok, false);
  }
  assert.equal(f.connections.length, 0);
  f.close();
});

test("only one pending desktop request; status survives popup closing", async () => {
  const f = await fixture({ post() {} });
  const filling = f.message({ action: "fill", tabId: 1, origin: "https://accounts.example" });
  await new Promise(resolve => setImmediate(resolve));
  assert.equal((await f.message({ action: "status" })).data.pending, true);
  assert.equal((await f.message({ action: "fill", tabId: 1, origin: "https://accounts.example" })).ok, false);
  assert.equal(f.requests.length, 1);
  f.close();
  await filling;
});
