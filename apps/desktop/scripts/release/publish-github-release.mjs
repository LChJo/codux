#!/usr/bin/env node
/* global console, process */

import fs from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";

const root = process.cwd();
const dryRun = process.argv.includes("--dry-run");
const version = requiredEnv("RELEASE_VERSION");
const channel = requiredEnv("RELEASE_CHANNEL");
const tagName = process.env.RELEASE_TAG || `v${version}`;
const repo = process.env.GITHUB_REPOSITORY || "LChJo/codux";
const notesPath = process.env.RELEASE_NOTES_PATH || path.join(root, "dist", `release-notes-${version}.md`);
const artifactsDir = process.env.RELEASE_ARTIFACTS_DIR || path.join(root, "release-artifacts");
const notes = fs.existsSync(notesPath) ? fs.readFileSync(notesPath, "utf8") : `Codux ${version}`;
if (!["stable", "beta"].includes(channel)) {
  throw new Error(`RELEASE_CHANNEL must be stable or beta, got ${channel}`);
}
// Stable releases also refresh the beta manifest so beta-channel installs
// are never stranded behind the stable channel.
const manifestChannels = channel === "stable" ? ["stable", "beta"] : [channel];
const requireExistingRelease = process.env.RELEASE_REQUIRE_EXISTING === "true";
const uploadLatest = process.env.RELEASE_UPLOAD_LATEST !== "false";
const publishManifest = process.env.RELEASE_PUBLISH_MANIFEST !== "false";
const uploadSigAssets = process.env.RELEASE_UPLOAD_SIG_ASSETS === "true";
const mergeExistingLatest = process.env.RELEASE_MERGE_EXISTING_LATEST === "true";

const assets = collectAssets(artifactsDir);
if (!assets.length) {
  throw new Error(`No release assets found in ${artifactsDir}`);
}
const uploadAssets = assets.filter((asset) => shouldUploadPublicAsset(asset) || (uploadSigAssets && asset.name.endsWith(".sig")));

const latestJson = mergeExistingLatest ? mergeWithExistingLatest(buildLatestJson(assets)) : buildLatestJson(assets);
const latestPath = path.join(artifactsDir, "latest.json");
fs.writeFileSync(latestPath, `${JSON.stringify(latestJson, null, 2)}\n`, "utf8");

if (!dryRun) {
  const releaseFlag = latestReleaseFlag();
  ensureRelease();
  for (const asset of uploadAssets) {
    run("gh", ["release", "upload", tagName, "--repo", repo, "--clobber", `${asset.path}#${asset.name}`]);
  }
  if (uploadLatest) {
    run("gh", ["release", "upload", tagName, "--repo", repo, "--clobber", `${latestPath}#latest.json`]);
  }
  run("gh", ["release", "edit", tagName, "--repo", repo, "--draft=false", "--title", `Codux ${version}`, "--notes-file", notesPath, releaseFlag]);
  if (publishManifest) {
    publishChannelManifest(latestPath);
  }
}

function mergeWithExistingLatest(next) {
  if (dryRun) return next;
  const tempDir = path.join(artifactsDir, `.existing-latest-${Date.now()}`);
  fs.rmSync(tempDir, { recursive: true, force: true });
  fs.mkdirSync(tempDir, { recursive: true });
  const downloaded = spawnSync(
    "gh",
    ["release", "download", tagName, "--repo", repo, "--pattern", "latest.json", "--dir", tempDir],
    { stdio: "ignore", env: process.env },
  );
  if (downloaded.status !== 0) {
    fs.rmSync(tempDir, { recursive: true, force: true });
    return next;
  }
  const existingPath = path.join(tempDir, "latest.json");
  if (!fs.existsSync(existingPath)) {
    fs.rmSync(tempDir, { recursive: true, force: true });
    return next;
  }
  const existing = JSON.parse(fs.readFileSync(existingPath, "utf8"));
  fs.rmSync(tempDir, { recursive: true, force: true });
  return {
    ...existing,
    version: next.version,
    notes: next.notes,
    pub_date: next.pub_date,
    platforms: {
      ...(existing.platforms || {}),
      ...next.platforms,
    },
  };
}

