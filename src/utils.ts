export function formatSize(bytes: number): string {
  if (bytes <= 0) return "";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value.toFixed(value >= 100 || unit === 0 ? 0 : 1)} ${units[unit]}`;
}

export function formatTime(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds < 0) return "0:00";
  const total = Math.floor(seconds);
  const s = total % 60;
  const m = Math.floor(total / 60) % 60;
  const h = Math.floor(total / 3600);
  const ss = s.toString().padStart(2, "0");
  if (h > 0) {
    const mm = m.toString().padStart(2, "0");
    return `${h}:${mm}:${ss}`;
  }
  return `${m}:${ss}`;
}

export function formatEpisodeNumber(value: number | null): string {
  if (value === null || !Number.isFinite(value)) return "Episode ?";
  return Number.isInteger(value) ? `Episode ${value}` : `Episode ${value.toFixed(1)}`;
}

export function progressPercent(position: number, duration: number): number {
  if (duration <= 0) return 0;
  return Math.min(100, Math.max(0, (position / duration) * 100));
}

export function errorMessage(error: unknown): string {
  return typeof error === "string" ? error : String(error);
}

/** Shown in the native window title (taskbar / Alt-Tab); keep OS limits in mind. */
export function shortenForOsTitle(text: string, maxChars = 42): string {
  const t = text.trim().replace(/\s+/g, " ");
  if (!t) return "Anime";
  if (t.length <= maxChars) return t;
  return `${t.slice(0, Math.max(1, maxChars - 1))}…`;
}

export const APP_WINDOW_TITLE = "Anime Player";

/** True when the event target is typing in a field that should keep window shortcuts from firing. */
export function isTextInputTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  const tag = target.tagName;
  if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") return true;
  return target.isContentEditable;
}
