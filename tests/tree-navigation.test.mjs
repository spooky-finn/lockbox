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
    let listenerTarget;
    let capture;
    let finish;
    const messages = [];
    const body = { ...background(), outsideWorkspace: true };
    const documentElement = { ...background(), outsideWorkspace: true };
    const workspace = {
        contains: target => !target.outsideWorkspace,
        querySelector: () => options.dialog ? {} : null,
        addEventListener: (_, callback) => { listener = callback; listenerTarget = "workspace"; },
        removeEventListener: (_, callback) => { assert.equal(callback, listener); listener = null; },
    };
    const tree = {
        isConnected: true,
        dataset: { navigateRecords: String(options.recordsPage !== false) },
        closest: () => workspace,
        querySelectorAll: selector => rows.filter(item => !item.disabled
            && (item.dataset.kind === "entry" || selector.includes(".folder-label"))),
    };
    const done = new AsyncFunction("document", "dioxus", source)(
        {
            body, documentElement,
            querySelector: () => tree,
            addEventListener: (_, callback, useCapture) => {
                listener = callback;
                listenerTarget = "document";
                capture = useCapture;
            },
            removeEventListener: (_, callback, useCapture) => {
                assert.equal(callback, listener);
                assert.equal(useCapture, capture);
                listener = null;
            },
        },
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
        messages, body, documentElement, tree,
        key(target, key, modifiers = {}) {
            const event = { target, key, ...modifiers,
                preventDefault() { this.prevented = true; },
                stopPropagation() { this.stopped = true; },
            };
            // Real events outside workspace never bubble through its listener.
            if (listenerTarget === "document" && capture) listener(event);
            if (!event.stopped) {
                event.reachedDelegatedListener = true;
                if (!capture && (listenerTarget === "document" || !target.outsideWorkspace)) listener(event);
            }
            return event;
        },
        async close() {
            if (!options.searching) {
                assert.ok(rows.every(item => !item.scrolled), "tree keyboard selection must not scroll");
            }
            finish(); await done; assert.equal(listener, null);
        },
    };
}

test("search arrows select matching records in tree order and keep focus in search", async () => {
    const rows = [row(), row("folder-label"), row(), row()];
    const [first, folder, disabled, last] = rows;
    folder.selected = true;
    disabled.disabled = true;
    const search = { ...control(), id: "search", value: "match" };
    const app = fixture(rows, { searching: true, recordsPage: false });
    assert.ok(app.key(search, "ArrowDown").prevented);
    assert.ok(first.selected && first.scrolled);
    assert.ok(app.key(search, "ArrowDown").prevented);
    assert.ok(last.selected && last.scrolled);
    assert.ok(app.key(search, "ArrowUp").prevented);
    assert.ok(first.selected);
    assert.equal(app.key(search, "ArrowLeft").prevented, undefined);
    assert.equal(app.key(search, "Home").prevented, undefined);
    assert.equal(app.key(search, "ArrowDown", { isComposing: true }).prevented, undefined);
    assert.equal(app.key({ ...search, value: " " }, "ArrowDown").prevented, undefined);
    assert.ok(rows.every(item => !item.focused));
    await app.close();
});

test("search arrows leave native behavior in place when no records match", async () => {
    const search = { ...control(), id: "search", value: "match" };
    const app = fixture([row("folder-label")]);
    assert.equal(app.key(search, "ArrowDown").prevented, undefined);
    assert.equal(app.messages.length, 0);
    await app.close();
});

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
        assert.equal(event.reachedDelegatedListener, undefined);
    }
    await app.close();
});

test("preempted events and shortcuts still reach their existing handlers", async () => {
    const rows = [row(), row()];
    const app = fixture(rows);
    assert.equal(app.key(rows[0], "ArrowDown", { defaultPrevented: true }).reachedDelegatedListener, true);
    assert.equal(app.key(rows[0], "Escape").reachedDelegatedListener, true);
    assert.equal(app.key(rows[0], "s", { ctrlKey: true }).reachedDelegatedListener, true);
    assert.equal(app.messages.length, 0);
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

test("body and document focus navigate selected records without intercepting other app controls", async () => {
    const rows = [row(), row(), row()];
    rows[0].selected = true;
    const app = fixture(rows);
    assert.ok(app.key(app.body, "ArrowDown").prevented);
    assert.ok(rows[1].selected);
    assert.ok(app.key(app.documentElement, "ArrowDown").prevented);
    assert.ok(rows[2].selected);
    assert.ok(app.key(app.body, "ArrowUp").prevented);
    assert.ok(rows[1].selected);
    const before = app.messages.length;
    const toolbarOrTab = { ...background(), outsideWorkspace: true };
    assert.equal(app.key(toolbarOrTab, "ArrowDown").prevented, undefined);
    assert.equal(app.messages.length, before);
    await app.close();
});

test("detached trees stop navigating before evaluator cleanup", async () => {
    const rows = [row(), row()];
    rows[0].selected = true;
    const app = fixture(rows);
    app.tree.isConnected = false;
    assert.equal(app.key(app.body, "ArrowDown").prevented, undefined);
    assert.equal(app.messages.length, 0);
    await app.close();
});
