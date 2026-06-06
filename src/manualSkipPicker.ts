export const MANUAL_SKIP_HIDE_MATCHED_KEY = "animePlayer.manualSkipHideMatched";

function readStoredBoolean(key: string, defaultValue: boolean): boolean {
  try {
    const raw = localStorage.getItem(key);
    if (raw === null) return defaultValue;
    return raw === "1" || raw === "true";
  } catch {
    return defaultValue;
  }
}

function storeBoolean(key: string, value: boolean): void {
  try {
    localStorage.setItem(key, value ? "1" : "0");
  } catch {
    /* ignore */
  }
}

export function readStoredManualSkipHideMatched(): boolean {
  return readStoredBoolean(MANUAL_SKIP_HIDE_MATCHED_KEY, false);
}

export function storeManualSkipHideMatched(value: boolean): void {
  storeBoolean(MANUAL_SKIP_HIDE_MATCHED_KEY, value);
}
