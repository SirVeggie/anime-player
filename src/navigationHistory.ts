/** Session-history snapshots for BrowserBack / BrowserForward navigation. */

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

export type HistoryMode = "push" | "replace" | "none";

export type NavSnapshot = {
  v: 1;
  view: AppView;
  selectedCategoryId: number | null;
  selectedAnimeId: number | null;
  selectedEpisodeId: number | null;
  episodeReturnView: EpisodeReturnView;
  manualSkipAnimeId: number | null;
};

export function isNavSnapshot(value: unknown): value is NavSnapshot {
  if (value == null || typeof value !== "object") return false;
  const snap = value as Partial<NavSnapshot>;
  return (
    snap.v === 1 &&
    typeof snap.view === "string" &&
    (snap.selectedCategoryId === null || typeof snap.selectedCategoryId === "number") &&
    (snap.selectedAnimeId === null || typeof snap.selectedAnimeId === "number") &&
    (snap.selectedEpisodeId === null || typeof snap.selectedEpisodeId === "number") &&
    (snap.episodeReturnView === "anime" ||
      snap.episodeReturnView === "search" ||
      snap.episodeReturnView === "bulkEdit" ||
      snap.episodeReturnView === "categories") &&
    (snap.manualSkipAnimeId === null || typeof snap.manualSkipAnimeId === "number")
  );
}

export function navSnapshotsEqual(a: NavSnapshot, b: NavSnapshot): boolean {
  return (
    a.v === b.v &&
    a.view === b.view &&
    a.selectedCategoryId === b.selectedCategoryId &&
    a.selectedAnimeId === b.selectedAnimeId &&
    a.selectedEpisodeId === b.selectedEpisodeId &&
    a.episodeReturnView === b.episodeReturnView &&
    a.manualSkipAnimeId === b.manualSkipAnimeId
  );
}

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
