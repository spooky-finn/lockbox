// Keep origin rules aligned with src/browser/mod.rs; never match by URL suffix.
export function secureOrigin(value) {
  if (typeof value !== "string" || value.length > 4096) throw new Error("Open an HTTPS website to use Lockbox.");
  const url = new URL(value);
  if (url.protocol !== "https:" || !url.hostname || url.username || url.password) {
    throw new Error("Lockbox only fills HTTPS websites without embedded credentials.");
  }
  return url.origin;
}

export function trustedPopup(sender, runtime) {
  return sender.id === runtime.id && !sender.tab && sender.url === runtime.getURL("popup.html");
}
