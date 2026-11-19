// Report continuous input without waiting for Rust on every DOM event.
const app = document.querySelector("main.app");
if (!app) return;

let last = -Infinity;
const activity = event => {
    if (!event.isTrusted) return;
    const now = performance.now();
    if (now - last < 1000) return;
    last = now;
    dioxus.send(null);
};

app.addEventListener("pointermove", activity, { passive: true });
app.addEventListener("wheel", activity, { passive: true });
try {
    await dioxus.recv();
} finally {
    app.removeEventListener("pointermove", activity);
    app.removeEventListener("wheel", activity);
}