console.log(
  `${dryRun ? "Prepared" : "Published"} ${uploadAssets.length} public assets and update metadata to ${repo}@${tagName} (${channel})`,
);

function requiredEnv(name) {
  const value = process.env[name]?.trim();
  if (!value) {
    throw new Error(`${name} is required`);
  }
  return value;
}

function collectAssets(dir) {
  return walk(dir)
    .filter((file) => !file.endsWith("latest.json"))
    .filter((file) => !file.endsWith(".blockmap"))
    .map((file) => ({
      path: file,
      name: normalizeAssetName(path.relative(dir, file)),
      ext: artifactExt(file),
    }))
    .filter((asset) => shouldUpload(asset));
}

function walk(dir) {
  if (!fs.existsSync(dir)) return [];
  const entries = fs.readdirSync(dir, { withFileTypes: true });
  return entries.flatMap((entry) => {
    const file = path.join(dir, entry.name);
    return entry.isDirectory() ? walk(file) : [file];
  });
}

function normalizeAssetName(relativePath) {
  return relativePath
    .split(path.sep)
    .join("-")
    .replace(/[ ()[\]{}]/g, ".")
    .replace(/\.\./g, ".")
    .normalize("NFD")
    .replace(/[\u0300-\u036f]/g, "");
}

function artifactExt(file) {
  const name = path.basename(file);
  if (name.endsWith(".app.tar.gz.sig")) return ".app.tar.gz.sig";
  if (name.endsWith(".app.tar.gz")) return ".app.tar.gz";
  if (name.endsWith(".tar.gz.sig")) return ".tar.gz.sig";
  if (name.endsWith(".nsis.zip.sig")) return ".nsis.zip.sig";
  if (name.endsWith(".msi.zip.sig")) return ".msi.zip.sig";
  if (name.endsWith(".app.zip")) return ".app.zip";
  if (name.endsWith(".tar.gz")) return ".tar.gz";
  if (name.endsWith(".sha256")) return ".sha256";
  if (name.endsWith(".exe.sig")) return ".exe.sig";
  if (name.endsWith(".msi.sig")) return ".msi.sig";
  return path.extname(name);
}

function shouldUpload(asset) {
  if (isAgentAsset(asset.name)) return true;
  return [
    ".dmg",
    ".exe",
    ".msi",
    ".app.tar.gz",
    ".tar.gz",
    ".app.tar.gz.sig",
    ".tar.gz.sig",
    ".nsis.zip.sig",
    ".msi.zip.sig",
    ".exe.sig",
    ".msi.sig",
  ].includes(asset.ext);
}

function shouldUploadPublicAsset(asset) {
  if (isAgentAsset(asset.name)) return true;
  const lower = asset.name.toLowerCase();
  if (lower.endsWith(".sha256") || lower.endsWith(".sig")) return false;
  if (lower.endsWith(".app.zip") || lower.endsWith(".zip")) return false;
  if (lower.includes("-debug")) return false;
  if (isVersionedUserInstallerAsset(lower)) return false;
  if (asset.ext === ".exe" && !lower.endsWith("-setup.exe")) return false;
  return [".dmg", ".app.tar.gz", ".exe", ".msi"].includes(asset.ext);
}

function isAgentAsset(name) {
  return /^codux-(?:(?:macos|linux)-(?:aarch64|x86_64)|windows-x86_64\.exe)$/.test(name);
}

function isVersionedUserInstallerAsset(lowerName) {
  return (
    /^codux-\d+\.\d+\.\d+(?:-[a-z0-9.]+)?-macos-(?:aarch64|x86_64|universal)\.dmg$/.test(
      lowerName,
    ) ||
    /^codux-\d+\.\d+\.\d+(?:-[a-z0-9.]+)?-macos-(?:aarch64|x86_64|universal)-updater\.app\.tar\.gz$/.test(
      lowerName,
    ) ||
    /^codux-\d+\.\d+\.\d+(?:-[a-z0-9.]+)?-windows-x86_64-setup\.exe$/.test(lowerName)
  );
}

