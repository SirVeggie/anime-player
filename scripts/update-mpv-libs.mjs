#!/usr/bin/env node
// Updates the bundled libmpv files under src-tauri/libs/mpv/.
//
// Downloads the latest libmpv development bundle from
// https://github.com/shinchiro/mpv-winbuild-cmake (a well-maintained
// continuous build of mpv for Windows) and copies the runtime DLL +
// import library into the repo. The committed copies in
// src-tauri/libs/mpv/ are what the build links against; this script
// is the way to refresh them when a new mpv release lands.
//
// Requirements (Windows):
//   - Node 18+
//   - 7z.exe on PATH (the mpv-dev archive is .7z)
//
// Usage:
//   node scripts/update-mpv-libs.mjs           # latest x64 build
//   node scripts/update-mpv-libs.mjs --arch=arm64

import {
  copyFileSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readdirSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { spawnSync } from "node:child_process";
import { dirname, resolve, basename } from "node:path";
import { tmpdir } from "node:os";
import { fileURLToPath } from "node:url";

const filename = fileURLToPath(import.meta.url);
const scriptDir = dirname(filename);
const projectRoot = dirname(scriptDir);
const outputDir = resolve(projectRoot, "src-tauri", "libs", "mpv");

const RELEASES_API =
  "https://api.github.com/repos/shinchiro/mpv-winbuild-cmake/releases?per_page=20";

function parseArgs(argv) {
  const args = { arch: "x86_64" };
  for (const raw of argv) {
    if (raw === "--help" || raw === "-h") {
      args.help = true;
    } else if (raw.startsWith("--arch=")) {
      args.arch = raw.slice("--arch=".length);
    } else {
      throw new Error(`Unknown argument: ${raw}`);
    }
  }
  return args;
}

function printUsage() {
  console.log("Usage: node scripts/update-mpv-libs.mjs [--arch=x86_64|arm64|i686]");
}

function ensureSevenZip() {
  // shell: true lets Node resolve "7z.cmd" / "7z.bat" wrappers via PATHEXT.
  const probe = spawnSync("7z", ["i"], { stdio: "ignore", shell: true });
  if (probe.status !== 0) {
    console.error(
      "[ERROR] 7z is required to extract the mpv-dev archive but was not found on PATH.\n" +
        "        Install 7-Zip (https://www.7-zip.org/) or `scoop install 7zip` and retry."
    );
    process.exit(1);
  }
}

async function fetchJson(url) {
  const headers = {
    Accept: "application/vnd.github+json",
    "User-Agent": "anime-player-update-mpv-libs",
  };
  const token = process.env.GITHUB_TOKEN || process.env.MPV_GITHUB_TOKEN;
  if (token) headers.Authorization = `Bearer ${token}`;
  const response = await fetch(url, { headers });
  if (!response.ok) {
    throw new Error(
      `GitHub API request failed (${response.status} ${response.statusText}): ${url}`
    );
  }
  return response.json();
}

function pickDevAsset(releases, arch) {
  // shinchiro publishes assets like:
  //   mpv-dev-x86_64-vYYYYMMDD-gitXXXXXXX.7z
  //   mpv-x86_64-vYYYYMMDD-gitXXXXXXX.7z (player, not what we want)
  // Walk recent releases until we find one carrying a mpv-dev-<arch>-*.7z asset.
  const pattern = new RegExp(`^mpv-dev-${arch}(?:-v[0-9].*)?\\.7z$`, "i");
  for (const release of releases) {
    if (release?.draft || release?.prerelease) continue;
    const assets = Array.isArray(release.assets) ? release.assets : [];
    const match = assets.find((asset) => pattern.test(asset?.name ?? ""));
    if (match?.browser_download_url) {
      return { release, asset: match };
    }
  }
  throw new Error(
    `No mpv-dev-${arch} asset found in the most recent shinchiro/mpv-winbuild-cmake releases.`
  );
}

async function downloadFile(url, destination) {
  const response = await fetch(url);
  if (!response.ok) {
    throw new Error(
      `Download failed (${response.status} ${response.statusText}): ${url}`
    );
  }
  const buffer = Buffer.from(await response.arrayBuffer());
  writeFileSync(destination, buffer);
}

function extract7z(archive, target) {
  const result = spawnSync("7z", ["x", "-y", `-o${target}`, archive], {
    stdio: "inherit",
    shell: true,
  });
  if (result.status !== 0) {
    throw new Error("7z extraction failed.");
  }
}

function findFile(dir, predicate) {
  const stack = [dir];
  while (stack.length > 0) {
    const current = stack.pop();
    let entries;
    try {
      entries = readdirSync(current, { withFileTypes: true });
    } catch {
      continue;
    }
    for (const entry of entries) {
      const full = resolve(current, entry.name);
      if (entry.isDirectory()) {
        stack.push(full);
      } else if (entry.isFile() && predicate(entry.name, full)) {
        return full;
      }
    }
  }
  return null;
}

async function main() {
  let args;
  try {
    args = parseArgs(process.argv.slice(2));
  } catch (err) {
    console.error(`[ERROR] ${err.message}`);
    printUsage();
    process.exit(1);
  }
  if (args.help) {
    printUsage();
    return;
  }

  if (process.platform !== "win32") {
    console.error("[ERROR] update-mpv-libs.mjs only supports Windows hosts.");
    process.exit(1);
  }

  ensureSevenZip();
  mkdirSync(outputDir, { recursive: true });

  console.log(`[INFO] Querying shinchiro/mpv-winbuild-cmake releases...`);
  const releases = await fetchJson(RELEASES_API);
  if (!Array.isArray(releases) || releases.length === 0) {
    throw new Error("GitHub returned no releases.");
  }
  const { release, asset } = pickDevAsset(releases, args.arch);
  console.log(`[INFO] Selected release: ${release.tag_name || release.name}`);
  console.log(`[INFO] Selected asset:   ${asset.name}`);

  const tmpRoot = mkdtempSync(resolve(tmpdir(), "anime-player-mpv-"));
  const archivePath = resolve(tmpRoot, asset.name);
  const extractDir = resolve(tmpRoot, "extracted");
  mkdirSync(extractDir);

  try {
    console.log(`[INFO] Downloading ${asset.name}...`);
    await downloadFile(asset.browser_download_url, archivePath);

    const sizeMb = (statSync(archivePath).size / (1024 * 1024)).toFixed(1);
    console.log(`[INFO] Downloaded ${sizeMb} MB to ${archivePath}`);

    console.log(`[INFO] Extracting ${asset.name}...`);
    extract7z(archivePath, extractDir);

    // The mpv-dev archive layout is roughly:
    //   libmpv-2.dll
    //   libmpv.dll.a   (MinGW import library; renamed to mpv.lib for MSVC)
    //   include/mpv/*.h
    const dllSource = findFile(extractDir, (name) =>
      /^libmpv-2\.dll$/i.test(name)
    );
    if (!dllSource) {
      throw new Error("Archive did not contain libmpv-2.dll.");
    }

    // MSVC link.exe accepts MinGW-style COFF .dll.a archives if renamed to .lib.
    // shinchiro ships either libmpv.dll.a or mpv.dll.a depending on the build.
    const importSource = findFile(extractDir, (name) =>
      /^(lib)?mpv(-2)?\.(dll\.a|lib)$/i.test(name)
    );
    if (!importSource) {
      throw new Error(
        "Archive did not contain an import library (libmpv.dll.a / mpv.lib)."
      );
    }

    const dllTarget = resolve(outputDir, "libmpv-2.dll");
    const libTarget = resolve(outputDir, "mpv.lib");

    copyFileSync(dllSource, dllTarget);
    copyFileSync(importSource, libTarget);

    console.log(`[INFO] Installed:`);
    console.log(`         ${basename(dllTarget)}  (${statSync(dllTarget).size} bytes)`);
    console.log(`         ${basename(libTarget)}  (${statSync(libTarget).size} bytes)`);

    // Record the source release so 'git diff' on the libs explains itself.
    const stamp = [
      `# Auto-generated by scripts/update-mpv-libs.mjs.`,
      `release: ${release.tag_name || release.name || "(unknown)"}`,
      `release_url: ${release.html_url || ""}`,
      `asset: ${asset.name}`,
      `asset_url: ${asset.browser_download_url}`,
      `installed_at: ${new Date().toISOString()}`,
      ``,
    ].join("\n");
    writeFileSync(resolve(outputDir, "VERSION.txt"), stamp);

    console.log(`[INFO] Wrote VERSION.txt with provenance.`);
    console.log(`[OK]   libmpv updated under ${outputDir}`);
  } finally {
    if (existsSync(tmpRoot)) {
      rmSync(tmpRoot, { recursive: true, force: true });
    }
  }
}

main().catch((err) => {
  console.error(`[ERROR] ${err?.message ?? err}`);
  process.exit(1);
});
