import { secureOrigin, trustedPopup } from "./policy.js";
import { inspectPage, fillPage } from "./page.js";

const api = globalThis.browser ?? globalThis.chrome;
const HOST = "app.lockbox.browser";
let port = null;
let sequence = 0;
let generation = 0;
let cancellation = 0;
const pending = new Map();
let chain = Promise.resolve();

function disconnect() {
  generation++;
  cancellation++;
  const old = port;
  port = null;
  old?.disconnect();
  for (const { reject, timer } of pending.values()) {
    clearTimeout(timer);
    reject(new Error("Vault disconnected or locked. Unlock again."));
  }
  pending.clear();
}

function native(request) {
  if (!port) {
    generation++;
    const connection = api.runtime.connectNative(HOST);
    port = connection;
    connection.onMessage.addListener((reply) => {
      if (connection !== port) return;
      const task = pending.get(reply.id);
      if (!task) return;
      pending.delete(reply.id);
      clearTimeout(task.timer);
      if (reply.status === "ok") task.resolve(reply.data);
      else task.reject(new Error(reply.error || "Native request failed."));
    });
    connection.onDisconnect.addListener(() => {
      void api.runtime.lastError; // Consume Chrome's error without logging sensitive context.
      if (connection === port) disconnect();
    });
  }
  const id = ++sequence;
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      disconnect();
      reject(new Error("Native host timed out. Check the companion installation."));
    }, 30_000);
    pending.set(id, { resolve, reject, timer });
    try { port.postMessage({ id, request }); }
    catch { disconnect(); }
  });
}

async function tabAt(tabId, expectedOrigin) {
  if (!Number.isInteger(tabId) || typeof expectedOrigin !== "string") throw new Error("Open Lockbox again.");
  const tab = await api.tabs.get(tabId);
  if (!tab.active || secureOrigin(tab.url) !== expectedOrigin) throw new Error("The active page changed. Open Lockbox again.");
  return tab;
}

async function inspect(tabId, origin, mode) {
  await tabAt(tabId, origin);
  const results = await api.scripting.executeScript({
    target: { tabId, frameIds: [0] },
    world: "ISOLATED",
    func: inspectPage,
    args: [origin, mode],
  });
  const result = results[0]?.result;
  if (!result?.token) throw new Error(result?.error || "This page does not allow autofill.");
  return result.token;
}

async function fill(tabId, origin, entryId, mode, token) {
  const version = generation;
  const credential = await native({ action: "credential", origin, entry_id: entryId });
  try {
    await tabAt(tabId, origin);
    if (version !== generation) throw new Error("Vault locked during fill. Unlock again.");
    const results = await api.scripting.executeScript({
      target: { tabId, frameIds: [0] },
      world: "ISOLATED",
      func: fillPage,
      args: [origin, token, credential.username, credential.password, mode],
    });
    const result = results[0]?.result;
    if (!result?.filled) throw new Error(result?.error || "Could not fill this page.");
    return { filled: true };
  } finally {
    credential.username = "";
    credential.password = "";
  }
}

function stillAllowed(ticket) {
  if (ticket !== cancellation) throw new Error("Vault locked during the request. Unlock again.");
}

async function handle(message, ticket) {
  switch (message.action) {
    case "status": return native({ action: "status" });
    case "unlock": {
      try { return await native({ action: "unlock", path: message.path, password: message.password }); }
      finally { message.password = ""; }
    }
    case "lock": disconnect(); return { unlocked: false };
    case "suggestions":
      await tabAt(message.tabId, message.origin);
      stillAllowed(ticket);
      return native({ action: "suggestions", origin: message.origin });
    case "generate":
      await tabAt(message.tabId, message.origin);
      stillAllowed(ticket);
      return native({ action: "generate", origin: message.origin, length: message.length, symbols: message.symbols });
    case "fill": {
      const token = await inspect(message.tabId, message.origin, "login");
      stillAllowed(ticket);
      return fill(message.tabId, message.origin, message.entryId, "login", token);
    }
    case "save_generated": {
      // Inspect before saving, then write durably before any credential reaches the page.
      const token = await inspect(message.tabId, message.origin, "new");
      stillAllowed(ticket);
      const saved = await native({
        action: "save_generated", origin: message.origin, title: message.title,
        username: message.username, entry_id: message.entryId || null,
      });
      try {
        return { ...saved, ...await fill(message.tabId, message.origin, saved.entry_id, "new", token) };
      } catch {
        throw new Error("Password saved in your vault, but filling was blocked. You can retrieve it in the desktop app to retry.");
      }
    }
    default: throw new Error("Unknown request.");
  }
}

api.runtime.onMessage.addListener((message, sender, sendResponse) => {
  if (!trustedPopup(sender, api.runtime)) return false;
  // Serialize requests so lock/unlock, generation and saving cannot overlap.
  if (message.action === "lock") {
    disconnect();
    sendResponse({ ok: true, data: { unlocked: false } });
    return false;
  }
  const ticket = cancellation;
  const work = chain.then(() => {
    stillAllowed(ticket);
    return handle(message, ticket);
  });
  chain = work.catch(() => {});
  work.then((data) => sendResponse({ ok: true, data }),
    (error) => sendResponse({ ok: false, error: error.message }));
  return true; // Callback form works in both Firefox and Chrome.
});

api.idle.setDetectionInterval(300);
api.idle.onStateChanged.addListener((state) => {
  if (state !== "active") disconnect();
});
api.runtime.onSuspend?.addListener(disconnect);
