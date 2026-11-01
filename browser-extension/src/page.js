// These functions are serialized into the browser's ISOLATED execution world.
// Keep them self-contained. Nothing runs in a page before an explicit fill click.
export function inspectPage(expectedOrigin, mode) {
  const fail = (error) => ({ error });
  globalThis.__lockboxFill = null;
  if (window.top !== window || location.origin !== expectedOrigin || location.protocol !== "https:") {
    return fail("The page changed. Open Lockbox again.");
  }
  const visible = (input) => {
    const style = getComputedStyle(input);
    const rect = input.getBoundingClientRect();
    const ancestorsVisible = () => {
      for (let element = input; element; element = element.parentElement) {
        const parentStyle = getComputedStyle(element);
        if (Number(parentStyle.opacity) === 0 || parentStyle.contentVisibility === "hidden") return false;
      }
      return true;
    };
    return input.isConnected && !input.disabled && !input.readOnly && !input.closest("[inert]") && ancestorsVisible() &&
      style.visibility === "visible" && style.display !== "none" && Number(style.opacity) > 0 &&
      input.getClientRects().length > 0 && rect.width >= 8 && rect.height >= 8 &&
      rect.bottom > 0 && rect.right > 0 && rect.top < innerHeight && rect.left < innerWidth;
  };
  const all = [...document.querySelectorAll('input[type="password"]')].filter(visible);
  if (!all.length) return fail("No visible password field. Scroll to the form and try again.");
  const active = all.find((input) => input === document.activeElement);
  const forms = new Set(all.map((input) => input.form));
  if (!active && forms.size !== 1) return fail("Several login forms found. Focus the password field first.");
  const form = active ? active.form : all[0].form;
  const candidates = all.filter((input) => input.form === form);
  const hints = (input) => `${input.name} ${input.id}`;
  const isNew = (input) => input.autocomplete === "new-password" || /new|confirm|repeat/i.test(hints(input));
  const isCurrent = (input) => input.autocomplete === "current-password" || /current|old/i.test(hints(input));
  let passwords;
  if (mode === "new") {
    passwords = candidates.filter(isNew);
    if (!passwords.length && candidates.length <= 2 && !candidates.some(isCurrent)) passwords = candidates;
  } else {
    passwords = candidates.filter((input) => !isNew(input));
    if (active && passwords.includes(active)) passwords = [active];
    if (passwords.length !== 1) return fail("Choose an unambiguous current-password field.");
  }
  if (!passwords.length || passwords.length > 2) return fail("Could not identify new-password fields safely.");
  const safeForm = () => {
    if (!form) return true;
    // GET forms can put secrets into URLs. External submit overrides are refused too.
    if (form.method.toLowerCase() !== "post") return false;
    const actions = [form.action, ...[...form.elements]
      .filter((element) => element.hasAttribute("formaction"))
      .map((element) => element.formAction)];
    return actions.every((action) => {
      const url = new URL(action, location.href);
      return url.origin === expectedOrigin && url.protocol === "https:" && !url.username && !url.password;
    });
  };
  if (!safeForm()) return fail("This form uses GET or submits to another origin. Fill was blocked.");
  const texts = [...document.querySelectorAll('input')].filter((input) =>
    input.form === form && ["text", "email"].includes(input.type) && visible(input));
  const usernames = texts.filter((input) => input.autocomplete === "username");
  const likely = texts.filter((input) => input.type === "email" || /user|email|login/i.test(hints(input)));
  const username = usernames.length === 1 ? usernames[0] : likely.length === 1 ? likely[0] :
    texts.length === 1 ? texts[0] : null;
  const token = crypto.randomUUID();
  globalThis.__lockboxFill = { token, origin: expectedOrigin, passwords, username, form, visible, safeForm, mode };
  return { token };
}

export function fillPage(expectedOrigin, token, username, password, mode) {
  const state = globalThis.__lockboxFill;
  globalThis.__lockboxFill = null; // One use, including failed attempts.
  if (!state || state.token !== token || state.origin !== expectedOrigin || state.mode !== mode ||
      window.top !== window || location.origin !== expectedOrigin || location.protocol !== "https:") {
    return { error: "The document changed. Open Lockbox again." };
  }
  const valid = (input) => state.visible(input) && input.form === state.form &&
    (input === state.username || input.type === "password");
  if (!state.safeForm() || !state.passwords.every(valid) || (state.username && !valid(state.username))) {
    return { error: "The form changed or is no longer visible. Fill was blocked." };
  }
  const targets = [...state.passwords];
  if (state.username && username) targets.unshift(state.username);
  if (targets.some((input) => input.maxLength >= 0 &&
      input.maxLength < (input === state.username ? username : password).length)) {
    return { error: "The form would truncate this credential. Nothing was filled." };
  }
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value").set;
  // Set all values before firing page handlers; handlers must not redirect a later write.
  for (const input of targets) setter.call(input, input === state.username ? username : password);
  for (const input of targets) {
    input.dispatchEvent(new Event("input", { bubbles: true }));
    input.dispatchEvent(new Event("change", { bubbles: true }));
  }
  return { filled: true };
}
