// The native Copy item (Cmd+C) would put revealed passwords, notes and codes
// on the pasteboard without the clipboard guard. Send the selection to lockbox,
// which marks it concealed and wipes it like its own copy buttons do.
function selectedText() {
    const field = document.activeElement;
    if (field?.tagName === "INPUT" || field?.tagName === "TEXTAREA") {
        // WebKit never copies hidden passwords; keep it that way.
        if (field.type === "password") return "";
        const { selectionStart: start, selectionEnd: end } = field;
        if (typeof start === "number" && typeof end === "number" && start !== end) {
            return field.value.slice(start, end);
        }
    }
    return document.getSelection()?.toString() ?? "";
}

function guardCopy(event) {
    if (!event.isTrusted) return;
    const text = selectedText();
    if (!text) return;
    event.preventDefault();
    dioxus.send(text);
}

document.addEventListener("copy", guardCopy, true);
try {
    await dioxus.recv();
} finally {
    document.removeEventListener("copy", guardCopy, true);
}
