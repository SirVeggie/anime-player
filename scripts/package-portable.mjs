import fs from 'node:fs/promises';
import path from 'node:path';
import { execSync } from 'node:child_process';

function getAndTagVersion() {
  let currentTag;
  try {
    currentTag = execSync('git describe --tags --exact-match', { encoding: 'utf-8', stdio: ['pipe', 'pipe', 'ignore'] }).trim();
    if (currentTag.startsWith('v')) {
      console.log(`Current commit is already tagged as ${currentTag}`);
      return currentTag;
    }
  } catch {
    // Not tagged exactly
  }

  let latestTag = 'v1.0';
  try {
    latestTag = execSync('git describe --tags --abbrev=0', { encoding: 'utf-8', stdio: ['pipe', 'pipe', 'ignore'] }).trim();
  } catch {
    console.log('No previous tags found, defaulting to v1.0');
  }

  const match = latestTag.match(/^v(\d+)\.(\d+)$/);
  let newVersion = 'v1.0';
  if (match) {
    const major = parseInt(match[1], 10);
    const minor = parseInt(match[2], 10);
    newVersion = `v${major}.${minor + 1}`;
  }

  console.log(`Creating new tag: ${newVersion}`);
  execSync(`git tag -a ${newVersion} -m "Release ${newVersion}"`);
  return newVersion;
}

async function packagePortable() {
  const root = process.cwd();
  const exePath = path.join(root, 'src-tauri', 'target', 'release', 'anime-player.exe');
  const dllPath = path.join(root, 'src-tauri', 'libs', 'mpv', 'libmpv-2.dll');

  try {
    await fs.access(exePath);
  } catch {
    console.error('Error: anime-player.exe not found in src-tauri/target/release.');
    console.error('Please run `npm run tauri build` first.');
    process.exit(1);
  }

  try {
    await fs.access(dllPath);
  } catch {
    console.error('Error: libmpv-2.dll not found in src-tauri/libs/mpv.');
    console.error('Please run `npm run setup:mpv` first.');
    process.exit(1);
  }

  const version = getAndTagVersion();
  const releasesDir = path.join(root, 'releases');
  const destDirName = `AnimePlayer-${version}`;
  const destDir = path.join(releasesDir, destDirName);
  const destExe = path.join(destDir, 'anime-player.exe');
  const destDll = path.join(destDir, 'libmpv-2.dll');
  const zipPath = path.join(releasesDir, `${destDirName}.zip`);

  console.log(`\nCreating ${destDirName}...`);
  
  await fs.mkdir(releasesDir, { recursive: true });
  await fs.rm(destDir, { recursive: true, force: true });
  await fs.mkdir(destDir, { recursive: true });

  console.log('Copying files...');
  await fs.copyFile(exePath, destExe);
  await fs.copyFile(dllPath, destDll);

  console.log(`Creating zip archive: ${destDirName}.zip...`);
  try {
    execSync(`powershell -NoProfile -Command "Compress-Archive -Path '${destDir}' -DestinationPath '${zipPath}' -Force"`, { stdio: 'inherit' });
  } catch (err) {
    console.error('Failed to create zip archive:', err.message);
  }

  console.log('\nSuccess! Your clean portable package is ready at:');
  console.log(destDir);
  console.log(`Archive created at:\n${zipPath}`);
}

packagePortable().catch(console.error);
