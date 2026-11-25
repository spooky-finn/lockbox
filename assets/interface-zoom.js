const scales = [0.5, 0.67, 0.8, 0.9, 1, 1.1, 1.25, 1.5, 1.75, 2];
const defaultLevel = scales.indexOf(1);
let level = defaultLevel;

// Listen on the window so shortcuts also work when no control has focus.
const zoom = event => {
    if (!event.isTrusted || event.isComposing || event.altKey || !(event.metaKey || event.ctrlKey)) {
        return;
    }

    switch (event.key) {
        case "+":
        case "=":
            level = Math.min(level + 1, scales.length - 1);
            break;
        case "-":
            level = Math.max(level - 1, 0);
            break;
        case "0":
            level = defaultLevel;
            break;
        default:
            return;
    }

    event.preventDefault();
    event.stopPropagation();
    const scale = scales[level];
    document.documentElement.style.setProperty("--interface-zoom", scale);
    dioxus.send(scale);
};

window.addEventListener("keydown", zoom, true);
try {
    await dioxus.recv();
} finally {
    window.removeEventListener("keydown", zoom, true);
}
