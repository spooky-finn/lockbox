import { readFile } from "node:fs/promises";
import test from "node:test";
import assert from "node:assert/strict";

const source = await readFile(new URL("../assets/tree-navigation.js", import.meta.url), "utf8");
const AsyncFunction = Object.getPrototypeOf(async function () {}).constructor;

function row(kind = "entry-row") {
    return {
        disabled: false, focused: false, scrolled: false,
        classList: { contains: (name) => name === kind },
        closest(selector) {
            return selector.includes(".entry-row,") ? this : null;
        },
        focus() { this.focused = true; },
        scrollIntoView() { this.scrolled = true; },
    };
}

function fixture(rows) {
    let listener;
    let finish;
    const tree = {
        querySelectorAll: () => rows.filter((item) => !item.disabled),
        addEventListener: (_, callback) => { listener = callback; },
        removeEventListener: (_, callback) => { assert.equal(callback, listener); listener = null; },
    };
    const done = new AsyncFunction("document", "dioxus", source)(
        { querySelector: () => tree },
        { recv: () => new Promise((resolve) => { finish = resolve; }) },
    );
    return {
        key(target, key, modifiers = {}) {
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

test("arrows and Home/End focus visible rows without activating records", async () => {
    const rows = [row(), row(), row()];
    const app = fixture(rows);
    for (const [start, key, expected] of [
        [0, "ArrowDown", 1], [1, "ArrowUp", 0],
        [0, "ArrowUp", 0], [2, "ArrowDown", 2],
        [0, "End", 2], [2, "Home", 0],
    ]) {
        rows.forEach((item) => { item.focused = false; });
        const event = app.key(rows[start], key);
        assert.ok(rows[expected].focused && rows[expected].scrolled);
        assert.ok(event.prevented && event.stopped);
    }
    await app.close();
});

test("modifiers, typing, activation keys and disabled rows retain native behavior", async () => {
    const rows = [row(), row()];
    const app = fixture(rows);
    for (const [key, modifiers] of [
        ["ArrowDown", { ctrlKey: true }], ["ArrowDown", { altKey: true }],
        ["ArrowDown", { metaKey: true }], ["ArrowDown", { shiftKey: true }],
        ["Enter", {}], [" ", {}], ["a", {}],
    ]) {
        assert.equal(app.key(rows[0], key, modifiers).prevented, undefined);
    }
    rows[0].disabled = true;
    assert.equal(app.key(rows[0], "ArrowDown").prevented, undefined);
    assert.ok(rows.every((item) => !item.focused));
    await app.close();
});

test("folder arrows expand and collapse using existing disclosure action", async () => {
    const folder = row("folder-label");
    let expanded = false;
    let clicks = 0;
    const toggle = {
        getAttribute: () => String(expanded),
        click() { expanded = !expanded; clicks++; },
    };
    folder.parentElement = { querySelector: () => toggle };
    const app = fixture([folder]);
    app.key(folder, "ArrowRight");
    assert.equal(expanded, true);
    app.key(folder, "ArrowLeft");
    assert.equal(expanded, false);
    assert.equal(clicks, 2);
    await app.close();
});

test("folder arrows focus children and parents without escaping an empty branch", async () => {
    const parent = row("folder-label");
    const child = row();
    const sibling = row();
    const toggle = { disabled: false, getAttribute: () => "true" };
    const folder = { querySelector: (selector) => selector === ".folder-label" ? parent : child };
    parent.parentElement = { querySelector: () => toggle };
    parent.closest = (selector) => selector.includes(".entry-row,") ? parent
        : selector === ".tree-folder" ? folder : null;
    child.closest = (selector) => selector.includes(".entry-row,") ? child
        : selector === "li" ? { parentElement: { closest: () => folder } } : folder;
    const app = fixture([parent, child, sibling]);
    app.key(parent, "ArrowRight");
    assert.ok(child.focused);
    app.key(child, "ArrowLeft");
    assert.ok(parent.focused);
    folder.querySelector = () => null;
    app.key(parent, "ArrowRight");
    assert.equal(sibling.focused, false);
    await app.close();
});
