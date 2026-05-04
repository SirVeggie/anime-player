import fs from 'node:fs/promises';
import path from 'node:path';
import { execSync } from 'node:child_process';

const SUPPORTED_EXTENSIONS = new Set(['.png', '.svg']);

function quoteForShell(value) {
  return `"${value.replace(/"/g, '\\"')}"`;
}

async function removePreviousSourceIcons(iconsDir) {
  const entries = await fs.readdir(iconsDir, { withFileTypes: true });
  await Promise.all(
    entries
      .filter((entry) => entry.isFile() && entry.name.startsWith('app-icon-source.'))
      .map((entry) => fs.rm(path.join(iconsDir, entry.name), { force: true })),
  );
}

async function setupIcon() {
  const input = process.argv[2];
  if (!input) {
    console.error('Usage: npm run icon:setup -- <path-to-square-icon.svg-or-png>');
    process.exit(1);
  }

  const root = process.cwd();
  const inputPath = path.resolve(root, input);
  const ext = path.extname(inputPath).toLowerCase();
  if (!SUPPORTED_EXTENSIONS.has(ext)) {
    console.error(`Unsupported icon type "${ext}". Use a square .svg or .png file.`);
    process.exit(1);
  }

  let sourceBytes;
  try {
    sourceBytes = await fs.readFile(inputPath);
  } catch {
    console.error(`Icon file not found: ${inputPath}`);
    process.exit(1);
  }

  console.log(`Generating Tauri icons from ${inputPath}...`);
  execSync(`npm run tauri -- icon ${quoteForShell(inputPath)}`, { stdio: 'inherit' });

  const iconsDir = path.join(root, 'src-tauri', 'icons');
  await removePreviousSourceIcons(iconsDir);

  const sourceDest = path.join(iconsDir, `app-icon-source${ext}`);
  await fs.writeFile(sourceDest, sourceBytes);

  console.log('\nIcon setup complete.');
  console.log(`Source icon saved at: ${sourceDest}`);
  console.log('Run `npm run tauri build` to embed the icon into the Windows executable.');
}

setupIcon().catch((err) => {
  console.error(err);
  process.exit(1);
});
