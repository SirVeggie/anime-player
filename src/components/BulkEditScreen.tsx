import { useEffect, useMemo, useState } from "react";
import type { AnimeSummary, Category, Episode, LibraryState, RenameEpisodeFileRequest } from "../types";
import { AnimeCardGrid } from "./AnimeGrid";
import { CustomDropdown } from "./CustomDropdown";

const ALL_CATEGORIES_ID = 0;
const REGEX_DEBOUNCE_MS = 1200;
type BulkEditTab = "category" | "rename";

type RenamePreviewRow = {
  episodeId: number;
  oldPath: string;
  newPath: string;
};

function categoryName(categories: Category[], categoryId: number): string {
  return categories.find((category) => category.id === categoryId)?.name ?? "Unknown";
}

function windowsPathKey(path: string): string {
  return path.replace(/\//g, "\\").toLocaleLowerCase();
}

export function BulkEditScreen(props: {
  library: LibraryState;
  busy: boolean;
  onOpenAnime: (anime: AnimeSummary) => void;
  onListEpisodes: (animeId: number) => Promise<Episode[]>;
  onMoveAnime: (animeIds: number[], categoryId: number) => void;
  onValidateEpisodeRenames: (renames: RenameEpisodeFileRequest[]) => Promise<void>;
  onRenameEpisodeFiles: (renames: RenameEpisodeFileRequest[]) => void;
}) {
  const {
    library,
    busy,
    onOpenAnime,
    onListEpisodes,
    onMoveAnime,
    onValidateEpisodeRenames,
    onRenameEpisodeFiles,
  } = props;
  const [activeTab, setActiveTab] = useState<BulkEditTab>("category");
  const [sourceCategoryId, setSourceCategoryId] = useState(ALL_CATEGORIES_ID);
  const [regexInput, setRegexInput] = useState("");
  const [debouncedRegexInput, setDebouncedRegexInput] = useState("");
  const [targetCategoryId, setTargetCategoryId] = useState<number>(library.categories[0]?.id ?? 0);
  const [matchingAnimeIds, setMatchingAnimeIds] = useState<Set<number>>(() => new Set());
  const [matchingPaths, setMatchingPaths] = useState(false);
  const [regexError, setRegexError] = useState<string | null>(null);
  const [pathError, setPathError] = useState<string | null>(null);
  const [renameRegexInput, setRenameRegexInput] = useState("");
  const [debouncedRenameRegexInput, setDebouncedRenameRegexInput] = useState("");
  const [renameReplacementInput, setRenameReplacementInput] = useState("");
  const [renameEpisodes, setRenameEpisodes] = useState<Episode[]>([]);
  const [loadingRenameEpisodes, setLoadingRenameEpisodes] = useState(false);
  const [renamePathError, setRenamePathError] = useState<string | null>(null);
  const [renameBackendError, setRenameBackendError] = useState<string | null>(null);
  const [validatingRenames, setValidatingRenames] = useState(false);

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

  useEffect(() => {
    const timeoutId = window.setTimeout(() => {
      setDebouncedRenameRegexInput(renameRegexInput);
    }, REGEX_DEBOUNCE_MS);

    return () => window.clearTimeout(timeoutId);
  }, [renameRegexInput]);

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

  useEffect(() => {
    if (activeTab !== "rename") return;

    let cancelled = false;
    setRenamePathError(null);
    setLoadingRenameEpisodes(true);

    void (async () => {
      try {
        const episodeGroups = await Promise.all(library.anime.map((anime) => onListEpisodes(anime.id)));
        if (cancelled) return;
        setRenameEpisodes(episodeGroups.flat());
      } catch (error) {
        if (cancelled) return;
        setRenamePathError(error instanceof Error ? error.message : "Failed to load episode paths.");
        setRenameEpisodes([]);
      } finally {
        if (!cancelled) setLoadingRenameEpisodes(false);
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [activeTab, library.anime, onListEpisodes]);

  const renamePreview = useMemo(() => {
    const trimmedRegex = debouncedRenameRegexInput.trim();
    if (!trimmedRegex) {
      return { rows: [] as RenamePreviewRow[], error: null as string | null };
    }

    let regex: RegExp;
    try {
      regex = new RegExp(trimmedRegex, "gi");
    } catch (error) {
      return {
        rows: [] as RenamePreviewRow[],
        error: error instanceof Error ? error.message : "Invalid regex.",
      };
    }

    const rows = renameEpisodes.flatMap((episode) => {
      regex.lastIndex = 0;
      const newPath = episode.path.replace(regex, renameReplacementInput);
      if (newPath === episode.path) return [];
      return [{ episodeId: episode.id, oldPath: episode.path, newPath }];
    });

    return { rows, error: null as string | null };
  }, [debouncedRenameRegexInput, renameEpisodes, renameReplacementInput]);

  const renameCollisionError = useMemo(() => {
    if (renamePreview.rows.length === 0) return null;

    const destinationOwners = new Map<string, string>();
    for (const row of renamePreview.rows) {
      const destinationKey = windowsPathKey(row.newPath);
      const existingOwner = destinationOwners.get(destinationKey);
      if (existingOwner) {
        return `Multiple files would be renamed to the same destination: ${existingOwner} and ${row.oldPath}`;
      }
      destinationOwners.set(destinationKey, row.oldPath);

      if (destinationKey === windowsPathKey(row.oldPath) && row.newPath !== row.oldPath) {
        return `Case-only renames are not supported in bulk mode: ${row.oldPath}`;
      }
    }

    const affectedSources = new Set(renamePreview.rows.map((row) => windowsPathKey(row.oldPath)));
    const knownEpisodePaths = new Map(renameEpisodes.map((episode) => [windowsPathKey(episode.path), episode.path]));
    for (const row of renamePreview.rows) {
      const destinationKey = windowsPathKey(row.newPath);
      const existingEpisodePath = knownEpisodePaths.get(destinationKey);
      if (existingEpisodePath && !affectedSources.has(destinationKey)) {
        return `Destination already belongs to another episode: ${existingEpisodePath}`;
      }
    }

    return null;
  }, [renameEpisodes, renamePreview.rows]);

  const renameRequests = useMemo(
    () =>
      renamePreview.rows.map((row) => ({
        episode_id: row.episodeId,
        old_path: row.oldPath,
        new_path: row.newPath,
      })),
    [renamePreview.rows],
  );

  useEffect(() => {
    if (activeTab !== "rename" || renameRequests.length === 0 || renamePreview.error || renameCollisionError) {
      setRenameBackendError(null);
      setValidatingRenames(false);
      return;
    }

    let cancelled = false;
    setRenameBackendError(null);
    setValidatingRenames(true);

    void (async () => {
      try {
        await onValidateEpisodeRenames(renameRequests);
      } catch (error) {
        if (!cancelled) {
          setRenameBackendError(error instanceof Error ? error.message : "Rename validation failed.");
        }
      } finally {
        if (!cancelled) setValidatingRenames(false);
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [activeTab, onValidateEpisodeRenames, renameCollisionError, renamePreview.error, renameRequests]);

  const handleRenameFiles = () => {
    if (renameRequests.length === 0) return;
    const confirmed = window.confirm(`Rename ${renameRequests.length} file${renameRequests.length === 1 ? "" : "s"}?`);
    if (!confirmed) return;
    onRenameEpisodeFiles(renameRequests);
  };

  const idleAllLibrary = sourceCategoryId === ALL_CATEGORIES_ID && !debouncedRegexInput.trim();
  const renameDisabled =
    busy ||
    loadingRenameEpisodes ||
    validatingRenames ||
    renameRequests.length === 0 ||
    Boolean(renamePreview.error || renameCollisionError || renamePathError || renameBackendError);

  return (
    <>
      <div className="bulk-edit-tabs" role="tablist" aria-label="Bulk edit tools">
        <button
          type="button"
          role="tab"
          aria-selected={activeTab === "category"}
          className={activeTab === "category" ? "bulk-edit-tab bulk-edit-tab--active" : "bulk-edit-tab"}
          onClick={() => setActiveTab("category")}
        >
          Category editor
        </button>
        <button
          type="button"
          role="tab"
          aria-selected={activeTab === "rename"}
          className={activeTab === "rename" ? "bulk-edit-tab bulk-edit-tab--active" : "bulk-edit-tab"}
          onClick={() => setActiveTab("rename")}
        >
          Filename replacer
        </button>
      </div>

      {activeTab === "category" ? (
        <>
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
              The regex is matched case-insensitively against full episode file paths after a short pause in typing. If
              any episode matches, its anime is included in the preview below.
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
                Apply category
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
      ) : (
        <div className="panel bulk-edit-panel">
          <div className="bulk-edit-grid bulk-edit-grid--rename">
            <label className="stacked-field">
              <span>Full filepath regex</span>
              <input
                type="text"
                value={renameRegexInput}
                onChange={(event) => setRenameRegexInput(event.currentTarget.value)}
                placeholder="(.+) - (\\d+)(\\.mkv)$"
                spellCheck={false}
              />
            </label>

            <label className="stacked-field">
              <span>Replace with</span>
              <input
                type="text"
                value={renameReplacementInput}
                onChange={(event) => setRenameReplacementInput(event.currentTarget.value)}
                placeholder="$1 - Episode $2$3"
                spellCheck={false}
              />
            </label>
          </div>

          <p className="muted">
            The regex is matched case-insensitively against full file paths after a short pause in typing. Replacement
            supports normal JavaScript regex groups such as $1 and $&lt;name&gt;.
          </p>

          {renamePreview.error ? <p className="error">Invalid regex: {renamePreview.error}</p> : null}
          {renamePathError ? <p className="error">{renamePathError}</p> : null}
          {renameCollisionError ? <p className="error">{renameCollisionError}</p> : null}
          {renameBackendError ? <p className="error">{renameBackendError}</p> : null}

          <div className="settings-actions bulk-edit-settings-actions">
            <span className="muted">
              {loadingRenameEpisodes
                ? "Loading episode paths..."
                : validatingRenames
                  ? "Checking rename safety..."
                  : `${renamePreview.rows.length} affected file${renamePreview.rows.length === 1 ? "" : "s"}.`}
            </span>
            <button type="button" onClick={handleRenameFiles} disabled={renameDisabled}>
              Rename files
            </button>
          </div>

          {renamePreview.rows.length === 0 ? (
            <div className="empty empty--wide bulk-rename-empty">
              <h2>No affected episodes</h2>
              <p className="muted">
                {debouncedRenameRegexInput.trim()
                  ? "Adjust the regex or replacement to preview filepath changes."
                  : "Enter a regex to preview filepath replacements."}
              </p>
            </div>
          ) : (
            <div className="bulk-rename-preview" aria-label="Affected episode file renames">
              {renamePreview.rows.map((row) => (
                <div className="bulk-rename-preview-row" key={`${row.episodeId}:${row.oldPath}`}>
                  <span>{row.oldPath}</span>
                  <strong>-&gt;</strong>
                  <span>{row.newPath}</span>
                </div>
              ))}
            </div>
          )}
        </div>
      )}
    </>
  );
}
