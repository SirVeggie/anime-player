import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { getAnilistCoverImage, getFileThumbnail, getMatchingDetectionRuleName } from "../api";
import { pickQuickPlayEpisode } from "../quickPlay";
import type { AnimeSummary, AnilistMediaStatus, AnilistSearchResult, Category, Episode } from "../types";
import { useRovingListNavigation } from "../useRovingListNavigation";
import { formatEpisodeNumber, formatSize, formatTime, isEpisodeNumberKnown, progressPercent } from "../utils";
import { CustomDropdown } from "./CustomDropdown";
import { ViewHeader } from "./ViewHeader";

type AnilistProgressUpdate = {
  animeId: number;
  progress: number;
  updatedAt: number;
};

function mergeWithLatestProgress(
  status: AnilistMediaStatus | null,
  current: AnilistMediaStatus | null,
): AnilistMediaStatus | null {
  if (!status) return null;
  const progress =
    current?.progress == null
      ? status.progress
      : status.progress == null
        ? current.progress
        : Math.max(status.progress, current.progress);
  return { ...status, progress };
}

export function EpisodeScreen(props: {
  anime: AnimeSummary;
  episodes: Episode[];
  categories: Category[];
  onBack: () => void;
  onPlay: (episode: Episode) => void;
  onMoveAnime: (categoryId: number) => void;
  onDeleteAnime: () => void;
  onSearchAnilist: (query: string) => Promise<AnilistSearchResult[]>;
  onGetAnilistStatus: (animeId: number) => Promise<AnilistMediaStatus | null>;
  onSetAnilistScore: (animeId: number, score: number | null) => Promise<AnilistMediaStatus>;
  anilistProgressUpdate: AnilistProgressUpdate | null;
  onLinkAnilist: (animeId: number, anilistId: number) => void;
  onUnlinkAnilist: (animeId: number) => void;
  onOpenAnilist: (url: string) => void;
}) {
  const {
    anime,
    episodes,
    categories,
    onBack,
    onPlay,
    onMoveAnime,
    onDeleteAnime,
    onSearchAnilist,
    onGetAnilistStatus,
    onSetAnilistScore,
    anilistProgressUpdate,
    onLinkAnilist,
    onUnlinkAnilist,
    onOpenAnilist,
  } = props;
  // Highlight whichever episode the Q hotkey would launch right now, so the
  // pill always points at the same target as the keybind.
  const quickPlayEpisodeId = useMemo(() => pickQuickPlayEpisode(episodes)?.id ?? null, [episodes]);
  const [episodeThumbnails, setEpisodeThumbnails] = useState<Record<number, string>>({});
  const [animeCover, setAnimeCover] = useState<string | null>(null);
  const [linkQuery, setLinkQuery] = useState(anime.title);
  const [linkResults, setLinkResults] = useState<AnilistSearchResult[]>([]);
  const [linkSearchOpen, setLinkSearchOpen] = useState(false);
  const [linkSearchBusy, setLinkSearchBusy] = useState(false);
  const [linkSearchError, setLinkSearchError] = useState<string | null>(null);
  const [deleteConfirmOpen, setDeleteConfirmOpen] = useState(false);
  const [anilistStatus, setAnilistStatus] = useState<AnilistMediaStatus | null>(null);
  const [scoreDraft, setScoreDraft] = useState("");
  const [scoreSaving, setScoreSaving] = useState(false);
  const [scoreError, setScoreError] = useState<string | null>(null);
  const [detectionRuleName, setDetectionRuleName] = useState<string | null | undefined>(undefined);
  const linkSearchRequestRef = useRef(0);
  const scoreSaveTimerRef = useRef<number | null>(null);
  const scoreSaveRequestRef = useRef(0);
  const remainingCount = episodes.filter((episode) => !episode.watched).length;
  const selectedCategory = categories.find((category) => category.id === anime.category_id);
  const getRovingItemProps = useRovingListNavigation(episodes.length, { enabled: !linkSearchOpen && !deleteConfirmOpen });

  useEffect(() => {
    setLinkQuery(anime.title);
    setLinkResults([]);
    setLinkSearchOpen(false);
    setLinkSearchError(null);
    setDeleteConfirmOpen(false);
  }, [anime.id, anime.title]);

  useEffect(() => {
    let cancelled = false;
    setDetectionRuleName(undefined);
    void getMatchingDetectionRuleName(anime.id)
      .then((name) => {
        if (!cancelled) setDetectionRuleName(name);
      })
      .catch(() => {
        if (!cancelled) setDetectionRuleName(null);
      });
    return () => {
      cancelled = true;
    };
  }, [anime.id]);

  useEffect(() => {
    let cancelled = false;
    setAnimeCover(null);
    if (!anime.anilist_cover_path) return;
    void getAnilistCoverImage(anime.id)
      .then((cover) => {
        if (!cancelled) setAnimeCover(cover);
      })
      .catch(() => {
        if (!cancelled) setAnimeCover(null);
      });
    return () => {
      cancelled = true;
    };
  }, [anime.anilist_cover_path, anime.id]);

  useEffect(() => {
    let cancelled = false;
    setAnilistStatus(null);
    setScoreDraft("");
    setScoreError(null);
    if (!anime.anilist_id) return;
    void onGetAnilistStatus(anime.id)
      .then((status) => {
        if (cancelled) return;
        setAnilistStatus((current) => mergeWithLatestProgress(status, current));
        setScoreDraft(status?.score == null ? "" : String(status.score));
      })
      .catch((e) => {
        if (!cancelled) setScoreError(e instanceof Error ? e.message : String(e));
      });
    return () => {
      cancelled = true;
    };
  }, [anime.anilist_id, anime.id, onGetAnilistStatus]);

  useEffect(() => {
    if (!anilistProgressUpdate || anilistProgressUpdate.animeId !== anime.id) return;
    setAnilistStatus((current) => ({
      progress:
        current?.progress == null
          ? anilistProgressUpdate.progress
          : Math.max(current.progress, anilistProgressUpdate.progress),
      episodes: current?.episodes ?? null,
      score: current?.score ?? null,
    }));
  }, [anime.id, anilistProgressUpdate]);

  useEffect(() => {
    return () => {
      if (scoreSaveTimerRef.current !== null) {
        window.clearTimeout(scoreSaveTimerRef.current);
      }
    };
  }, []);

  useEffect(() => {
    let cancelled = false;
    setEpisodeThumbnails({});

    void Promise.all(
      episodes.map(async (episode) => {
        try {
          const thumbnail = await getFileThumbnail(episode.path, 184);
          return thumbnail ? ([episode.id, thumbnail] as const) : null;
        } catch {
          return null;
        }
      }),
    ).then((entries) => {
      if (cancelled) return;

      setEpisodeThumbnails(
        Object.fromEntries(entries.filter((entry): entry is readonly [number, string] => entry !== null)),
      );
    });

    return () => {
      cancelled = true;
    };
  }, [episodes]);

  const closeLinkSearch = useCallback(() => {
    linkSearchRequestRef.current += 1;
    setLinkSearchOpen(false);
    setLinkSearchBusy(false);
    setLinkSearchError(null);
  }, []);

  const runLinkSearch = useCallback(
    async (queryOverride?: string) => {
      const query = (queryOverride ?? linkQuery).trim();
      if (!query) return;
      const requestId = linkSearchRequestRef.current + 1;
      linkSearchRequestRef.current = requestId;
      setLinkSearchBusy(true);
      setLinkSearchError(null);
      try {
        const results = await onSearchAnilist(query);
        if (linkSearchRequestRef.current === requestId) {
          setLinkResults(results);
        }
      } catch (e) {
        if (linkSearchRequestRef.current === requestId) {
          setLinkSearchError(e instanceof Error ? e.message : String(e));
        }
      } finally {
        if (linkSearchRequestRef.current === requestId) {
          setLinkSearchBusy(false);
        }
      }
    },
    [linkQuery, onSearchAnilist],
  );

  const openLinkSearch = useCallback(() => {
    setLinkQuery(anime.title);
    setLinkResults([]);
    setLinkSearchOpen(true);
    void runLinkSearch(anime.title);
  }, [anime.title, runLinkSearch]);

  const confirmDeleteAnime = useCallback(() => {
    setDeleteConfirmOpen(false);
    onDeleteAnime();
  }, [onDeleteAnime]);

  const saveScore = useCallback(
    async (value: string) => {
      if (!anime.anilist_id) return;
      if (scoreSaveTimerRef.current !== null) {
        window.clearTimeout(scoreSaveTimerRef.current);
        scoreSaveTimerRef.current = null;
      }
      const trimmed = value.trim();
      const score = trimmed === "" ? null : Number(trimmed);
      if (score !== null && !Number.isFinite(score)) {
        setScoreError("Score must be a number.");
        return;
      }
      const requestId = scoreSaveRequestRef.current + 1;
      scoreSaveRequestRef.current = requestId;
      setScoreSaving(true);
      setScoreError(null);
      try {
        const status = await onSetAnilistScore(anime.id, score);
        if (scoreSaveRequestRef.current === requestId) {
          setAnilistStatus(status);
          setScoreDraft(status.score == null ? "" : String(status.score));
        }
      } catch (e) {
        if (scoreSaveRequestRef.current === requestId) {
          setScoreError(e instanceof Error ? e.message : String(e));
        }
      } finally {
        if (scoreSaveRequestRef.current === requestId) {
          setScoreSaving(false);
        }
      }
    },
    [anime.anilist_id, anime.id, onSetAnilistScore],
  );

  const scheduleScoreSave = useCallback(
    (value: string) => {
      if (scoreSaveTimerRef.current !== null) {
        window.clearTimeout(scoreSaveTimerRef.current);
      }
      scoreSaveTimerRef.current = window.setTimeout(() => {
        scoreSaveTimerRef.current = null;
        void saveScore(value);
      }, 650);
    },
    [saveScore],
  );

  const updateScoreDraft = useCallback(
    (value: string) => {
      setScoreDraft(value);
      scheduleScoreSave(value);
    },
    [scheduleScoreSave],
  );

  const stepScoreDraft = useCallback(
    (delta: number) => {
      const parsedDraft = Number(scoreDraft);
      const baseScore = scoreDraft.trim() && Number.isFinite(parsedDraft) ? parsedDraft : (anilistStatus?.score ?? 0);
      const nextScore = Math.min(100, Math.max(0, Math.round(baseScore + delta)));
      updateScoreDraft(String(nextScore));
    },
    [anilistStatus?.score, scoreDraft, updateScoreDraft],
  );

  return (
    <>
      <ViewHeader
        title={anime.title}
        subtitle={[
          `${episodes.length} episode${episodes.length === 1 ? "" : "s"}`,
          `${remainingCount} remaining`,
          ...(detectionRuleName !== undefined && detectionRuleName !== null ? [detectionRuleName] : []),
        ].join(" · ")}
        onBack={onBack}
        action={
          <>
            <CustomDropdown
              label={selectedCategory?.name ?? "Select category"}
              options={categories.map((category) => ({ value: category.id, label: category.name }))}
              value={anime.category_id}
              onChange={onMoveAnime}
            />
            {anime.anilist_site_url ? (
              <button type="button" onClick={() => onUnlinkAnilist(anime.id)}>
                Unlink
              </button>
            ) : (
              <button type="button" onClick={openLinkSearch}>
                Link AniList
              </button>
            )}
            <button type="button" className="button-danger" onClick={() => setDeleteConfirmOpen(true)}>
              Delete Anime
            </button>
          </>
        }
      />

      {anime.anilist_id ? (
        <section className="anime-detail-panel">
          <button
            type="button"
            className="anime-detail-main"
            onClick={() => {
              const url = anime.anilist_site_url;
              if (url) onOpenAnilist(url);
            }}
            disabled={!anime.anilist_site_url}
            aria-label={`Open ${anime.anilist_title ?? anime.title} on AniList`}
          >
            <div className={`anime-detail-cover${animeCover ? " anime-detail-cover--image" : ""}`}>
              {animeCover ? <img src={animeCover} alt="" /> : anime.title.slice(0, 2).toUpperCase()}
            </div>
            <div>
              <h2>{anime.anilist_title ?? anime.title}</h2>
              <p className="muted">Linked to AniList #{anime.anilist_id}</p>
              <p className="muted">
                Progress: {anilistStatus?.progress ?? "?"}/{anilistStatus?.episodes ?? "?"}
              </p>
            </div>
          </button>
          <div className="anilist-score-control">
            <label>
              <span>Score</span>
              <div className="score-stepper">
                <input
                  type="number"
                  min="0"
                  max="100"
                  step="1"
                  value={scoreDraft}
                  placeholder="No score"
                  disabled={scoreSaving}
                  onChange={(e) => updateScoreDraft(e.currentTarget.value)}
                  onBlur={(e) => {
                    if (scoreSaveTimerRef.current !== null) {
                      window.clearTimeout(scoreSaveTimerRef.current);
                      scoreSaveTimerRef.current = null;
                      void saveScore(e.currentTarget.value);
                    }
                  }}
                />
                <div className="score-stepper-buttons">
                  <button
                    type="button"
                    className="score-stepper-button score-stepper-button--up"
                    aria-label="Increase AniList score"
                    disabled={scoreSaving}
                    onClick={() => stepScoreDraft(1)}
                  />
                  <button
                    type="button"
                    className="score-stepper-button score-stepper-button--down"
                    aria-label="Decrease AniList score"
                    disabled={scoreSaving}
                    onClick={() => stepScoreDraft(-1)}
                  />
                </div>
              </div>
            </label>
            {scoreError ? <span className="anilist-score-error">{scoreError}</span> : null}
          </div>
        </section>
      ) : null}

      {linkSearchOpen && !anime.anilist_id ? (
        <div
          className="modal-backdrop"
          role="presentation"
          onMouseDown={(e) => {
            if (e.target === e.currentTarget) closeLinkSearch();
          }}
        >
          <section
            className="modal anilist-link-modal"
            role="dialog"
            aria-modal="true"
            aria-labelledby="anilist-link-title"
          >
            <div className="modal-heading">
              <div>
                <h2 id="anilist-link-title">Link AniList</h2>
                <p className="muted">Pick the AniList entry that matches "{anime.title}".</p>
              </div>
              <button type="button" onClick={closeLinkSearch} aria-label="Close AniList linking">
                Close
              </button>
            </div>
            <form
              className="form-row"
              onSubmit={(e) => {
                e.preventDefault();
                void runLinkSearch();
              }}
            >
              <input type="text" value={linkQuery} onChange={(e) => setLinkQuery(e.currentTarget.value)} />
              <button type="submit" disabled={linkSearchBusy || !linkQuery.trim()}>
                {linkSearchBusy ? "Searching..." : "Search"}
              </button>
            </form>
            {linkSearchError ? <p className="error">{linkSearchError}</p> : null}
            <div className="anilist-results" aria-busy={linkSearchBusy}>
              {linkResults.map((result) => (
                <button
                  type="button"
                  className="anilist-result"
                  key={result.id}
                  onClick={() => {
                    closeLinkSearch();
                    onLinkAnilist(anime.id, result.id);
                  }}
                >
                  {result.cover_image_url ? <img src={result.cover_image_url} alt="" loading="lazy" /> : null}
                  <span>
                    <strong>{result.title}</strong>
                    {result.native_title ? <em>{result.native_title}</em> : null}
                    <small>
                      {[result.season_year, result.format, result.episodes ? `${result.episodes} eps` : null]
                        .filter(Boolean)
                        .join(" - ")}
                    </small>
                  </span>
                </button>
              ))}
              {!linkSearchBusy && linkResults.length === 0 ? (
                <p className="muted">No matches yet. Try a different search title.</p>
              ) : null}
            </div>
          </section>
        </div>
      ) : null}

      {deleteConfirmOpen ? (
        <div
          className="modal-backdrop"
          role="presentation"
          onMouseDown={(e) => {
            if (e.target === e.currentTarget) setDeleteConfirmOpen(false);
          }}
        >
          <section
            className="modal delete-confirm-modal"
            role="dialog"
            aria-modal="true"
            aria-labelledby="delete-anime-title"
            aria-describedby="delete-anime-description"
          >
            <div className="modal-heading">
              <div>
                <h2 id="delete-anime-title">Delete Anime Files?</h2>
                <p className="muted" id="delete-anime-description">
                  This will delete {episodes.length} episode file{episodes.length === 1 ? "" : "s"} for "{anime.title}".
                </p>
              </div>
            </div>
            <p className="delete-confirm-warning">
              Files will be moved to the trash when possible. The database entries stay until you run cleanup in
              Settings, but the local episode files will no longer be available.
            </p>
            <div className="modal-actions">
              <button type="button" onClick={() => setDeleteConfirmOpen(false)}>
                Cancel
              </button>
              <button type="button" className="button-danger" onClick={confirmDeleteAnime}>
                Delete Files
              </button>
            </div>
          </section>
        </div>
      ) : null}

      <section className="episode-list">
        {episodes.map((episode, index) => {
          const percent = episode.watched ? 100 : progressPercent(episode.position_seconds, episode.duration_seconds);
          const thumbnail = episodeThumbnails[episode.id];
          const episodeTitle = isEpisodeNumberKnown(episode.episode_number)
            ? formatEpisodeNumber(episode.episode_number)
            : episode.file_name;
          return (
            <button
              type="button"
              key={episode.id}
              className={`episode-row${episode.watched ? " episode-row--watched" : ""}${episode.id === quickPlayEpisodeId ? " episode-row--last" : ""}`}
              onClick={() => onPlay(episode)}
              title={episode.path}
              {...getRovingItemProps(index)}
            >
              <div className={`episode-thumb${thumbnail ? " episode-thumb--image" : ""}`}>
                {thumbnail ? <img src={thumbnail} alt="" loading="lazy" /> : episode.file_type.toUpperCase()}
              </div>
              <div className="episode-main">
                <div className="episode-title">
                  <span>{episodeTitle}</span>
                  {episode.id === quickPlayEpisodeId ? <span className="pill">Up next</span> : null}
                </div>
                <div className="episode-meta">
                  {isEpisodeNumberKnown(episode.episode_number) ? <span>{episode.file_name}</span> : null}
                  <span>{formatSize(episode.size)}</span>
                  {episode.duration_seconds > 0 ? <span>{formatTime(episode.duration_seconds)}</span> : null}
                </div>
                <div className="episode-progress">
                  <span style={{ width: `${percent}%` }} />
                </div>
              </div>
            </button>
          );
        })}
      </section>
    </>
  );
}
