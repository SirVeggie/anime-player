import fs from 'node:fs/promises';
import path from 'node:path';

async function packagePortable() {
  const root = process.cwd();
  const exePath = path.join(root, 'src-tauri', 'target', 'release', 'anime-player.exe');
  const dllPath = path.join(root, 'src-tauri', 'libs', 'mpv', 'libmpv-2.dll');
  const ffmpegDir = path.join(root, 'src-tauri', 'libs', 'ffmpeg');
  const ffmpegPaths = ['ffmpeg.exe', 'ffprobe.exe'].map((name) =>
    path.join(ffmpegDir, name),
  );
  const updateBat = path.join(root, 'scripts', 'update.bat');
  const updatePs1 = path.join(root, 'scripts', '_update.ps1');

  try {
    await fs.access(exePath);
  } catch {
    console.error('Error: anime-player.exe not found in src-tauri/target/release.');
    console.error('Run `npm run portable` to build and package.');
    process.exit(1);
  }

  try {
    await fs.access(dllPath);
  } catch {
    console.error('Error: libmpv-2.dll not found in src-tauri/libs/mpv.');
    console.error('Please run `npm run setup:mpv` first.');
    process.exit(1);
  }

  for (const toolPath of ffmpegPaths) {
    try {
      await fs.access(toolPath);
    } catch {
      console.error(`Error: ${path.basename(toolPath)} not found in src-tauri/libs/ffmpeg.`);
      console.error('Please run `npm run setup:ffmpeg` first.');
      process.exit(1);
    }
  }

  for (const scriptPath of [updateBat, updatePs1]) {
    try {
      await fs.access(scriptPath);
    } catch {
      console.error(`Error: ${path.basename(scriptPath)} not found in scripts/.`);
      process.exit(1);
    }
  }

  const destDir = path.join(root, 'releases', 'dev');

  console.log('\nCreating local dev portable package...');

  await fs.mkdir(destDir, { recursive: true });

  console.log('Copying files...');
  await fs.copyFile(exePath, path.join(destDir, 'anime-player.exe'));
  await fs.copyFile(dllPath, path.join(destDir, 'libmpv-2.dll'));
  for (const toolPath of ffmpegPaths) {
    await fs.copyFile(toolPath, path.join(destDir, path.basename(toolPath)));
  }
  await fs.copyFile(updateBat, path.join(destDir, 'update.bat'));
  await fs.copyFile(updatePs1, path.join(destDir, '_update.ps1'));

  console.log('\nSuccess! Dev portable package ready at:');
  console.log(destDir);
}

packagePortable().catch((err) => {
  console.error(err);
  process.exit(1);
});
