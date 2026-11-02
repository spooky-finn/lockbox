import { installAutomatic } from "./automatic.js";
import { secureOrigin, trustedPopup } from "./policy.js";
import { inspectPage, fillPage } from "./page.js";
import { createNativeSession } from "./native.js";
const api = globalThis.browser ?? globalThis.chrome;
const companion = createNativeSession(api.runtime);
let pending = false;
let status = "Ready. Open Lockbox to choose an account.";

async function tabAt(tabId, origin) {
  if (!Number.isInteger(tabId) || typeof origin !== "string") throw new Error("Open Lockbox again.");
  const tab = await api.tabs.get(tabId);
  if (!tab.active || secureOrigin(tab.url) !== origin) throw new Error("The active page changed. Open Lockbox again.");
}
function script(tabId, func, args) {
  return { target: { tabId, frameIds: [0] }, world: "ISOLATED", func, args };
}
async function fill(message) {
  if (pending) throw new Error("A request is already open in Lockbox.");
  pending = true;
  const ticket = companion.cancellation;
  let credential;
  try {
    await tabAt(message.tabId, message.origin);
    const results = await api.scripting.executeScript(script(message.tabId, inspectPage, [message.origin, "login"]));
    const token = results[0]?.result?.token;
    if (!token) throw new Error(results[0]?.result?.error || "This page does not allow autofill.");
    if (ticket !== companion.cancellation) throw new Error("Request cancelled.");
    status = "Choose an account in the Lockbox desktop app.";
    credential = await companion.request({ action: "fill", origin: message.origin });
    await tabAt(message.tabId, message.origin);
    if (ticket !== companion.cancellation) throw new Error("Request cancelled.");
    if (typeof credential?.username !== "string" || typeof credential?.password !== "string" ||
        credential.username.length > 4096 || credential.password.length > 4096) throw new Error("Invalid desktop reply.");
    const filled = await api.scripting.executeScript(script(message.tabId, fillPage,
      [message.origin, token, credential.username, credential.password, "login"]));
    if (!filled[0]?.result?.filled) throw new Error(filled[0]?.result?.error || "Could not fill this page.");
    status = "Login filled. Submit the website form when ready.";
    return { filled: true };
  } catch (error) {
    status = error.message;
    throw error;
  } finally {
    if (credential) { credential.username = ""; credential.password = ""; }
    pending = false;
  }
}
api.runtime.onMessage.addListener((message, sender, respond) => {
  if (!trustedPopup(sender, api.runtime)) return false;
  if (message.action === "status") { respond({ ok: true, data: { pending, status } }); return false; }
  if (message.action === "cancel") {
    companion.disconnect(); status = "Browser request cancelled.";
    respond({ ok: true, data: { cancelled: true } }); return false;
  }
  if (message.action !== "fill") { respond({ ok: false, error: "Unknown request." }); return false; }
  fill(message).then(data => respond({ ok: true, data }), error => respond({ ok: false, error: error.message }));
  return true;
});
api.idle.setDetectionInterval(300);
api.idle.onStateChanged.addListener(state => { if (state !== "active") companion.disconnect(); });
api.runtime.onSuspend?.addListener(companion.disconnect);

installAutomatic(api, async origin => {
  const reply = await companion.request({ action: "match", origin });
  return reply?.matched === true;
}, () => pending);
