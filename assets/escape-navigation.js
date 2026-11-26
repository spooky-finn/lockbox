// Escape climbs one level everywhere in the unlocked workspace. WebKit leaves
// focus on body after a pointer click, and body keydowns never reach Rust, so
// forward Escape from body and non-text controls. Text fields keep it, so
// Escape while typing never leaves a page; Rust's own handler covers drafts.
// The record sidebar owns its own Escape (menus, selection, body focus).
const textEntry = "input:not([type=checkbox]):not([type=radio]):not([type=button])"
    + ":not([type=submit]):not([type=range]), textarea, select, "
    + "[contenteditable]:not([contenteditable=false]), [role=textbox]";

function keydown(event) {
    if (event.key !== "Escape" || event.defaultPrevented || event.repeat || event.isComposing) return;
    if (event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) return;
    const workspace = document.querySelector(".workspace");
    if (!workspace || document.querySelector("dialog[open]")) return;
    const { target } = event;
    const unfocused = target === document.body || target === document.documentElement;
    if (unfocused && document.querySelector("#vault-tree")) return;
    if (!unfocused && (!workspace.contains(target) || target.closest(`#vault-sidebar, ${textEntry}`))) return;
    event.preventDefault();
    event.stopPropagation();
    dioxus.send("escape");
}

// Capture runs before Dioxus's delegated listener, so Rust never sees it twice.
document.addEventListener("keydown", keydown, true);
try {
    await dioxus.recv();
} finally {
    document.removeEventListener("keydown", keydown, true);
}
