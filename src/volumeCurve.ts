export const STEP_COUNT = 50;

const CURVE = 0.005;
const STEP_SIZE = (1 / CURVE) ** (1 / STEP_COUNT);

/** Convert a step (0–STEP_COUNT) to mpv volume (0–100). */
export function stepsToVolume(steps: number): number {
  const clamped = Math.max(0, Math.min(STEP_COUNT, steps));
  const ratio = STEP_SIZE ** (clamped - STEP_COUNT);
  return ((ratio - CURVE) / (1 - CURVE)) * 100;
}

/** Convert mpv volume (0–100) back to the nearest step. */
export function volumeToSteps(volume: number): number {
  const v = Math.max(0, Math.min(100, volume)) / 100;
  const mapped = v * (1 - CURVE) + CURVE;
  if (mapped <= 0) return 0;
  const raw = Math.log(mapped) / Math.log(STEP_SIZE) + STEP_COUNT;
  return Math.round(Math.max(0, Math.min(STEP_COUNT, raw)));
}

export function clampSteps(steps: number): number {
  return Math.max(0, Math.min(STEP_COUNT, steps));
}
