import { useEffect, useMemo, useRef } from "react";
import { matchingAnimeIds, parseSearchQuery } from "../search";
import type { AnimeSearchEntry, AnimeSummary } from "../types";
import { AnimeCardGrid } from "./AnimeGrid";
import { ViewHeader } from "./ViewHeader";

export function SearchScreen(props: {
  anime: AnimeSummary[];
  searchIndex: AnimeSearchEntry[];
  preferAnilistDisplayTitle: boolean;
  query: string;
  focusToken: number;
  onQueryChange: (query: string) => void;
  onOpenAnime: (anime: AnimeSummary) => void;
}) {
  const { anime, searchIndex, preferAnilistDisplayTitle, query, focusToken, onQueryChange, onOpenAnime } = props;
  const inputRef = useRef<HTMLInputElement | null>(null);
  const searchBranches = useMemo(() => parseSearchQuery(query), [query]);
  const matchingAnime = useMemo(() => {
    if (searchBranches.length === 0) return [];
    const ids = matchingAnimeIds(searchIndex, query);
    return anime.filter((item) => ids.has(item.id));
  }, [anime, query, searchIndex]);

  useEffect(() => {
    const input = inputRef.current;
    if (!input) return;
    input.focus();
    input.select();
  }, [focusToken]);

  const subtitle = searchBranches.length
    ? `${matchingAnime.length} matching title${matchingAnime.length === 1 ? "" : "s"}.`
    : "Search titles, AniList names, or episode file names.";

  return (
    <>
      <ViewHeader title="Search" subtitle={subtitle} />
      <form className="search-panel" onSubmit={(event) => event.preventDefault()}>
        <label className="stacked-field">
          <span>Query</span>
          <input
            ref={inputRef}
            type="text"
            value={query}
            onChange={(event) => onQueryChange(event.currentTarget.value)}
            placeholder="Words match anywhere; use | or OR between alternatives"
            aria-label="Search library"
          />
        </label>
      </form>

      {searchBranches.length === 0 ? (
        <div className="empty empty--wide">
          <h2>Start typing to search</h2>
          <p className="muted">Press Ctrl+F anytime to return here. Separate alternatives with | or OR.</p>
        </div>
      ) : matchingAnime.length === 0 ? (
        <div className="empty empty--wide">
          <h2>No matches found</h2>
          <p className="muted">Try other words, a filename fragment, or an alternate OR branch.</p>
        </div>
      ) : (
        <AnimeCardGrid
          anime={matchingAnime}
          preferAnilistDisplayTitle={preferAnilistDisplayTitle}
          onOpenAnime={onOpenAnime}
        />
      )}
    </>
  );
}
