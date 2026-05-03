import fs from 'node:fs/promises';
import path from 'node:path';

async function packagePortable() {
  const root = process.cwd();
  const exePath = path.join(root, 'src-tauri', 'target', 'release', 'anime-player.exe');
  const dllPath = path.join(root, 'src-tauri', 'libs', 'mpv', 'libmpv-2.dll');
  
  const destDir = path.join(root, 'AnimePlayer-Portable');
  const destExe = path.join(destDir, 'anime-player.exe');
  const destDll = path.join(destDir, 'libmpv-2.dll');

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

  console.log('Creating AnimePlayer-Portable...');
  
  // Create or clear the directory
  await fs.rm(destDir, { recursive: true, force: true });
  await fs.mkdir(destDir, { recursive: true });

  console.log('Copying files...');
  await fs.copyFile(exePath, destExe);
  await fs.copyFile(dllPath, destDll);

  console.log('\nSuccess! Your clean portable package is ready at:');
  console.log(destDir);
  console.log('\nYou can now zip this folder and share it with your friends!');
}

packagePortable().catch(console.error);
