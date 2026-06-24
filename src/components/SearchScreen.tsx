import { useEffect, useMemo, useRef, useState } from "react";
import {
  GRID_SORT_OPTIONS,
  gridSortLabel,
  readStoredSearchGridSort,
  sortAnimeForGrid,
  storeSearchGridSort,
} from "../animeGridSort";
import {
  hasActiveSearch,
  isValidSearchRegex,
  matchingAnimeIds,
  readStoredSearchIncludeFilenames,
  readStoredSearchUseRegex,
  storeSearchIncludeFilenames,
  storeSearchUseRegex,
} from "../search";
import type { AnimeSearchEntry, AnimeSummary } from "../types";
import { AnimeCardGrid, type AnimeContextMenuHandlers } from "./AnimeGrid";
import { CustomCheckbox } from "./CustomCheckbox";
import { CustomDropdown } from "./CustomDropdown";
import { ViewHeader } from "./ViewHeader";

export function SearchScreen(props: {
  anime: AnimeSummary[];
  searchIndex: AnimeSearchEntry[];
  preferAnilistDisplayTitle: boolean;
  query: string;
  focusToken: number;
  onQueryChange: (query: string) => void;
  onOpenAnime: (anime: AnimeSummary) => void;
  contextMenu: AnimeContextMenuHandlers;
}) {
  const {
    anime,
    searchIndex,
    preferAnilistDisplayTitle,
    query,
    focusToken,
    onQueryChange,
    onOpenAnime,
    contextMenu,
  } = props;
  const inputRef = useRef<HTMLInputElement | null>(null);
  const [includeFilenames, setIncludeFilenames] = useState(readStoredSearchIncludeFilenames);
  const [useRegex, setUseRegex] = useState(readStoredSearchUseRegex);
  const [sortValue, setSortValue] = useState(readStoredSearchGridSort);

  const searchOptions = useMemo(
    () => ({ includeFilenames, useRegex }),
    [includeFilenames, useRegex],
  );

  const activeSearch = hasActiveSearch(query, searchOptions);
  const regexInvalid = useRegex && activeSearch && !isValidSearchRegex(query);

  const matchingAnime = useMemo(() => {
    if (!activeSearch || regexInvalid) return [];
    const ids = matchingAnimeIds(searchIndex, query, searchOptions);
    return anime.filter((item) => ids.has(item.id));
  }, [activeSearch, anime, query, regexInvalid, searchIndex, searchOptions]);

  const sortedMatchingAnime = useMemo(
    () => sortAnimeForGrid(matchingAnime, sortValue, preferAnilistDisplayTitle),
    [matchingAnime, preferAnilistDisplayTitle, sortValue],
  );

  const sortLabel = gridSortLabel(sortValue);

  const handleSortChange = (value: number) => {
    setSortValue(value);
    storeSearchGridSort(value);
  };

  const clearQuery = () => {
    onQueryChange("");
    inputRef.current?.focus();
  };

  useEffect(() => {
    const input = inputRef.current;
    if (!input) return;
    input.focus();
    input.select();
  }, [focusToken]);

  const subtitle = !activeSearch
    ? useRegex
      ? "Search titles and AniList names with a regular expression."
      : includeFilenames
        ? "Search titles, AniList names, or episode file names."
        : "Search titles and AniList names."
    : regexInvalid
      ? "Invalid regular expression."
      : `${matchingAnime.length} matching title${matchingAnime.length === 1 ? "" : "s"}.`;

  const placeholder = useRegex
    ? "Regular expression (case-insensitive)"
    : "Words match anywhere; use | or OR between alternatives";

  return (
    <>
      <ViewHeader title="Search" subtitle={subtitle} />
      <form className="search-panel" onSubmit={(event) => event.preventDefault()}>
        <div className="search-panel__options">
          <div className="search-panel__options-left">
            <CustomCheckbox
              checked={includeFilenames}
              onChange={(checked) => {
                setIncludeFilenames(checked);
                storeSearchIncludeFilenames(checked);
              }}
              label="Include filenames"
            />
            <CustomCheckbox
              checked={useRegex}
              onChange={(checked) => {
                setUseRegex(checked);
                storeSearchUseRegex(checked);
              }}
              label="Regular expression"
            />
          </div>
          <div className="search-panel__sort">
            <CustomDropdown
              label={`Sort: ${sortLabel}`}
              options={[...GRID_SORT_OPTIONS]}
              value={sortValue}
              onChange={handleSortChange}
            />
          </div>
        </div>
        <label className="stacked-field">
          <span>Query</span>
          <div className="search-panel__query">
            <input
              ref={inputRef}
              type="text"
              value={query}
              onChange={(event) => onQueryChange(event.currentTarget.value)}
              onKeyDown={(event) => {
                if (event.key !== "Escape" || !query.trim()) return;
                event.preventDefault();
                clearQuery();
              }}
              placeholder={placeholder}
              aria-label="Search library"
            />
            {query ? (
              <button
                type="button"
                className="search-panel__clear"
                onClick={clearQuery}
                aria-label="Clear search"
                title="Clear search"
              >
                ×
              </button>
            ) : null}
          </div>
        </label>
      </form>

      {!activeSearch ? (
        <div className="empty empty--wide">
          <h2>Start typing to search</h2>
          <p className="muted">
            Press Ctrl+F anytime to return here. Esc clears the query.
            {useRegex
              ? " Regular expression mode matches enabled fields."
              : includeFilenames
                ? " Separate alternatives with | or OR; filenames are included."
                : " Separate alternatives with | or OR."}
          </p>
        </div>
      ) : regexInvalid ? (
        <div className="empty empty--wide">
          <h2>Invalid regular expression</h2>
          <p className="muted">Check the pattern syntax and try again.</p>
        </div>
      ) : matchingAnime.length === 0 ? (
        <div className="empty empty--wide">
          <h2>No matches found</h2>
          <p className="muted">
            {useRegex
              ? "Try a different pattern or turn off regular expression mode."
              : "Try other words, a filename fragment, or an alternate OR branch."}
          </p>
        </div>
      ) : (
        <AnimeCardGrid
          anime={sortedMatchingAnime}
          preferAnilistDisplayTitle={preferAnilistDisplayTitle}
          onOpenAnime={onOpenAnime}
          contextMenu={contextMenu}
        />
      )}
    </>
  );
}
