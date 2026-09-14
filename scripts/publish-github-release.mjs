import fs from 'node:fs/promises';
import path from 'node:path';
import { execSync } from 'node:child_process';
import { getExactTagAtHead, createAnnotatedTag, getNextVersion, getHeadCommit } from './release-version.mjs';
import { BINARIES_TAG, hashedAssetName } from './release-assets.mjs';

function runGh(args, options = {}) {
  return execSync(`gh ${args}`, {
    encoding: 'utf-8',
    stdio: options.stdio ?? ['pipe', 'pipe', 'pipe'],
    ...options,
  });
}

async function verifyBuildMeta(releasesDir, targetTag) {
  const buildMetaPath = path.join(releasesDir, '.build-meta.json');
  let raw;
  try {
    raw = await fs.readFile(buildMetaPath, 'utf8');
  } catch {
    console.error(`Error: Build metadata not found at: ${buildMetaPath}`);
    console.error('Please run `npm run release` first to generate build artifacts.');
    process.exit(1);
  }

  let meta;
  try {
    meta = JSON.parse(raw);
  } catch {
    console.error(`Error: Invalid build metadata at: ${buildMetaPath}`);
    process.exit(1);
  }

  const headCommit = getHeadCommit();
  if (meta.commit !== headCommit) {
    console.error('Error: Release artifacts were built for a different commit than HEAD.');
    console.error(`  Built for: ${meta.commit}`);
    console.error(`  HEAD is:   ${headCommit}`);
    console.error('Run `npm run release` again at the current commit before publishing.');
    process.exit(1);
  }

  if (meta.tag !== targetTag) {
    console.error('Error: Release artifacts were built for a different version tag.');
    console.error(`  Built for: ${meta.tag}`);
    console.error(`  Publishing: ${targetTag}`);
    console.error('Run `npm run release` again for this version, or pass `--tag` matching the build.');
    process.exit(1);
  }

  if (!Array.isArray(meta.files) || meta.files.length === 0) {
    console.error('Error: Build metadata is missing the hashed install-file list.');
    console.error('Run `npm run release` again to generate a updater manifest.');
    process.exit(1);
  }

  return meta;
}

function existingBinaryAssets() {
  try {
    const raw = runGh(`release view ${BINARIES_TAG} --json assets`);
    const parsed = JSON.parse(raw);
    return new Set((parsed.assets ?? []).map((asset) => asset.name));
  } catch {
    return null;
  }
}

function ensureBinariesRelease() {
  const existing = existingBinaryAssets();
  if (existing) {
    return existing;
  }

  console.log(`Creating prerelease ${BINARIES_TAG} for shared install files...`);
  try {
    runGh(
      `release create ${BINARIES_TAG} --prerelease --title "Shared runtime binaries" --notes "Content-addressed install files for the in-app updater. Not a versioned app release."`,
      { stdio: 'inherit' },
    );
  } catch {
    console.error(`Error: Failed to create the ${BINARIES_TAG} prerelease.`);
    process.exit(1);
  }
  return existingBinaryAssets() ?? new Set();
}

async function publishHashedBinaries(releasesDir, files) {
  const existing = ensureBinariesRelease();
  const stagingDir = path.join(releasesDir, '.hashed-upload');
  await fs.mkdir(stagingDir, { recursive: true });
  try {
    for (const file of files) {
      const assetName = hashedAssetName(file.name, file.sha256);
      if (existing.has(assetName)) {
        console.log(`Already on ${BINARIES_TAG}: ${assetName}`);
        continue;
      }
      const localPath = path.join(releasesDir, file.localPath);
      try {
        await fs.access(localPath);
      } catch {
        console.error(`Error: Hashed source file not found: ${localPath}`);
        process.exit(1);
      }
      // Upload from a hashed filename. `path#name` only sets the GitHub
      // display label; the download URL still uses the local basename.
      const stagedPath = path.join(stagingDir, assetName);
      await fs.copyFile(localPath, stagedPath);
      console.log(`Uploading ${assetName} to ${BINARIES_TAG}...`);
      try {
        runGh(`release upload ${BINARIES_TAG} "${stagedPath}"`, { stdio: 'inherit' });
        existing.add(assetName);
      } catch {
        console.error(`Error: Failed to upload ${assetName} to ${BINARIES_TAG}.`);
        process.exit(1);
      }
    }
  } finally {
    await fs.rm(stagingDir, { recursive: true, force: true });
  }
}

