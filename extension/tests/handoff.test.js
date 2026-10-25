const assert = require("node:assert/strict");
const { spawn } = require("node:child_process");
const fs = require("node:fs");
const net = require("node:net");
const os = require("node:os");
const path = require("node:path");
const { after, before, test } = require("node:test");
const vm = require("node:vm");

const root = path.resolve(__dirname, "..");
const socketPath = process.platform === "win32"
  ? `\\\\.\\pipe\\freelife-handoff-test-${process.pid}`
  : path.join(os.tmpdir(), `freelife-handoff-${process.pid}.sock`);
const received = [];
let server;
let messageListener;

before(async () => {
  server = net.createServer((socket) => {
    let input = "";
    socket.on("data", (chunk) => {
      input += chunk.toString("utf8");
      const newline = input.indexOf("\n");
      if (newline < 0) return;
      const request = JSON.parse(input.slice(0, newline));
      if (request.op === "getDevices") {
        socket.end(`${JSON.stringify({
          ok: true,
          devices: [{ id: "paired-device", name: "Phone" }],
        })}\n`);
      } else if (request.op === "handoff") {
        received.push({
          deviceId: request.deviceId,
          message: { type: "HandoffUrl", url: request.url },
        });
        socket.end('{"ok":true}\n');
      } else {
        socket.end('{"ok":false,"error":"unknown request"}\n');
      }
    });
  });
  await new Promise((resolve) => server.listen(socketPath, resolve));

  const runtime = {
    onMessage: {
      addListener(listener) {
        messageListener = listener;
      },
    },
    connectNative() {
      return nativePort();
    },
  };
  vm.runInNewContext(fs.readFileSync(path.join(root, "background.js"), "utf8"), {
    browser: { runtime },
    setTimeout,
    clearTimeout,
    Promise,
    Error,
  });
});

after(async () => {
  await new Promise((resolve) => server.close(resolve));
  if (process.platform !== "win32") fs.rmSync(socketPath, { force: true });
});

test("extension URL reaches a paired device through the native host", async () => {
  const available = await messageListener({ type: "listDevices" });
  assert.deepEqual(available.devices, [{ id: "paired-device", name: "Phone" }]);

  const url = "https://example.com/article";
  const result = await messageListener({
    type: "handoff",
    deviceId: available.devices[0].id,
    url,
  });
  assert.equal(result.ok, true);
  assert.deepEqual(received, [{
    deviceId: "paired-device",
    message: { type: "HandoffUrl", url },
  }]);
});

function nativePort() {
  const child = spawn(process.execPath, [path.join(root, "host.js")], {
    env: { ...process.env, FREELIFE_SOCKET_PATH: socketPath },
    stdio: ["pipe", "pipe", "pipe"],
  });
  const messageListeners = [];
  const disconnectListeners = [];
  let output = Buffer.alloc(0);
  child.stdout.on("data", (chunk) => {
    output = Buffer.concat([output, chunk]);
    while (output.length >= 4) {
      const size = output.readUInt32LE(0);
      if (output.length < size + 4) return;
      const value = JSON.parse(output.subarray(4, size + 4).toString("utf8"));
      output = output.subarray(size + 4);
      for (const listener of messageListeners) listener(value);
    }
  });
  child.on("exit", () => {
    for (const listener of disconnectListeners) listener();
  });
  return {
    onMessage: { addListener: (listener) => messageListeners.push(listener) },
    onDisconnect: { addListener: (listener) => disconnectListeners.push(listener) },
    postMessage(value) {
      const body = Buffer.from(JSON.stringify(value));
      const prefix = Buffer.alloc(4);
      prefix.writeUInt32LE(body.length);
      child.stdin.write(Buffer.concat([prefix, body]));
    },
    disconnect() {
      child.stdin.end();
    },
  };
}
