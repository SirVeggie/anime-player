#!/usr/bin/env node
// Downloads Windows Chromaprint fpcalc into src-tauri/libs/chromaprint/ (gitignored).
import {
  copyFileSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readdirSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { spawnSync } from "node:child_process";
import { dirname, resolve } from "node:path";
import { tmpdir } from "node:os";
import { fileURLToPath } from "node:url";

const scriptDir = dirname(fileURLToPath(import.meta.url));
const projectRoot = dirname(scriptDir);
const outputDir = resolve(projectRoot, "src-tauri", "libs", "chromaprint");

const RELEASES_API =
  "https://api.github.com/repos/acoustid/chromaprint/releases?per_page=10";

const ASSET_PATTERN = /^chromaprint-fpcalc-.+-windows-x86_64\.zip$/i;

function ensureSevenZip() {
  const probe = spawnSync("7z", ["i"], { stdio: "ignore", shell: true });
  if (probe.status !== 0) {
    console.error(
      "[ERROR] 7z is required but was not found on PATH.\n" +
        "        Install 7-Zip (https://www.7-zip.org/) and retry.",
    );
    process.exit(1);
  }
}

async function fetchJson(url) {
  const headers = {
    Accept: "application/vnd.github+json",
    "User-Agent": "anime-player-setup-chromaprint",
  };
  const token = process.env.GITHUB_TOKEN;
  if (token) headers.Authorization = `Bearer ${token}`;
  const response = await fetch(url, { headers });
  if (!response.ok) {
    throw new Error(`GitHub API failed (${response.status}): ${url}`);
  }
  return response.json();
}

function pickAsset(releases) {
  for (const release of releases) {
    if (release?.draft || release?.prerelease) continue;
    for (const asset of release.assets ?? []) {
      if (ASSET_PATTERN.test(asset?.name ?? "")) {
        return { release, asset };
      }
    }
  }
  return null;
}

function findFpcalc(root) {
  const queue = [root];
  while (queue.length > 0) {
    const dir = queue.shift();
    const entries = readdirSync(dir, { withFileTypes: true });
    const fpcalc = entries.find(
      (e) => e.isFile() && e.name.toLowerCase() === "fpcalc.exe",
    );
    if (fpcalc) return resolve(dir, fpcalc.name);
    for (const entry of entries) {
      if (entry.isDirectory()) queue.push(resolve(dir, entry.name));
    }
  }
  return null;
}

async function main() {
  ensureSevenZip();
  console.log("[chromaprint] Resolving latest Windows fpcalc build...");
  const releases = await fetchJson(RELEASES_API);
  const picked = pickAsset(releases);
  if (!picked) {
    throw new Error("Could not find a Windows x86_64 fpcalc zip on Chromaprint releases.");
  }

  const tmpRoot = mkdtempSync(resolve(tmpdir(), "anime-player-chromaprint-"));
  const zipPath = resolve(tmpRoot, picked.asset.name);
  console.log(`[chromaprint] Downloading ${picked.asset.name}...`);
  const blob = await fetch(picked.asset.browser_download_url);
  if (!blob.ok) throw new Error(`Download failed (${blob.status})`);
  const buffer = Buffer.from(await blob.arrayBuffer());
  writeFileSync(zipPath, buffer);

  const extractDir = resolve(tmpRoot, "extract");
  mkdirSync(extractDir, { recursive: true });
  const extract = spawnSync("7z", ["x", zipPath, `-o${extractDir}`, "-y"], {
    stdio: "inherit",
    shell: true,
  });
  if (extract.status !== 0) process.exit(extract.status ?? 1);

  const fpcalc = findFpcalc(extractDir);
  if (!fpcalc || !existsSync(fpcalc)) {
    throw new Error("fpcalc.exe not found inside archive.");
  }

  mkdirSync(outputDir, { recursive: true });
  copyFileSync(fpcalc, resolve(outputDir, "fpcalc.exe"));
  writeFileSync(
    resolve(outputDir, "VERSION.txt"),
    `${picked.release.tag_name}\n${picked.asset.name}\n`,
  );

  rmSync(tmpRoot, { recursive: true, force: true });
  console.log(`[chromaprint] Installed to ${outputDir}`);
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
