#!/usr/bin/env node
/* global console, process */

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const packageManifests = [
  ["apps/desktop/Cargo.toml", "codux"],
  ["apps/desktop/runtime/Cargo.toml", "codux-runtime"],
  ["apps/agent/Cargo.toml", "codux-agent"],
  ["apps/wrapper-helper/Cargo.toml", "codux-wrapper-helper"],
];

export function nextLocalPackageVersion(currentVersion) {
  const match = currentVersion.match(
    /^(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.-]+))?(?:\+[0-9A-Za-z.-]+)?$/,
  );
  if (!match) {
    throw new Error(`Invalid SemVer version "${currentVersion}".`);
  }

  const [, major, minor, patch, prerelease] = match;
  if (!prerelease) {
    return `${major}.${minor}.${Number(patch) + 1}`;
  }

  const identifiers = prerelease.split(".");
  const lastIndex = identifiers.length - 1;
  if (/^\d+$/.test(identifiers[lastIndex])) {
    identifiers[lastIndex] = String(Number(identifiers[lastIndex]) + 1);
  } else {
    identifiers.push("1");
  }
  return `${major}.${minor}.${patch}-${identifiers.join(".")}`;
}

export function bumpWorkspaceVersion(repoRoot) {
  const manifestStates = packageManifests.map(([relativePath, packageName]) => {
    const filePath = path.join(repoRoot, relativePath);
    const content = fs.readFileSync(filePath, "utf8");
    const version = content.match(/^version = "([^"]+)"$/m)?.[1];
    if (!version) {
      throw new Error(`Package version not found in ${relativePath}.`);
    }
    return { relativePath, packageName, filePath, content, version };
  });

  const previousVersion = manifestStates[0].version;
  for (const manifest of manifestStates) {
    if (manifest.version !== previousVersion) {
      throw new Error(
        `Package versions are out of sync: ${manifest.relativePath} is ${manifest.version}, expected ${previousVersion}.`,
      );
    }
  }

  const lockPath = path.join(repoRoot, "Cargo.lock");
  const lockContent = fs.readFileSync(lockPath, "utf8");
  for (const { packageName } of manifestStates) {
    const pattern = packageVersionPattern(packageName);
    const lockedVersion = lockContent.match(pattern)?.[2];
    if (!lockedVersion) {
      throw new Error(`Package ${packageName} not found in Cargo.lock.`);
    }
    if (lockedVersion !== previousVersion) {
      throw new Error(
        `Package versions are out of sync: Cargo.lock ${packageName} is ${lockedVersion}, expected ${previousVersion}.`,
      );
    }
  }

  const version = nextLocalPackageVersion(previousVersion);
  for (const manifest of manifestStates) {
    const updated = manifest.content.replace(
      /^version = "[^"]+"$/m,
      `version = "${version}"`,
    );
    fs.writeFileSync(manifest.filePath, updated, "utf8");
  }

  let updatedLock = lockContent;
  for (const { packageName } of manifestStates) {
    updatedLock = updatedLock.replace(
      packageVersionPattern(packageName),
      `$1${version}$3`,
    );
  }
  fs.writeFileSync(lockPath, updatedLock, "utf8");

  return { previousVersion, version };
}

function packageVersionPattern(packageName) {
  return new RegExp(
    `(name = "${escapeRegExp(packageName)}"\\r?\\nversion = ")([^"]+)(")`,
  );
}

function escapeRegExp(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

const scriptPath = fileURLToPath(import.meta.url);
if (process.argv[1] && path.resolve(process.argv[1]) === scriptPath) {
  const repoRoot = process.env.CODUX_REPO_ROOT
    ? path.resolve(process.env.CODUX_REPO_ROOT)
    : path.resolve(path.dirname(scriptPath), "../../../..");
  const result = bumpWorkspaceVersion(repoRoot);
  console.log(`previous_version=${result.previousVersion}`);
  console.log(`version=${result.version}`);
}
