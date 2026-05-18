import fs from 'node:fs/promises';
import path from 'node:path';
import { execSync } from 'node:child_process';
import { getExactTagAtHead, createAnnotatedTag, getNextVersion } from './release-version.mjs';

async function publishGithubRelease() {
  const root = process.cwd();
  const releasesDir = path.join(root, 'releases');

  // 1. Verify gh is available
  try {
    execSync('gh --version', { stdio: 'ignore' });
  } catch {
    console.error('Error: GitHub CLI (gh) is not installed or not in PATH.');
    console.error('Please install it from https://cli.github.com/ and run `gh auth login`.');
    process.exit(1);
  }

  // Parse arguments
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

  // 2. Determine tag
  let targetTag = tagArg;
  if (!targetTag) {
    const headTag = getExactTagAtHead();
    if (headTag && headTag.startsWith('v')) {
      targetTag = headTag;
    } else {
      // If we are not at a tag, use next version, and we'll tag it shortly.
      targetTag = getNextVersion();
    }
  }

  // 3. Verify artifacts exist
  const zipName = `AnimePlayer-${targetTag}.zip`;
  const zipPath = path.join(releasesDir, zipName);
  const exePath = path.join(releasesDir, 'anime-player.exe');
  const shaPath = path.join(releasesDir, 'anime-player.exe.sha256');

  const requiredFiles = [zipPath, exePath, shaPath];
  for (const file of requiredFiles) {
    try {
      await fs.access(file);
    } catch {
      console.error(`Error: Required artifact not found: ${file}`);
      console.error(`Please run \`npm run release\` first to generate the build artifacts for ${targetTag}.`);
      process.exit(1);
    }
  }

  // 4. Verify notes exist
  const notesPath = notesPathArg ? path.resolve(root, notesPathArg) : path.join(releasesDir, `NOTES-${targetTag}.md`);
  try {
    await fs.access(notesPath);
  } catch {
    console.error(`Error: Release notes not found at: ${notesPath}`);
    console.error(`Please run \`npm run release:notes\` and polish the file before publishing.`);
    process.exit(1);
  }

  // 5. Ensure annotated tag exists at current commit
  const headTag = getExactTagAtHead();
  if (headTag !== targetTag) {
    console.log(`Tagging current commit as ${targetTag}...`);
    createAnnotatedTag(targetTag);
  }

  // Push the tag to remote so gh release create works properly against it
  console.log(`Pushing tag ${targetTag} to origin...`);
  try {
    execSync(`git push origin ${targetTag}`, { stdio: 'inherit' });
  } catch (err) {
    console.error(`Failed to push tag ${targetTag}. Is the remote set up correctly?`);
    process.exit(1);
  }

  // 6. Create GitHub Release
  console.log(`\nCreating GitHub release for ${targetTag}...`);
  try {
    const cmd = [
      'gh release create',
      targetTag,
      `--title "Anime Player ${targetTag}"`,
      `--notes-file "${notesPath}"`,
      `"${zipPath}"`,
      `"${exePath}"`,
      `"${shaPath}"`
    ].join(' ');

    execSync(cmd, { stdio: 'inherit' });
  } catch (err) {
    console.error('\nError: Failed to create GitHub release.');
    console.error('If the release already exists, you may need to delete it first:');
    console.error(`  gh release delete ${targetTag}`);
    process.exit(1);
  }

  console.log(`\nSuccessfully published release ${targetTag} to GitHub!`);
  console.log('Reminder: Remember to run `git push origin main` if you have unpushed commits.');
}

publishGithubRelease().catch(console.error);
