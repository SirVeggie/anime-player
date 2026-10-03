import type { AnimeSearchEntry, AnimeSummary } from "./types";

export const GRID_FILTER_STORAGE_KEY = "animePlayer.animeGridFilter";
export const SEARCH_GRID_FILTER_STORAGE_KEY = "animePlayer.searchGridFilter";
export const GRID_TEXT_FILTER_STORAGE_KEY = "animePlayer.animeGridTextFilter";

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

export function readStoredGridTextFilter(): string {
  try {
    return sessionStorage.getItem(GRID_TEXT_FILTER_STORAGE_KEY) ?? "";
  } catch {
    return "";
  }
}

export function storeGridTextFilter(value: string): void {
  try {
    if (value) sessionStorage.setItem(GRID_TEXT_FILTER_STORAGE_KEY, value);
    else sessionStorage.removeItem(GRID_TEXT_FILTER_STORAGE_KEY);
  } catch {
    /* ignore */
  }
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

function fieldContains(value: string | null | undefined, needle: string): boolean {
  return Boolean(value && value.toLowerCase().includes(needle));
}

export function animeMatchesTextFilter(
  anime: AnimeSummary,
  query: string,
  searchEntry?: AnimeSearchEntry | null,
): boolean {
  const needle = query.trim().toLowerCase();
  if (!needle) return true;
  if (fieldContains(anime.title, needle) || fieldContains(anime.anilist_title, needle)) return true;
  if (!searchEntry) return false;
  if (fieldContains(searchEntry.title, needle) || fieldContains(searchEntry.anilist_title, needle)) {
    return true;
  }
  return searchEntry.file_names.some((name) => fieldContains(name, needle));
}

export function filterAnimeByText(
  anime: AnimeSummary[],
  query: string,
  searchIndex: AnimeSearchEntry[],
): AnimeSummary[] {
  const needle = query.trim();
  if (!needle) return anime;
  const byId = new Map(searchIndex.map((entry) => [entry.id, entry]));
  return anime.filter((item) => animeMatchesTextFilter(item, query, byId.get(item.id)));
}
