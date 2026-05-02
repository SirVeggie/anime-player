import { useEffect, useMemo, useState } from "react";
import type { AnimeSummary, Category, Episode, LibraryState } from "../types";
import { AnimeCardGrid } from "./AnimeGrid";
import { CustomDropdown } from "./CustomDropdown";
import { ViewHeader } from "./ViewHeader";

const ALL_CATEGORIES_ID = 0;
const REGEX_DEBOUNCE_MS = 2000;

function categoryName(categories: Category[], categoryId: number): string {
  return categories.find((category) => category.id === categoryId)?.name ?? "Unknown";
}

export function BulkEditScreen(props: {
  library: LibraryState;
  busy: boolean;
  onOpenAnime: (anime: AnimeSummary) => void;
  onListEpisodes: (animeId: number) => Promise<Episode[]>;
  onMoveAnime: (animeIds: number[], categoryId: number) => void;
}) {
  const { library, busy, onOpenAnime, onListEpisodes, onMoveAnime } = props;
  const [sourceCategoryId, setSourceCategoryId] = useState(ALL_CATEGORIES_ID);
  const [regexInput, setRegexInput] = useState("");
  const [debouncedRegexInput, setDebouncedRegexInput] = useState("");
  const [targetCategoryId, setTargetCategoryId] = useState<number>(library.categories[0]?.id ?? 0);
  const [matchingAnimeIds, setMatchingAnimeIds] = useState<Set<number>>(() => new Set());
  const [matchingPaths, setMatchingPaths] = useState(false);
  const [regexError, setRegexError] = useState<string | null>(null);
  const [pathError, setPathError] = useState<string | null>(null);

  useEffect(() => {
    if (library.categories.some((category) => category.id === targetCategoryId)) return;
    setTargetCategoryId(library.categories[0]?.id ?? 0);
  }, [library.categories, targetCategoryId]);

  useEffect(() => {
    const timeoutId = window.setTimeout(() => {
      setDebouncedRegexInput(regexInput);
    }, REGEX_DEBOUNCE_MS);

    return () => window.clearTimeout(timeoutId);
  }, [regexInput]);

  const candidateAnime = useMemo(() => {
    if (sourceCategoryId === ALL_CATEGORIES_ID) return library.anime;
    return library.anime.filter((anime) => anime.category_id === sourceCategoryId);
  }, [library.anime, sourceCategoryId]);

  useEffect(() => {
    const trimmedRegex = debouncedRegexInput.trim();
    setPathError(null);

    if (!trimmedRegex) {
      setRegexError(null);
      setMatchingPaths(false);
      // "All categories" with no regex = no scope; require a category filter or a regex to select anime.
      setMatchingAnimeIds(
        sourceCategoryId === ALL_CATEGORIES_ID ? new Set() : new Set(candidateAnime.map((anime) => anime.id)),
      );
      return;
    }

    let regex: RegExp;
    try {
      regex = new RegExp(trimmedRegex, "i");
    } catch (error) {
      setRegexError(error instanceof Error ? error.message : "Invalid regex.");
      setMatchingPaths(false);
      setMatchingAnimeIds(new Set());
      return;
    }

    let cancelled = false;
    setRegexError(null);
    setMatchingPaths(true);

    void (async () => {
      try {
        const results = await Promise.all(
          candidateAnime.map(async (anime) => {
            const episodes = await onListEpisodes(anime.id);
            return {
              animeId: anime.id,
              matches: episodes.some((episode) => regex.test(episode.path)),
            };
          }),
        );

        if (cancelled) return;
        setMatchingAnimeIds(new Set(results.filter((result) => result.matches).map((result) => result.animeId)));
      } catch (error) {
        if (cancelled) return;
        setPathError(error instanceof Error ? error.message : "Failed to load episode paths.");
        setMatchingAnimeIds(new Set());
      } finally {
        if (!cancelled) setMatchingPaths(false);
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [candidateAnime, debouncedRegexInput, onListEpisodes, sourceCategoryId]);

  const matchingAnime = useMemo(
    () => candidateAnime.filter((anime) => matchingAnimeIds.has(anime.id)),
    [candidateAnime, matchingAnimeIds],
  );

  const animeToMove = useMemo(
    () => matchingAnime.filter((anime) => anime.category_id !== targetCategoryId),
    [matchingAnime, targetCategoryId],
  );

  const sourceLabel =
    sourceCategoryId === ALL_CATEGORIES_ID ? "all categories" : categoryName(library.categories, sourceCategoryId);
  const targetLabel = categoryName(library.categories, targetCategoryId);
  const sourceOptions = useMemo(
    () => [
      { value: ALL_CATEGORIES_ID, label: "All categories" },
      ...library.categories.map((category) => ({ value: category.id, label: category.name })),
    ],
    [library.categories],
  );
  const targetOptions = useMemo(
    () => library.categories.map((category) => ({ value: category.id, label: category.name })),
    [library.categories],
  );
  const sourceDropdownLabel =
    sourceCategoryId === ALL_CATEGORIES_ID ? "All categories" : categoryName(library.categories, sourceCategoryId);
  const targetDropdownLabel = categoryName(library.categories, targetCategoryId);

  const handleApply = () => {
    if (animeToMove.length === 0) return;
    const confirmed = window.confirm(
      `Move ${animeToMove.length} anime from ${sourceLabel} to "${targetLabel}"? This only changes anime categories.`,
    );
    if (!confirmed) return;
    onMoveAnime(
      animeToMove.map((anime) => anime.id),
      targetCategoryId,
    );
  };

  const idleAllLibrary = sourceCategoryId === ALL_CATEGORIES_ID && !debouncedRegexInput.trim();
  const subtitle = matchingPaths
    ? "Checking episode paths..."
    : idleAllLibrary
      ? "Pick a category or enter a path regex to see which anime are affected."
      : `${matchingAnime.length} affected title${matchingAnime.length === 1 ? "" : "s"}.`;

  return (
    <>
      <ViewHeader title="Bulk Edit" subtitle={subtitle} />

      <section className="panel bulk-edit-panel">
        <div className="bulk-edit-grid">
          <div className="stacked-field">
            <span>Only affect category</span>
            <CustomDropdown
              label={sourceDropdownLabel}
              options={sourceOptions}
              value={sourceCategoryId}
              onChange={setSourceCategoryId}
            />
          </div>

          <label className="stacked-field">
            <span>Episode path regex</span>
            <input
              type="text"
              value={regexInput}
              onChange={(event) => setRegexInput(event.currentTarget.value)}
              placeholder="Anime\\Watching or \[SubsPlease\]"
              spellCheck={false}
            />
          </label>

          <div className="stacked-field">
            <span>Set category to</span>
            <CustomDropdown
              label={targetDropdownLabel}
              options={targetOptions}
              value={targetCategoryId}
              onChange={setTargetCategoryId}
            />
          </div>
        </div>

        <p className="muted">
          The regex is matched case-insensitively against full episode file paths after a short pause in typing. If any
          episode matches, its anime is included in the preview below.
        </p>

        {regexError ? <p className="error">Invalid regex: {regexError}</p> : null}
        {pathError ? <p className="error">{pathError}</p> : null}

        <div className="settings-actions bulk-edit-settings-actions">
          {animeToMove.length > 0 ? (
            <span className="muted">
              {animeToMove.length} anime will move to "{targetLabel}"
            </span>
          ) : null}
          <button type="button" onClick={handleApply} disabled={busy || matchingPaths || animeToMove.length === 0}>
            Apply category edit
          </button>
        </div>
      </section>

      {matchingPaths ? (
        <div className="empty empty--wide">
          <h2>Checking episode paths...</h2>
          <p className="muted">Matching the regex against full file paths for the selected anime.</p>
        </div>
      ) : matchingAnime.length === 0 ? (
        <div className="empty empty--wide">
          <h2>No affected anime</h2>
          <p className="muted">
            {idleAllLibrary
              ? "Choose a specific category or type a path regex (after a short pause) to preview matches."
              : "Adjust the category filter or regex to preview the titles that will be edited."}
          </p>
        </div>
      ) : (
        <AnimeCardGrid anime={matchingAnime} onOpenAnime={onOpenAnime} />
      )}
    </>
  );
}
