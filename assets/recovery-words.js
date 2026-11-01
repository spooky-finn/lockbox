// Capture multiline phrases before text inputs strip clipboard line breaks.
// This listener belongs to the editor element and leaves with its DOM subtree.
document.getElementById("recovery-words").addEventListener("paste", event => {
    const field = event.target;
    if (!(field instanceof HTMLInputElement) || field.disabled) return;

    const text = event.clipboardData?.getData("text/plain");
    if (!text || !/\s/u.test(text.trim())) return;

    event.preventDefault();
    field.value = text.trim().replace(/\s+/gu, " ");
    field.dispatchEvent(new Event("input", { bubbles: true }));
});
