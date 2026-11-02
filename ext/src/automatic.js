import { secureOrigin } from "./policy.js";

// No page scripts or credentials: ask the desktop only whether this origin matches.
export function installAutomatic(api, lookup, isPending) {
  const opened = new Map();
  let checking = false;
  async function check() {
    if (checking || isPending()) return;
    checking = true;
    try {
      const window = await api.windows.getLastFocused();
      if (!window.focused) return;
      const [tab] = await api.tabs.query({ active: true, windowId: window.id });
      if (!tab || tab.status === "loading") return;
      const url = tab.url;
      let origin;
      try { origin = secureOrigin(url); }
      catch { await api.action.setBadgeText({ tabId: tab.id, text: "" }); return; }
      const matched = await lookup(origin);
      if (isPending()) return;
      const current = await api.tabs.get(tab.id);
      const focused = await api.windows.getLastFocused();
      if (!current.active || focused.id !== window.id || !focused.focused || current.url !== url) return;
      await api.action.setBadgeText({ tabId: tab.id, text: matched ? "1" : "" });
      await api.action.setTitle({ tabId: tab.id, title: matched ? "Lockbox: saved login available" : "Open Lockbox" });
      if (!matched) return;
      // Once per loaded document. Closing the popup must not cause it to reopen.
      if (opened.get(tab.id) === url) return;
      opened.set(tab.id, url);
      try { await api.action.openPopup({ windowId: window.id }); }
      catch { /* Older browsers keep the badge as the manual entry point. */ }
    } catch { /* Desktop absent, locked, or browser changed during lookup. */ }
    finally { checking = false; }
  }
  api.tabs.onUpdated.addListener((id, change) => {
    if (change.status === "loading") opened.delete(id);
    if (change.status === "complete" || change.url) void check();
  });
  api.tabs.onActivated.addListener(() => { void check(); });
  api.tabs.onRemoved.addListener(id => opened.delete(id));
  api.windows.onFocusChanged.addListener(() => { void check(); });
  // Detect desktop unlock while the user stays on the same page.
  const timer = setInterval(() => { void check(); }, 5000);
  api.runtime.onSuspend?.addListener(() => clearInterval(timer));
  void check();
  return check;
}
