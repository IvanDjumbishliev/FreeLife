#!/usr/bin/env node

const net = require("node:net");
const os = require("node:os");
const path = require("node:path");

const MAX_MESSAGE = 1024 * 1024;
let input = Buffer.alloc(0);
let pending = Promise.resolve();

process.stdin.on("data", (chunk) => {
  input = Buffer.concat([input, chunk]);
  while (input.length >= 4) {
    const size = input.readUInt32LE(0);
    if (size > MAX_MESSAGE) {
      process.stderr.write("FreeLife native message exceeds the size limit\n");
      process.exit(1);
    }
    if (input.length < size + 4) return;
    const message = input.subarray(4, size + 4);
    input = input.subarray(size + 4);
    pending = pending.then(() => handleMessage(message));
  }
});

process.stdin.on("end", () => {
  pending.catch((error) => process.stderr.write(`${error.message}\n`));
});

async function handleMessage(buffer) {
  let response;
  try {
    const request = JSON.parse(buffer.toString("utf8"));
    response = await callDesktop(request);
  } catch (error) {
    response = { ok: false, error: error.message };
  }
  writeNativeMessage(response);
}

function callDesktop(request) {
  const socketPath = process.env.FREELIFE_SOCKET_PATH ?? defaultSocketPath();
  return new Promise((resolve, reject) => {
    const socket = net.createConnection(socketPath);
    let response = "";
    socket.setTimeout(15000);
    socket.once("connect", () => socket.write(`${JSON.stringify(request)}\n`));
    socket.on("data", (chunk) => {
      response += chunk.toString("utf8");
      if (response.length > MAX_MESSAGE) {
        socket.destroy(new Error("Desktop response exceeds the size limit"));
        return;
      }
      const newline = response.indexOf("\n");
      if (newline >= 0) {
        socket.end();
        try {
          resolve(JSON.parse(response.slice(0, newline)));
        } catch {
          reject(new Error("Desktop returned an invalid response"));
        }
      }
    });
    socket.once("timeout", () => socket.destroy(new Error("Desktop IPC timed out")));
    socket.once("error", reject);
  });
}

function defaultSocketPath() {
  if (process.platform === "win32") return "\\\\.\\pipe\\freelife-handoff";
  const appData = process.platform === "darwin"
    ? path.join(os.homedir(), "Library", "Application Support")
    : process.env.XDG_DATA_HOME ?? path.join(os.homedir(), ".local", "share");
  return path.join(appData, "dev.freelife.desktop", "handoff.sock");
}

function writeNativeMessage(value) {
  const body = Buffer.from(JSON.stringify(value), "utf8");
  const header = Buffer.alloc(4);
  header.writeUInt32LE(body.length, 0);
  process.stdout.write(Buffer.concat([header, body]));
}
