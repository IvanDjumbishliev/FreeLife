const fs = require("node:fs");
const path = require("node:path");
const { execFileSync } = require("node:child_process");

const root = __dirname;
const output = path.join(root, "dist");
const targets = process.argv[2] === "all" ? ["chrome", "firefox"] : [process.argv[2]];
if (targets.some((target) => !["chrome", "firefox"].includes(target))) {
  throw new Error("Usage: node package.js <chrome|firefox|all>");
}

fs.mkdirSync(output, { recursive: true });
for (const target of targets) {
  const staging = path.join(output, target);
  fs.rmSync(staging, { recursive: true, force: true });
  fs.mkdirSync(staging, { recursive: true });
  for (const file of ["background.js", "popup.html", "popup.js", "popup.css"]) {
    fs.copyFileSync(path.join(root, file), path.join(staging, file));
  }
  const manifestSource = target === "firefox" ? "manifest.firefox.json" : "manifest.json";
  fs.copyFileSync(path.join(root, manifestSource), path.join(staging, "manifest.json"));
  const extension = target === "firefox" ? "xpi" : "zip";
  const archive = path.join(output, `freelife-handoff-${target}.${extension}`);
  fs.rmSync(archive, { force: true });
  if (process.platform === "win32") {
    execFileSync("powershell", [
      "-NoProfile", "-Command", "Compress-Archive", "-Path",
      path.join(staging, "*"), "-DestinationPath", archive,
    ], { stdio: "inherit" });
  } else {
    execFileSync("zip", ["-qr", archive, "."], { cwd: staging, stdio: "inherit" });
  }
}
process.stdout.write(`Packaged ${targets.join(" and ")} extension archive(s) in extension/dist.\n`);
