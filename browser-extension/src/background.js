import { secureOrigin, trustedPopup } from "./policy.js";
import { inspectPage, fillPage } from "./page.js";
import { createNativeSession } from "./native.js";

const api = globalThis.browser ?? globalThis.chrome;
const companion = createNativeSession(api.runtime);
let chain = Promise.resolve();

async function tabAt(tabId, expectedOrigin) {
  if (!Number.isInteger(tabId) || typeof expectedOrigin !== "string") throw new Error("Open Lockbox again.");
  const tab = await api.tabs.get(tabId);
  if (!tab.active || secureOrigin(tab.url) !== expectedOrigin) throw new Error("The active page changed. Open Lockbox again.");
  return tab;
}

function pageScript(tabId, func, args) {
  return {
    target: { tabId, frameIds: [0] },
    world: "ISOLATED",
    func,
    args,
  };
}

async function inspect(tabId, origin, mode) {
  await tabAt(tabId, origin);
  const results = await api.scripting.executeScript(pageScript(tabId, inspectPage, [origin, mode]));
  const result = results[0]?.result;
  if (!result?.token) throw new Error(result?.error || "This page does not allow autofill.");
  return result.token;
}

async function fill(tabId, origin, entryId, mode, token) {
  const version = companion.generation;
  const credential = await companion.request({ action: "credential", origin, entry_id: entryId });
  try {
    await tabAt(tabId, origin);
    if (version !== companion.generation) throw new Error("Vault locked during fill. Unlock again.");
    const results = await api.scripting.executeScript(pageScript(
      tabId, fillPage, [origin, token, credential.username, credential.password, mode],
    ));
    const result = results[0]?.result;
    if (!result?.filled) throw new Error(result?.error || "Could not fill this page.");
    return { filled: true };
  } finally {
    credential.username = "";
    credential.password = "";
  }
}

function stillAllowed(ticket) {
  if (ticket !== companion.cancellation) throw new Error("Vault locked during the request. Unlock again.");
}

async function handle(message, ticket) {
  switch (message.action) {
    case "status": return companion.request({ action: "status" });
    case "unlock": {
      try { return await companion.request({ action: "unlock", path: message.path, password: message.password }); }
      finally { message.password = ""; }
    }
    case "suggestions":
      await tabAt(message.tabId, message.origin);
      stillAllowed(ticket);
      return companion.request({ action: "suggestions", origin: message.origin });
    case "generate":
      await tabAt(message.tabId, message.origin);
      stillAllowed(ticket);
      return companion.request({ action: "generate", origin: message.origin, length: message.length, symbols: message.symbols });
    case "fill": {
      const token = await inspect(message.tabId, message.origin, "login");
      stillAllowed(ticket);
      return fill(message.tabId, message.origin, message.entryId, "login", token);
    }
    case "save_generated": {
      // Inspect before saving, then write durably before any credential reaches the page.
      const token = await inspect(message.tabId, message.origin, "new");
      stillAllowed(ticket);
      const saved = await companion.request({
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
  // Lock immediately so pending work cannot delay cancellation.
  if (message.action === "lock") {
    companion.disconnect();
    sendResponse({ ok: true, data: { unlocked: false } });
    return false;
  }
  // Serialize other requests so unlocking, generation and saving cannot overlap.
  const ticket = companion.cancellation;
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
  if (state !== "active") companion.disconnect();
});
api.runtime.onSuspend?.addListener(companion.disconnect);
