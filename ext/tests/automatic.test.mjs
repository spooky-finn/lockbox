import test from "node:test";
import assert from "node:assert/strict";
import { installAutomatic } from "../src/automatic.js";
class Event {
  listeners = [];
  addListener(listener) { this.listeners.push(listener); }
  fire(...args) { for (const listener of this.listeners) listener(...args); }
}
const settle = () => new Promise(resolve => setImmediate(resolve));
function fixture(options = {}) {
  const tab = { id: 7, windowId: 1, active: true, status: "complete", url: options.url ?? "https://accounts.example/login" };
  const window = { id: 1, focused: options.focused ?? true };
  const badges = [], popups = [], lookups = [];
  const api = {
    tabs: { query: async () => [tab], get: async () => tab, onActivated: new Event(), onUpdated: new Event(), onRemoved: new Event() },
    windows: { getLastFocused: async () => window, onFocusChanged: new Event() },
    action: {
      setBadgeText: async value => badges.push(value), setTitle: async () => {},
      openPopup: async value => { popups.push(value); if (options.blocked) throw new Error("User gesture required"); },
    },
    runtime: { onSuspend: new Event() },
  };
  const check = installAutomatic(api, async origin => {
    lookups.push(origin);
    return options.lookup ? options.lookup() : (options.matched ?? true);
  }, () => options.pending ?? false);
  return { api, tab, window, badges, popups, lookups, check, stop: () => api.runtime.onSuspend.fire() };
}
test("matching login opens once per document and never requests credentials", async () => {
  const f = fixture();
  try {
    await settle(); await f.check();
    assert.deepEqual(f.lookups, ["https://accounts.example", "https://accounts.example"]);
    assert.equal(f.popups.length, 1);
    assert.deepEqual(f.badges.at(-1), { tabId: 7, text: "1" });
    f.api.tabs.onUpdated.fire(7, { status: "loading" });
    f.api.tabs.onUpdated.fire(7, { status: "complete" });
    await settle();
    assert.equal(f.popups.length, 2);
  } finally { f.stop(); }
});
test("no matching login clears badge without opening", async () => {
  const f = fixture({ matched: false });
  try { await settle(); assert.equal(f.popups.length, 0); assert.equal(f.badges.at(-1).text, ""); }
  finally { f.stop(); }
});
test("pending consent, unfocused browser and HTTP never trigger lookup", async () => {
  for (const options of [{ pending: true }, { focused: false }, { url: "http://accounts.example" }]) {
    const f = fixture(options);
    try { await settle(); assert.equal(f.lookups.length, 0); assert.equal(f.popups.length, 0); }
    finally { f.stop(); }
  }
});
test("navigation during match lookup prevents popup for previous website", async () => {
  let release;
  const answer = new Promise(resolve => { release = resolve; });
  const f = fixture({ lookup: () => answer });
  try {
    await settle(); f.tab.url = "https://other.example"; release(true); await settle();
    assert.equal(f.popups.length, 0); assert.equal(f.badges.length, 0);
  } finally { f.stop(); }
});
test("restricted openPopup keeps badge and does not repeatedly retry", async () => {
  const f = fixture({ blocked: true });
  try {
    await settle(); await f.check();
    assert.equal(f.popups.length, 1); assert.equal(f.badges.at(-1).text, "1");
  } finally { f.stop(); }
});
test("desktop unavailable produces no popup", async () => {
  const f = fixture({ lookup: () => { throw new Error("Desktop absent"); } });
  try { await settle(); assert.equal(f.popups.length, 0); }
  finally { f.stop(); }
});
