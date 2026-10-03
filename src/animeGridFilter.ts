import type { AnimeSearchEntry, AnimeSummary } from "./types";

export const GRID_TEXT_FILTER_STORAGE_KEY = "animePlayer.animeGridTextFilter";

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
