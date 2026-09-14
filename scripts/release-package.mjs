import fs from 'node:fs/promises';
import path from 'node:path';
import { createHash } from 'node:crypto';
import { execSync } from 'node:child_process';
import { getAndTagVersion, getHeadCommit } from './release-version.mjs';
import { INSTALL_FILES, binariesDownloadUrl } from './release-assets.mjs';

async function sha256File(filePath) {
  const data = await fs.readFile(filePath);
  return createHash('sha256').update(data).digest('hex').toLowerCase();
}

async function packageRelease() {
  const root = process.cwd();
  const cargoTargetDir = process.env.CARGO_TARGET_DIR
    ? path.resolve(process.env.CARGO_TARGET_DIR)
    : path.join(root, 'src-tauri', 'target');
  const exePath = path.join(cargoTargetDir, 'release', 'anime-player.exe');
  const dllPath = path.join(root, 'src-tauri', 'libs', 'mpv', 'libmpv-2.dll');
  const ffmpegDir = path.join(root, 'src-tauri', 'libs', 'ffmpeg');
  const ffmpegPaths = ['ffmpeg.exe', 'ffprobe.exe'].map((name) =>
    path.join(ffmpegDir, name),
  );
  const fpcalcPath = path.join(root, 'src-tauri', 'libs', 'chromaprint', 'fpcalc.exe');
  const updateBat = path.join(root, 'scripts', 'update.bat');
  const updatePs1 = path.join(root, 'scripts', '_update.ps1');

  const sourceByName = {
    'anime-player.exe': exePath,
    'libmpv-2.dll': dllPath,
    'ffmpeg.exe': ffmpegPaths[0],
    'ffprobe.exe': ffmpegPaths[1],
    'fpcalc.exe': fpcalcPath,
    'update.bat': updateBat,
    '_update.ps1': updatePs1,
  };

  for (const name of INSTALL_FILES) {
    const sourcePath = sourceByName[name];
    if (!sourcePath) {
      console.error(`Error: no source mapping for ${name}.`);
      process.exit(1);
    }
    try {
      await fs.access(sourcePath);
    } catch {
      console.error(`Error: ${name} not found at ${sourcePath}.`);
      process.exit(1);
    }
  }

  const version = getAndTagVersion();
  const releasesDir = path.join(root, 'releases');
  const destDirName = `AnimePlayer-${version}`;
  const destDir = path.join(releasesDir, destDirName);
  const destVersion = path.join(destDir, 'VERSION.txt');
  const zipPath = path.join(releasesDir, `${destDirName}.zip`);
  const manifestPath = path.join(releasesDir, 'manifest.json');
  const buildMetaPath = path.join(releasesDir, '.build-meta.json');

  console.log(`\nCreating ${destDirName}...`);

  await fs.mkdir(releasesDir, { recursive: true });
  await fs.rm(destDir, { recursive: true, force: true });
  await fs.mkdir(destDir, { recursive: true });

  console.log('Copying files...');
  for (const name of INSTALL_FILES) {
    await fs.copyFile(sourceByName[name], path.join(destDir, name));
  }
  await fs.writeFile(destVersion, version, 'utf8');

  const files = [];
  for (const name of INSTALL_FILES) {
    const destFile = path.join(destDir, name);
    const stat = await fs.stat(destFile);
    const sha256 = await sha256File(destFile);
    files.push({
      name,
      sha256,
      size: stat.size,
      url: binariesDownloadUrl(name, sha256),
      localPath: path.join(destDirName, name).replaceAll('\\', '/'),
    });
  }

  const manifest = {
    schema: 1,
    version,
    notes: '',
    files: files.map(({ name, sha256, size, url }) => ({ name, sha256, size, url })),
  };
  await fs.writeFile(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`, 'utf8');

  console.log(`Creating zip archive: ${destDirName}.zip...`);
  try {
    execSync(`powershell -NoProfile -Command "Compress-Archive -Path '${destDir}\\*' -DestinationPath '${zipPath}' -Force"`, { stdio: 'inherit' });
  } catch (err) {
    console.error('Failed to create zip archive:', err.message);
    process.exit(1);
  }

  const commit = getHeadCommit();
  await fs.writeFile(
    buildMetaPath,
    JSON.stringify({ tag: version, commit, files }, null, 2) + '\n',
    'utf8',
  );

  console.log('\nSuccess! Your clean portable package is ready at:');
  console.log(destDir);
  console.log(`Archive created at:\n${zipPath}`);
  console.log(`Updater manifest:\n${manifestPath}`);
  console.log('\nWhen publishing the GitHub release, attach:');
  console.log(`  - ${destDirName}.zip`);
  console.log('  - manifest.json');
}

packageRelease().catch((err) => {
  console.error(err);
  process.exit(1);
});
