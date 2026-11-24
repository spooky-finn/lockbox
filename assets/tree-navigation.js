// Browser chooses visible neighbors; Rust owns selection and unsaved-draft guards.
const tree = document.querySelector("#vault-tree");
const workspace = tree.closest(".workspace");
const entryRows = ".entry-row:not(:disabled)";
const treeRows = `${entryRows}, .folder-label:not(:disabled)`;
const nativeControls = "input, textarea, select, [contenteditable]:not([contenteditable=false]), "
    + "[role=textbox], [role=separator], [role=slider], summary, dialog, [role=dialog]";
const navigationKeys = new Set(["ArrowDown", "ArrowUp", "ArrowLeft", "ArrowRight", "Home", "End"]);

function currentRow(rows) {
    return rows.find(row => row.getAttribute("aria-current") === "true");
}

function adjacentRow(rows, current, key) {
    const index = rows.indexOf(current);
    if (key === "ArrowDown") return rows[Math.min(index + 1, rows.length - 1)];
    return index === -1 ? rows.at(-1) : rows[Math.max(index - 1, 0)];
}

function selectionAction(row, current) {
    return row && row !== current
        ? { kind: row.dataset.kind, id: row.dataset.id }
        : { kind: "activity" };
}

function navigateSearch(input, key) {
    if ((key !== "ArrowDown" && key !== "ArrowUp") || !input.value.trim()) return;
    const rows = [...tree.querySelectorAll(entryRows)];
    if (!rows.length) return;

    const current = currentRow(rows);
    const next = adjacentRow(rows, current, key);
    next.scrollIntoView({ block: "nearest" });
    return selectionAction(next, current);
}

function navigateTree(target, key) {
    if (target.closest(nativeControls)) return;
    const row = target.closest(".entry-row, .folder-label, .tree-disclosure");
    if (row?.disabled || (!row && tree.dataset.navigateRecords !== "true")) return;
    const rows = [...tree.querySelectorAll(treeRows)];
    if (!rows.length) return;

    const focused = row?.classList.contains("tree-disclosure")
        ? row.parentElement.querySelector(".folder-label") : row;
    const current = currentRow(rows) ?? focused;
    let next;
    switch (key) {
        case "ArrowDown":
        case "ArrowUp":
            next = adjacentRow(rows, current, key);
            break;
        case "Home":
            next = rows[0];
            break;
        case "End":
            next = rows.at(-1);
            break;
        case "ArrowRight":
        case "ArrowLeft": {
            const expanding = key === "ArrowRight";
            const disclosure = current?.classList.contains("folder-label")
                ? current.parentElement.querySelector(".tree-disclosure") : null;
            const toggleFrom = expanding ? "false" : "true";
            if (disclosure && !disclosure.disabled && disclosure.getAttribute("aria-expanded") === toggleFrom) {
                return { kind: "toggle", id: current.dataset.id };
            }
            if (expanding && disclosure) {
                next = current.closest(".tree-folder")?.querySelector(
                    ".tree-branch .entry-row:not(:disabled), .tree-branch .folder-label:not(:disabled)",
                );
            } else if (!expanding) {
                const parent = current?.closest("li")?.parentElement?.closest(".tree-folder");
                next = parent?.querySelector(".folder-label");
            }
            break;
        }
    }

    next?.focus({ preventScroll: true });
    return selectionAction(next, current);
}

function keydown(event) {
    const { key, target } = event;
    if (event.defaultPrevented || !tree.isConnected) return;
    // Native WebViews can leave focus on body after a pointer selection.
    const unfocused = target === document.body || target === document.documentElement;
    if (!unfocused && !workspace.contains(target)) return;
    if (event.altKey || event.ctrlKey || event.metaKey || event.shiftKey || event.isComposing) return;
    if (workspace.querySelector("dialog[open]")) return;
    // Focused controls reach Rust's own Escape handler; body focus never does.
    if (key === "Escape" && unfocused && !event.repeat) {
        event.preventDefault();
        event.stopPropagation();
        dioxus.send({ kind: "close" });
        return;
    }
    if (!navigationKeys.has(key)) return;

    const action = target.id === "search" ? navigateSearch(target, key) : navigateTree(target, key);
    if (!action) return;
    event.preventDefault();
    event.stopPropagation();
    dioxus.send(action);
}

// Consume tree navigation before Dioxus's delegated synchronous DOM listener.
document.addEventListener("keydown", keydown, true);
try {
    await dioxus.recv();
} finally {
    document.removeEventListener("keydown", keydown, true);
}
