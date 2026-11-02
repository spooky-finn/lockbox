import test from "node:test";
import assert from "node:assert/strict";
import { inspectPage, fillPage } from "../src/page.js";

// DOM boundary fixtures deliberately exercise the serialized functions without
// adding a content script, real credentials, or an external test dependency.
function fixture({ type = "submit", method = "get", associated = true, disabled = false, action } = {}) {
  const previous = new Map(["window", "location", "innerHeight", "innerWidth", "getComputedStyle", "HTMLInputElement", "document", "__lockboxFill"].map((key) => [key, Object.getOwnPropertyDescriptor(globalThis, key)]));
  class Input {
    constructor(type, name) {
      Object.assign(this, { type, name, id: name, autocomplete: "", isConnected: true, disabled: false, readOnly: false, parentElement: null, maxLength: -1 });
    }
    set value(value) { this.text = value; }
    get value() { return this.text || ""; }
    closest() { return null; }
    hasAttribute() { return false; }
    getBoundingClientRect() { return { width: 200, height: 30, bottom: 100, right: 220, top: 70, left: 20 }; }
    getClientRects() { return [this.getBoundingClientRect()]; }
    dispatchEvent() { return true; }
  }
  const form = { method: "post", action: "https://accounts.example/login" };
  const username = new Input("text", "username");
  const password = new Input("password", "password");
  username.form = password.form = form;
  // The control is outside form.elements, covering external associations and
  // image submit inputs (which browsers omit from form.elements).
  const control = { type, formMethod: method, formAction: action, form: associated ? form : {}, disabled,
    hasAttribute: (name) => name === "formmethod" || (name === "formaction" && Boolean(action)) };
  form.elements = [username, password];
  globalThis.window = {}; window.top = window;
  globalThis.location = new URL(form.action);
  globalThis.innerHeight = 800; globalThis.innerWidth = 1200;
  globalThis.getComputedStyle = () => ({ opacity: "1", contentVisibility: "visible", visibility: "visible", display: "block" });
  globalThis.HTMLInputElement = Input;
  globalThis.document = { activeElement: password, querySelectorAll: (selector) =>
    selector.startsWith("button") ? [control] : selector.includes("password") ? [password] : [username, password] };
  return { control, username, password, restore() {
    for (const [key, descriptor] of previous) {
      if (descriptor) Object.defineProperty(globalThis, key, descriptor);
      else delete globalThis[key];
    }
  } };
}

for (const type of ["submit", "image"]) {
  test(`refuses external action on associated ${type} control omitted from form.elements`, () => {
    const f = fixture({ type, method: "post", action: "https://attacker.test/collect" });
    try { assert.ok(inspectPage(location.origin, "login").error); }
    finally { f.restore(); }
  });
  test(`refuses GET override on associated ${type} control before filling`, () => {
    const f = fixture({ type });
    try {
      assert.ok(inspectPage(location.origin, "login").error);
      assert.equal(f.password.value, "");
    } finally { f.restore(); }
  });
}

test("refuses method override added after inspection and consumes the token", () => {
  const f = fixture({ method: "post" });
  try {
    const { token } = inspectPage(location.origin, "login");
    assert.ok(token);
    f.control.formMethod = "get";
    assert.ok(fillPage(location.origin, token, "fictional-user", "fictional-password", "login").error);
    assert.equal(f.password.value, "");
    f.control.formMethod = "post";
    assert.ok(fillPage(location.origin, token, "fictional-user", "fictional-password", "login").error);
  } finally { f.restore(); }
});

for (const options of [{ method: "post" }, { associated: false }, { disabled: true }, { type: "button" }]) {
  test(`safe POST remains fillable with unrelated/inactive controls ${JSON.stringify(options)}`, () => {
    const f = fixture(options);
    try {
      const { token } = inspectPage(location.origin, "login");
      assert.ok(token);
      assert.equal(fillPage(location.origin, token, "fictional-user", "fictional-password", "login").filled, true);
      assert.equal(f.password.value, "fictional-password");
    } finally { f.restore(); }
  });
}
