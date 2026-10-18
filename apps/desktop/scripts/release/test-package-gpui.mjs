#!/usr/bin/env node
/* global console, process */

import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";

if (process.platform === "win32") {
  const localGuardResult = spawnSync("node", ["apps/desktop/scripts/release/package-gpui.mjs"], {
    cwd: process.cwd(),
    env: {
      ...process.env,
      CODUX_PACKAGE_GPUI_TEST_MODE: "false",
      CODUX_PACKAGE_VERSION_PREPARED: "",
      GITHUB_ACTIONS: "",
      RELEASE_STAGE_DIR: path.join(os.tmpdir(), "codux-unprepared-package-test"),
    },
    encoding: "utf8",
  });
  assert.notEqual(localGuardResult.status, 0);
  assert.match(
    localGuardResult.stderr,
    /Local packaging must increment and build the version first\. Run tools\/package-windows\.ps1/,
  );
}

process.env.CODUX_PACKAGE_GPUI_TEST_MODE = "true";
process.env.RELEASE_STAGE_DIR = "target/release-package-test";

const { __testPackageWindows, __testStageRuntimeAssets, __testWindowsNsisScript } = await import("./package-gpui.mjs");

const script = __testWindowsNsisScript(
  path.join("C:", "tmp", "Codux"),
  path.join("C:", "tmp", "codux-setup.exe"),
);

assert.match(script, /!include MUI2\.nsh/);
assert.match(script, /ManifestDPIAware true/);
const packageVersion = fs.readFileSync("apps/desktop/Cargo.toml", "utf8")
  .match(/^version = "([^"]+)"$/m)[1];
const [versionCore, prerelease = ""] = packageVersion.split("-");
const revision = prerelease.match(/(?:^|\.)(\d+)$/)?.[1] || "0";
assert.ok(script.includes(`VIProductVersion "${versionCore}.${revision}"`));
assert.ok(script.includes(`VIAddVersionKey /LANG=0 "ProductVersion" "${packageVersion}"`));
assert.match(script, /!insertmacro MUI_PAGE_DIRECTORY/);
assert.match(script, /!insertmacro MUI_LANGUAGE "English"/);
assert.match(script, /!insertmacro MUI_LANGUAGE "SimpChinese"/);
assert.match(script, /MUI_HEADERIMAGE_BITMAP/);
assert.doesNotMatch(script, /MUI_PAGE_WELCOME/);
assert.doesNotMatch(script, /Page custom/);
assert.match(script, /Function EnsureCoduxCanBeUpdated/);
assert.match(script, /Codux is still running or the executable is locked/);
assert.match(script, /CreateShortcut "\$DESKTOP\\Codux\.lnk"/);
assert.match(script, /CreateShortcut "\$SMPROGRAMS\\Codux\\Codux\.lnk"/);
assert.match(script, /Uninstall\\Codux"$/m);
assert.match(script, /"UninstallString"/);
assert.match(script, /Exec '"\$INSTDIR\\Codux\.exe"'/);
assert.match(script, /RMDir \/r "\$INSTDIR\\Data"/);
assert.match(script, /\/SD IDNO/);

const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "codux-package-runtime-"));
try {
  const runtimeRoot = path.join(tempDir, "runtime-root");
  __testStageRuntimeAssets(runtimeRoot);
  for (const relativePath of [
    "scripts/shell-hooks/dmux-ai-hook.zsh",
    "scripts/shell-hooks/zsh/.zlogin",
    "scripts/shell-hooks/zsh/.zprofile",
    "scripts/shell-hooks/zsh/.zshenv",
    "scripts/shell-hooks/zsh/.zshrc",
    "scripts/wrappers/tool-wrapper.sh",
    "scripts/wrappers/dmux-ai-state.sh",
    "scripts/wrappers/codux-ssh.ps1",
    "scripts/wrappers/codux-db.ps1",
    "scripts/wrappers/bin/codex",
    "scripts/wrappers/bin/codux-ssh",
    "scripts/wrappers/bin/codux-ssh.ps1",
    "scripts/wrappers/bin/codux-db",
    "scripts/wrappers/bin/codux-db.ps1",
  ]) {
    assert.equal(fs.existsSync(path.join(runtimeRoot, relativePath)), true, `${relativePath} should be packaged`);
  }
  assertNoSymlinks(runtimeRoot);
} finally {
  fs.rmSync(tempDir, { recursive: true, force: true });
}

console.log("package-gpui installer test passed");

if (process.platform === "win32") {
  const binaryDir = fs.mkdtempSync(path.join(os.tmpdir(), "codux-package-binaries-"));
  const packageDir = fs.mkdtempSync(path.join(os.tmpdir(), "codux-package-output-"));
  try {
    fs.writeFileSync(path.join(binaryDir, "codux.exe"), "gui");
    fs.writeFileSync(path.join(binaryDir, "codux-wrapper-helper.exe"), "console-helper");
    const oldBinaryDir = process.env.CODUX_RELEASE_BINARY_DIR;
    const oldTestPackageDir = process.env.CODUX_TEST_PACKAGE_DIR;
    const oldSkipMakensis = process.env.CODUX_TEST_SKIP_MAKENSIS;
    process.env.CODUX_RELEASE_BINARY_DIR = binaryDir;
    process.env.CODUX_TEST_PACKAGE_DIR = packageDir;
    process.env.CODUX_TEST_SKIP_MAKENSIS = "true";
    __testPackageWindows();
    process.env.CODUX_RELEASE_BINARY_DIR = oldBinaryDir;
    process.env.CODUX_TEST_PACKAGE_DIR = oldTestPackageDir;
    process.env.CODUX_TEST_SKIP_MAKENSIS = oldSkipMakensis;

    assert.equal(
      fs.readFileSync(path.join(packageDir, "runtime-root", "scripts", "wrappers", "codux-wrapper-helper.exe"), "utf8"),
      "console-helper",
      "Windows package should include the console wrapper helper",
    );
    for (const conptyFile of ["conpty.dll", "OpenConsole.exe", "conpty-LICENSE"]) {
      assert.equal(
        fs.existsSync(path.join(packageDir, conptyFile)),
        true,
        `Windows package should vendor the ConPTY pair (${conptyFile})`,
      );
    }

    const outputDir = path.join(
      process.cwd(),
      process.env.RELEASE_STAGE_DIR,
      process.env.RELEASE_BUILD_ID || `${process.platform}-${process.arch}`,
    );
    assert.equal(
      fs.readdirSync(outputDir).some((name) => name.endsWith("-setup.exe")),
      true,
      "mock installer should be produced",
    );
  } finally {
    fs.rmSync(binaryDir, { recursive: true, force: true });
    fs.rmSync(packageDir, { recursive: true, force: true });
  }
}

function assertNoSymlinks(root) {
  for (const entry of fs.readdirSync(root, { withFileTypes: true })) {
    const entryPath = path.join(root, entry.name);
    assert.equal(entry.isSymbolicLink(), false, `${entryPath} should not be a symlink`);
    if (entry.isDirectory()) {
      assertNoSymlinks(entryPath);
    }
  }
}
