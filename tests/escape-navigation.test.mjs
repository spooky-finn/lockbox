import { readFile } from "node:fs/promises";
import test from "node:test";
import assert from "node:assert/strict";

const source = await readFile(new URL("../assets/escape-navigation.js", import.meta.url), "utf8");
const AsyncFunction = Object.getPrototypeOf(async function () {}).constructor;

// `kind` says which selector family the element matches.
function element(kind, inside = true) {
    return {
        inside,
        closest(selector) {
            if (kind === "text" && selector.includes("textarea")) return {};
            if (kind === "sidebar" && selector.includes("#vault-sidebar")) return {};
            return null;
        },
    };
}

function fixture({ tree = false, dialog = false } = {}) {
    let listener = null;
    let finish;
    const messages = [];
    const body = {};
    const documentElement = {};
    const workspace = { contains: target => target.inside === true };
    const done = new AsyncFunction("document", "dioxus", source)(
        {
            body, documentElement,
            querySelector(selector) {
                if (selector === ".workspace") return workspace;
                if (selector === "#vault-tree") return tree ? {} : null;
                if (selector === "dialog[open]") return dialog ? {} : null;
                return null;
            },
            addEventListener(type, callback, capture) {
                assert.equal(type, "keydown");
                assert.equal(capture, true);
                listener = callback;
            },
            removeEventListener(_, callback) {
                assert.equal(callback, listener);
                listener = null;
            },
        },
        {
            recv: () => new Promise(resolve => { finish = resolve; }),
            send: message => messages.push(message),
        },
    );
    return {
        messages, body,
        key(target, key = "Escape", modifiers = {}) {
            const event = { target, key, ...modifiers,
                preventDefault() { this.prevented = true; },
                stopPropagation() { this.stopped = true; },
            };
            listener(event);
            return event;
        },
        async close() { finish(); await done; assert.equal(listener, null); },
    };
}

test("body Escape climbs a level on pages without the record sidebar", async () => {
    const app = fixture();
    const event = app.key(app.body);
    assert.deepEqual(app.messages, ["escape"]);
    assert.ok(event.prevented && event.stopped);
    await app.close();
});

test("body Escape stays with the record sidebar when it is mounted", async () => {
    const app = fixture({ tree: true });
    assert.equal(app.key(app.body).prevented, undefined);
    assert.deepEqual(app.messages, []);
    await app.close();
});

test("focused page controls climb, text fields and the sidebar keep Escape", async () => {
    const app = fixture({ tree: true });
    app.key(element("button"));
    assert.deepEqual(app.messages, ["escape"]);
    for (const target of [element("text"), element("sidebar"), element("button", false)]) {
        assert.equal(app.key(target).prevented, undefined);
    }
    assert.deepEqual(app.messages, ["escape"]);
    await app.close();
});

test("dialogs, modifiers, repeats, composition and other keys are ignored", async () => {
    const dialog = fixture({ dialog: true });
    dialog.key(dialog.body);
    assert.deepEqual(dialog.messages, []);
    await dialog.close();

    const app = fixture();
    for (const modifiers of [{ shiftKey: true }, { metaKey: true }, { altKey: true },
        { ctrlKey: true }, { repeat: true }, { isComposing: true }, { defaultPrevented: true }]) {
        app.key(app.body, "Escape", modifiers);
    }
    app.key(app.body, "Enter");
    assert.deepEqual(app.messages, []);
    await app.close();
});
