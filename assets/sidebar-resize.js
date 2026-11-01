// This evaluator lives with Sidebar; Rust signals unmount through dioxus.recv().
const sidebar = document.getElementById("vault-sidebar");
const handle = document.getElementById("sidebar-resizer");
const workspace = sidebar.parentElement;
const listeners = new AbortController();
const options = { signal: listeners.signal };
let drag = null;

const maximum = () => Math.max(200, Math.min(480, workspace.clientWidth - 320));
const width = () => sidebar.getBoundingClientRect().width;
const setWidth = value => {
    workspace.style.setProperty("--sidebar-width", `${Math.max(200, Math.min(maximum(), value))}px`);
};
const stop = () => {
    if (drag && handle.hasPointerCapture(drag.id)) handle.releasePointerCapture(drag.id);
    drag = null;
    workspace.classList.remove("sidebar-resizing");
};

handle.addEventListener("pointerdown", event => {
    if (event.button !== 0 || !event.isPrimary) return;
    event.preventDefault();
    handle.focus();
    drag = { id: event.pointerId, x: event.clientX, width: width() };
    handle.setPointerCapture(event.pointerId);
    workspace.classList.add("sidebar-resizing");
}, options);
handle.addEventListener("pointermove", event => {
    if (drag && drag.id === event.pointerId) setWidth(drag.width + event.clientX - drag.x);
}, options);
for (const name of ["pointerup", "pointercancel", "lostpointercapture"]) {
    handle.addEventListener(name, stop, options);
}
handle.addEventListener("keydown", event => {
    let next;
    switch (event.key) {
        case "ArrowLeft": next = width() - (event.shiftKey ? 40 : 10); break;
        case "ArrowRight": next = width() + (event.shiftKey ? 40 : 10); break;
        case "Home": next = 200; break;
        case "End": next = maximum(); break;
        default: return;
    }
    event.preventDefault();
    setWidth(next);
}, options);
handle.addEventListener("dblclick", () => workspace.style.removeProperty("--sidebar-width"), options);
window.addEventListener("blur", stop, options);

const observer = new ResizeObserver(() => {
    handle.setAttribute("aria-valuenow", String(Math.round(width())));
    handle.setAttribute("aria-valuemax", String(maximum()));
    if (window.innerWidth <= 640) stop();
});
observer.observe(sidebar);
observer.observe(workspace);

try {
    await dioxus.recv();
} finally {
    stop();
    observer.disconnect();
    listeners.abort();
}
