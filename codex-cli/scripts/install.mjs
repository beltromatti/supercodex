#!/usr/bin/env node
// Super Codex postinstall: download the right platform binary from the
// GitHub Release that matches this package version. Kept to stdlib only so
// the npm package stays minimal and has zero install-time dependencies.

import { createWriteStream, mkdirSync, readFileSync, chmodSync } from "node:fs";
import path from "node:path";
import https from "node:https";
import { fileURLToPath } from "node:url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

const pkg = JSON.parse(
  readFileSync(path.join(__dirname, "..", "package.json"), "utf8"),
);
const version = pkg.version;

const { platform, arch } = process;
let triple = null;
let isWindows = false;
if (platform === "darwin" && arch === "arm64") {
  triple = "aarch64-apple-darwin";
} else if (platform === "linux" && arch === "x64") {
  triple = "x86_64-unknown-linux-gnu";
} else if (platform === "win32" && arch === "x64") {
  triple = "x86_64-pc-windows-msvc";
  isWindows = true;
} else {
  console.error(
    `[supercodex] Unsupported platform ${platform} (${arch}). ` +
      "Supported: macOS arm64, Linux x64, Windows x64.",
  );
  process.exit(1);
}

const suffix = isWindows ? ".exe" : "";
const assetName = `supercodex-${version}-${triple}${suffix}`;
const url =
  `https://github.com/beltromatti/supercodex/releases/download/` +
  `super-v${version}/${assetName}`;

const outDir = path.join(__dirname, "..", "vendor", triple, "codex");
const outPath = path.join(outDir, isWindows ? "supercodex.exe" : "supercodex");
mkdirSync(outDir, { recursive: true });

function download(targetUrl, destPath, redirectsLeft = 5) {
  return new Promise((resolve, reject) => {
    https
      .get(targetUrl, { headers: { "user-agent": "supercodex-postinstall" } }, (res) => {
        if (
          res.statusCode &&
          res.statusCode >= 300 &&
          res.statusCode < 400 &&
          res.headers.location
        ) {
          if (redirectsLeft === 0) {
            reject(new Error(`too many redirects`));
            return;
          }
          res.resume();
          download(res.headers.location, destPath, redirectsLeft - 1)
            .then(resolve)
            .catch(reject);
          return;
        }
        if (res.statusCode !== 200) {
          reject(
            new Error(`unexpected status ${res.statusCode} for ${targetUrl}`),
          );
          res.resume();
          return;
        }
        const out = createWriteStream(destPath);
        res.pipe(out);
        out.on("finish", () => out.close(resolve));
        out.on("error", reject);
      })
      .on("error", reject);
  });
}

console.log(`[supercodex] Downloading ${assetName}...`);
try {
  await download(url, outPath);
  if (!isWindows) {
    chmodSync(outPath, 0o755);
  }
  console.log(`[supercodex] Installed binary at ${outPath}`);
} catch (err) {
  console.error(
    `[supercodex] Failed to download ${url}: ${err.message || err}`,
  );
  console.error(
    `[supercodex] You can download it manually from ` +
      `https://github.com/beltromatti/supercodex/releases/tag/super-v${version}`,
  );
  process.exit(1);
}
