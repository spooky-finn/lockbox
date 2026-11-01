import test from "node:test";
import assert from "node:assert/strict";
import { spawn, execFileSync } from "node:child_process";
import { mkdtemp, rm, readFile } from "node:fs/promises";
import { createServer } from "node:https";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { inspectPage, fillPage } from "../src/page.js";

// Optional real-browser boundary tests; no npm packages or real websites needed.
test("real Chromium: document binding and safe form filling", { skip: !process.env.CHROME_BIN }, async (t) => {
  const directory = await mkdtemp(join(tmpdir(), "lockbox-browser-test-"));
  execFileSync("openssl", ["req", "-x509", "-newkey", "rsa:2048", "-nodes", "-keyout", join(directory, "key.pem"),
    "-out", join(directory, "cert.pem"), "-subj", "/CN=localhost", "-days", "1"], { stdio: "ignore" });
  const server = createServer({ key: await readFile(join(directory, "key.pem")), cert: await readFile(join(directory, "cert.pem")) },
    (_request, response) => { response.writeHead(200, { "Content-Type": "text/html" }); response.end("<!doctype html><html><body></body></html>"); });
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  const testOrigin = `https://127.0.0.1:${server.address().port}`;
  const extensionPath = fileURLToPath(new URL("../dist/chrome", import.meta.url));
  const child = spawn(process.env.CHROME_BIN, ["--headless=new", "--no-sandbox", "--disable-gpu",
    "--ignore-certificate-errors", "--no-proxy-server",
    `--disable-extensions-except=${extensionPath}`, `--load-extension=${extensionPath}`,
    "--disable-background-networking", "--disable-component-update", "--disable-sync", "--no-first-run",
    "--remote-debugging-port=0", `--user-data-dir=${directory}`, "about:blank"], { stdio: ["ignore", "ignore", "pipe"] });
  let socket;
  const tasks = new Map();
  try {
    const endpoint = await new Promise((resolve, reject) => {
      let output = "";
      const timer = setTimeout(() => reject(new Error("Chromium startup timeout")), 15000);
      child.stderr.on("data", (chunk) => {
        output += chunk;
        const match = output.match(/DevTools listening on (ws:\/\/\S+)/);
        if (match) { clearTimeout(timer); resolve(match[1]); }
      });
      child.once("error", (error) => { clearTimeout(timer); reject(error); });
      child.once("exit", (code) => { clearTimeout(timer); reject(new Error(`Chromium exited (${code}): ${output.slice(-600)}`)); });
    });
    socket = new WebSocket(endpoint);
    await new Promise((resolve, reject) => { socket.onopen = resolve; socket.onerror = reject; });
    let sequence = 0;
    let sessionId;
    function send(method, params = {}, target = sessionId) {
      const id = ++sequence;
      return new Promise((resolve, reject) => {
        const timer = setTimeout(() => { tasks.delete(id); reject(new Error(`CDP timeout: ${method}`)); }, 10000);
        tasks.set(id, { resolve, reject, timer });
        socket.send(JSON.stringify({ id, method, params, ...(target ? { sessionId: target } : {}) }));
      });
    }
    socket.onmessage = ({ data }) => {
      const message = JSON.parse(data);
      if (message.id) {
        const task = tasks.get(message.id);
        if (!task) return;
        tasks.delete(message.id);
        clearTimeout(task.timer);
        message.error ? task.reject(new Error(message.error.message)) : task.resolve(message.result);
      }
    };
    const { targetId } = await send("Target.createTarget", { url: "about:blank" }, null);
    ({ sessionId } = await send("Target.attachToTarget", { targetId, flatten: true }, null));
    await send("Page.enable");
    await t.test("Chrome package loads its module service worker", async () => {
      let worker;
      for (let attempt = 0; attempt < 30 && !worker; attempt++) {
        const { targetInfos } = await send("Target.getTargets", {}, null);
        worker = targetInfos.find((item) => item.type === "service_worker" && item.url.endsWith("/background.js"));
        if (!worker) await new Promise((resolve) => setTimeout(resolve, 100));
      }
      assert.ok(worker, "extension service worker must start without manifest or CSP errors");
      const { sessionId: workerSession } = await send("Target.attachToTarget", { targetId: worker.targetId, flatten: true }, null);
      const { result } = await send("Runtime.evaluate", { expression: "typeof chrome.runtime.onMessage.addListener === 'function' && typeof chrome.idle.setDetectionInterval === 'function'", returnByValue: true }, workerSession);
      assert.equal(result.value, true);
      await send("Target.detachFromTarget", { sessionId: workerSession }, null);
    });
    await send("Page.navigate", { url: `${testOrigin}/login` });
    const { frameTree } = await send("Page.getFrameTree");
    let contextId;
    async function world() {
      ({ executionContextId: contextId } = await send("Page.createIsolatedWorld", { frameId: frameTree.frame.id, worldName: "lockbox-test" }));
    }
    await world();
    async function evaluate(expression) {
      const value = await send("Runtime.evaluate", { expression, contextId, returnByValue: true, awaitPromise: true });
      if (value.exceptionDetails) throw new Error(value.exceptionDetails.text + ": " + value.exceptionDetails.exception?.description);
      return value.result.value;
    }
    async function html(content) {
      await send("Page.setDocumentContent", { frameId: frameTree.frame.id,
        html: `<!doctype html><html><head><style>input{width:200px;height:30px;margin:5px;display:block}</style></head><body>${content}</body></html>` });
    }
    const inspect = (mode = "login", origin = testOrigin) =>
      evaluate(`(${inspectPage.toString()})(...${JSON.stringify([origin, mode])})`);
    const fill = (token, mode = "login") => evaluate(`(${fillPage.toString()})(...${JSON.stringify([
      testOrigin, token, "fictional-user", "fictional-password", mode])})`);
    const values = () => evaluate("[...document.querySelectorAll('input')].map(input=>input.value)");

    await t.test("fills visible same-origin POST login without submitting", async () => {
      await html('<form method="post"><input name="username"><input type="password"></form>');
      await evaluate("globalThis.submitted=false;document.querySelector('form').onsubmit=()=>{globalThis.submitted=true;return false}");
      const result = await inspect();
      assert.ok(result.token);
      assert.equal((await fill(result.token)).filled, true);
      assert.deepEqual(await values(), ["fictional-user", "fictional-password"]);
      assert.equal(await evaluate("globalThis.submitted"), false);
      assert.ok((await fill(result.token)).error, "token cannot be reused");
    });
    for (const [name, content] of [
      ["hidden input", '<form method="post"><input type="password" style="display:none"></form>'],
      ["invisible ancestor", '<form method="post" style="opacity:0"><input type="password"></form>'],
      ["disabled field", '<form method="post"><input type="password" disabled></form>'],
      ["read-only field", '<form method="post"><input type="password" readonly></form>'],
      ["offscreen field", '<form method="post"><input type="password" style="position:absolute;left:-500px"></form>'],
      ["GET form", '<form method="get"><input type="password"></form>'],
      ["external action", '<form method="post" action="https://attacker.test"><input type="password"></form>'],
      ["HTTP action", '<form method="post" action="http://accounts.example"><input type="password"></form>'],
      ["external submit override", '<form method="post"><input type="password"><button formaction="https://attacker.test">Submit</button></form>'],
      ["embedded form credentials", '<form method="post" action="https://attacker:pass@accounts.example"><input type="password"></form>'],
      ["multiple forms", '<form method="post"><input type="password"></form><form method="post"><input type="password"></form>'],
      ["new-password field during login", '<form method="post"><input type="password" autocomplete="new-password"></form>'],
    ]) {
      await t.test(`refuses ${name}`, async () => { await html(content); assert.ok((await inspect()).error); });
    }
    await t.test("origin rechecked inside the page", async () => {
      await html('<form method="post"><input type="password"></form>');
      assert.ok((await inspect("login", "https://accounts.example:444")).error);
    });
    await t.test("password changes fill only new and confirmation fields", async () => {
      await html('<form method="post"><input type="password" autocomplete="current-password" value="old"><input type="password" autocomplete="new-password"><input type="password" name="confirm"></form>');
      const result = await inspect("new");
      assert.equal((await fill(result.token, "new")).filled, true);
      assert.deepEqual(await values(), ["old", "fictional-password", "fictional-password"]);
    });
    await t.test("focused password selects the intended form", async () => {
      await html('<form method="post"><input type="password"></form><form method="post"><input id="chosen" type="password"></form>');
      await evaluate("document.getElementById('chosen').focus()");
      const result = await inspect();
      assert.equal((await fill(result.token)).filled, true);
      assert.deepEqual(await values(), ["", "fictional-password"]);
    });
    for (const [name, mutation] of [
      ["changed action", "document.querySelector('form').action='https://attacker.test'"],
      ["detached input", "document.querySelector('input').remove()"],
      ["hidden input", "document.querySelector('input').style.opacity=0"],
      ["shortened field", "document.querySelector('input').maxLength=3"],
      ["moved field", "document.querySelector('input').setAttribute('form','other')"],
    ]) {
      await t.test(`refuses ${name} after inspection`, async () => {
        await html('<form method="post"><input type="password"></form><form id="other" method="post"></form>');
        const result = await inspect();
        await evaluate(mutation);
        assert.ok((await fill(result.token)).error);
        assert.ok(!(await values()).includes("fictional-password"));
      });
    }
    await t.test("same-origin navigation invalidates document token", async () => {
      await html('<form method="post"><input type="password"></form>');
      const result = await inspect();
      await send("Page.navigate", { url: `${testOrigin}/other` });
      await world();
      await html('<form method="post"><input type="password"></form>');
      assert.ok((await fill(result.token)).error);
    });
    await t.test("page handlers cannot redirect a later password write", async () => {
      await html('<form method="post"><input name="username"><input type="password"></form>');
      await evaluate("document.querySelector('input').oninput=()=>{document.querySelector('form').action='https://attacker.test';document.querySelector('input[type=password]').remove()}");
      const result = await inspect();
      assert.equal((await fill(result.token)).filled, true);
      assert.deepEqual(await values(), ["fictional-user"]);
    });
    await send("Browser.close", {}, null);
  } finally {
    socket?.close();
    child.kill();
    server.closeAllConnections();
    await new Promise((resolve) => server.close(resolve));
    for (const task of tasks.values()) clearTimeout(task.timer);
    await new Promise((resolve) => child.exitCode !== null || child.signalCode !== null ? resolve() : child.once("exit", resolve));
    await rm(directory, { recursive: true, force: true });
  }
});
