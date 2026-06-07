import { type FormEvent, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import {
  getAnilistCoverImage,
  getMatchingDetectionRuleName,
  jobsCancel,
  jobsEnqueueEpisodePageOpEd,
  jobsEnqueueEpisodePageScrubSprites,
  countManualOpEdTemplates,
  jobsEnqueueOpEdDetect,
  jobsSetOpEdChromaPriorityForAnime,
  jobsSetScrubSpritePriorityForPaths,
  resetAnimeOpEdAnalysis,
} from "../api";
import { loadEpisodeThumbnailUrls } from "../animePoster";
import { pickQuickPlayEpisode } from "../quickPlay";
import { jobProgressBarPercent, shouldShowJobProgressBar } from "../jobs/jobUi";
import { useJobsSnapshot } from "../jobs/jobClient";
import { findActiveOpEdJob, manualRematchBatchFraction, opEdDetectBatchFraction, opEdSegmentLabel } from "../opEd";
import type {
  AnimeSummary,
  AnilistMediaStatus,
  AnilistSearchResult,
  Category,
  Episode,
} from "../types";
import { useRovingListNavigation } from "../useRovingListNavigation";
import {
  animeDisplayTitle,
  buildEpisodeListItems,
  computeGapEpisodeCount,
  errorMessage,
  formatEpisodeNumber,
  formatMissingEpisodesLabel,
  formatSize,
  formatTime,
  isEpisodeNumberKnown,
  progressPercent,
} from "../utils";
import { ConfirmModal } from "./ConfirmModal";
import { ContextMenu, useContextMenu, type ContextMenuItem } from "./ContextMenu";
import { CustomDropdown } from "./CustomDropdown";
import { FolderOpenIcon, ManualSkipIcon, SettingsIcon } from "./Icons";
import { PromptModal } from "./PromptModal";
import { ViewHeader } from "./ViewHeader";

type AnilistProgressUpdate = {
  animeId: number;
  progress: number;
  forceReplace?: boolean;
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

function parseIntegerDraft(value: string, label: string): number {
  const trimmed = value.trim();
  if (!trimmed) throw new Error(`${label} is required.`);
  const parsed = Number(trimmed);
  if (!Number.isInteger(parsed)) throw new Error(`${label} must be an integer.`);
  return parsed;
}

/** Parent folder of a file path (Windows-style separators supported). */
function parentFolderPath(filePath: string): string | null {
  const normalized = filePath.replace(/\//g, "\\");
  const sep = Math.max(normalized.lastIndexOf("\\"), normalized.lastIndexOf("/"));
  if (sep <= 0) return null;
  return normalized.slice(0, sep);
}

/**
 * Matches `shortest_episode_folder_for_anime` in the Rust backend: choose the
 * shortest common episode parent path so Browse opens the same folder as
 * "Open episode folder".
 */
function shortestEpisodeParentFolder(episodes: Episode[], firstEpisodePath: string | null): string | undefined {
  if (episodes.length > 0) {
    let shortest: string | null = null;
    for (const episode of episodes) {
      const parent = parentFolderPath(episode.path);
      if (!parent) continue;
      if (
        shortest === null ||
        parent.length < shortest.length ||
        (parent.length === shortest.length && parent.localeCompare(shortest) < 0)
      ) {
        shortest = parent;
      }
    }
    if (shortest) return shortest;
  }
  if (firstEpisodePath) {
    return parentFolderPath(firstEpisodePath) ?? undefined;
  }
  return undefined;
}

export function EpisodeScreen(props: {
  anime: AnimeSummary;
  episodes: Episode[];
  episodesLoading?: boolean;
  categories: Category[];
  onBack: () => void;
  onPlay: (episode: Episode) => void;
  onMoveAnime: (categoryId: number) => void;
  onOpenEpisodeFolder: () => void;
  onDeleteAnime: () => void;
  onSearchAnilist: (query: string) => Promise<AnilistSearchResult[]>;
  onGetAnilistStatus: (animeId: number) => Promise<AnilistMediaStatus | null>;
  onSetAnilistScore: (animeId: number, score: number | null) => Promise<AnilistMediaStatus>;
  onSaveAnimeSettings: (
    animeId: number,
    title: string,
    trackerOffset: number,
    progressOverride: number | null,
    customThumbnailPath: string | null,
  ) => Promise<void>;
  onClearAnimeCustomThumbnail: (animeId: number) => Promise<void>;
  anilistProgressUpdate: AnilistProgressUpdate | null;
  onLinkAnilist: (animeId: number, anilistId: number) => void;
  onUnlinkAnilist: (animeId: number) => void;
  onOpenAnilist: (url: string) => void;
  anilistConnected: boolean;
  preferAnilistDisplayTitle: boolean;
  onOpEdAnalysisUpdated: () => void;
  onOpenManualSkip: () => void;
  onShowToast: (kind: "success" | "error", message: string) => void;
  onDeleteEpisode: (episode: Episode) => void;
  onRenameEpisode: (episode: Episode, newFileName: string) => Promise<void>;
  onResetEpisodeProgress: (episode: Episode) => Promise<void>;
  onMarkEpisodeWatched: (episode: Episode) => Promise<void>;
}) {
  const {
    anime,
    episodes,
    episodesLoading = false,
    categories,
    preferAnilistDisplayTitle,
    onBack,
    onPlay,
    onMoveAnime,
    onOpenEpisodeFolder,
    onDeleteAnime,
    onSearchAnilist,
    onGetAnilistStatus,
    onSetAnilistScore,
    onSaveAnimeSettings,
    onClearAnimeCustomThumbnail,
    anilistProgressUpdate,
    onLinkAnilist,
    onUnlinkAnilist,
    onOpenAnilist,
    anilistConnected,
    onOpEdAnalysisUpdated,
    onOpenManualSkip,
    onShowToast,
    onDeleteEpisode,
    onRenameEpisode,
    onResetEpisodeProgress,
    onMarkEpisodeWatched,
  } = props;
  const jobsSnapshot = useJobsSnapshot();
  // Highlight whichever episode the Q hotkey would launch right now, so the
  // pill always points at the same target as the keybind.
  const quickPlayEpisodeId = useMemo(() => pickQuickPlayEpisode(episodes)?.id ?? null, [episodes]);
  const thumbnailBrowseDefaultPath = useMemo(
    () => shortestEpisodeParentFolder(episodes, anime.first_episode_path),
    [anime.first_episode_path, episodes],
  );
  const [episodeThumbnails, setEpisodeThumbnails] = useState<Record<number, string>>({});
  const [animeCover, setAnimeCover] = useState<string | null>(null);
  const [linkQuery, setLinkQuery] = useState(anime.title);
  const [linkResults, setLinkResults] = useState<AnilistSearchResult[]>([]);
  const [linkSearchOpen, setLinkSearchOpen] = useState(false);
  const [linkSearchBusy, setLinkSearchBusy] = useState(false);
  const [linkSearchError, setLinkSearchError] = useState<string | null>(null);
  const [deleteConfirmOpen, setDeleteConfirmOpen] = useState(false);
  const [animeSettingsOpen, setAnimeSettingsOpen] = useState(false);
  const [animeTitleDraft, setAnimeTitleDraft] = useState(anime.title);
  const [trackerOffsetDraft, setTrackerOffsetDraft] = useState(String(anime.tracker_offset));
  const [customThumbnailDraft, setCustomThumbnailDraft] = useState(anime.custom_thumbnail_path ?? "");
  const [progressOverrideDraft, setProgressOverrideDraft] = useState("");
  const [animeSettingsSaving, setAnimeSettingsSaving] = useState(false);
  const [animeSettingsError, setAnimeSettingsError] = useState<string | null>(null);
  const [anilistStatus, setAnilistStatus] = useState<AnilistMediaStatus | null>(null);
  const [scoreDraft, setScoreDraft] = useState("");
  const [scoreSaving, setScoreSaving] = useState(false);
  const [scoreError, setScoreError] = useState<string | null>(null);
  const [detectionRuleName, setDetectionRuleName] = useState<string | null | undefined>(undefined);
  const [opEdResetBusy, setOpEdResetBusy] = useState(false);
  const [opEdRunBusy, setOpEdRunBusy] = useState(false);
  const [opEdError, setOpEdError] = useState<string | null>(null);
  const { menu, openMenu, closeMenu } = useContextMenu();
  const [renameEpisode, setRenameEpisode] = useState<Episode | null>(null);
  const [renameBusy, setRenameBusy] = useState(false);
  const [renameError, setRenameError] = useState<string | null>(null);
  const [deleteEpisode, setDeleteEpisode] = useState<Episode | null>(null);
  const activeOpEdJob = useMemo(
    () => findActiveOpEdJob(jobsSnapshot, anime.id),
    [anime.id, jobsSnapshot],
  );
  const activeOpEdBatch = activeOpEdJob
    ? (manualRematchBatchFraction(activeOpEdJob) ??
      opEdDetectBatchFraction(activeOpEdJob.name))
    : null;
  const linkSearchRequestRef = useRef(0);
  const scoreSaveTimerRef = useRef<number | null>(null);
  const scoreSaveRequestRef = useRef(0);
  const progressOverrideDraftTouchedRef = useRef(false);
  const remainingCount = episodes.filter((episode) => !episode.watched).length;
  const gapCount = computeGapEpisodeCount(
    episodes,
    anilistStatus?.episodes,
    anime.tracker_offset,
    anilistStatus?.status,
  );
  const episodeListItems = useMemo(
    () =>
      buildEpisodeListItems(episodes, {
        trackerOffset: anime.tracker_offset,
        anilistTotalEpisodes: anilistStatus?.episodes,
        anilistStatus: anilistStatus?.status,
      }),
    [anilistStatus?.episodes, anilistStatus?.status, anime.tracker_offset, episodes],
  );
  const selectedCategory = categories.find((category) => category.id === anime.category_id);
  const getRovingItemProps = useRovingListNavigation(episodes.length, {
    enabled: !linkSearchOpen && !deleteConfirmOpen && !animeSettingsOpen && !renameEpisode && !deleteEpisode,
  });

  const buildEpisodeMenuItems = useCallback(
    (episode: Episode): ContextMenuItem[] => [
      {
        type: "submenu",
        id: "progress",
        label: "Progress",
        items: [
          {
            id: "reset-progress",
            label: "Reset",
            onSelect: () => void onResetEpisodeProgress(episode),
          },
          {
            id: "mark-watched",
            label: "Watched",
            disabled: episode.watched,
            onSelect: () => void onMarkEpisodeWatched(episode),
          },
        ],
      },
      {
        type: "action",
        id: "rename-episode",
        label: "Rename",
        onSelect: () => {
          setRenameError(null);
          setRenameEpisode(episode);
        },
      },
      { type: "separator", id: "delete-separator" },
      {
        type: "action",
        id: "delete-episode",
        label: "Delete",
        danger: true,
        onSelect: () => setDeleteEpisode(episode),
      },
    ],
    [onMarkEpisodeWatched, onResetEpisodeProgress],
  );

  const submitEpisodeRename = useCallback(
    async (newFileName: string) => {
      if (!renameEpisode) return;
      const trimmed = newFileName.trim();
      if (!trimmed) {
        setRenameError("Filename is required.");
        return;
      }
      setRenameBusy(true);
      setRenameError(null);
      try {
        await onRenameEpisode(renameEpisode, trimmed);
        setRenameEpisode(null);
      } catch (error) {
        setRenameError(errorMessage(error));
      } finally {
        setRenameBusy(false);
      }
    },
    [onRenameEpisode, renameEpisode],
  );

  useEffect(() => {
    setLinkQuery(anime.title);
    setLinkResults([]);
    setLinkSearchOpen(false);
    setLinkSearchError(null);
    setDeleteConfirmOpen(false);
    setAnimeSettingsOpen(false);
    setAnimeSettingsError(null);
    setAnimeTitleDraft(anime.title);
    setTrackerOffsetDraft(String(anime.tracker_offset));
    setCustomThumbnailDraft(anime.custom_thumbnail_path ?? "");
    setProgressOverrideDraft("");
  }, [anime.id, anime.title, anime.tracker_offset]);

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
    setAnilistStatus((current) => {
      if (!current) {
        return {
          progress: anilistProgressUpdate.progress,
          episodes: null,
          score: null,
          status: null,
        };
      }
      return {
        ...current,
        progress:
          anilistProgressUpdate.forceReplace || current.progress == null
            ? anilistProgressUpdate.progress
            : Math.max(current.progress, anilistProgressUpdate.progress),
      };
    });
  }, [anime.id, anilistProgressUpdate]);

  useEffect(() => {
    if (!animeSettingsOpen || anime.anilist_id == null) return;
    if (progressOverrideDraftTouchedRef.current) return;
    const p = anilistStatus?.progress;
    if (p == null) return;
    setProgressOverrideDraft(String(p));
  }, [animeSettingsOpen, anime.anilist_id, anilistStatus?.progress]);

  const handleCancelOpEdJob = useCallback(async () => {
    if (!activeOpEdJob) return;
    try {
      await jobsCancel(activeOpEdJob.id);
    } catch (e) {
      setOpEdError(errorMessage(e));
    }
  }, [activeOpEdJob]);

  const handleRunOpEdAnalysis = useCallback(async () => {
    setOpEdRunBusy(true);
    setAnimeSettingsError(null);
    try {
      const manualCount = await countManualOpEdTemplates(anime.id);
      if (manualCount > 0) {
        onShowToast(
          "success",
          "Manual skip areas are active. Analysis will fingerprint episodes if needed and rematch skip regions using your templates.",
        );
      }
      await jobsEnqueueOpEdDetect({
        animeId: anime.id,
        priority: "medium",
        animeTitle: animeDisplayTitle(anime, preferAnilistDisplayTitle),
      });
    } catch (e) {
      setAnimeSettingsError(errorMessage(e));
    } finally {
      setOpEdRunBusy(false);
    }
  }, [anime, onShowToast, preferAnilistDisplayTitle]);

  const handleResetOpEdAnalysis = useCallback(async () => {
    const manualCount = await countManualOpEdTemplates(anime.id);
    const confirmMessage =
      manualCount > 0
        ? "Clear OP/ED match results for this title? Your manual skip templates will be kept."
        : "Clear all OP/ED analysis data for this title?";
    if (!window.confirm(confirmMessage)) return;
    setOpEdResetBusy(true);
    try {
      await resetAnimeOpEdAnalysis(anime.id);
      onOpEdAnalysisUpdated();
    } catch (e) {
      setAnimeSettingsError(errorMessage(e));
    } finally {
      setOpEdResetBusy(false);
    }
  }, [anime.id, onOpEdAnalysisUpdated]);

  useEffect(() => {
    return () => {
      if (scoreSaveTimerRef.current !== null) {
        window.clearTimeout(scoreSaveTimerRef.current);
      }
    };
  }, []);

  useEffect(() => {
    if (episodesLoading || episodes.length === 0) {
      setEpisodeThumbnails({});
      return;
    }

    let cancelled = false;
    setEpisodeThumbnails({});

    void loadEpisodeThumbnailUrls(
      episodes,
      184,
      (episodeId, url) => {
        setEpisodeThumbnails((current) =>
          cancelled ? current : { ...current, [episodeId]: url },
        );
      },
      () => !cancelled,
    );

    return () => {
      cancelled = true;
    };
  }, [episodes, episodesLoading]);

  useEffect(() => {
    if (episodes.length === 0) return;

    const animeTitle = anime.anilist_title?.trim() || anime.title;
    const paths = episodes.map((episode) => episode.path);
    const scrubEpisodes = episodes.map((episode) => ({
      path: episode.path,
      episodeLabel: isEpisodeNumberKnown(episode.episode_number)
        ? formatEpisodeNumber(episode.episode_number)
        : episode.file_name,
    }));

    void jobsEnqueueEpisodePageScrubSprites({
      priority: "medium",
      animeTitle,
      episodes: scrubEpisodes,
    }).catch(() => {
      /* background queue; ignore */
    });

    return () => {
      if (paths.length === 0) return;
      void jobsSetScrubSpritePriorityForPaths(paths, "low").catch(() => {
        /* background queue; ignore */
      });
    };
  }, [anime.anilist_title, anime.id, anime.title, episodes]);

  useEffect(() => {
    if (episodes.length === 0) return;

    void jobsSetOpEdChromaPriorityForAnime(anime.id, "medium").catch(() => {
      /* background queue; ignore */
    });

    return () => {
      void jobsSetOpEdChromaPriorityForAnime(anime.id, "low").catch(() => {
        /* background queue; ignore */
      });
    };
  }, [anime.id, episodes.length]);

  useEffect(() => {
    void jobsEnqueueEpisodePageOpEd({
      animeId: anime.id,
      priority: "medium",
      animeTitle: animeDisplayTitle(anime, preferAnilistDisplayTitle),
    }).catch(() => {
      /* background queue; ignore */
    });
  }, [anime.id, preferAnilistDisplayTitle]);

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

  const openAnimeSettings = useCallback(() => {
    progressOverrideDraftTouchedRef.current = false;
    setAnimeTitleDraft(anime.title);
    setTrackerOffsetDraft(String(anime.tracker_offset));
    setCustomThumbnailDraft(anime.custom_thumbnail_path ?? "");
    setProgressOverrideDraft(
      anime.anilist_id != null && anilistStatus?.progress != null ? String(anilistStatus.progress) : "",
    );
    setAnimeSettingsError(null);
    setAnimeSettingsOpen(true);
  }, [anime.anilist_id, anime.custom_thumbnail_path, anime.title, anime.tracker_offset, anilistStatus?.progress]);

  const closeAnimeSettings = useCallback(() => {
    if (animeSettingsSaving) return;
    setAnimeSettingsOpen(false);
    setAnimeSettingsError(null);
  }, [animeSettingsSaving]);

  const stepTrackerOffsetDraft = useCallback(
    (delta: number) => {
      const parsedDraft = Number(trackerOffsetDraft);
      const baseOffset =
        trackerOffsetDraft.trim() && Number.isFinite(parsedDraft) ? parsedDraft : anime.tracker_offset;
      setTrackerOffsetDraft(String(Math.round(baseOffset + delta)));
    },
    [anime.tracker_offset, trackerOffsetDraft],
  );

  const stepProgressOverrideDraft = useCallback(
    (delta: number) => {
      progressOverrideDraftTouchedRef.current = true;
      const parsedDraft = Number(progressOverrideDraft);
      const baseProgress =
        progressOverrideDraft.trim() && Number.isFinite(parsedDraft)
          ? parsedDraft
          : anime.anilist_id != null && anilistStatus?.progress != null
            ? anilistStatus.progress
            : 0;
      setProgressOverrideDraft(String(Math.max(0, Math.round(baseProgress + delta))));
    },
    [anime.anilist_id, anilistStatus?.progress, progressOverrideDraft],
  );

  const saveAnimeSettings = useCallback(
    async (e: FormEvent<HTMLFormElement>) => {
      e.preventDefault();
      const title = animeTitleDraft.trim();
      let trackerOffset: number;
      let progressOverride: number | null = null;
      try {
        if (!title) throw new Error("Title is required.");
        trackerOffset = parseIntegerDraft(trackerOffsetDraft, "Tracker offset");
        if (progressOverrideDraft.trim()) {
          progressOverride = parseIntegerDraft(progressOverrideDraft, "Override progress");
          if (progressOverride < 0) throw new Error("Override progress must be 0 or greater.");
        }
        if (
          anime.anilist_id != null &&
          anilistStatus?.progress != null &&
          progressOverride !== null &&
          progressOverride === anilistStatus.progress
        ) {
          progressOverride = null;
        }
      } catch (error) {
        setAnimeSettingsError(errorMessage(error));
        return;
      }

      setAnimeSettingsSaving(true);
      setAnimeSettingsError(null);
      try {
        const customThumbnailPath = customThumbnailDraft.trim() || null;
        await onSaveAnimeSettings(anime.id, title, trackerOffset, progressOverride, customThumbnailPath);
        setAnimeSettingsOpen(false);
        setProgressOverrideDraft("");
      } catch (error) {
        setAnimeSettingsError(errorMessage(error));
      } finally {
        setAnimeSettingsSaving(false);
      }
    },
    [
      anime.anilist_id,
      anime.id,
      anilistStatus?.progress,
      animeTitleDraft,
      customThumbnailDraft,
      onSaveAnimeSettings,
      progressOverrideDraft,
      trackerOffsetDraft,
    ],
  );

  const browseCustomThumbnail = useCallback(async () => {
    if (animeSettingsSaving) return;
    const picked = await open({
      directory: false,
      multiple: false,
      ...(thumbnailBrowseDefaultPath ? { defaultPath: thumbnailBrowseDefaultPath } : {}),
      filters: [
        {
          name: "Image files",
          extensions: ["png", "jpg", "jpeg", "webp", "bmp", "gif"],
        },
      ],
    });
    if (typeof picked === "string" && picked) {
      setCustomThumbnailDraft(picked);
      setAnimeSettingsError(null);
    }
  }, [animeSettingsSaving, thumbnailBrowseDefaultPath]);

  const clearCustomThumbnail = useCallback(async () => {
    if (animeSettingsSaving) return;
    setAnimeSettingsSaving(true);
    setAnimeSettingsError(null);
    try {
      await onClearAnimeCustomThumbnail(anime.id);
      setCustomThumbnailDraft("");
    } catch (error) {
      setAnimeSettingsError(errorMessage(error));
    } finally {
      setAnimeSettingsSaving(false);
    }
  }, [anime.id, animeSettingsSaving, onClearAnimeCustomThumbnail]);

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
        title={animeDisplayTitle(anime, preferAnilistDisplayTitle)}
        subtitle={
          <>
            {episodes.length} episode{episodes.length === 1 ? "" : "s"} · {remainingCount} remaining
            {gapCount > 0 ? <> · <span className="stat-warning">{gapCount} missing</span></> : null}
            {detectionRuleName != null ? ` · ${detectionRuleName}` : null}
          </>
        }
        onBack={onBack}
        action={
          <>
            <CustomDropdown
              label={selectedCategory?.name ?? "Select category"}
              options={categories.map((category) => ({ value: category.id, label: category.name }))}
              value={anime.category_id}
              onChange={onMoveAnime}
            />
            <button
              type="button"
              className="header-icon-button"
              onClick={onOpenEpisodeFolder}
              disabled={episodes.length === 0}
              aria-label="Open episode folder"
              title="Open episode folder"
            >
              <FolderOpenIcon />
            </button>
            <button
              type="button"
              className="header-icon-button"
              onClick={onOpenManualSkip}
              disabled={episodes.length === 0}
              aria-label="Manual skip areas"
              title="Manual skip areas"
            >
              <ManualSkipIcon />
            </button>
            <button
              type="button"
              className="header-icon-button"
              onClick={openAnimeSettings}
              aria-label="Open title settings"
              title="Title settings"
            >
              <SettingsIcon />
            </button>
            {anime.anilist_site_url ? (
              <button type="button" onClick={() => onUnlinkAnilist(anime.id)}>
                Unlink
              </button>
            ) : anilistConnected ? (
              <button type="button" onClick={openLinkSearch}>
                Link AniList
              </button>
            ) : null}
            <button type="button" className="button-danger" onClick={() => setDeleteConfirmOpen(true)}>
              Delete Files
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
                <h2 id="delete-anime-title">Delete Title Files?</h2>
                <p className="muted" id="delete-anime-description">
                  This will delete {episodes.length} episode file{episodes.length === 1 ? "" : "s"} for "{anime.title}".
                </p>
              </div>
            </div>
            <p className="delete-confirm-warning">
              Files will be moved to the trash when possible. Library progress, cached covers, and scrub
              thumbnails for this title will also be removed.
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

      {animeSettingsOpen ? (
        <div
          className="modal-backdrop"
          role="presentation"
          onMouseDown={(e) => {
            if (e.target === e.currentTarget) closeAnimeSettings();
          }}
        >
          <section
            className="modal anime-settings-modal"
            role="dialog"
            aria-modal="true"
            aria-labelledby="anime-settings-title"
            aria-describedby="anime-settings-description"
          >
            <div className="modal-heading">
              <div>
                <h2 id="anime-settings-title">Title Settings</h2>
                <p className="muted" id="anime-settings-description">
                  Adjust various per-title settings.
                </p>
              </div>
            </div>
            <form className="anime-settings-form" onSubmit={(e) => void saveAnimeSettings(e)}>
              <div className="anime-settings-field">
                <label>
                  <span>Title</span>
                  <input
                    type="text"
                    value={animeTitleDraft}
                    disabled={animeSettingsSaving}
                    onChange={(e) => setAnimeTitleDraft(e.currentTarget.value)}
                  />
                </label>
                <p className="muted">
                  Renames the episode files on disk and keeps settings intact.
                </p>
              </div>
              <div className="anime-settings-field">
                <label>
                  <span>Custom thumbnail</span>
                  <div className="anime-settings-path-row">
                    <input
                      type="text"
                      value={customThumbnailDraft}
                      placeholder="Image path"
                      disabled={animeSettingsSaving}
                      onChange={(e) => setCustomThumbnailDraft(e.currentTarget.value)}
                    />
                    <button type="button" disabled={animeSettingsSaving} onClick={() => void browseCustomThumbnail()}>
                      Browse
                    </button>
                    <button type="button" disabled={animeSettingsSaving} onClick={() => void clearCustomThumbnail()}>
                      Clear
                    </button>
                  </div>
                </label>
              </div>
              <div className="anime-settings-field">
                <label>
                  <span>Tracker offset</span>
                  <div className="score-stepper anime-settings-stepper">
                    <input
                      type="number"
                      step="1"
                      value={trackerOffsetDraft}
                      disabled={animeSettingsSaving}
                      onChange={(e) => setTrackerOffsetDraft(e.currentTarget.value)}
                    />
                    <div className="score-stepper-buttons">
                      <button
                        type="button"
                        className="score-stepper-button score-stepper-button--up"
                        aria-label="Increase tracker offset"
                        disabled={animeSettingsSaving}
                        onClick={() => stepTrackerOffsetDraft(1)}
                      />
                      <button
                        type="button"
                        className="score-stepper-button score-stepper-button--down"
                        aria-label="Decrease tracker offset"
                        disabled={animeSettingsSaving}
                        onClick={() => stepTrackerOffsetDraft(-1)}
                      />
                    </div>
                  </div>
                </label>
                <p className="muted">
                  Adjust episode values, mainly for AniList compatibility.
                </p>
              </div>
              <div className="anime-settings-field">
                <label>
                  <span>Override progress</span>
                  <div className="score-stepper anime-settings-stepper">
                    <input
                      type="number"
                      min="0"
                      step="1"
                      value={progressOverrideDraft}
                      placeholder="Episode number as an integer, or blank"
                      disabled={animeSettingsSaving}
                      onChange={(e) => {
                        progressOverrideDraftTouchedRef.current = true;
                        setProgressOverrideDraft(e.currentTarget.value);
                      }}
                    />
                    <div className="score-stepper-buttons">
                      <button
                        type="button"
                        className="score-stepper-button score-stepper-button--up"
                        aria-label="Increase override progress"
                        disabled={animeSettingsSaving}
                        onClick={() => stepProgressOverrideDraft(1)}
                      />
                      <button
                        type="button"
                        className="score-stepper-button score-stepper-button--down"
                        aria-label="Decrease override progress"
                        disabled={animeSettingsSaving}
                        onClick={() => stepProgressOverrideDraft(-1)}
                      />
                    </div>
                  </div>
                </label>
                <p className="muted">
                  Manually override local and AniList progress.
                  {anime.anilist_id ? " Saving the same value as AniList leaves progress unchanged." : ""}
                </p>
              </div>
              <div className="anime-settings-field">
                <span className="anime-settings-section-heading">OP/ED analysis</span>
                <div className="anime-settings-op-ed-actions">
                  <button
                    type="button"
                    disabled={animeSettingsSaving || opEdRunBusy || opEdResetBusy || episodes.length < 2}
                    onClick={() => void handleRunOpEdAnalysis()}
                  >
                    {opEdRunBusy ? "Starting…" : "Run analysis"}
                  </button>
                  <button
                    type="button"
                    className="button-danger"
                    disabled={animeSettingsSaving || opEdRunBusy || opEdResetBusy}
                    onClick={() => void handleResetOpEdAnalysis()}
                  >
                    {opEdResetBusy ? "Resetting…" : "Reset"}
                  </button>
                </div>
              </div>
              {animeSettingsError ? <p className="error">{animeSettingsError}</p> : null}
              <div className="modal-actions">
                <button type="button" onClick={closeAnimeSettings} disabled={animeSettingsSaving}>
                  Cancel
                </button>
                <button type="submit" disabled={animeSettingsSaving}>
                  {animeSettingsSaving ? "Saving..." : "Save"}
                </button>
              </div>
            </form>
          </section>
        </div>
      ) : null}

      {episodes.length >= 2 && (activeOpEdJob || opEdError) ?
        <section
          className="op-ed-analysis-banner"
          aria-live="polite"
          aria-label="OP/ED detection progress"
        >
          {activeOpEdJob ?
            <>
              <div className="op-ed-analysis-banner__header">
                <strong className="op-ed-analysis-banner__title">Detecting OP/ED</strong>
                {activeOpEdBatch ?
                  <span className="muted op-ed-analysis-banner__batch">{activeOpEdBatch}</span>
                : null}
              </div>
              {activeOpEdJob.stepLabel ?
                <p className="muted op-ed-analysis-banner__step">{activeOpEdJob.stepLabel}</p>
              : null}
              {activeOpEdJob.prerequisitePending > 0 ?
                <div className="job-row-prerequisites">
                  <span className="muted">Waiting for</span>
                  {activeOpEdJob.waitingFor.map((prereq) => (
                    <span key={prereq.jobId} className="job-prerequisite-pill">
                      #{prereq.shortId}
                    </span>
                  ))}
                  {activeOpEdJob.prerequisitePending > activeOpEdJob.waitingFor.length ?
                    <span className="job-prerequisite-pill">
                      +{activeOpEdJob.prerequisitePending - activeOpEdJob.waitingFor.length} more
                    </span>
                  : null}
                </div>
              : null}
              {shouldShowJobProgressBar(activeOpEdJob) ?
                <div className="job-progress-track" aria-hidden>
                  <div
                    className="job-progress-fill"
                    style={{ width: `${jobProgressBarPercent(activeOpEdJob)}%` }}
                  />
                </div>
              : null}
              {activeOpEdJob.cancelable ?
                <button type="button" onClick={() => void handleCancelOpEdJob()}>
                  Cancel
                </button>
              : null}
            </>
          : null}
          {opEdError ? <p className="error">{opEdError}</p> : null}
        </section>
      : null}

      <section className="episode-list">
        {episodesLoading ?
          <p className="muted episode-list-loading">Loading episodes…</p>
        : null}
        {!episodesLoading ?
          episodeListItems.map((item) => {
          if (item.kind === "gap") {
            return (
              <div key={item.key} className="episode-gap-separator stat-warning" role="presentation">
                <span className="episode-gap-separator__line" aria-hidden="true" />
                <span className="episode-gap-separator__label">
                  {formatMissingEpisodesLabel(item.missingCount)}
                </span>
                <span className="episode-gap-separator__line" aria-hidden="true" />
              </div>
            );
          }
          const { episode, episodeIndex } = item;
          const percent = episode.watched ? 100 : progressPercent(episode.position_seconds, episode.duration_seconds);
          const thumbnail = episodeThumbnails[episode.id];
          const episodeTitle = isEpisodeNumberKnown(episode.episode_number)
            ? formatEpisodeNumber(episode.episode_number - anime.tracker_offset)
            : episode.file_name;
          const opLabel = opEdSegmentLabel(episode.op_ed_segments, "op");
          const edLabel = opEdSegmentLabel(episode.op_ed_segments, "ed");
          return (
            <button
              type="button"
              key={episode.id}
              className={`episode-row${episode.watched ? " episode-row--watched" : ""}${episode.id === quickPlayEpisodeId ? " episode-row--last" : ""}`}
              onClick={() => onPlay(episode)}
              onContextMenu={(event) => openMenu(event, buildEpisodeMenuItems(episode))}
              title={episode.path}
              {...getRovingItemProps(episodeIndex)}
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
                  <div className="episode-meta-details">
                    {isEpisodeNumberKnown(episode.episode_number) ? <span>{episode.file_name}</span> : null}
                    <span>{formatSize(episode.size)}</span>
                    {episode.duration_seconds > 0 ? <span>{formatTime(episode.duration_seconds)}</span> : null}
                  </div>
                  {opLabel || edLabel ?
                    <div className="episode-op-ed">
                      {opLabel ? <span className="op-ed-pill">OP {opLabel}</span> : null}
                      {edLabel ? <span className="op-ed-pill">ED {edLabel}</span> : null}
                    </div>
                  : null}
                </div>
                <div className="episode-progress">
                  <span style={{ width: `${percent}%` }} />
                </div>
              </div>
            </button>
          );
        })
        : null}
      </section>

      <ContextMenu menu={menu} onClose={closeMenu} />

      {deleteEpisode ? (
        <ConfirmModal
          title="Delete episode?"
          description={`Delete "${deleteEpisode.file_name}"?`}
          warning="The file will be moved to the trash when possible and removed from the library."
          onConfirm={() => {
            onDeleteEpisode(deleteEpisode);
            setDeleteEpisode(null);
          }}
          onClose={() => setDeleteEpisode(null)}
        />
      ) : null}

      {renameEpisode ? (
        <PromptModal
          title="Rename episode"
          description="Rename the episode file on disk."
          label="Filename"
          initialValue={renameEpisode.file_name}
          submitLabel="Rename"
          busy={renameBusy}
          error={renameError}
          onSubmit={(value) => void submitEpisodeRename(value)}
          onClose={() => {
            if (renameBusy) return;
            setRenameEpisode(null);
            setRenameError(null);
          }}
        />
      ) : null}

    </>
  );
}