async function writeManifestNotes(manifestPath, notesPath, version) {
  const manifest = JSON.parse(await fs.readFile(manifestPath, 'utf8'));
  const notes = (await fs.readFile(notesPath, 'utf8')).trim();
  manifest.version = version;
  manifest.notes = notes;
  await fs.writeFile(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`, 'utf8');
}

async function publishGithubRelease() {
  const root = process.cwd();
  const releasesDir = path.join(root, 'releases');

  try {
    execSync('gh --version', { stdio: 'ignore' });
  } catch {
    console.error('Error: GitHub CLI (gh) is not installed or not in PATH.');
    console.error('Please install it from https://cli.github.com/ and run `gh auth login`.');
    process.exit(1);
  }

  try {
    execSync('gh auth status', { stdio: 'ignore' });
  } catch {
    console.error('Error: GitHub CLI is not authenticated.');
    console.error('Run `gh auth login` with repo scope, or set GITHUB_TOKEN.');
    process.exit(1);
  }

  const args = process.argv.slice(2);
  let notesPathArg = null;
  let tagArg = null;

  for (let i = 0; i < args.length; i++) {
    if (args[i] === '--notes' && args[i + 1]) {
      notesPathArg = args[i + 1];
      i++;
    } else if (args[i] === '--tag' && args[i + 1]) {
      tagArg = args[i + 1];
      i++;
    }
  }

  let targetTag = tagArg;
  if (!targetTag) {
    const headTag = getExactTagAtHead();
    if (headTag && headTag.startsWith('v')) {
      targetTag = headTag;
    } else {
      targetTag = getNextVersion();
    }
  }

  const zipName = `AnimePlayer-${targetTag}.zip`;
  const zipPath = path.join(releasesDir, zipName);
  const manifestPath = path.join(releasesDir, 'manifest.json');

  for (const file of [zipPath, manifestPath]) {
    try {
      await fs.access(file);
    } catch {
      console.error(`Error: Required artifact not found: ${file}`);
      console.error(`Please run \`npm run release\` first to generate the build artifacts for ${targetTag}.`);
      process.exit(1);
    }
  }

  const meta = await verifyBuildMeta(releasesDir, targetTag);

  const notesPath = notesPathArg ? path.resolve(root, notesPathArg) : path.join(releasesDir, `NOTES-${targetTag}.md`);
  try {
    await fs.access(notesPath);
  } catch {
    console.error(`Error: Release notes not found at: ${notesPath}`);
    console.error(`Please run \`npm run release:notes\` and polish the file before publishing.`);
    process.exit(1);
  }

  await writeManifestNotes(manifestPath, notesPath, targetTag);

  const headTag = getExactTagAtHead();
  if (headTag !== targetTag) {
    console.log(`Tagging current commit as ${targetTag}...`);
    try {
      createAnnotatedTag(targetTag);
    } catch {
      process.exit(1);
    }
  }

  await publishHashedBinaries(releasesDir, meta.files);

  console.log(`Pushing tag ${targetTag} to origin...`);
  try {
    execSync(`git push origin ${targetTag}`, { stdio: 'inherit' });
  } catch {
    console.error(`Failed to push tag ${targetTag}. Is the remote set up correctly?`);
    process.exit(1);
  }

  console.log(`\nCreating GitHub release for ${targetTag}...`);
  try {
    const cmd = [
      'gh release create',
      targetTag,
      `--title "${targetTag}"`,
      `--notes-file "${notesPath}"`,
      `"${zipPath}"`,
      `"${manifestPath}"`,
    ].join(' ');

    execSync(cmd, { stdio: 'inherit' });
  } catch {
    console.error('\nError: Failed to create GitHub release.');
    console.error('If the release already exists, you may need to delete it first:');
    console.error(`  gh release delete ${targetTag}`);
    process.exit(1);
  }

  console.log(`\nSuccessfully published release ${targetTag} to GitHub!`);
  console.log('Reminder: Remember to run `git push origin main` if you have unpushed commits.');
}

publishGithubRelease().catch((err) => {
  console.error(err);
  process.exit(1);
});
