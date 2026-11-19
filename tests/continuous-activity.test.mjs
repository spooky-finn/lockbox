import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const source = await readFile(new URL("../assets/continuous-activity.js", import.meta.url), "utf8");
const AsyncFunction = Object.getPrototypeOf(async function () {}).constructor;

test("trusted wheel and pointer bursts share one asynchronous heartbeat per second", async () => {
    const listeners = new Map();
    const app = {
        addEventListener(name, handler, options) {
            assert.equal(options.passive, true);
            listeners.set(name, handler);
        },
        removeEventListener(name, handler) {
            assert.equal(listeners.get(name), handler);
            listeners.delete(name);
        },
    };
    const sent = [];
    let stop;
    let now = 0;
    const done = new AsyncFunction("document", "performance", "dioxus", source)(
        { querySelector: () => app }, { now: () => now },
        {
            send(payload) { sent.push([now, payload]); },
            recv: () => new Promise(resolve => { stop = resolve; }),
        },
    );
    listeners.get("pointermove")({ isTrusted: false });
    assert.deepEqual(sent, []);
    for (; now < 3000; now += 5) {
        listeners.get("pointermove")({ isTrusted: true });
        listeners.get("wheel")({ isTrusted: true });
    }
    assert.deepEqual(sent, [[0, null], [1000, null], [2000, null]]);
    stop();
    await done;
    assert.equal(listeners.size, 0);
});
