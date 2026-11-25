import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const source = await readFile(new URL("../assets/unlock-navigation.js", import.meta.url), "utf8");
const AsyncFunction = Object.getPrototypeOf(async function () {}).constructor;

function fixture() {
    let keydown;
    let finish;
    const clicks = [];
    const password = {};
    const previous = { click: () => clicks.push("previous") };
    const next = { click: () => clicks.push("next") };
    const app = { unlocking: true, pickerOpen: false, password, previous, next, clicks };
    const screen = {
        contains: target => target === password,
        querySelector(selector) {
            if (selector === ".unlock-previous") return app.previous;
            if (selector === ".unlock-next") return app.next;
            assert.equal(selector, "#space-switch-trigger[aria-expanded=true]");
            return app.pickerOpen ? {} : null;
        },
    };
    const body = {};
    const documentElement = {};
    const done = new AsyncFunction("document", "dioxus", source)({
        body, documentElement,
        querySelector(selector) {
            assert.equal(selector, ".base-unlock");
            return app.unlocking ? screen : null;
        },
        addEventListener(type, handler) {
            assert.equal(type, "keydown");
            keydown = handler;
        },
        removeEventListener(type, handler) {
            assert.equal(type, "keydown");
            assert.equal(handler, keydown);
            keydown = undefined;
        },
    }, { recv: () => new Promise(resolve => { finish = resolve; }) });
    return Object.assign(app, {
        body, documentElement,
        press(key, target = body, flags = {}) {
            const event = {
                key, target, ...flags,
                preventDefault() { this.defaultPrevented = true; },
            };
            keydown?.(event);
            return event;
        },
        async close() { finish(); await done; },
    });
}

test("arrows activate vault controls with focus inside the panel or on the background", async () => {
    const app = fixture();
    for (const [key, target] of [
        ["ArrowRight", app.body],
        ["ArrowLeft", app.documentElement],
        ["ArrowRight", app.password],
    ]) {
        const event = app.press(key, target);
        assert.ok(event.defaultPrevented);
    }
    assert.deepEqual(app.clicks, ["next", "previous", "next"]);
    await app.close();
    assert.equal(app.press("ArrowRight").defaultPrevented, undefined);
});

test("arrows respect other views, the vault picker, and unavailable controls", async () => {
    const app = fixture();
    assert.equal(app.press("ArrowRight", {}).defaultPrevented, undefined);
    app.press("ArrowRight", app.password, { defaultPrevented: true });
    assert.equal(app.press("ArrowRight", app.password, { shiftKey: true }).defaultPrevented, undefined);
    assert.equal(app.press("ArrowLeft", app.password, { isComposing: true }).defaultPrevented, undefined);
    app.pickerOpen = true;
    assert.equal(app.press("ArrowLeft").defaultPrevented, undefined);
    app.pickerOpen = false;
    app.next.disabled = true;
    app.previous = null;
    assert.equal(app.press("ArrowRight").defaultPrevented, undefined);
    assert.equal(app.press("ArrowLeft").defaultPrevented, undefined);
    app.next.disabled = false;
    app.unlocking = false;
    assert.equal(app.press("ArrowRight").defaultPrevented, undefined);
    assert.deepEqual(app.clicks, []);
    await app.close();
});
