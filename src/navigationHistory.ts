/** In-app session history for Back / Forward navigation.
 *
 * The stack lives only in memory. `window.history` is deliberately left
 * untouched: WebView2 performs its own native history step for BrowserBack /
 * mouse X1 even after `preventDefault`, so mirroring entries there made one
 * gesture walk more than one step. With a single native entry that native
 * step is a no-op. */

export type AppView =
  | "categories"
  | "anime"
  | "search"
  | "bulkEdit"
  | "missing"
  | "episodes"
  | "manualSkip"
  | "jobs"
  | "settings"
  | "player";

export type EpisodeReturnView = "anime" | "search" | "bulkEdit" | "categories";

/**
 * - `push`: new entry after the current one (drops forward entries).
 * - `replace`: overwrite the current entry (forced leaves). Collapses into the
 *   previous entry when that is the same page, so no duplicate slot remains.
 * - `back`: a leave path (player Q / back arrow, manual skip exit). Steps back
 *   onto the previous entry when it is the target page, keeping forward
 *   entries; otherwise behaves like `replace`.
 * - `none`: a Back / Forward restore already moved the index; only refresh it.
 */
export type HistoryMode = "push" | "replace" | "back" | "none";

export type NavSnapshot = {
  view: AppView;
  selectedCategoryId: number | null;
  selectedAnimeId: number | null;
  selectedEpisodeId: number | null;
  episodeReturnView: EpisodeReturnView;
  manualSkipAnimeId: number | null;
};

export type NavHistory = {
  entries: NavSnapshot[];
  index: number;
};

export function pageKeyFromSnapshot(snapshot: NavSnapshot): string {
  switch (snapshot.view) {
    case "anime":
      return `anime:${snapshot.selectedCategoryId ?? "none"}`;
    case "episodes":
      return `episodes:${snapshot.selectedAnimeId ?? "none"}:${snapshot.episodeReturnView}`;
    case "player":
      return `player:${snapshot.selectedEpisodeId ?? "none"}`;
    case "manualSkip":
      return `manualSkip:${snapshot.manualSkipAnimeId ?? "none"}`;
    default:
      return snapshot.view;
  }
}

/** Record `snapshot` as the newly shown view. Mutates `history`. */
export function recordNavigation(history: NavHistory, snapshot: NavSnapshot, mode: HistoryMode): void {
  const { entries } = history;
  const current = entries[history.index];
  if (!current) {
    history.entries = [snapshot];
    history.index = 0;
    return;
  }

  const key = pageKeyFromSnapshot(snapshot);
  // Episode switches inside the player (next/prev, EOF advance) are one session.
  const samePlayerSession = snapshot.view === "player" && current.view === "player";
  if (mode === "none" || key === pageKeyFromSnapshot(current) || samePlayerSession) {
    entries[history.index] = snapshot;
    return;
  }

  const previous = entries[history.index - 1];
  const previousMatches = previous != null && pageKeyFromSnapshot(previous) === key;
  if (mode === "back" && previousMatches) {
    history.index -= 1;
    entries[history.index] = snapshot;
    return;
  }
  if (mode === "back" || mode === "replace") {
    if (previousMatches) {
      entries.splice(history.index, 1);
      history.index -= 1;
    }
    entries[history.index] = snapshot;
    return;
  }

  history.entries = entries.slice(0, history.index + 1);
  history.entries.push(snapshot);
  history.index += 1;
}

/** Collapse a BrowserBack key and X1 mouse event delivered for one press. */
export const HISTORY_GESTURE_MS = 150;

export function isBrowserBackKey(event: KeyboardEvent): boolean {
  return event.key === "BrowserBack" || event.code === "BrowserBack";
}

export function isBrowserForwardKey(event: KeyboardEvent): boolean {
  return event.key === "BrowserForward" || event.code === "BrowserForward";
}

/** Mouse X1 — the usual "browser back" side button (`button === 3`). */
export function isBrowserBackButton(event: MouseEvent): boolean {
  return event.button === 3;
}

/** Mouse X2 — the usual "browser forward" side button (`button === 4`). */
export function isBrowserForwardButton(event: MouseEvent): boolean {
  return event.button === 4;
}