function buildLatestJson(assets) {
  const platforms = {};
  const byName = new Map(assets.map((asset) => [asset.name, asset]));
  const signatureAssets = assets
    .filter((asset) => asset.name.endsWith(".sig"))
    .sort((left, right) => signaturePriority(right.name) - signaturePriority(left.name));

  for (const signatureAsset of signatureAssets) {
    const bundleName = signatureAsset.name.slice(0, -".sig".length);
    const bundleAsset = byName.get(bundleName);
    if (!bundleAsset) {
      console.warn(`Skipping ${signatureAsset.name}: matching bundle ${bundleName} was not found`);
      continue;
    }
    for (const platform of platformKeys(signatureAsset.name)) {
      if (platforms[platform]) continue;
      platforms[platform] = {
        signature: fs.readFileSync(signatureAsset.path, "utf8").trim(),
        url: releaseAssetUrl(publicUpdaterAssetName(bundleAsset, byName)),
      };
    }
  }
  for (const asset of assets.filter((entry) => /^codux-(?:windows-x86_64-setup\.exe|macos-(?:aarch64|x86_64|universal)\.dmg)$/.test(entry.name))) {
    const url = releaseAssetUrl(asset.name);
    for (const platform of platformKeys(asset.name)) {
      if (platforms[platform]) {
        platforms[platform].manualUrl = url;
      } else {
        platforms[platform] = { url };
      }
    }
  }
  if (!Object.keys(platforms).length) {
    throw new Error("No desktop updater or installer assets found.");
  }
  return {
    version,
    notes,
    pub_date: new Date().toISOString(),
    platforms,
  };
}

function publicUpdaterAssetName(bundleAsset, byName) {
  const stableName = bundleAsset.name
    .replace(/^codux-\d+\.\d+\.\d+(?:-[a-z0-9.]+)?-(macos-(?:aarch64|x86_64|universal)-updater\.app\.tar\.gz)$/i, "codux-$1")
    .replace(/^codux-\d+\.\d+\.\d+(?:-[a-z0-9.]+)?-(windows-x86_64-setup\.(?:exe|msi))$/i, "codux-$1");
  if (stableName === bundleAsset.name) return bundleAsset.name;
  if (!byName.has(stableName)) {
    throw new Error(`Stable updater asset ${stableName} was not found for ${bundleAsset.name}`);
  }
  return stableName;
}

function releaseAssetUrl(assetName) {
  return `https://github.com/${repo}/releases/download/${encodeURIComponent(tagName)}/${encodeURIComponent(assetName)}`;
}

function platformKeys(assetName) {
  const lower = assetName.toLowerCase();
  if ((lower.includes("macos") || lower.includes("darwin")) && lower.includes("universal")) {
    return ["darwin-aarch64", "darwin-x86_64", "darwin-aarch64-app", "darwin-x86_64-app"];
  }
  if ((lower.includes("macos") || lower.includes("darwin")) && lower.includes("aarch64")) {
    return ["darwin-aarch64", "darwin-aarch64-app"];
  }
  if ((lower.includes("macos") || lower.includes("darwin")) && lower.includes("x86_64")) {
    return ["darwin-x86_64", "darwin-x86_64-app"];
  }
  if (lower.includes("windows") && lower.includes("x86_64")) {
    if (lower.includes("msi")) return ["windows-x86_64", "windows-x86_64-msi"];
    return ["windows-x86_64", "windows-x86_64-nsis"];
  }
  if (lower.includes("linux") && lower.includes("x86_64")) {
    return ["linux-x86_64"];
  }
  return [];
}

