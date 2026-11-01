import test from "node:test";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

test("real Firefox: installs MV3 package and loads popup/background modules", { skip: !process.env.FIREFOX_BIN }, async () => {
  const directory = await mkdtemp(join(tmpdir(), "lockbox-firefox-test-"));
  await writeFile(join(directory, "user.js"), [
    'user_pref("browser.shell.checkDefaultBrowser", false);',
    'user_pref("browser.startup.homepage_override.mstone", "ignore");',
    'user_pref("datareporting.policy.dataSubmissionEnabled", false);',
    'user_pref("toolkit.telemetry.enabled", false);',
  ].join("\n"));
  const child = spawn(process.env.FIREFOX_BIN, ["--headless", "--no-remote", "--profile", directory, "--remote-debugging-port=0"],
    { env: { ...process.env, MOZ_REMOTE_ALLOW_SYSTEM_ACCESS: "1" }, stdio: ["ignore", "pipe", "pipe"] });
  let socket;
  const tasks = new Map();
  try {
    const endpoint = await new Promise((resolve, reject) => {
      let output = "";
      const timer = setTimeout(() => reject(new Error("Firefox startup timeout: " + output.slice(-600))), 20000);
      const onData = (chunk) => {
        output += chunk;
        const match = output.match(/WebDriver BiDi listening on (ws:\/\/\S+)/);
        if (match) { clearTimeout(timer); resolve(match[1].replace(/\/$/, "") + "/session"); }
      };
      child.stderr.on("data", onData);
      child.stdout.on("data", onData);
      child.once("error", (error) => { clearTimeout(timer); reject(error); });
      child.once("exit", (code) => { clearTimeout(timer); reject(new Error(`Firefox exited (${code}): ${output.slice(-600)}`)); });
    });
    socket = new WebSocket(endpoint);
    await new Promise((resolve, reject) => { socket.onopen = resolve; socket.onerror = reject; });
    let sequence = 0;
    function send(method, params = {}) {
      const id = ++sequence;
      return new Promise((resolve, reject) => {
        const timer = setTimeout(() => { tasks.delete(id); reject(new Error(`BiDi timeout: ${method}`)); }, 15000);
        tasks.set(id, { resolve, reject, timer });
        socket.send(JSON.stringify({ id, method, params }));
      });
    }
    socket.onmessage = ({ data }) => {
      const message = JSON.parse(data);
      const task = tasks.get(message.id);
      if (!task) return;
      tasks.delete(message.id);
      clearTimeout(task.timer);
      message.type === "error" ? task.reject(new Error(`${message.error}: ${message.message}`)) : task.resolve(message.result);
    };
    const session = await send("session.new", { capabilities: { alwaysMatch: {} } });
    assert.equal(session.capabilities.browserName, "firefox");
    const installed = await send("webExtension.install", {
      extensionData: { type: "path", path: fileURLToPath(new URL("../dist/firefox", import.meta.url)) },
    });
    assert.equal(installed.extension, "lockbox@lockbox.local");
    const { extensions } = await send("webExtension.moz:listExtensions");
    const extension = extensions.find((item) => item.id === installed.extension);
    assert.equal(extension.isActive, true);
    assert.equal(extension.manifestVersion, 3);
    assert.deepEqual(extension.policy.backgroundScripts.map((url) => new URL(url).pathname), ["/background.js"]);
    const { context } = await send("browsingContext.create", { type: "tab" });
    await send("browsingContext.navigate", { context, url: extension.policy.extensionURL + "popup.html", wait: "complete" });
    const evaluate = async (expression) => {
      const result = await send("script.evaluate", { expression, target: { context }, awaitPromise: true });
      if (result.type === "exception") throw new Error("Popup script exception: " + JSON.stringify(result.exceptionDetails));
      return result.result;
    };
    assert.equal((await evaluate("document.title")).value, "Lockbox");
    const background = await evaluate("browser.runtime.getBackgroundPage().then(page=>page.document.readyState==='complete')");
    assert.equal(background.value, true, "background document loaded");
    const imported = await evaluate("import(browser.runtime.getURL('background.js')).then(()=>true)");
    assert.equal(imported.value, true, "background module and dependencies execute without errors");
    // An extension page opened as a tab cannot impersonate the actual popup.
    assert.equal((await evaluate("browser.runtime.sendMessage({action:'status'}).then(reply=>reply===undefined)")).value, true);
    assert.equal((await evaluate("document.documentElement.scrollWidth <= innerWidth")).value, true);
    const screenshot = await send("browsingContext.captureScreenshot", { context });
    if (process.env.LOCKBOX_FIREFOX_SCREENSHOT) await writeFile(process.env.LOCKBOX_FIREFOX_SCREENSHOT, Buffer.from(screenshot.data, "base64"));
    await send("webExtension.uninstall", { extension: installed.extension });
    await send("browser.close");
  } finally {
    socket?.close();
    child.kill();
    for (const task of tasks.values()) clearTimeout(task.timer);
    await new Promise((resolve) => child.exitCode !== null || child.signalCode !== null ? resolve() : child.once("exit", resolve));
    await rm(directory, { recursive: true, force: true });
  }
});
