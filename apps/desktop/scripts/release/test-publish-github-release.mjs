#!/usr/bin/env node
/* global console, process */

import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";

const root = process.cwd();
const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "codux-release-test-"));
const artifactsDir = path.join(tempDir, "artifacts");
const notesPath = path.join(tempDir, "notes.md");

fs.mkdirSync(path.join(artifactsDir, "macos"), { recursive: true });
fs.mkdirSync(path.join(artifactsDir, "windows"), { recursive: true });
fs.writeFileSync(notesPath, "Codux release notes", "utf8");
writeAsset("codux-1.5.0-macos-aarch64-updater.app.tar.gz", "mac-arm");
writeAsset("codux-1.5.0-macos-aarch64-updater.app.tar.gz.sig", "mac-arm-signature");
writeAsset("codux-macos-aarch64-updater.app.tar.gz", "mac-arm");
writeAsset("codux-1.5.0-macos-aarch64-updater.app.tar.gz.sha256", "mac-arm-sha");
writeAsset("codux-1.5.0-macos-aarch64.dmg", "dmg-arm");
writeAsset("codux-macos-aarch64.dmg", "dmg-arm");
writeAsset("codux-1.5.0-macos-aarch64.dmg.sha256", "dmg-arm-sha");
writeAsset("codux-1.5.0-macos-x86_64-updater.app.tar.gz", "mac-intel");
writeAsset("codux-1.5.0-macos-x86_64-updater.app.tar.gz.sig", "mac-intel-signature");
writeAsset("codux-macos-x86_64-updater.app.tar.gz", "mac-intel");
writeAsset("codux-1.5.0-macos-x86_64-updater.app.tar.gz.sha256", "mac-intel-sha");
writeAsset("codux-1.5.0-macos-x86_64.dmg", "dmg-intel");
writeAsset("codux-macos-x86_64.dmg", "dmg-intel");
writeAsset("codux-1.5.0-macos-x86_64.dmg.sha256", "dmg-intel-sha");
writeAsset("codux-1.5.0-windows-x86_64-setup.exe", "win");
writeAsset("codux-1.5.0-windows-x86_64-setup.exe.sig", "win-signature");
writeAsset("codux-windows-x86_64-setup.exe", "win");
writeAsset("codux-1.5.0-windows-x86_64-setup.exe.sha256", "win-sha");
writeAsset("codux-1.5.0-windows-x86_64.zip", "win-zip");
writeAsset("Codux/Codux.exe", "raw-exe");

const result = spawnSync(
  "node",
  ["apps/desktop/scripts/release/publish-github-release.mjs", "--dry-run"],
  {
    cwd: root,
    stdio: "pipe",
    encoding: "utf8",
    env: {
      ...process.env,
      RELEASE_VERSION: "1.5.0",
      RELEASE_CHANNEL: "stable",
      RELEASE_TAG: "v1.5.0",
      RELEASE_ARTIFACTS_DIR: artifactsDir,
      RELEASE_NOTES_PATH: notesPath,
    },
  },
);

if (result.status !== 0) {
  process.stdout.write(result.stdout || "");
  process.stderr.write(result.stderr || "");
  process.exit(result.status ?? 1);
}
assert(
  result.stdout.includes("Prepared 5 public assets and update metadata"),
  `unexpected dry-run output: ${result.stdout}`,
);
assert(releaseScriptUsesStableBetaReleaseFlag(), "beta releases must not be marked as GitHub prereleases");

const manifest = JSON.parse(fs.readFileSync(path.join(artifactsDir, "latest.json"), "utf8"));
assertEqual(manifest.version, "1.5.0");
assertEqual(manifest.notes, "Codux release notes");
assert(!("automaticInstallSupported" in manifest), "manifest must stay Tauri-compatible");
assert(!("downloadUrl" in manifest), "manifest must not contain GPUI-only downloadUrl");
assert(!("checksum" in manifest), "manifest must not contain GPUI-only checksum");

for (const key of ["darwin-aarch64", "darwin-aarch64-app"]) {
  assertEqual(manifest.platforms[key].signature, "mac-arm-signature");
  assert(manifest.platforms[key].url.endsWith("/codux-macos-aarch64-updater.app.tar.gz"), `${key} should use stable aarch64 updater name`);
  assert(manifest.platforms[key].url.endsWith(".app.tar.gz"), `${key} should use app.tar.gz`);
}

for (const key of ["darwin-x86_64", "darwin-x86_64-app"]) {
  assertEqual(manifest.platforms[key].signature, "mac-intel-signature");
  assert(manifest.platforms[key].url.endsWith("/codux-macos-x86_64-updater.app.tar.gz"), `${key} should use stable x86_64 updater name`);
  assert(manifest.platforms[key].url.endsWith(".app.tar.gz"), `${key} should use app.tar.gz`);
}

for (const key of ["windows-x86_64", "windows-x86_64-nsis"]) {
  assertEqual(manifest.platforms[key].signature, "win-signature");
  assert(manifest.platforms[key].url.endsWith("/codux-windows-x86_64-setup.exe"), `${key} should use stable NSIS name`);
}

for (const file of fs.readdirSync(artifactsDir).filter((name) => name.endsWith(".sig"))) {
  fs.unlinkSync(path.join(artifactsDir, file));
}
writeAsset("codux-linux-x86_64", "agent");
const unsigned = spawnSync("node", ["apps/desktop/scripts/release/publish-github-release.mjs", "--dry-run"], {
  cwd: root,
  encoding: "utf8",
  env: {
    ...process.env,
    GITHUB_REPOSITORY: "LChJo/codux",
    RELEASE_VERSION: "1.5.0",
    RELEASE_CHANNEL: "beta",
    RELEASE_ARTIFACTS_DIR: artifactsDir,
    RELEASE_NOTES_PATH: notesPath,
  },
});
assertEqual(unsigned.status, 0);
assert(unsigned.stdout.includes("Prepared 6 public assets"), unsigned.stdout);
const manual = JSON.parse(fs.readFileSync(path.join(artifactsDir, "latest.json"), "utf8"));
assertEqual(manual.platforms["windows-x86_64"].url, "https://github.com/LChJo/codux/releases/download/v1.5.0/codux-windows-x86_64-setup.exe");
assertEqual(manual.platforms["darwin-aarch64"].url, "https://github.com/LChJo/codux/releases/download/v1.5.0/codux-macos-aarch64.dmg");
assert(!("signature" in manual.platforms["windows-x86_64"]), "unsigned updates must be manual");

fs.rmSync(tempDir, { recursive: true, force: true });
console.log("release manifest test passed");

function releaseScriptUsesStableBetaReleaseFlag() {
  const script = fs.readFileSync(path.join(root, "apps/desktop/scripts/release/publish-github-release.mjs"), "utf8");
  return (
    script.includes('if (channel === "beta") return "--latest=false";') &&
    !script.includes('channel === "beta" ? "--prerelease"')
  );
}

function writeAsset(relativePath, content) {
  const assetPath = path.join(artifactsDir, relativePath);
  fs.mkdirSync(path.dirname(assetPath), { recursive: true });
  fs.writeFileSync(assetPath, content, "utf8");
}

function assert(condition, message) {
  if (!condition) {
    throw new Error(message);
  }
}

function assertEqual(actual, expected) {
  if (actual !== expected) {
    throw new Error(`expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`);
  }
}
