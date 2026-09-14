export const GITHUB_REPO = 'SirVeggie/anime-player';
export const BINARIES_TAG = 'binaries';

/** Files the updater may replace. VERSION.txt is written from the manifest version. */
export const INSTALL_FILES = [
  'anime-player.exe',
  'libmpv-2.dll',
  'ffmpeg.exe',
  'ffprobe.exe',
  'fpcalc.exe',
  'update.bat',
  '_update.ps1',
];

export const LATEST_MANIFEST_URL =
  `https://github.com/${GITHUB_REPO}/releases/latest/download/manifest.json`;

export function hashedAssetName(fileName, sha256) {
  const hash = String(sha256).toLowerCase();
  const lastDot = fileName.lastIndexOf('.');
  if (lastDot <= 0) {
    return `${fileName}-${hash}`;
  }
  return `${fileName.slice(0, lastDot)}-${hash}${fileName.slice(lastDot)}`;
}

export function binariesDownloadUrl(fileName, sha256) {
  const asset = hashedAssetName(fileName, sha256);
  return `https://github.com/${GITHUB_REPO}/releases/download/${BINARIES_TAG}/${asset}`;
}
