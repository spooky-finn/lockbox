import test from "node:test";
import assert from "node:assert/strict";
import { secureOrigin, trustedPopup } from "../src/policy.js";

test("HTTPS origins normalize ports and reject unsafe schemes and credentials", () => {
  assert.equal(secureOrigin("https://ACCOUNTS.example:443/login?q=1"), "https://accounts.example");
  assert.equal(secureOrigin("https://accounts.example:444/"), "https://accounts.example:444");
  for (const value of ["http://accounts.example", "https://user:pass@accounts.example", "file:///secret", "javascript:alert(1)", null]) {
    assert.throws(() => secureOrigin(value));
  }
  assert.notEqual(secureOrigin("https://accounts.example.attacker.test"), secureOrigin("https://accounts.example"));
  assert.notEqual(secureOrigin("https://sub.accounts.example"), secureOrigin("https://accounts.example"));
});

test("only exact internal popup sender can access native operations", () => {
  const runtime = { id: "extension-id", getURL: (file) => `moz-extension://internal/${file}` };
  const valid = { id: runtime.id, url: runtime.getURL("popup.html") };
  assert.equal(trustedPopup(valid, runtime), true);
  for (const sender of [{ ...valid, id: "other-extension" }, { ...valid, url: "https://attacker.test/popup.html" },
    { ...valid, url: runtime.getURL("popup.html") + "?other=1" }, { ...valid, tab: { id: 1 } }, {}]) {
    assert.equal(trustedPopup(sender, runtime), false);
  }
});
