import type { AnimeSearchEntry } from "./types";

export type SearchOptions = {
  includeFilenames: boolean;
  useRegex: boolean;
};

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

function searchableFields(entry: AnimeSearchEntry, includeFilenames: boolean): string[] {
  const fields: string[] = [entry.title];
  if (entry.anilist_title) fields.push(entry.anilist_title);
  if (includeFilenames) fields.push(...entry.file_names);
  return fields;
}

function termMatchesEntry(entry: AnimeSearchEntry, term: string, options: SearchOptions): boolean {
  return searchableFields(entry, options.includeFilenames).some((field) => field.toLowerCase().includes(term));
}

function regexMatchesEntry(entry: AnimeSearchEntry, pattern: RegExp, options: SearchOptions): boolean {
  return searchableFields(entry, options.includeFilenames).some((field) => pattern.test(field));
}

function branchMatchesEntry(entry: AnimeSearchEntry, terms: string[], options: SearchOptions): boolean {
  return terms.every((term) => termMatchesEntry(entry, term, options));
}

export function hasActiveSearch(query: string, options: SearchOptions): boolean {
  const trimmed = query.trim();
  if (!trimmed) return false;
  if (options.useRegex) return true;
  return parseSearchQuery(query).length > 0;
}

export function isValidSearchRegex(query: string): boolean {
  try {
    new RegExp(query.trim(), "i");
    return true;
  } catch {
    return false;
  }
}

export function matchingAnimeIds(
  index: AnimeSearchEntry[],
  query: string,
  options: SearchOptions,
): Set<number> {
  const trimmed = query.trim();
  if (!trimmed) return new Set();

  if (options.useRegex) {
    let pattern: RegExp;
    try {
      pattern = new RegExp(trimmed, "i");
    } catch {
      return new Set();
    }

    const ids = new Set<number>();
    for (const entry of index) {
      if (regexMatchesEntry(entry, pattern, options)) {
        ids.add(entry.id);
      }
    }
    return ids;
  }

  const branches = parseSearchQuery(query);
  if (branches.length === 0) return new Set();

  const ids = new Set<number>();
  for (const entry of index) {
    if (branches.some((terms) => branchMatchesEntry(entry, terms, options))) {
      ids.add(entry.id);
    }
  }
  return ids;
}
