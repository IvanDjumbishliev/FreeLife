#!/usr/bin/env node

const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { execFileSync } = require("node:child_process");

const [browser, extensionId] = process.argv.slice(2);
if (!["chrome", "firefox"].includes(browser)) {
  throw new Error("Usage: node install-host.js <chrome|firefox> [Chrome extension ID]");
}
if (browser === "chrome" && !/^[a-p]{32}$/.test(extensionId ?? "")) {
  throw new Error("Chrome requires its 32-character Web Store extension ID.");
}

const base = __dirname;
const hostName = "dev.freelife.handoff";
const executable = process.platform === "win32"
  ? path.join(base, "host-launcher.cmd")
  : path.join(base, "host-launcher.sh");
const nativeManifest = {
  name: hostName,
  description: "FreeLife browser handoff bridge",
  path: executable,
  type: "stdio",
};
if (browser === "chrome") {
  nativeManifest.allowed_origins = [`chrome-extension://${extensionId}/`];
} else {
  nativeManifest.allowed_extensions = ["handoff@freelife.dev"];
}

if (process.platform === "win32") {
  const installDir = path.join(process.env.LOCALAPPDATA ?? os.homedir(), "FreeLife");
  fs.mkdirSync(installDir, { recursive: true });
  const manifestPath = path.join(installDir, `${hostName}.json`);
  fs.writeFileSync(manifestPath, JSON.stringify(nativeManifest, null, 2));
  const browserKey = browser === "chrome"
    ? "Software\\Google\\Chrome\\NativeMessagingHosts"
    : "Software\\Mozilla\\NativeMessagingHosts";
  execFileSync("reg", [
    "add", `HKCU\\${browserKey}\\${hostName}`, "/ve", "/t", "REG_SZ", "/d", manifestPath, "/f",
  ], { stdio: "inherit" });
} else {
  const roots = browser === "chrome"
    ? process.platform === "darwin"
      ? [path.join(os.homedir(), "Library", "Application Support", "Google", "Chrome", "NativeMessagingHosts")]
      : [
          path.join(process.env.XDG_CONFIG_HOME ?? path.join(os.homedir(), ".config"), "google-chrome", "NativeMessagingHosts"),
          path.join(process.env.XDG_CONFIG_HOME ?? path.join(os.homedir(), ".config"), "chromium", "NativeMessagingHosts"),
        ]
    : process.platform === "darwin"
      ? [path.join(os.homedir(), "Library", "Application Support", "Mozilla", "NativeMessagingHosts")]
      : [path.join(os.homedir(), ".mozilla", "native-messaging-hosts")];
  for (const root of roots) {
    fs.mkdirSync(root, { recursive: true });
    fs.writeFileSync(path.join(root, `${hostName}.json`), JSON.stringify(nativeManifest, null, 2));
  }
}

process.stdout.write(`Installed FreeLife native host for ${browser}.\n`);
