import { useEffect, useMemo, useRef } from "react";
import type { AnimeSummary } from "../types";
import { AnimeCardGrid } from "./AnimeGrid";
import { ViewHeader } from "./ViewHeader";

function normalizeSearchText(value: string) {
  return value.trim().toLowerCase();
}

function matchesAnime(anime: AnimeSummary, query: string) {
  const normalizedTitle = anime.title.toLowerCase();
  const normalizedAnilistTitle = anime.anilist_title?.toLowerCase() ?? "";
  return normalizedTitle.includes(query) || normalizedAnilistTitle.includes(query);
}

export function SearchScreen(props: {
  anime: AnimeSummary[];
  preferAnilistDisplayTitle: boolean;
  query: string;
  focusToken: number;
  onQueryChange: (query: string) => void;
  onOpenAnime: (anime: AnimeSummary) => void;
}) {
  const { anime, preferAnilistDisplayTitle, query, focusToken, onQueryChange, onOpenAnime } = props;
  const inputRef = useRef<HTMLInputElement | null>(null);
  const normalizedQuery = normalizeSearchText(query);
  const matchingAnime = useMemo(() => {
    if (!normalizedQuery) return [];
    return anime.filter((item) => matchesAnime(item, normalizedQuery));
  }, [anime, normalizedQuery]);

  useEffect(() => {
    const input = inputRef.current;
    if (!input) return;
    input.focus();
    input.select();
  }, [focusToken]);

  const subtitle = normalizedQuery
    ? `${matchingAnime.length} matching title${matchingAnime.length === 1 ? "" : "s"}.`
    : "Search your local library.";

  return (
    <>
      <ViewHeader title="Search" subtitle={subtitle} />
      <form className="search-panel" onSubmit={(event) => event.preventDefault()}>
        <label className="stacked-field">
          <span>Title</span>
          <input
            ref={inputRef}
            type="text"
            value={query}
            onChange={(event) => onQueryChange(event.currentTarget.value)}
            placeholder="Search titles..."
            aria-label="Search titles"
          />
        </label>
      </form>

      {!normalizedQuery ? (
        <div className="empty empty--wide">
          <h2>Start typing to search</h2>
          <p className="muted">Press Ctrl+F anytime to return here and keep searching.</p>
        </div>
      ) : matchingAnime.length === 0 ? (
        <div className="empty empty--wide">
          <h2>No matches found</h2>
          <p className="muted">Try a different title or AniList name.</p>
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
