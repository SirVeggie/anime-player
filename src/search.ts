import type { AnimeSearchEntry } from "./types";

/** Split the query into OR branches; each branch is a list of AND terms (lowercased). */
export function parseSearchQuery(query: string): string[][] {
  const trimmed = query.trim();
  if (!trimmed) return [];

  return trimmed
    .split(/\s*\|\|\s*|\s*\|\s*|\s+OR\s+/i)
    .map((branch) =>
      branch
        .trim()
        .split(/\s+/)
        .filter(Boolean)
        .map((term) => term.toLowerCase()),
    )
    .filter((terms) => terms.length > 0);
}

function termMatchesEntry(entry: AnimeSearchEntry, term: string): boolean {
  if (entry.title.toLowerCase().includes(term)) return true;
  if (entry.anilist_title?.toLowerCase().includes(term)) return true;
  return entry.file_names.some((fileName) => fileName.toLowerCase().includes(term));
}

function branchMatchesEntry(entry: AnimeSearchEntry, terms: string[]): boolean {
  return terms.every((term) => termMatchesEntry(entry, term));
}

export function entryMatchesSearch(entry: AnimeSearchEntry, query: string): boolean {
  const branches = parseSearchQuery(query);
  if (branches.length === 0) return false;
  return branches.some((terms) => branchMatchesEntry(entry, terms));
}

export function matchingAnimeIds(index: AnimeSearchEntry[], query: string): Set<number> {
  const branches = parseSearchQuery(query);
  if (branches.length === 0) return new Set();

  const ids = new Set<number>();
  for (const entry of index) {
    if (branches.some((terms) => branchMatchesEntry(entry, terms))) {
      ids.add(entry.id);
    }
  }
  return ids;
}
