// Native connection lifecycle; page policy and request ordering stay in background.js.
const HOST = "app.lockbox.browser";

export function createNativeSession(runtime) {
  let port = null;
  let sequence = 0;
  let cancellation = 0;
  const pending = new Map();

  function disconnect() {
    cancellation++;
    const old = port;
    port = null;
    old?.disconnect();
    for (const { reject, timer } of pending.values()) {
      clearTimeout(timer);
      reject(new Error("Browser request cancelled or desktop disconnected."));
    }
    pending.clear();
  }

  function native(request) {
    if (!port) {
      const connection = runtime.connectNative(HOST);
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
        void runtime.lastError; // Consume Chrome's error without logging sensitive context.
        if (connection === port) disconnect();
      });
    }
    const id = ++sequence;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        disconnect();
        reject(new Error("Native host timed out. Check the companion installation."));
      }, 125_000);
      pending.set(id, { resolve, reject, timer });
      try { port.postMessage({ id, request }); }
      catch { disconnect(); }
    });
  }

  return {
    request: native,
    disconnect,
    get cancellation() { return cancellation; },
  };
}
