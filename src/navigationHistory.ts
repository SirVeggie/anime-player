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
  /** Position in the in-app session stack. 0 is the first entry (usually home). */
  index?: number;
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
    (snap.manualSkipAnimeId === null || typeof snap.manualSkipAnimeId === "number") &&
    (snap.index === undefined || typeof snap.index === "number")
  );
}

export function attachHistoryIndex(snapshot: NavSnapshot, index: number): NavSnapshot {
  return { ...snapshot, index };
}

export function snapshotHistoryIndex(snapshot: unknown): number {
  if (isNavSnapshot(snapshot) && typeof snapshot.index === "number") {
    return snapshot.index;
  }
  return 0;
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
    case "missing":
      return "missing";
    case "bulkEdit":
      return "bulkEdit";
    case "jobs":
      return "jobs";
    default:
      return snapshot.view;
  }
}

/** Distinct URLs so WebView2 keeps one history slot per view instead of coalescing. */
export function historyUrlForPageKey(pageKey: string): string {
  return `#${encodeURIComponent(pageKey)}`;
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
