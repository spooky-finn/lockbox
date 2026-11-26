import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const source = await readFile(new URL("../assets/guarded-copy.js", import.meta.url), "utf8");
const AsyncFunction = Object.getPrototypeOf(async function () {}).constructor;

function fixture() {
    let copy;
    let finish;
    const sent = [];
    const page = { selection: "", activeElement: null, sent };
    const document = {
        get activeElement() { return page.activeElement; },
        getSelection: () => ({ toString: () => page.selection }),
        addEventListener(type, handler, capture) {
            assert.equal(type, "copy");
            assert.equal(capture, true);
            copy = handler;
        },
        removeEventListener(type, handler, capture) {
            assert.equal(type, "copy");
            assert.equal(capture, true);
            assert.equal(handler, copy);
            copy = undefined;
        },
    };
    const done = new AsyncFunction("document", "dioxus", source)(document, {
        send(text) { sent.push(text); },
        recv: () => new Promise(resolve => { finish = resolve; }),
    });
    return Object.assign(page, {
        copy(flags = {}) {
            const event = {
                isTrusted: true, ...flags,
                preventDefault() { this.defaultPrevented = true; },
            };
            copy?.(event);
            return event;
        },
        async close() { finish(); await done; },
    });
}

test("Cmd+C on selected secret text goes to lockbox instead of the pasteboard", async () => {
    const page = fixture();
    page.selection = "fictional-s3cret";
    assert.equal(page.copy().defaultPrevented, true);
    assert.deepEqual(page.sent, ["fictional-s3cret"]);
    await page.close();
});

test("a revealed password field copies only its selected part", async () => {
    const page = fixture();
    page.activeElement = {
        tagName: "INPUT", type: "text", value: "fictional-password",
        selectionStart: 10, selectionEnd: 18,
    };
    assert.equal(page.copy().defaultPrevented, true);
    assert.deepEqual(page.sent, ["password"]);
    await page.close();
});

test("hidden password fields, empty selections and synthetic events are left alone", async () => {
    const page = fixture();
    page.activeElement = {
        tagName: "INPUT", type: "password", value: "fictional-password",
        selectionStart: 0, selectionEnd: 18,
    };
    page.selection = "fictional-password";
    assert.equal(page.copy().defaultPrevented, undefined);
    page.activeElement = null;
    page.selection = "";
    assert.equal(page.copy().defaultPrevented, undefined);
    page.selection = "fictional-s3cret";
    assert.equal(page.copy({ isTrusted: false }).defaultPrevented, undefined);
    assert.deepEqual(page.sent, []);
    await page.close();
});

test("closing the script removes the listener", async () => {
    const page = fixture();
    await page.close();
    page.selection = "fictional-s3cret";
    assert.equal(page.copy().defaultPrevented, undefined);
    assert.deepEqual(page.sent, []);
});
