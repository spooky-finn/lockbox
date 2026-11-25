function navigateVault(event) {
    if (event.defaultPrevented || event.isComposing || event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) return;
    const arrow = event.key === "ArrowLeft" ? ".unlock-previous"
        : event.key === "ArrowRight" ? ".unlock-next" : null;
    if (!arrow) return;

    const screen = document.querySelector(".base-unlock");
    if (!screen) return;
    // A pointer click can leave the WebView focused on the page background.
    const { target } = event;
    if (target !== document.body && target !== document.documentElement && !screen.contains(target)) return;
    if (screen.querySelector("#space-switch-trigger[aria-expanded=true]")) return;

    const button = screen.querySelector(arrow);
    if (!button || button.disabled) return;
    event.preventDefault();
    button.click();
}

document.addEventListener("keydown", navigateVault);
try {
    await dioxus.recv();
} finally {
    document.removeEventListener("keydown", navigateVault);
}
