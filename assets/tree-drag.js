// Track an in-app drag without starting an OS drag or writing to a pasteboard.
const sidebar = document.getElementById("vault-sidebar");
const scroll = sidebar.querySelector(".tree-scroll");
const rows = "#vault-tree [data-kind][data-id]";
const listeners = new AbortController();
const options = { signal: listeners.signal };
const capture = { ...options, capture: true };
let drag = null;
let hovered = null;
let frame = 0;
let suppressClick = false;

function highlight(target) {
    if (target === hovered) return;
    hovered?.classList.remove("tree-drop-target");
    hovered = target;
    hovered?.classList.add("tree-drop-target");
}

function finish() {
    if (!drag) return;
    const { row, id } = drag;
    drag = null;
    if (row.hasPointerCapture(id)) row.releasePointerCapture(id);
    row.classList.remove("tree-drag-source");
    sidebar.classList.remove("tree-dragging");
    highlight(null);
    cancelAnimationFrame(frame);
    frame = 0;
}

function canDrag(row) {
    return row && sidebar.contains(row) && !row.disabled && sidebar.dataset.treeCanDrag === "true";
}

function targetAt(x, y) {
    const target = document.elementFromPoint(x, y)?.closest("[data-drop-folder], [data-drop-root]");
    if (!target || !sidebar.contains(target) || drag.folder?.contains(target)) return null;
    return target;
}

function scrollStep() {
    const bounds = scroll.getBoundingClientRect();
    const { x, y } = drag;
    if (x < bounds.left || x >= bounds.right || y < bounds.top || y >= bounds.bottom) return 0;
    const edge = Math.min(28, bounds.height / 3);
    if (y < bounds.top + edge) return -8;
    if (y > bounds.bottom - edge) return 8;
    return 0;
}

function updateHover() {
    highlight(targetAt(drag.x, drag.y));
    if (frame || !scrollStep()) return;
    frame = requestAnimationFrame(() => {
        frame = 0;
        if (!canDrag(drag.row)) {
            finish();
            return;
        }
        const before = scroll.scrollTop;
        scroll.scrollTop += scrollStep();
        if (scroll.scrollTop !== before) updateHover();
    });
}

window.addEventListener("pointerdown", event => {
    if (!event.isPrimary || event.button !== 0) return;
    finish();
    suppressClick = false;
    if (event.pointerType === "touch") return;
    const row = event.target.closest?.(rows);
    if (!canDrag(row)) return;
    const folder = row.dataset.kind === "directory" ? row.closest(".tree-folder") : null;
    drag = { row, folder, id: event.pointerId, x: event.clientX, y: event.clientY, active: false };
}, capture);

window.addEventListener("pointermove", event => {
    if (!drag || event.pointerId !== drag.id) return;
    // WebKit can report buttons=0 during a drag. Track the pointer from down to up.
    if (!canDrag(drag.row)) {
        finish();
        return;
    }
    if (!drag.active) {
        if (Math.hypot(event.clientX - drag.x, event.clientY - drag.y) < 5) return;
        const { row, id } = drag;
        row.setPointerCapture(id);
        drag.active = true;
        suppressClick = true;
        row.classList.add("tree-drag-source");
        sidebar.classList.add("tree-dragging");
    }
    drag.x = event.clientX;
    drag.y = event.clientY;
    updateHover();
}, capture);

window.addEventListener("pointerup", event => {
    if (!drag || event.pointerId !== drag.id || event.button !== 0) return;
    const target = drag.active && canDrag(drag.row) && targetAt(event.clientX, event.clientY);
    if (target) {
        const { kind, id } = drag.row.dataset;
        dioxus.send({
            item: { kind: kind === "directory" ? "folder" : kind, id },
            parent: target.dataset.dropFolder ?? null,
        });
    }
    finish();
}, capture);

for (const type of ["pointercancel", "lostpointercapture"]) {
    window.addEventListener(type, event => {
        if (event.pointerId === drag?.id) finish();
    }, capture);
}
window.addEventListener("blur", finish, options);
document.documentElement.addEventListener("pointerleave", event => {
    if (!drag?.active && event.pointerId === drag?.id) finish();
}, options);
sidebar.addEventListener("contextmenu", finish, options);
window.addEventListener("keydown", event => {
    if (drag?.active && event.key === "Escape") {
        event.preventDefault();
        event.stopImmediatePropagation();
        finish();
    }
}, capture);
window.addEventListener("click", event => {
    if (suppressClick && event.detail > 0) {
        suppressClick = false;
        event.preventDefault();
        event.stopImmediatePropagation();
    }
}, capture);
sidebar.addEventListener("dragstart", event => {
    if (event.target.closest?.(rows)) event.preventDefault();
}, options);

try {
    await dioxus.recv();
} finally {
    listeners.abort();
    finish();
}