function signaturePriority(name) {
  const lower = name.toLowerCase();
  if (lower.endsWith(".app.tar.gz.sig")) return 100;
  if (lower.endsWith(".nsis.zip.sig")) return 100;
  if (lower.endsWith(".exe.sig")) return 100;
  if (lower.endsWith(".msi.zip.sig")) return 90;
  if (lower.endsWith(".msi.sig")) return 90;
  return 0;
}

function run(command, args) {
  const result = spawnSync(command, args, { stdio: "inherit", env: process.env });
  if (result.status !== 0) {
    throw new Error(`${command} ${args.join(" ")} failed with exit code ${result.status}`);
  }
}

function latestReleaseFlag() {
  if (channel === "beta") return "--latest=false";
  run("git", ["fetch", "origin", "refs/heads/main:refs/remotes/origin/main"]);
  const previous = spawnSync("git", ["show", "origin/main:updates/stable/latest.json"], { encoding: "utf8" });
  const published = previous.status === 0 ? JSON.parse(previous.stdout).version : null;
  return published && compareVersions(published, version) > 0 ? "--latest=false" : "--latest";
}

function ensureRelease() {
  const releaseExists =
    spawnSync("gh", ["release", "view", tagName, "--repo", repo], {
      stdio: "ignore",
      env: process.env,
    }).status === 0;
  if (!releaseExists && requireExistingRelease) {
    throw new Error(`Release ${tagName} does not exist.`);
  }
  if (releaseExists) return;
  run("gh", [
    "release",
    "create",
    tagName,
    "--repo",
    repo,
    "--title",
    `Codux ${version}`,
    "--notes-file",
    notesPath,
    "--verify-tag",
    "--draft",
  ]);
}

function publishChannelManifest(sourcePath) {
  const latestContent = fs.readFileSync(sourcePath, "utf8");
  run("git", ["fetch", "origin", "refs/heads/main:refs/remotes/origin/main"]);
  run("git", ["checkout", "-B", "main", "origin/main"]);
  for (const manifestChannel of manifestChannels) {
    const manifestPath = path.join(root, "updates", manifestChannel, "latest.json");
    if (fs.existsSync(manifestPath)) {
      const current = JSON.parse(fs.readFileSync(manifestPath, "utf8")).version;
      if (current && compareVersions(current, version) > 0) continue;
    }
    fs.mkdirSync(path.dirname(manifestPath), { recursive: true });
    fs.writeFileSync(manifestPath, latestContent, "utf8");
    run("git", ["add", manifestPath]);
  }
  run("git", ["config", "user.name", "github-actions[bot]"]);
  run("git", ["config", "user.email", "41898282+github-actions[bot]@users.noreply.github.com"]);
  const diff = spawnSync("git", ["diff", "--cached", "--quiet"], { stdio: "inherit", env: process.env });
  if (diff.status === 0) return;
  run("git", ["commit", "-m", `chore: update ${manifestChannels.join("+")} updater manifest for ${version}`]);
  run("git", ["push", "origin", "HEAD:main"]);
}

function compareVersions(left, right) {
  const parts = (value) => {
    const [core, ...prerelease] = value.split("+")[0].split("-");
    return [core.split(".").map(Number), prerelease.join("-").split(".")];
  };
  const [leftCore, leftPre] = parts(left);
  const [rightCore, rightPre] = parts(right);
  for (let index = 0; index < 3; index += 1) {
    if (leftCore[index] !== rightCore[index]) return leftCore[index] - rightCore[index];
  }
  if (!leftPre[0] || !rightPre[0]) return Number(!leftPre[0]) - Number(!rightPre[0]);
  for (let index = 0; index < Math.min(leftPre.length, rightPre.length); index += 1) {
    const a = leftPre[index];
    const b = rightPre[index];
    if (a === b) continue;
    const aNumeric = /^\d+$/.test(a);
    const bNumeric = /^\d+$/.test(b);
    if (aNumeric && bNumeric) return Number(a) - Number(b);
    if (aNumeric !== bNumeric) return aNumeric ? -1 : 1;
    return a < b ? -1 : 1;
  }
  return leftPre.length - rightPre.length;
}
