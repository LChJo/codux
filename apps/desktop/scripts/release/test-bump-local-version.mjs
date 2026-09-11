#!/usr/bin/env node

import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

import { nextLocalPackageVersion } from "./bump-local-version.mjs";

assert.equal(nextLocalPackageVersion("2.0.0-beta.6"), "2.0.0-beta.7");
assert.equal(nextLocalPackageVersion("2.0.0-rc.9"), "2.0.0-rc.10");
assert.equal(nextLocalPackageVersion("2.0.0-preview"), "2.0.0-preview.1");
assert.equal(nextLocalPackageVersion("2.0.0"), "2.0.1");
assert.equal(nextLocalPackageVersion("2.0.0-beta.6+local"), "2.0.0-beta.7");
assert.throws(() => nextLocalPackageVersion("not-semver"), /Invalid SemVer/);

const tempRoot = fs.mkdtempSync(path.join(os.tmpdir(), "codux-version-bump-test-"));
try {
  const manifests = [
    ["apps/desktop/Cargo.toml", "codux"],
    ["apps/desktop/runtime/Cargo.toml", "codux-runtime"],
    ["apps/agent/Cargo.toml", "codux-agent"],
    ["apps/wrapper-helper/Cargo.toml", "codux-wrapper-helper"],
  ];
  for (const [relativePath, packageName] of manifests) {
    const filePath = path.join(tempRoot, relativePath);
    fs.mkdirSync(path.dirname(filePath), { recursive: true });
    fs.writeFileSync(
      filePath,
      `[package]\nname = "${packageName}"\nversion = "2.0.0-beta.6"\nedition = "2024"\n`,
      "utf8",
    );
  }
  fs.writeFileSync(
    path.join(tempRoot, "Cargo.lock"),
    [
      ...manifests.flatMap(([, packageName]) => [
        "[[package]]",
        `name = "${packageName}"`,
        'version = "2.0.0-beta.6"',
        "",
      ]),
      "[[package]]",
      'name = "unrelated"',
      'version = "9.9.9"',
      "",
    ].join("\n"),
    "utf8",
  );

  const bumpResult = spawnSync(process.execPath, [fileURLToPath(new URL("./bump-local-version.mjs", import.meta.url))], {
    cwd: process.cwd(),
    env: { ...process.env, CODUX_REPO_ROOT: tempRoot },
    encoding: "utf8",
  });
  assert.equal(bumpResult.status, 0, bumpResult.stderr || bumpResult.stdout);
  assert.match(bumpResult.stdout, /^previous_version=2\.0\.0-beta\.6$/m);
  assert.match(bumpResult.stdout, /^version=2\.0\.0-beta\.7$/m);

  for (const [relativePath] of manifests) {
    assert.match(
      fs.readFileSync(path.join(tempRoot, relativePath), "utf8"),
      /^version = "2\.0\.0-beta\.7"$/m,
    );
  }
  const lock = fs.readFileSync(path.join(tempRoot, "Cargo.lock"), "utf8");
  assert.equal((lock.match(/version = "2\.0\.0-beta\.7"/g) || []).length, 4);
  assert.match(lock, /name = "unrelated"\nversion = "9\.9\.9"/);
} finally {
  fs.rmSync(tempRoot, { recursive: true, force: true });
}

console.log("local package version bump test passed");
