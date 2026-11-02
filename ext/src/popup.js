import { secureOrigin } from "./policy.js";
const api = globalThis.browser ?? globalThis.chrome;
const $ = id => document.getElementById(id);
let site;
async function send(action) {
  const reply = await api.runtime.sendMessage({ action, ...site });
  if (!reply?.ok) throw new Error(reply?.error || "Could not connect to Lockbox.");
  return reply.data;
}
async function refresh() {
  const data = await send("status");
  $("message").textContent = data.status;
  $("open").disabled = !site || data.pending;
  $("cancel").hidden = !data.pending;
}
$("open").addEventListener("click", async () => {
  $("open").disabled = true;
  $("cancel").hidden = false;
  $("message").textContent = "Opening Lockbox. Continue in the desktop app…";
  try {
    // A programmatically opened popup does not grant activeTab permission.
    const pattern = `https://${new URL(site.origin).hostname}/*`;
    if (!(await api.permissions.request({ origins: [pattern] }))) {
      throw new Error("Allow access to this website to fill the selected login.");
    }
    await send("fill"); await refresh();
  }
  catch (error) { $("message").textContent = error.message; $("open").disabled = !site; $("cancel").hidden = true; }
});
$("cancel").addEventListener("click", async () => {
  try { await send("cancel"); await refresh(); }
  catch (error) { $("message").textContent = error.message; }
});
async function init() {
  $("extension-id").textContent = api.runtime.id;
  const [tab] = await api.tabs.query({ active: true, currentWindow: true });
  try { site = { tabId: tab.id, origin: secureOrigin(tab.url) }; }
  catch { $("message").textContent = "Open an HTTPS login page, then open Lockbox again."; return; }
  $("origin").textContent = site.origin;
  await refresh();
  setInterval(() => refresh().catch(() => {}), 1000);
}
void init().catch(error => { $("message").textContent = error.message; });
