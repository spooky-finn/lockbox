import { secureOrigin } from "./policy.js";

// No page scripts or credentials: ask the desktop only whether this origin matches.
// A match only sets the badge. Opening the popup on its own would take focus from
// the page, letting the site see whether a login for it is saved.
export function installAutomatic(api, lookup, isPending) {
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
    } catch { /* Desktop absent, locked, or browser changed during lookup. */ }
    finally { checking = false; }
  }
  api.tabs.onUpdated.addListener((id, change) => {
    if (change.status === "complete" || change.url) void check();
  });
  api.tabs.onActivated.addListener(() => { void check(); });
  api.windows.onFocusChanged.addListener(() => { void check(); });
  // Detect desktop unlock while the user stays on the same page.
  const timer = setInterval(() => { void check(); }, 5000);
  api.runtime.onSuspend?.addListener(() => clearInterval(timer));
  void check();
  return check;
}
