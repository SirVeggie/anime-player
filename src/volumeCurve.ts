/** mpv already applies a logarithmic volume scale internally,
 *  so we use its native 0–130 range directly. */
export const MAX_VOLUME = 130;

export const HOTKEY_STEP = 5;

export function clampVolume(volume: number): number {
  return Math.max(0, Math.min(MAX_VOLUME, Math.round(volume)));
}
