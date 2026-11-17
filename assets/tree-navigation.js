// Browser chooses visible neighbors; Rust owns selection and unsaved-draft guards.
const tree = document.querySelector("#vault-tree");
const workspace = tree.closest(".workspace");
const rows = () => [...tree.querySelectorAll(".entry-row:not(:disabled), .folder-label:not(:disabled)")];
const select = (row, current) => {
    if (!row) return;
    row.focus({ preventScroll: true });
    if (row !== current) {
        return { kind: row.dataset.kind, id: row.dataset.id };
    }
};
const keydown = (event) => {
    if (event.altKey || event.ctrlKey || event.metaKey || event.shiftKey || event.isComposing) return;
    if (event.target.closest("input, textarea, select, [contenteditable]:not([contenteditable=false]), [role=textbox], [role=separator], [role=slider], summary, dialog, [role=dialog]")) return;
    if (workspace.querySelector("dialog[open]")) return;
    const row = event.target.closest(".entry-row, .folder-label, .tree-disclosure");
    if (row?.disabled || (!row && tree.dataset.navigateRecords !== "true")) return;
    const visible = rows();
    if (!visible.length) return;
    const focused = row?.classList.contains("tree-disclosure")
        ? row.parentElement.querySelector(".folder-label") : row;
    const current = visible.find(item => item.getAttribute("aria-current") === "true") ?? focused;
    const index = visible.indexOf(current);
    const folder = current?.closest(".tree-folder");
    const toggle = current?.classList.contains("folder-label")
        ? current.parentElement.querySelector(".tree-disclosure") : null;
    let action;
    switch (event.key) {
        case "ArrowDown": action = select(visible[Math.min(index + 1, visible.length - 1)], current); break;
        case "ArrowUp": action = select(visible[index < 0 ? visible.length - 1 : Math.max(index - 1, 0)], current); break;
        case "Home": action = select(visible[0], current); break;
        case "End": action = select(visible.at(-1), current); break;
        case "ArrowRight":
            if (toggle?.getAttribute("aria-expanded") === "false" && !toggle.disabled) action = { kind: "toggle", id: current.dataset.id };
            else if (toggle) action = select(folder?.querySelector(".tree-branch .entry-row:not(:disabled), .tree-branch .folder-label:not(:disabled)"), current);
            break;
        case "ArrowLeft":
            if (toggle?.getAttribute("aria-expanded") === "true" && !toggle.disabled) action = { kind: "toggle", id: current.dataset.id };
            else {
                const parent = current?.closest("li")?.parentElement?.closest(".tree-folder");
                action = select(parent?.querySelector(".folder-label"), current);
            }
            break;
        default: return;
    }
    event.preventDefault();
    event.stopPropagation();
    dioxus.send(action ?? { kind: "activity" });
};
workspace.addEventListener("keydown", keydown);
try {
    await dioxus.recv();
} finally {
    workspace.removeEventListener("keydown", keydown);
}
