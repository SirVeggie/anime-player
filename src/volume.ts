/** mpv already applies a logarithmic volume scale internally,
 *  so we use its native 0–130 range directly. */
export const MAX_VOLUME = 130;

export const HOTKEY_STEP = 2;

const STORAGE_KEY = "animePlayer.volume";
const DEFAULT_VOLUME = 100;

export function clampVolume(volume: number): number {
  return Math.max(0, Math.min(MAX_VOLUME, Math.round(volume)));
}

export function loadVolume(): number {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw === null) return DEFAULT_VOLUME;
    const n = Number(raw);
    return Number.isFinite(n) ? clampVolume(n) : DEFAULT_VOLUME;
  } catch {
    return DEFAULT_VOLUME;
  }
}

export function saveVolume(volume: number): void {
  try {
    localStorage.setItem(STORAGE_KEY, String(volume));
  } catch {
    /* ignore */
  }
}
