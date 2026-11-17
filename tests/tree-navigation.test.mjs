import { readFile } from "node:fs/promises";
import test from "node:test";
import assert from "node:assert/strict";

const source = await readFile(new URL("../assets/tree-navigation.js", import.meta.url), "utf8");
const AsyncFunction = Object.getPrototypeOf(async function () {}).constructor;
let nextId = 0;

function row(kind = "entry-row") {
    return {
        disabled: false, focused: false, selected: false, scrolled: false,
        dataset: { kind: kind === "folder-label" ? "directory" : "entry", id: String(++nextId) },
        classList: { contains: (name) => name === kind },
        getAttribute(name) { return name === "aria-current" ? String(this.selected) : null; },
        closest(selector) {
            if (selector.startsWith(".entry-row,")) return this;
            if (selector === ".tree-folder") return this.folder ?? null;
            if (selector === "li") return this.li ?? null;
            return null;
        },
        focus(options) { assert.deepEqual(options, { preventScroll: true }); this.focused = true; },
        scrollIntoView() { this.scrolled = true; },
    };
}

const background = () => ({ closest: () => null });
const control = () => ({ closest: selector => selector.startsWith("input,") ? {} : null });

function fixture(rows, options = {}) {
    let listener;
    let finish;
    const messages = [];
    const workspace = {
        querySelector: () => options.dialog ? {} : null,
        addEventListener: (_, callback) => { listener = callback; },
        removeEventListener: (_, callback) => { assert.equal(callback, listener); listener = null; },
    };
    const tree = {
        dataset: { navigateRecords: String(options.recordsPage !== false) },
        closest: () => workspace,
        querySelectorAll: () => rows.filter(item => !item.disabled),
    };
    const done = new AsyncFunction("document", "dioxus", source)(
        { querySelector: () => tree },
        {
            recv: () => new Promise(resolve => { finish = resolve; }),
            send(message) {
                messages.push(message);
                if (message.kind === "entry" || message.kind === "directory") {
                    rows.forEach(item => { item.selected = item.dataset.id === message.id; });
                }
                options.onSend?.(message);
            },
        },
    );
    return {
        messages,
        key(target, key, modifiers = {}) {
            const event = { target, key, ...modifiers,
                preventDefault() { this.prevented = true; },
                stopPropagation() { this.stopped = true; },
            };
            listener(event);
            return event;
        },
        async close() {
            assert.ok(rows.every(item => !item.scrolled), "keyboard selection must not scroll");
            finish(); await done; assert.equal(listener, null);
        },
    };
}

test("arrows and Home/End change selection without scrolling; boundaries keep selection", async () => {
    const rows = [row(), row(), row()];
    const app = fixture(rows);
    for (const [start, key, expected] of [
        [0, "ArrowDown", 1], [1, "ArrowUp", 0],
        [0, "ArrowUp", 0], [2, "ArrowDown", 2],
        [0, "End", 2], [2, "Home", 0],
    ]) {
        rows.forEach((item, index) => { item.focused = false; item.selected = index === start; });
        const event = app.key(rows[start], key);
        assert.ok(rows[expected].focused && rows[expected].selected);
        assert.deepEqual(app.messages.at(-1), expected === start ? { kind: "activity" }
            : { kind: "entry", id: rows[expected].dataset.id });
        assert.ok(event.prevented && event.stopped);
    }
    await app.close();
});

test("navigation starts at current selection, including from detail or scroll container", async () => {
    const rows = [row(), row(), row()];
    rows[0].selected = true;
    const app = fixture(rows);
    app.key(rows[2], "ArrowDown"); // Tab focus may differ from selected item.
    assert.ok(rows[1].selected);
    const event = app.key(background(), "ArrowDown");
    assert.ok(rows[2].selected);
    assert.ok(event.prevented);
    await app.close();
});

test("modifiers, inputs, composition, activation keys and disabled rows retain native behavior", async () => {
    const rows = [row(), row()];
    const app = fixture(rows);
    for (const [key, modifiers] of [
        ["ArrowDown", { ctrlKey: true }], ["ArrowDown", { altKey: true }],
        ["ArrowDown", { metaKey: true }], ["ArrowDown", { shiftKey: true }],
        ["ArrowDown", { isComposing: true }],
        ["Enter", {}], [" ", {}], ["a", {}],
    ]) {
        assert.equal(app.key(rows[0], key, modifiers).prevented, undefined);
    }
    assert.equal(app.key(control(), "ArrowDown").prevented, undefined);
    rows[0].disabled = true;
    assert.equal(app.key(rows[0], "ArrowDown").prevented, undefined);
    assert.equal(app.messages.length, 0);
    assert.ok(rows.every(item => !item.focused));
    await app.close();
});

test("folder arrows request expansion and collapse without toggling selection", async () => {
    const folder = row("folder-label");
    folder.selected = true;
    let expanded = false;
    const toggle = { disabled: false, getAttribute: () => String(expanded) };
    folder.parentElement = { querySelector: () => toggle };
    const app = fixture([folder], { onSend(message) { if (message.kind === "toggle") expanded = !expanded; } });
    app.key(folder, "ArrowRight");
    assert.equal(expanded, true);
    app.key(folder, "ArrowLeft");
    assert.equal(expanded, false);
    assert.deepEqual(app.messages, [
        { kind: "toggle", id: folder.dataset.id }, { kind: "toggle", id: folder.dataset.id },
    ]);
    await app.close();
});

test("folder arrows select children and parents without escaping an empty branch", async () => {
    const parent = row("folder-label");
    const child = row();
    const sibling = row();
    parent.selected = true;
    const toggle = { disabled: false, getAttribute: () => "true" };
    const folder = { querySelector: selector => selector === ".folder-label" ? parent : child };
    parent.parentElement = { querySelector: () => toggle };
    parent.folder = folder;
    child.folder = folder;
    child.li = { parentElement: { closest: () => folder } };
    const app = fixture([parent, child, sibling]);
    app.key(parent, "ArrowRight");
    assert.ok(child.selected && child.focused);
    app.key(child, "ArrowLeft");
    assert.ok(parent.selected && parent.focused);
    assert.deepEqual(app.messages.at(-1), { kind: "directory", id: parent.dataset.id });
    folder.querySelector = () => null;
    app.key(parent, "ArrowRight");
    assert.equal(sibling.selected, false);
    await app.close();
});

test("no selection starts at first or last enabled row; disabled neighbors are skipped", async () => {
    const rows = [row(), row(), row()];
    rows[1].disabled = true;
    const app = fixture(rows);
    app.key(background(), "ArrowDown");
    assert.ok(rows[0].selected);
    app.key(rows[0], "ArrowDown");
    assert.ok(rows[2].selected);
    rows.forEach(item => { item.selected = false; });
    app.key(background(), "ArrowUp");
    assert.ok(rows[2].selected);
    await app.close();
});

test("modal dialogs and non-record pages keep their own keyboard behavior", async () => {
    for (const options of [{ dialog: true }, { recordsPage: false }]) {
        const rows = [row(), row()];
        const app = fixture(rows, options);
        assert.equal(app.key(background(), "ArrowDown").prevented, undefined);
        assert.equal(app.messages.length, 0);
        await app.close();
    }
});
