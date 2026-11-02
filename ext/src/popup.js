const api = globalThis.browser ?? globalThis.chrome;
const $ = (id) => document.getElementById(id);
let site = null;
let busy = false;
let unlocked = false;

async function send(action, extra = {}) {
  const reply = await api.runtime.sendMessage({ action, ...site, ...extra });
  if (!reply?.ok) throw new Error(reply?.error || "Could not connect to Lockbox.");
  return reply.data;
}

function notice(text, success = false) {
  $("message").textContent = text;
  $("message").classList.toggle("success", success);
  $("message").hidden = !text;
}

function state(isUnlocked) {
  unlocked = isUnlocked;
  $("connect").hidden = isUnlocked;
  $("vault").hidden = !isUnlocked;
  $("lock").hidden = !isUnlocked;
  $("session-label").textContent = isUnlocked ? "Auto-lock 5 min" : "Locked";
  if (!isUnlocked) {
    $("accounts").replaceChildren();
    $("save-form").hidden = true;
    $("generated").hidden = true;
    $("account").replaceChildren(new Option("New login", ""));
    $("username").value = "";
    $("title").value = "";
  }
}

async function perform(work) {
  if (busy) return;
  busy = true;
  notice("");
  for (const button of document.querySelectorAll("button:not(#lock)")) button.disabled = true;
  try { await work(); }
  catch (error) {
    notice(error.message);
    // Detect expiry without extending the native deadline.
    try { state((await send("status")).unlocked); } catch { state(false); $("setup").open = true; }
  } finally {
    busy = false;
    for (const button of document.querySelectorAll("button")) button.disabled = false;
  }
}

async function accounts() {
  if (!site) return;
  const { items } = await send("suggestions");
  $("accounts").replaceChildren();
  $("account").replaceChildren(new Option("New login", ""));
  $("count").textContent = items.length;
  $("empty").hidden = items.length > 0;
  for (const item of items) {
    const row = document.createElement("div");
    row.className = "account";
    const labels = document.createElement("div");
    const title = document.createElement("strong");
    title.textContent = item.title || "Untitled login";
    const username = document.createElement("small");
    username.textContent = item.username || "No username";
    labels.append(title, username);
    const button = document.createElement("button");
    button.type = "button";
    button.textContent = "Fill";
    button.setAttribute("aria-label", `Fill ${item.title || "login"} for ${item.username || "this account"}`);
    button.addEventListener("click", () => perform(async () => {
      await send("fill", { entryId: item.id });
      notice("Login filled. Submit the form when you’re ready.", true);
    }));
    row.append(labels, button);
    $("accounts").append(row);
    $("account").append(new Option(`${item.title || "Login"} · ${item.username || "No username"}`, item.id));
  }
}

$("unlock-form").addEventListener("submit", (event) => {
  event.preventDefault();
  // Clear the field immediately; never persist path or password in browser storage.
  let password = $("master").value;
  $("master").value = "";
  void perform(async () => {
    try {
      const result = await send("unlock", { path: $("path").value.trim(), password });
      state(result.unlocked);
      await accounts();
    } finally { password = ""; }
  });
});

$("lock").addEventListener("click", async () => {
  state(false);
  try { await send("lock"); notice("Vault locked.", true); }
  catch { notice("Connection closed. Vault will lock automatically."); }
});

$("length").addEventListener("input", () => { $("length-value").value = $("length").value; });
$("generate").addEventListener("click", () => perform(async () => {
  const result = await send("generate", { length: Number($("length").value), symbols: $("symbols").checked });
  $("generated").textContent = `${result.length}-character password ready in native memory. Save it before filling.`;
  $("generated").hidden = false;
  $("save-form").hidden = false;
}));

$("account").addEventListener("change", () => {
  const existing = Boolean($("account").value);
  $("new-fields").hidden = existing;
  $("replace-note").hidden = !existing;
});

$("save-form").addEventListener("submit", (event) => {
  event.preventDefault();
  void perform(async () => {
    try {
      await send("save_generated", { title: $("title").value, username: $("username").value, entryId: $("account").value });
      notice("Password saved and filled. Submit the website’s form to apply it there.", true);
      $("save-form").hidden = true;
      $("generated").hidden = true;
    } finally {
      await accounts();
      $("new-fields").hidden = false;
      $("replace-note").hidden = true;
    }
  });
});

async function init() {
  $("extension-status").textContent = "Extension is running. Connecting to the local vault companion…";
  $("extension-id").textContent = api.runtime.id;
  const [tab] = await api.tabs.query({ active: true, currentWindow: true });
  try {
    const { secureOrigin } = await import("./policy.js");
    site = { tabId: tab.id, origin: secureOrigin(tab.url) };
    $("origin").textContent = site.origin;
  } catch {
    $("origin").textContent = "No HTTPS website selected";
    $("site-tools").hidden = true;
    $("unsupported").hidden = false;
  }
  await perform(async () => {
    state((await send("status")).unlocked);
    if (unlocked) await accounts();
  });
  $("extension-status").textContent = "Extension is running. Vault connection status is shown below.";
}

// Only status is polled. Account labels and generated-password UI vanish on expiry.
setInterval(async () => {
  if (busy || !unlocked) return;
  try { if (!(await send("status")).unlocked) { state(false); notice("Session expired. Unlock again."); } }
  catch { state(false); notice("Native companion disconnected."); }
}, 5000);
window.addEventListener("pagehide", () => { $("master").value = ""; $("username").value = ""; });
void init().catch(() => {
  $("extension-status").textContent = "Popup opened, but extension startup failed.";
  notice("Could not open Lockbox. Reload the extension and try again.");
});
