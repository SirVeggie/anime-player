import fs from 'node:fs/promises';
import path from 'node:path';
import { createHash } from 'node:crypto';
import { execSync } from 'node:child_process';
import { getAndTagVersion } from './release-version.mjs';

async function sha256File(filePath) {
  const data = await fs.readFile(filePath);
  return createHash('sha256').update(data).digest('hex').toUpperCase();
}

async function packagePortable() {
  const root = process.cwd();
  const exePath = path.join(root, 'src-tauri', 'target', 'release', 'anime-player.exe');
  const dllPath = path.join(root, 'src-tauri', 'libs', 'mpv', 'libmpv-2.dll');
  const updateBat = path.join(root, 'scripts', 'update.bat');
  const updatePs1 = path.join(root, 'scripts', '_update.ps1');

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

  for (const scriptPath of [updateBat, updatePs1]) {
    try {
      await fs.access(scriptPath);
    } catch {
      console.error(`Error: ${path.basename(scriptPath)} not found in scripts/.`);
      process.exit(1);
    }
  }

  const version = getAndTagVersion();
  const releasesDir = path.join(root, 'releases');
  const destDirName = `AnimePlayer-${version}`;
  const destDir = path.join(releasesDir, destDirName);
  const destExe = path.join(destDir, 'anime-player.exe');
  const destDll = path.join(destDir, 'libmpv-2.dll');
  const destVersion = path.join(destDir, 'VERSION.txt');
  const zipPath = path.join(releasesDir, `${destDirName}.zip`);
  const hashPath = path.join(releasesDir, 'anime-player.exe.sha256');
  const looseExePath = path.join(releasesDir, 'anime-player.exe');

  console.log(`\nCreating ${destDirName}...`);

  await fs.mkdir(releasesDir, { recursive: true });
  await fs.rm(destDir, { recursive: true, force: true });
  await fs.mkdir(destDir, { recursive: true });

  console.log('Copying files...');
  await fs.copyFile(exePath, destExe);
  await fs.copyFile(dllPath, destDll);
  await fs.copyFile(updateBat, path.join(destDir, 'update.bat'));
  await fs.copyFile(updatePs1, path.join(destDir, '_update.ps1'));
  await fs.writeFile(destVersion, version, 'utf8');

  await fs.copyFile(destExe, looseExePath);

  const exeHash = await sha256File(destExe);
  const hashLine = `${exeHash}  anime-player.exe\n`;
  await fs.writeFile(hashPath, hashLine, 'utf8');

  console.log(`Creating zip archive: ${destDirName}.zip...`);
  try {
    execSync(`powershell -NoProfile -Command "Compress-Archive -Path '${destDir}\\*' -DestinationPath '${zipPath}' -Force"`, { stdio: 'inherit' });
  } catch (err) {
    console.error('Failed to create zip archive:', err.message);
  }

  console.log('\nSuccess! Your clean portable package is ready at:');
  console.log(destDir);
  console.log(`Archive created at:\n${zipPath}`);
  console.log(`\nSHA256 for GitHub upload:\n${hashPath}`);
  console.log('\nWhen publishing the GitHub release, attach:');
  console.log(`  - ${destDirName}.zip`);
  console.log('  - anime-player.exe');
  console.log('  - anime-player.exe.sha256');
}

packagePortable().catch(console.error);
