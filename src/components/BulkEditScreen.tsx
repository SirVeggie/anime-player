import { useEffect, useMemo, useState } from "react";
import type { AnimeSummary, Category, Episode, LibraryState } from "../types";
import { AnimeCardGrid } from "./AnimeGrid";
import { ViewHeader } from "./ViewHeader";

const ALL_CATEGORIES = "all";

type CategoryFilterValue = number | typeof ALL_CATEGORIES;

function parseCategoryFilter(value: string): CategoryFilterValue {
  if (value === ALL_CATEGORIES) return ALL_CATEGORIES;
  const parsed = Number.parseInt(value, 10);
  return Number.isFinite(parsed) ? parsed : ALL_CATEGORIES;
}

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
  const [sourceCategoryId, setSourceCategoryId] = useState<CategoryFilterValue>(ALL_CATEGORIES);
  const [regexInput, setRegexInput] = useState("");
  const [targetCategoryId, setTargetCategoryId] = useState<number>(library.categories[0]?.id ?? 0);
  const [matchingAnimeIds, setMatchingAnimeIds] = useState<Set<number>>(() => new Set(library.anime.map((anime) => anime.id)));
  const [matchingPaths, setMatchingPaths] = useState(false);
  const [regexError, setRegexError] = useState<string | null>(null);
  const [pathError, setPathError] = useState<string | null>(null);

  useEffect(() => {
    if (library.categories.some((category) => category.id === targetCategoryId)) return;
    setTargetCategoryId(library.categories[0]?.id ?? 0);
  }, [library.categories, targetCategoryId]);

  const candidateAnime = useMemo(() => {
    if (sourceCategoryId === ALL_CATEGORIES) return library.anime;
    return library.anime.filter((anime) => anime.category_id === sourceCategoryId);
  }, [library.anime, sourceCategoryId]);

  useEffect(() => {
    const trimmedRegex = regexInput.trim();
    setPathError(null);

    if (!trimmedRegex) {
      setRegexError(null);
      setMatchingPaths(false);
      setMatchingAnimeIds(new Set(candidateAnime.map((anime) => anime.id)));
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
  }, [candidateAnime, onListEpisodes, regexInput]);

  const matchingAnime = useMemo(
    () => candidateAnime.filter((anime) => matchingAnimeIds.has(anime.id)),
    [candidateAnime, matchingAnimeIds],
  );

  const animeToMove = useMemo(
    () => matchingAnime.filter((anime) => anime.category_id !== targetCategoryId),
    [matchingAnime, targetCategoryId],
  );

  const sourceLabel =
    sourceCategoryId === ALL_CATEGORIES ? "all categories" : categoryName(library.categories, sourceCategoryId);
  const targetLabel = categoryName(library.categories, targetCategoryId);

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

  const subtitle = matchingPaths
    ? "Checking episode paths..."
    : `${matchingAnime.length} affected title${matchingAnime.length === 1 ? "" : "s"}.`;

  return (
    <>
      <ViewHeader title="Bulk Edit" subtitle={subtitle} />

      <section className="panel bulk-edit-panel">
        <div className="bulk-edit-grid">
          <label className="stacked-field">
            <span>Only affect category</span>
            <select value={String(sourceCategoryId)} onChange={(event) => setSourceCategoryId(parseCategoryFilter(event.currentTarget.value))}>
              <option value={ALL_CATEGORIES}>All categories</option>
              {library.categories.map((category) => (
                <option key={category.id} value={category.id}>
                  {category.name}
                </option>
              ))}
            </select>
          </label>

          <label className="stacked-field">
            <span>Episode path regex</span>
            <input
              type="text"
              value={regexInput}
              onChange={(event) => setRegexInput(event.currentTarget.value)}
              placeholder="Anime\\Old or \\[SubsPlease\\]"
              spellCheck={false}
            />
          </label>

          <label className="stacked-field">
            <span>Set category to</span>
            <select value={targetCategoryId} onChange={(event) => setTargetCategoryId(Number.parseInt(event.currentTarget.value, 10))}>
              {library.categories.map((category) => (
                <option key={category.id} value={category.id}>
                  {category.name}
                </option>
              ))}
            </select>
          </label>
        </div>

        <p className="muted">
          The regex is matched case-insensitively against full episode file paths. If any episode matches, its anime is
          selected for the bulk edit.
        </p>

        {regexError ? <p className="error">Invalid regex: {regexError}</p> : null}
        {pathError ? <p className="error">{pathError}</p> : null}

        <div className="settings-actions">
          <span className="muted">
            {animeToMove.length} category change{animeToMove.length === 1 ? "" : "s"} ready
          </span>
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
          <p className="muted">Adjust the category filter or regex to preview the titles that will be edited.</p>
        </div>
      ) : (
        <AnimeCardGrid anime={matchingAnime} onOpenAnime={onOpenAnime} />
      )}
    </>
  );
}
