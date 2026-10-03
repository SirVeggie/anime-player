import type { AnimeSummary } from "./types";

export const GRID_FILTER_STORAGE_KEY = "animePlayer.animeGridFilter";
export const SEARCH_GRID_FILTER_STORAGE_KEY = "animePlayer.searchGridFilter";

export const GRID_FILTER_OPTIONS = [
  { value: 0, label: "All" },
  { value: 1, label: "Remaining" },
  { value: 2, label: "Completed" },
  { value: 3, label: "Missing" },
] as const;

const FILTER_VALUE_MAX = GRID_FILTER_OPTIONS.length - 1;

function readStoredFilter(storageKey: string): number {
  try {
    const raw = localStorage.getItem(storageKey);
    if (raw === null) return 0;
    const n = Number.parseInt(raw, 10);
    if (!Number.isFinite(n) || n < 0 || n > FILTER_VALUE_MAX) return 0;
    return n;
  } catch {
    return 0;
  }
}

function storeFilter(storageKey: string, value: number): void {
  try {
    localStorage.setItem(storageKey, String(value));
  } catch {
    /* ignore */
  }
}

export function readStoredGridFilter(): number {
  return readStoredFilter(GRID_FILTER_STORAGE_KEY);
}

export function storeGridFilter(value: number): void {
  storeFilter(GRID_FILTER_STORAGE_KEY, value);
}

export function readStoredSearchGridFilter(): number {
  return readStoredFilter(SEARCH_GRID_FILTER_STORAGE_KEY);
}

export function storeSearchGridFilter(value: number): void {
  storeFilter(SEARCH_GRID_FILTER_STORAGE_KEY, value);
}

export function animeMatchesGridFilter(anime: AnimeSummary, filterValue: number): boolean {
  switch (filterValue) {
    case 1:
      return anime.unwatched_count > 0;
    case 2:
      return anime.unwatched_count === 0;
    case 3:
      return anime.gap_episode_count > 0;
    default:
      return true;
  }
}

export function filterAnimeForGrid(anime: AnimeSummary[], filterValue: number): AnimeSummary[] {
  if (filterValue === 0) return anime;
  return anime.filter((item) => animeMatchesGridFilter(item, filterValue));
}
