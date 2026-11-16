// Focus stays inside the visible list. Enter/Space activate native buttons;
// arrows never select a record or expose a secret merely by moving focus.
const tree = document.querySelector("#vault-tree");
const rows = () => [...tree.querySelectorAll(".entry-row:not(:disabled), .folder-label:not(:disabled)")];
const focus = (row) => {
    if (!row) return;
    row.focus({ preventScroll: true });
    row.scrollIntoView({ block: "nearest", inline: "nearest" });
};
const keydown = (event) => {
    if (event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) return;
    const row = event.target.closest(".entry-row, .folder-label, .tree-disclosure");
    if (!row || row.disabled) return;
    const current = row.classList.contains("tree-disclosure")
        ? row.parentElement.querySelector(".folder-label") : row;
    const visible = rows();
    const index = visible.indexOf(current);
    if (index < 0) return;
    const folder = current.closest(".tree-folder");
    const toggle = current.classList.contains("folder-label")
        ? current.parentElement.querySelector(".tree-disclosure") : null;
    switch (event.key) {
        case "ArrowDown": focus(visible[Math.min(index + 1, visible.length - 1)]); break;
        case "ArrowUp": focus(visible[Math.max(index - 1, 0)]); break;
        case "Home": focus(visible[0]); break;
        case "End": focus(visible.at(-1)); break;
        case "ArrowRight":
            if (toggle?.getAttribute("aria-expanded") === "false" && !toggle.disabled) toggle.click();
            else if (toggle) focus(folder?.querySelector(".tree-branch .entry-row:not(:disabled), .tree-branch .folder-label:not(:disabled)"));
            break;
        case "ArrowLeft":
            if (toggle?.getAttribute("aria-expanded") === "true" && !toggle.disabled) toggle.click();
            else {
                const parent = current.closest("li")?.parentElement?.closest(".tree-folder");
                focus(parent?.querySelector(".folder-label"));
            }
            break;
        default: return;
    }
    event.preventDefault();
    event.stopPropagation();
};
tree.addEventListener("keydown", keydown);
try {
    await dioxus.recv();
} finally {
    tree.removeEventListener("keydown", keydown);
}
