import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { diagnosticLog } from "./diagnosticLog";
import { open } from "@tauri-apps/plugin-dialog";
import { getCurrent, onOpenUrl } from "@tauri-apps/plugin-deep-link";
import { openUrl } from "@tauri-apps/plugin-opener";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  addRootFolder,
  applyAnilistProgressToLocal,
  cleanLocalData,
  completeAnilistLogin,
  createCategory,
  createRegexRule,
  deleteAnimeFiles,
  deleteCategory,
  deleteRegexRule,
  resetRegexRulesToDefaults,
  getAnilistAuthState,
  getAnilistLoginUrl,
  getAnilistMediaStatus,
  getAnimeOpEdSummary,
  getAnimeSearchIndex,
  getLibraryState,
  getLocalDataStats,
  linkAnimeAnilist,
  listEpisodes,
  listRootVideoFiles,
  logoutAnilist,
  moveAnimeToCategory,
  openAnimeEpisodeFolder,
  overrideAnimeProgress,
  reorderCategories,
  renameAnime,
  renameFiles,
  removeRootFolder,
  rescanLibrary,
  searchAnilistAnime,
  setPreferAnilistDisplayTitle,
  setSkipOpEd,
  setAnimeCustomThumbnailPath,
  setAnimeTrackerOffset,
  setDefaultCategory,
  setAnilistClientId,
  setAnilistMediaProgress,
  setAnilistMediaScore,
  stopMpv,
  unlinkAnimeAnilist,
  updateRegexRule,
  validateFileRenames,
} from "./api";
import { AnimeGrid } from "./components/AnimeGrid";
import { BulkEditScreen } from "./components/BulkEditScreen";
import { CategoryScreen } from "./components/CategoryScreen";
import { EpisodeScreen } from "./components/EpisodeScreen";
import {
  BulkEditIcon,
  JobsIcon,
  LibraryIcon,
  MissingIcon,
  RescanIcon,
  SearchIcon,
  SettingsIcon,
} from "./components/Icons";
import { JobsScreen } from "./components/JobsScreen";
import { useJobsActiveCount } from "./jobs/jobClient";
import { MissingScreen } from "./components/MissingScreen";
import { PlayerView } from "./components/PlayerView";
import { SearchScreen } from "./components/SearchScreen";
import { SettingsScreen } from "./components/SettingsScreen";
import { type Toast, ToastStack } from "./components/ToastStack";
import { WindowTitleBar } from "./components/WindowTitleBar";
import { pickQuickPlayEpisode } from "./quickPlay";
import type {
  AnimeSearchEntry,
  AnimeSummary,
  AnilistAuthState,
  AnilistMediaStatus,
  AnilistProgressSyncResult,
  AnilistSearchResult,
  Category,
  Episode,
  LibraryState,
  LocalDataStats,
  RegexRule,
  RegexRuleInput,
  RenameFileRequest,
  RootFolder,
} from "./types";
import {
  animeDisplayTitle,
  APP_WINDOW_TITLE,
  errorMessage,
  formatSize,
  isAnilistConnected,
  isTextInputTarget,
  shortenForOsTitle,
} from "./utils";
import "./App.css";

type View =
  | "categories"
  | "anime"
  | "search"
  | "bulkEdit"
  | "missing"
  | "episodes"
  | "jobs"
  | "settings"
  | "player";
type EpisodeReturnView = "anime" | "search" | "bulkEdit" | "categories";
type ScrollRestoration = "top" | "restore";

type AnilistProgressUpdate = {
  animeId: number;
  progress: number;
  forceReplace?: boolean;
  updatedAt: number;
};

type ScreenTransitionPhase = "idle" | "cover" | "uncover";

// Cover hides the outgoing view, we swap state, then uncover reveals the
// incoming view. Symmetric durations keep enter/exit feeling identical.
const SCREEN_COVER_MS = 220;
const SCREEN_UNCOVER_MS = 240;

const appWindow = getCurrentWindow();

function waitMs(ms: number) {
  return new Promise<void>((resolve) => window.setTimeout(resolve, ms));
}

// Wait two animation frames so the React commit triggered by `between()` has
// actually been painted before we start the uncover transition. One rAF can
// fire before paint in some engines; two is the standard "next paint" guard.
function waitTwoFrames() {
  return new Promise<void>((resolve) => {
    requestAnimationFrame(() => requestAnimationFrame(() => resolve()));
  });
}

function hasEpisodeProgress(episode: Episode): boolean {
  return episode.watched || episode.position_seconds > 0;
}

function App() {
  const [library, setLibrary] = useState<LibraryState | null>(null);
  const [animeSearchIndex, setAnimeSearchIndex] = useState<AnimeSearchEntry[]>([]);
  const [view, setView] = useState<View>("categories");
  const viewRef = useRef<View>("categories");
  viewRef.current = view;
  const [selectedCategoryId, setSelectedCategoryId] = useState<number | null>(null);
  const [selectedAnime, setSelectedAnime] = useState<AnimeSummary | null>(null);
  const [episodeReturnView, setEpisodeReturnView] = useState<EpisodeReturnView>("anime");
  const [anilistAuth, setAnilistAuth] = useState<AnilistAuthState | null>(null);
  const [localDataStats, setLocalDataStats] = useState<LocalDataStats | null>(null);
  const [episodes, setEpisodes] = useState<Episode[]>([]);
  const [episodesLoading, setEpisodesLoading] = useState(false);
  const [selectedEpisode, setSelectedEpisode] = useState<Episode | null>(null);
  const [searchQuery, setSearchQuery] = useState("");
  const [searchFocusToken, setSearchFocusToken] = useState(0);
  const [anilistProgressUpdate, setAnilistProgressUpdate] = useState<AnilistProgressUpdate | null>(null);
  const [rootInput, setRootInput] = useState("");
  const [newCategoryName, setNewCategoryName] = useState("");
  const [newRuleEditorKey, setNewRuleEditorKey] = useState(0);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [fatalError, setFatalError] = useState<string | null>(null);
  const [toasts, setToasts] = useState<Toast[]>([]);
  const jobsActiveCount = useJobsActiveCount();
  const [screenTransition, setScreenTransition] = useState<ScreenTransitionPhase>("idle");
  const screenTransitionRef = useRef<ScreenTransitionPhase>("idle");
  const [playerControlsChromeVisible, setPlayerControlsChromeVisible] = useState(true);
  const [windowFullscreen, setWindowFullscreen] = useState(false);
  /** Whether the window was fullscreen when the current player session was entered. */
  const fullscreenAtPlayerEntryRef = useRef(false);
  const [playerPaused, setPlayerPaused] = useState(true);
  const contentRef = useRef<HTMLElement | null>(null);
  const selectedAnimeIdRef = useRef<number | null>(null);
  const currentPageKeyRef = useRef("categories");
  const pendingScrollRestorationRef = useRef<ScrollRestoration>("top");
  const scrollPositionsRef = useRef(new Map<string, number>());
  /** Set by `PlayerView` when a session is active; used to flush SQLite before `destroy()` on window close. */
  const playbackProgressFlushRef = useRef<(() => Promise<void>) | null>(null);

  const showToast = useCallback((kind: Toast["kind"], message: string) => {
    const id = Date.now() + Math.random();
    setToasts((current) => [...current, { id, kind, message }]);
  }, []);

  const reloadSearchIndex = useCallback(async () => {
    setAnimeSearchIndex(await getAnimeSearchIndex());
  }, []);

  const reloadLibrary = useCallback(async () => {
    const state = await getLibraryState();
    setLibrary(state);
    setSelectedCategoryId((current) => current ?? state.categories[0]?.id ?? null);
    return state;
  }, []);

  /** Library plus filename/title search index — skip on progress-only updates. */
  const reloadLibraryAndSearchIndex = useCallback(async () => {
    const [state, searchIndex] = await Promise.all([getLibraryState(), getAnimeSearchIndex()]);
    setLibrary(state);
    setAnimeSearchIndex(searchIndex);
    setSelectedCategoryId((current) => current ?? state.categories[0]?.id ?? null);
    return state;
  }, []);

  const reloadAnilistAuth = useCallback(async () => {
    const state = await getAnilistAuthState();
    setAnilistAuth(state);
    return state;
  }, []);

  const reloadLocalDataStats = useCallback(async () => {
    const stats = await getLocalDataStats();
    setLocalDataStats(stats);
    return stats;
  }, []);

  useEffect(() => {
    selectedAnimeIdRef.current = selectedAnime?.id ?? null;
  }, [selectedAnime?.id]);

  useEffect(() => {
    void (async () => {
      try {
        diagnosticLog("startup: loading library");
        const state = await reloadLibraryAndSearchIndex();
        diagnosticLog(
          `startup: library loaded (${state.anime.length} anime, ${state.root_folders.length} roots)`,
        );
        diagnosticLog("startup: loading AniList auth");
        await reloadAnilistAuth();
        diagnosticLog("startup: loading local data stats");
        await reloadLocalDataStats();
        if (state.root_folders.length > 0) {
          try {
            diagnosticLog("startup: rescan_library begin");
            const summary = await rescanLibrary();
            diagnosticLog(
              `startup: rescan_library ok (roots=${summary.roots_scanned}, imported=${summary.episodes_imported}, unmatched=${summary.unmatched_files})`,
            );
            await reloadLibraryAndSearchIndex();
            await reloadLocalDataStats();
          } catch (e) {
            const msg = errorMessage(e);
            diagnosticLog(`startup: rescan_library failed: ${msg}`, "ERROR");
            showToast("error", msg);
          }
        } else {
          diagnosticLog("startup: skipping rescan (no root folders)");
        }
        diagnosticLog("startup: complete");
      } catch (e) {
        const msg = errorMessage(e);
        diagnosticLog(`startup: fatal error: ${msg}`, "ERROR");
        setFatalError(msg);
      } finally {
        setLoading(false);
      }
    })();
  }, [reloadAnilistAuth, reloadLibraryAndSearchIndex, reloadLocalDataStats, showToast]);

  const handleAnilistCallback = useCallback(
    async (url: string) => {
      if (!url.startsWith("anime-player://anilist-auth")) return;
      setBusy(true);
      try {
        const state = await completeAnilistLogin(url);
        setAnilistAuth(state);
        showToast("success", `Logged in to AniList as ${state.viewer_name ?? "your account"}.`);
      } catch (e) {
        showToast("error", errorMessage(e));
      } finally {
        setBusy(false);
      }
    },
    [showToast],
  );

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    void (async () => {
      try {
        const currentUrls = await getCurrent();
        if (!cancelled) {
          currentUrls?.forEach((url) => void handleAnilistCallback(url));
        }
        unlisten = await onOpenUrl((urls) => {
          urls.forEach((url) => void handleAnilistCallback(url));
        });
        if (cancelled) unlisten();
      } catch (e) {
        if (!cancelled) showToast("error", errorMessage(e));
      }
    })();
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [handleAnilistCallback, showToast]);

  useEffect(() => {
    let unlisten: UnlistenFn | undefined;
    void (async () => {
      unlisten = await appWindow.onCloseRequested(async (event) => {
        const flush = playbackProgressFlushRef.current;
        if (!flush) return;
        event.preventDefault();
        try {
          await flush();
        } catch (e) {
          showToast("error", errorMessage(e));
        } finally {
          try {
            await appWindow.destroy();
          } catch (e) {
            showToast("error", errorMessage(e));
          }
        }
      });
    })();
    return () => {
      unlisten?.();
    };
  }, [showToast]);

  const pageKey = useMemo(() => {
    switch (view) {
      case "anime":
        return `anime:${selectedCategoryId ?? "none"}`;
      case "episodes":
        return `episodes:${selectedAnime?.id ?? "none"}:${episodeReturnView}`;
      case "player":
        return `player:${selectedEpisode?.id ?? "none"}`;
      case "missing":
        return "missing";
      case "bulkEdit":
        return "bulkEdit";
      case "jobs":
        return "jobs";
      default:
        return view;
    }
  }, [episodeReturnView, selectedAnime?.id, selectedCategoryId, selectedEpisode?.id, view]);

  const saveCurrentScrollPosition = useCallback(() => {
    const content = contentRef.current;
    const currentPageKey = currentPageKeyRef.current;
    if (!content || currentPageKey.startsWith("player:")) return;
    scrollPositionsRef.current.set(currentPageKey, content.scrollTop);
  }, []);

  const navigateToView = useCallback(
    (nextView: View, restoration: ScrollRestoration = "top") => {
      saveCurrentScrollPosition();
      pendingScrollRestorationRef.current = restoration;
      setView(nextView);
    },
    [saveCurrentScrollPosition],
  );

  useLayoutEffect(() => {
    const content = contentRef.current;
    if (!content) {
      currentPageKeyRef.current = pageKey;
      return;
    }

    if (view === "player") {
      currentPageKeyRef.current = pageKey;
      return;
    }

    const restoration = pendingScrollRestorationRef.current;
    const top = restoration === "restore" ? (scrollPositionsRef.current.get(pageKey) ?? 0) : 0;
    content.scrollTo({ top, left: 0, behavior: "auto" });
    currentPageKeyRef.current = pageKey;
    pendingScrollRestorationRef.current = "top";
  }, [pageKey, view]);

  useEffect(() => {
    let unlisten: UnlistenFn | undefined;
    let cancelled = false;
    void (async () => {
      try {
        setWindowFullscreen(await appWindow.isFullscreen());
      } catch {
        /* ignore */
      }
      if (cancelled) return;
      unlisten = await appWindow.onResized(async () => {
        try {
          setWindowFullscreen(await appWindow.isFullscreen());
        } catch {
          /* ignore */
        }
      });
    })();
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.repeat) return;
      if (e.code !== "F11") return;
      if (isTextInputTarget(e.target)) return;
      e.preventDefault();
      void (async () => {
        try {
          const next = !(await appWindow.isFullscreen());
          await appWindow.setFullscreen(next);
          setWindowFullscreen(next);
        } catch {
          /* ignore */
        }
      })();
    };
    window.addEventListener("keydown", onKeyDown, true);
    return () => window.removeEventListener("keydown", onKeyDown, true);
  }, []);

  const openSearch = useCallback(() => {
    navigateToView("search");
    setSearchFocusToken((current) => current + 1);
  }, [navigateToView]);

  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.repeat) return;
      if (!(e.ctrlKey || e.metaKey) || e.code !== "KeyF") return;
      e.preventDefault();
      openSearch();
    };
    window.addEventListener("keydown", onKeyDown, true);
    return () => window.removeEventListener("keydown", onKeyDown, true);
  }, [openSearch]);

  useEffect(() => {
    if (!(view === "player" && selectedEpisode)) {
      setPlayerControlsChromeVisible(true);
    }
  }, [view, selectedEpisode]);

  useEffect(() => {
    if (view === "missing" && library?.missing_anime.length === 0) {
      navigateToView("categories", "restore");
    }
  }, [library?.missing_anime.length, navigateToView, view]);

  const selectedCategory = useMemo(() => {
    if (!library || selectedCategoryId === null) return null;
    return library.categories.find((category) => category.id === selectedCategoryId) ?? null;
  }, [library, selectedCategoryId]);

  const animeInCategory = useMemo(() => {
    if (!library || selectedCategoryId === null) return [];
    return library.anime.filter((anime) => anime.category_id === selectedCategoryId);
  }, [library, selectedCategoryId]);

  const runAction = useCallback(
    async (action: () => Promise<string | void>) => {
      setBusy(true);
      try {
        const message = await action();
        if (message) showToast("success", message);
      } catch (e) {
        showToast("error", errorMessage(e));
      } finally {
        setBusy(false);
      }
    },
    [showToast],
  );

  const applyLocalAnilistProgress = useCallback(async (animeId: number) => {
    const result = await applyAnilistProgressToLocal(animeId);
    if (result) {
      setAnilistProgressUpdate({ animeId, progress: result.progress, updatedAt: Date.now() });
    }
    return result;
  }, []);

  /** Reload episode rows (and OP/ED flags) for one title without rebuilding the whole library. */
  const refreshAnimeEpisodes = useCallback(async (animeId: number) => {
    const [nextEpisodes, opEd] = await Promise.all([
      listEpisodes(animeId),
      getAnimeOpEdSummary(animeId),
    ]);
    if (selectedAnimeIdRef.current !== animeId) return;
    setEpisodes(nextEpisodes);
    setSelectedEpisode((current) =>
      current?.anime_id === animeId
        ? (nextEpisodes.find((episode) => episode.id === current.id) ?? current)
        : current,
    );
    setSelectedAnime((current) =>
      current?.id === animeId ? { ...current, no_op_ed: opEd.noOpEd } : current,
    );
  }, []);

  const refreshAnimePageData = useCallback(
    async (animeId: number) => {
      const [state, nextEpisodes] = await Promise.all([reloadLibrary(), listEpisodes(animeId)]);
      if (selectedAnimeIdRef.current === animeId) {
        setEpisodes(nextEpisodes);
        setSelectedEpisode((current) =>
          current?.anime_id === animeId ? (nextEpisodes.find((episode) => episode.id === current.id) ?? current) : current,
        );
        const updated = state.anime.find((anime) => anime.id === animeId);
        if (updated) setSelectedAnime(updated);
      }
      return state;
    },
    [reloadLibrary],
  );

  useEffect(() => {
    let unlisten: UnlistenFn | undefined;
    void listen("op-ed://analysis-updated", () => {
      const animeId = selectedAnimeIdRef.current;
      if (animeId != null && viewRef.current === "episodes") {
        void refreshAnimeEpisodes(animeId);
      }
    }).then((fn) => {
      unlisten = fn;
    });
    return () => {
      void unlisten?.();
    };
  }, [refreshAnimeEpisodes]);

  const openAnime = useCallback(
    async (anime: AnimeSummary, returnView: EpisodeReturnView = "anime") => {
      selectedAnimeIdRef.current = anime.id;
      setSelectedAnime(anime);
      setEpisodeReturnView(returnView);
      setEpisodes([]);
      setEpisodesLoading(true);
      navigateToView("episodes");
      try {
        const nextEpisodes = await listEpisodes(anime.id);
        setEpisodes(nextEpisodes);
        const shouldImportAnilistProgress =
          anime.anilist_id && nextEpisodes.length > 0 && nextEpisodes.every((episode) => !hasEpisodeProgress(episode));
        if (shouldImportAnilistProgress) {
          void (async () => {
            const result = await applyLocalAnilistProgress(anime.id);
            if (result?.updated_episodes) {
              await refreshAnimePageData(anime.id);
            }
          })().catch((e) => showToast("error", errorMessage(e)));
        }
      } catch (e) {
        showToast("error", errorMessage(e));
      } finally {
        setEpisodesLoading(false);
      }
    },
    [applyLocalAnilistProgress, navigateToView, refreshAnimePageData, showToast],
  );

  const handleAddRoot = useCallback(
    async (path: string) => {
      const trimmed = path.trim();
      if (!trimmed) {
        showToast("error", "Choose or paste a folder path first.");
        return;
      }
      await runAction(async () => {
        await addRootFolder(trimmed);
        setRootInput("");
        await reloadLibraryAndSearchIndex();
        return "Root folder added.";
      });
    },
    [reloadLibraryAndSearchIndex, runAction, showToast],
  );

  const handlePickFolder = useCallback(async () => {
    const picked = await open({ directory: true, multiple: false });
    if (typeof picked === "string" && picked) {
      setRootInput(picked);
      await handleAddRoot(picked);
    }
  }, [handleAddRoot]);

  const handleRemoveRoot = useCallback(
    async (root: RootFolder) => {
      await runAction(async () => {
        await removeRootFolder(root.id);
        await reloadLibraryAndSearchIndex();
        return "Root folder removed; episodes from that folder were dropped from the library.";
      });
    },
    [reloadLibraryAndSearchIndex, runAction],
  );

  const handleRescan = useCallback(async () => {
    await runAction(async () => {
      const summary = await rescanLibrary();
      const state = await reloadLibraryAndSearchIndex();
      if (view === "episodes" && selectedAnimeIdRef.current !== null) {
        const selectedId = selectedAnimeIdRef.current;
        const updated = state.anime.find((anime) => anime.id === selectedId);
        if (updated) {
          setSelectedAnime(updated);
          setEpisodes(await listEpisodes(selectedId));
        } else {
          setSelectedAnime(null);
          setEpisodes([]);
          const isMissing = state.missing_anime.some((anime) => anime.id === selectedId);
          navigateToView(isMissing ? "missing" : "categories", "restore");
        }
      }
      await reloadLocalDataStats();
      return `Scanned ${summary.roots_scanned} root folder${summary.roots_scanned === 1 ? "" : "s"}: ${summary.episodes_imported} episode${summary.episodes_imported === 1 ? "" : "s"} added or updated, ${summary.unmatched_files} unmatched.`;
    });
  }, [navigateToView, reloadLibraryAndSearchIndex, reloadLocalDataStats, runAction, view]);

  const handleCleanLocalData = useCallback(async () => {
    const confirmed = window.confirm(
      "Clean local data now? This removes database entries for episodes that are missing or no longer match your current detection rules, plus unused saved thumbnails and scrub sprites.",
    );
    if (!confirmed) return;

    await runAction(async () => {
      const summary = await cleanLocalData();
      await reloadLibraryAndSearchIndex();
      await reloadLocalDataStats();
      const staleEpisodes = `${summary.stale_episodes_removed} stale episode${summary.stale_episodes_removed === 1 ? "" : "s"}`;
      const emptyAnime = `${summary.empty_anime_removed} empty title entr${summary.empty_anime_removed === 1 ? "y" : "ies"}`;
      const thumbnails = `${summary.thumbnails_removed} unused thumbnail${summary.thumbnails_removed === 1 ? "" : "s"}`;
      const scrubSprites = `${summary.scrub_sprites_removed} unused scrub sprite${summary.scrub_sprites_removed === 1 ? "" : "s"}`;
      const opEdFp = `${summary.op_ed_fingerprints_removed} unused OP/ED fingerprint${summary.op_ed_fingerprints_removed === 1 ? "" : "s"}`;
      return `Cleaned local data: removed ${staleEpisodes}, ${emptyAnime}, ${thumbnails}, ${scrubSprites}, and ${opEdFp}.`;
    });
  }, [reloadLibraryAndSearchIndex, reloadLocalDataStats, runAction]);

  const handleCreateCategory = useCallback(async () => {
    const name = newCategoryName.trim();
    if (!name) return;
    await runAction(async () => {
      const category = await createCategory(name);
      setNewCategoryName("");
      setSelectedCategoryId(category.id);
      await reloadLibrary();
      return "Category created.";
    });
  }, [newCategoryName, reloadLibrary, runAction]);

  const handleDeleteCategory = useCallback(
    async (category: Category) => {
      await runAction(async () => {
        await deleteCategory(category.id);
        const state = await reloadLibrary();
        setSelectedCategoryId(state.categories[0]?.id ?? null);
        return "Category deleted. Titles were moved to the default category.";
      });
    },
    [reloadLibrary, runAction],
  );

  const handleSetDefaultCategory = useCallback(
    async (category: Category) => {
      await runAction(async () => {
        await setDefaultCategory(category.id);
        await reloadLibrary();
        return `"${category.name}" is now the default category.`;
      });
    },
    [reloadLibrary, runAction],
  );

  const handleReorderCategory = useCallback(
    async (category: Category, direction: "up" | "down") => {
      if (!library) return;
      const currentIndex = library.categories.findIndex((item) => item.id === category.id);
      if (currentIndex < 0) return;
      const targetIndex = direction === "up" ? currentIndex - 1 : currentIndex + 1;
      if (targetIndex < 0 || targetIndex >= library.categories.length) return;

      const reordered = [...library.categories];
      const [moved] = reordered.splice(currentIndex, 1);
      reordered.splice(targetIndex, 0, moved);
      const categoryIds = reordered.map((item) => item.id);

      await runAction(async () => {
        await reorderCategories(categoryIds);
        await reloadLibrary();
        return "Category order updated.";
      });
    },
    [library, reloadLibrary, runAction],
  );

  const handleCreateRule = useCallback(
    async (input: RegexRuleInput) => {
      await runAction(async () => {
        await createRegexRule(input);
        await reloadLibrary();
        setNewRuleEditorKey((k) => k + 1);
        return "Detection rule added.";
      });
    },
    [reloadLibrary, runAction],
  );

  const handleUpdateRule = useCallback(
    async (id: number, input: RegexRuleInput) => {
      await runAction(async () => {
        await updateRegexRule(id, input);
        await reloadLibrary();
        return "Detection rule updated.";
      });
    },
    [reloadLibrary, runAction],
  );

  const handleDeleteRule = useCallback(
    async (rule: RegexRule) => {
      await runAction(async () => {
        await deleteRegexRule(rule.id);
        await reloadLibrary();
        return "Detection rule deleted.";
      });
    },
    [reloadLibrary, runAction],
  );

  const handleResetRegexRules = useCallback(async () => {
    const confirmed = window.confirm(
      "Replace all detection rules with the built-in defaults? Custom rules will be removed.",
    );
    if (!confirmed) return;

    await runAction(async () => {
      await resetRegexRulesToDefaults();
      await reloadLibrary();
      setNewRuleEditorKey((k) => k + 1);
      return "Detection rules reset to defaults.";
    });
  }, [reloadLibrary, runAction]);

  const handleMoveAnime = useCallback(
    async (categoryId: number) => {
      if (!selectedAnime) return;
      await runAction(async () => {
        await moveAnimeToCategory(selectedAnime.id, categoryId);
        const state = await reloadLibrary();
        const updated = state.anime.find((anime) => anime.id === selectedAnime.id);
        if (updated) setSelectedAnime(updated);
      });
    },
    [reloadLibrary, runAction, selectedAnime],
  );

  const handleOpenEpisodeFolder = useCallback(async () => {
    if (!selectedAnime) return;
    if (episodes.length === 0) {
      showToast("error", "No episode folder is available.");
      return;
    }
    try {
      await openAnimeEpisodeFolder(selectedAnime.id);
    } catch (e) {
      showToast("error", errorMessage(e));
    }
  }, [episodes.length, selectedAnime, showToast]);

  const handleDeleteSelectedAnimeFiles = useCallback(async () => {
    if (!selectedAnime) return;
    if (episodes.length === 0) {
      showToast("error", "There are no visible episode files to delete.");
      return;
    }

    setBusy(true);
    try {
      const deletingLoadedEpisode = selectedEpisode?.anime_id === selectedAnime.id;
      if (deletingLoadedEpisode) {
        await stopMpv().catch(() => undefined);
        setSelectedEpisode(null);
      }

      const summary = await deleteAnimeFiles(selectedAnime.id);
      await reloadLibraryAndSearchIndex();
      await reloadLocalDataStats();
      setEpisodes([]);
      setSelectedAnime(null);
      navigateToView(episodeReturnView, "restore");

      const deleted = `${summary.episodes_deleted} episode file${summary.episodes_deleted === 1 ? "" : "s"}`;
      const bytes = summary.bytes_deleted > 0 ? ` (${formatSize(summary.bytes_deleted)})` : "";
      const cover = summary.cover_deleted ? " Cached cover removed." : "";
      const coverFailure = summary.cover_failed ? " Cached cover could not be removed." : "";
      const permanent = summary.permanent_delete_used ? " Some files could not be trashed and were deleted permanently." : "";
      if (summary.episodes_failed > 0 || summary.cover_failed) {
        const episodeFailure = summary.episodes_failed > 0 ? `; ${summary.episodes_failed} failed` : "";
        showToast("error", `Deleted ${deleted}${bytes}${episodeFailure}.${cover}${coverFailure}${permanent}`);
      } else {
        showToast("success", `Deleted ${deleted}${bytes}.${cover}${permanent}`);
      }
    } catch (e) {
      showToast("error", errorMessage(e));
    } finally {
      setBusy(false);
    }
  }, [
    episodeReturnView,
    episodes.length,
    navigateToView,
    reloadLibraryAndSearchIndex,
    reloadLocalDataStats,
    selectedAnime,
    selectedEpisode?.anime_id,
    showToast,
  ]);

  const handleBulkMoveAnime = useCallback(
    async (animeIds: number[], categoryId: number) => {
      if (animeIds.length === 0) return;
      await runAction(async () => {
        for (const animeId of animeIds) {
          await moveAnimeToCategory(animeId, categoryId);
        }
        const state = await reloadLibrary();
        if (selectedAnimeIdRef.current !== null) {
          const updated = state.anime.find((anime) => anime.id === selectedAnimeIdRef.current);
          if (updated) setSelectedAnime(updated);
        }
        return `Moved ${animeIds.length} title${animeIds.length === 1 ? "" : "s"} to a new category.`;
      });
    },
    [reloadLibrary, runAction],
  );

  const handleRenameFiles = useCallback(
    async (renames: RenameFileRequest[]) => {
      if (renames.length === 0) return;
      await runAction(async () => {
        const summary = await renameFiles(renames);
        await rescanLibrary();
        const state = await reloadLibraryAndSearchIndex();
        if (selectedAnimeIdRef.current !== null) {
          const updated = state.anime.find((anime) => anime.id === selectedAnimeIdRef.current);
          if (updated) {
            setSelectedAnime(updated);
            setEpisodes(await listEpisodes(updated.id));
          }
        }
        return `Renamed ${summary.files_renamed} file${summary.files_renamed === 1 ? "" : "s"}.`;
      });
    },
    [reloadLibraryAndSearchIndex, runAction],
  );

  const handleSaveAnilistClientId = useCallback(
    async (clientId: string) => {
      await runAction(async () => {
        const state = await setAnilistClientId(clientId);
        setAnilistAuth(state);
        return clientId.trim() ? "AniList client ID saved." : "AniList client ID reset to the default.";
      });
    },
    [runAction],
  );

  const handleLoginAnilist = useCallback(async () => {
    await runAction(async () => {
      const url = await getAnilistLoginUrl();
      await openUrl(url);
    });
  }, [runAction]);

  const handleLogoutAnilist = useCallback(async () => {
    await runAction(async () => {
      setAnilistAuth(await logoutAnilist());
      return "Logged out of AniList.";
    });
  }, [runAction]);

  const handlePreferAnilistDisplayTitle = useCallback(
    async (enabled: boolean) => {
      const state = await setPreferAnilistDisplayTitle(enabled);
      setLibrary(state);
    },
    [],
  );

  const handleSkipOpEd = useCallback(async (enabled: boolean) => {
    const state = await setSkipOpEd(enabled);
    setLibrary(state);
  }, []);

  const handleSearchAnilist = useCallback((query: string): Promise<AnilistSearchResult[]> => {
    return searchAnilistAnime(query);
  }, []);

  const handleGetAnilistStatus = useCallback((animeId: number): Promise<AnilistMediaStatus | null> => {
    return getAnilistMediaStatus(animeId);
  }, []);

  const handleSetAnilistScore = useCallback((animeId: number, score: number | null): Promise<AnilistMediaStatus> => {
    return setAnilistMediaScore(animeId, score);
  }, []);

  const handleSaveAnimeSettings = useCallback(
    async (
      animeId: number,
      title: string,
      trackerOffset: number,
      progressOverride: number | null,
      customThumbnailPath: string | null,
    ) => {
      const currentAnime = selectedAnime?.id === animeId ? selectedAnime : library?.anime.find((anime) => anime.id === animeId);
      let didMutate = false;
      let searchFieldsChanged = false;
      if (currentAnime && title.trim() !== currentAnime.title) {
        await stopMpv().catch(() => undefined);
        if (selectedEpisode?.anime_id === animeId) {
          setSelectedEpisode(null);
        }
        await renameAnime(animeId, title);
        didMutate = true;
        searchFieldsChanged = true;
      }
      if (!currentAnime || trackerOffset !== currentAnime.tracker_offset) {
        await setAnimeTrackerOffset(animeId, trackerOffset);
        didMutate = true;
      }
      const prevThumbnail = currentAnime?.custom_thumbnail_path ?? null;
      if (!currentAnime || customThumbnailPath !== prevThumbnail) {
        await setAnimeCustomThumbnailPath(animeId, customThumbnailPath);
        didMutate = true;
      }
      if (progressOverride !== null) {
        const result = await overrideAnimeProgress(animeId, progressOverride);
        const linkedAnime =
          selectedAnime?.id === animeId ? selectedAnime : library?.anime.find((anime) => anime.id === animeId);
        if (linkedAnime?.anilist_id) {
          const status = await setAnilistMediaProgress(animeId, result.progress);
          setAnilistProgressUpdate({
            animeId,
            progress: status.progress ?? result.progress,
            forceReplace: true,
            updatedAt: Date.now(),
          });
        }
        didMutate = true;
      }
      if (didMutate) {
        await refreshAnimePageData(animeId);
        if (searchFieldsChanged) await reloadSearchIndex();
      }
    },
    [library?.anime, refreshAnimePageData, reloadSearchIndex, selectedAnime, selectedEpisode?.anime_id],
  );

  const handleClearAnimeCustomThumbnail = useCallback(
    async (animeId: number) => {
      await setAnimeCustomThumbnailPath(animeId, null);
      await refreshAnimePageData(animeId);
    },
    [refreshAnimePageData],
  );

  const handleAnilistProgressSynced = useCallback((animeId: number, result: AnilistProgressSyncResult) => {
    const progress = result.remote_progress ?? result.target_progress;
    if (progress === null) return;
    setAnilistProgressUpdate({ animeId, progress, updatedAt: Date.now() });
  }, []);

  const handleLinkAnilist = useCallback(
    async (animeId: number, anilistId: number) => {
      await runAction(async () => {
        await linkAnimeAnilist(animeId, anilistId);
        const result = await applyLocalAnilistProgress(animeId);
        if (result?.updated_episodes) {
          await refreshAnimePageData(animeId);
          await reloadSearchIndex();
        } else {
          const state = await reloadLibraryAndSearchIndex();
          const updated = state.anime.find((anime) => anime.id === animeId);
          if (updated) setSelectedAnime(updated);
        }
      });
    },
    [applyLocalAnilistProgress, refreshAnimePageData, reloadLibraryAndSearchIndex, reloadSearchIndex, runAction],
  );

  const handleUnlinkAnilist = useCallback(
    async (animeId: number) => {
      await runAction(async () => {
        await unlinkAnimeAnilist(animeId);
        const state = await reloadLibraryAndSearchIndex();
        const updated = state.anime.find((anime) => anime.id === animeId);
        if (updated) setSelectedAnime(updated);
        return "Title unlinked from AniList.";
      });
    },
    [reloadLibraryAndSearchIndex, runAction],
  );

  const handleOpenAnilist = useCallback(async (url: string) => {
    await openUrl(url);
  }, []);

  const handleProgressSaved = useCallback(
    (saved: Episode) => {
      setEpisodes((current) => {
        const next = current.map((episode) => {
          if (episode.id !== saved.id) return episode;
          const opEdSegments =
            saved.op_ed_segments.length > 0 ? saved.op_ed_segments : episode.op_ed_segments;
          return { ...saved, op_ed_segments: opEdSegments };
        });
        setSelectedAnime((anime) =>
          anime && anime.id === saved.anime_id
            ? { ...anime, unwatched_count: next.filter((episode) => !episode.watched).length }
            : anime,
        );
        return next;
      });
      void reloadLibrary().catch((e) => showToast("error", errorMessage(e)));
    },
    [reloadLibrary, showToast],
  );

  const navigateToCategory = useCallback((categoryId: number) => {
    setSelectedCategoryId(categoryId);
    setSelectedAnime(null);
    setSelectedEpisode(null);
    setEpisodes([]);
    navigateToView("anime");
  }, [navigateToView]);

  // Cover the screen with an opaque fade, run `between` to swap views/state,
  // then uncover. PlayerView's own .player-load-fade keeps the screen black
  // for fresh playback after uncover until mpv emits playback-restart, so the
  // hand-off is seamless without us coordinating with mpv directly.
  const captureFullscreenAtPlayerEntry = useCallback(async () => {
    try {
      fullscreenAtPlayerEntryRef.current = await appWindow.isFullscreen();
    } catch {
      fullscreenAtPlayerEntryRef.current = false;
    }
  }, []);

  const restoreFullscreenAfterPlayerIfNeeded = useCallback(async () => {
    try {
      const enteredFullscreen = fullscreenAtPlayerEntryRef.current;
      const currentFullscreen = await appWindow.isFullscreen();
      if (!enteredFullscreen && currentFullscreen) {
        await appWindow.setFullscreen(false);
        setWindowFullscreen(false);
      }
    } catch {
      /* ignore */
    }
  }, []);

  const runScreenTransition = useCallback(async (between: () => void) => {
    if (screenTransitionRef.current !== "idle") return;
    screenTransitionRef.current = "cover";
    setScreenTransition("cover");
    await waitMs(SCREEN_COVER_MS);
    between();
    await waitTwoFrames();
    screenTransitionRef.current = "uncover";
    setScreenTransition("uncover");
    await waitMs(SCREEN_UNCOVER_MS);
    screenTransitionRef.current = "idle";
    setScreenTransition("idle");
  }, []);

  const openEpisode = useCallback(
    (episode: Episode) => {
      void (async () => {
        await captureFullscreenAtPlayerEntry();
        await runScreenTransition(() => {
          setSelectedEpisode(episode);
          navigateToView("player");
        });
      })();
    },
    [captureFullscreenAtPlayerEntry, navigateToView, runScreenTransition],
  );

  const closePlayer = useCallback(
    (options?: { unload?: boolean }) => {
      const unload = options?.unload === true;
      void (async () => {
        await restoreFullscreenAfterPlayerIfNeeded();
        await runScreenTransition(() => {
          if (unload) setSelectedEpisode(null);
          navigateToView("episodes", "restore");
        });
      })();
    },
    [navigateToView, restoreFullscreenAfterPlayerIfNeeded, runScreenTransition],
  );

  // Q on the episodes screen jumps into the current anime's last-played episode
  // (or the next one if that episode is already watched). Scoped to the
  // episodes view so PlayerView keeps owning Q while playback is visible.
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.repeat || e.code !== "KeyQ") return;
      if (isTextInputTarget(e.target)) return;
      if (view !== "episodes" || !selectedAnime) return;
      const target = pickQuickPlayEpisode(episodes);
      if (!target) return;
      e.preventDefault();
      if (selectedEpisode && selectedEpisode.id === target.id) {
        // Same file already loaded; transition through the fade anyway so the
        // reveal feels consistent with a fresh play. PlayerView's `visible`
        // effect handles the unpause once we navigate.
        void (async () => {
          await captureFullscreenAtPlayerEntry();
          await runScreenTransition(() => navigateToView("player"));
        })();
      } else {
        openEpisode(target);
      }
    };
    window.addEventListener("keydown", onKeyDown, true);
    return () => window.removeEventListener("keydown", onKeyDown, true);
  }, [
    captureFullscreenAtPlayerEntry,
    episodes,
    navigateToView,
    openEpisode,
    runScreenTransition,
    selectedAnime,
    selectedEpisode,
    view,
  ]);

  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.repeat || e.code !== "Escape") return;
      if (view === "search" && searchQuery.trim()) {
        e.preventDefault();
        setSearchQuery("");
        return;
      }
      if (isTextInputTarget(e.target)) return;
      if (view === "player") return;
      if (view === "categories") return;
      e.preventDefault();
      if (view === "anime") {
        navigateToView("categories", "restore");
        return;
      }
      if (view === "search") {
        navigateToView("categories", "restore");
        return;
      }
      if (view === "bulkEdit") {
        navigateToView("categories", "restore");
        return;
      }
      if (view === "missing") {
        navigateToView("categories", "restore");
        return;
      }
      if (view === "episodes" && selectedAnime) {
        navigateToView(episodeReturnView, "restore");
        return;
      }
      if (view === "settings" || view === "jobs") {
        navigateToView("categories", "restore");
      }
    };
    window.addEventListener("keydown", onKeyDown, true);
    return () => window.removeEventListener("keydown", onKeyDown, true);
  }, [episodeReturnView, navigateToView, searchQuery, selectedAnime, view]);

  const showPlayer = view === "player" && Boolean(selectedEpisode);

  const osTitleAnimeLabel = useMemo(() => {
    if (!selectedEpisode || !library) return null;
    const anime =
      selectedAnime?.id === selectedEpisode.anime_id
        ? selectedAnime
        : library.anime.find((a) => a.id === selectedEpisode.anime_id);
    if (!anime) return null;
    const label = animeDisplayTitle(anime, library.prefer_anilist_display_title).trim();
    return label || null;
  }, [library, selectedAnime, selectedEpisode]);

  useEffect(() => {
    if (!showPlayer) {
      setPlayerPaused(true);
    }
  }, [showPlayer]);

  useEffect(() => {
    if (!showPlayer) {
      void appWindow.setTitle(APP_WINDOW_TITLE).catch(() => {});
      return;
    }
    const raw = osTitleAnimeLabel ?? selectedEpisode?.file_name ?? "Title";
    const short = shortenForOsTitle(raw);
    const prefix = playerPaused ? "Paused" : "Playing";
    void appWindow.setTitle(`${prefix} - ${short} - ${APP_WINDOW_TITLE}`).catch(() => {});
  }, [osTitleAnimeLabel, playerPaused, selectedEpisode, showPlayer]);

  const handlePlayerPausedChange = useCallback((paused: boolean) => {
    setPlayerPaused(paused);
  }, []);

  if (loading) {
    return (
      <main className="app app--loading">
        {!windowFullscreen ? (
          <WindowTitleBar playerOpen={false} playerControlsChromeVisible />
        ) : null}
        <div className="empty">
          <h2>Loading library...</h2>
        </div>
      </main>
    );
  }

  if (!library) {
    return (
      <main className="app app--loading">
        {!windowFullscreen ? (
          <WindowTitleBar playerOpen={false} playerControlsChromeVisible />
        ) : null}
        <div className="empty">
          <h2>Library failed to load</h2>
          {fatalError ? <p className="muted">{fatalError}</p> : null}
        </div>
      </main>
    );
  }

  const playerLoadedInBackground = selectedEpisode && !showPlayer;
  const libraryViewActive =
    view === "categories" ||
    view === "anime" ||
    (view === "episodes" && episodeReturnView !== "search" && episodeReturnView !== "bulkEdit");
  const searchViewActive = view === "search" || (view === "episodes" && episodeReturnView === "search");
  const bulkEditViewActive = view === "bulkEdit" || (view === "episodes" && episodeReturnView === "bulkEdit");
  const missingViewActive = view === "missing";

  return (
    <main
      className={`app${showPlayer ? " app--player-open" : ""}${
        playerLoadedInBackground ? " app--player-background" : ""
      }`}
    >
      {!windowFullscreen ? (
        <WindowTitleBar
          playerOpen={Boolean(showPlayer)}
          playerControlsChromeVisible={playerControlsChromeVisible}
        />
      ) : null}
      <aside className="sidebar">
        <nav className="nav-list" aria-label="Primary navigation">
          <button
            type="button"
            className={libraryViewActive ? "nav-item active" : "nav-item"}
            onClick={() => navigateToView("categories")}
            aria-label="Library"
            title="Library"
          >
            <LibraryIcon />
            <span className="nav-label">Library</span>
          </button>
          <button
            type="button"
            className={searchViewActive ? "nav-item active" : "nav-item"}
            onClick={openSearch}
            aria-label="Search"
            title="Search (Ctrl+F)"
          >
            <SearchIcon />
            <span className="nav-label">Search</span>
          </button>
          <button
            type="button"
            className={bulkEditViewActive ? "nav-item active" : "nav-item"}
            onClick={() => navigateToView("bulkEdit")}
            aria-label="Bulk edit"
            title="Bulk edit"
          >
            <BulkEditIcon />
            <span className="nav-label">Bulk edit</span>
          </button>
          {library.missing_anime.length > 0 ? (
            <button
              type="button"
              className={missingViewActive ? "nav-item active" : "nav-item"}
              onClick={() => navigateToView("missing")}
              aria-label="Missing"
              title="Missing"
            >
              <MissingIcon />
              <span className="nav-label">Missing</span>
            </button>
          ) : null}
          <button
            type="button"
            className={view === "settings" ? "nav-item active" : "nav-item"}
            onClick={() => navigateToView("settings")}
            aria-label="Settings"
            title="Settings"
          >
            <SettingsIcon />
            <span className="nav-label">Settings</span>
          </button>
          <button
            type="button"
            className={view === "jobs" ? "nav-item active" : "nav-item"}
            onClick={() => navigateToView("jobs")}
            aria-label="Background jobs"
            title="Background jobs"
          >
            <JobsIcon />
            <span className="nav-label">Jobs</span>
            {jobsActiveCount > 0 ?
              <span className="nav-item-badge" aria-hidden>
                {jobsActiveCount > 99 ? "99+" : jobsActiveCount}
              </span>
            : null}
          </button>
          <button
            type="button"
            className="nav-item nav-item--action"
            onClick={() => void handleRescan()}
            disabled={busy}
            aria-label={busy ? "Rescanning library" : "Rescan library"}
            title={busy ? "Rescanning library" : "Rescan library"}
          >
            <RescanIcon />
            <span className="nav-label">{busy ? "Working..." : "Rescan library"}</span>
          </button>
        </nav>
      </aside>

      {selectedEpisode ? (
        <PlayerView
          episode={selectedEpisode}
          anime={
            selectedAnime?.id === selectedEpisode.anime_id
              ? selectedAnime
              : library.anime.find((a) => a.id === selectedEpisode.anime_id) ?? null
          }
          preferAnilistDisplayTitle={library.prefer_anilist_display_title}
          skipOpEdEnabled={library.skip_op_ed}
          onSkipOpEdEnabledChange={(enabled) => void handleSkipOpEd(enabled)}
          playlist={episodes}
          visible={Boolean(showPlayer)}
          playbackProgressFlushRef={playbackProgressFlushRef}
          onSelectEpisode={setSelectedEpisode}
          onBack={() => closePlayer()}
          onClose={() => closePlayer({ unload: true })}
          onProgressSaved={handleProgressSaved}
          onAnilistProgressSynced={handleAnilistProgressSynced}
          onError={(message) => showToast("error", message)}
          onControlsVisibilityChange={setPlayerControlsChromeVisible}
          onPausedStateChange={handlePlayerPausedChange}
        />
      ) : null}

      <section className="content" ref={contentRef}>
        <div className="content-inner">
          {view === "categories" ? (
            <CategoryScreen
              library={library}
              onOpenCategory={navigateToCategory}
              onOpenAnime={(anime) => void openAnime(anime, "categories")}
              onOpenSettings={() => navigateToView("settings")}
            />
          ) : null}

          {view === "anime" ? (
            <AnimeGrid
              category={selectedCategory}
              anime={animeInCategory}
              preferAnilistDisplayTitle={library.prefer_anilist_display_title}
              onBack={() => navigateToView("categories", "restore")}
              onOpenAnime={(anime) => void openAnime(anime, "anime")}
              onOpenSettings={() => navigateToView("settings")}
            />
          ) : null}

          {view === "search" ? (
            <SearchScreen
              anime={library.anime}
              searchIndex={animeSearchIndex}
              preferAnilistDisplayTitle={library.prefer_anilist_display_title}
              query={searchQuery}
              focusToken={searchFocusToken}
              onQueryChange={setSearchQuery}
              onOpenAnime={(anime) => void openAnime(anime, "search")}
            />
          ) : null}

          {bulkEditViewActive ? (
            <div hidden={view !== "bulkEdit"}>
              <BulkEditScreen
                library={library}
                busy={busy}
                onOpenAnime={(anime) => void openAnime(anime, "bulkEdit")}
                onListEpisodes={listEpisodes}
                onListRootVideoFiles={listRootVideoFiles}
                onMoveAnime={(animeIds, categoryId) => void handleBulkMoveAnime(animeIds, categoryId)}
                onValidateRenames={validateFileRenames}
                onRenameFiles={(renames) => void handleRenameFiles(renames)}
              />
            </div>
          ) : null}

          {view === "missing" ? (
            <MissingScreen
              anime={library.missing_anime}
              preferAnilistDisplayTitle={library.prefer_anilist_display_title}
            />
          ) : null}

          {view === "episodes" && selectedAnime ? (
            <EpisodeScreen
              anime={selectedAnime}
              episodes={episodes}
              episodesLoading={episodesLoading}
              categories={library.categories}
              onBack={() => navigateToView(episodeReturnView, "restore")}
              onPlay={openEpisode}
              onMoveAnime={(categoryId) => void handleMoveAnime(categoryId)}
              onOpenEpisodeFolder={() => void handleOpenEpisodeFolder()}
              onDeleteAnime={() => void handleDeleteSelectedAnimeFiles()}
              onSearchAnilist={handleSearchAnilist}
              onGetAnilistStatus={handleGetAnilistStatus}
              onSetAnilistScore={handleSetAnilistScore}
              onSaveAnimeSettings={handleSaveAnimeSettings}
              onClearAnimeCustomThumbnail={handleClearAnimeCustomThumbnail}
              anilistProgressUpdate={anilistProgressUpdate}
              onLinkAnilist={(animeId, anilistId) => void handleLinkAnilist(animeId, anilistId)}
              onUnlinkAnilist={(animeId) => void handleUnlinkAnilist(animeId)}
              onOpenAnilist={(url) => void handleOpenAnilist(url)}
              anilistConnected={isAnilistConnected(anilistAuth)}
              preferAnilistDisplayTitle={library.prefer_anilist_display_title}
              onOpEdAnalysisUpdated={() => {
                if (selectedAnime) void refreshAnimeEpisodes(selectedAnime.id);
              }}
            />
          ) : null}

          {view === "jobs" ? (
            <JobsScreen
              onBack={() => navigateToView("categories", "restore")}
              onError={(message) => showToast("error", message)}
            />
          ) : null}

          {view === "settings" ? (
            <SettingsScreen
              library={library}
              busy={busy}
              rootInput={rootInput}
              newCategoryName={newCategoryName}
              newRuleEditorKey={newRuleEditorKey}
              anilistAuth={anilistAuth}
              localDataStats={localDataStats}
              onBack={() => navigateToView("categories", "restore")}
              onRootInput={setRootInput}
              onPickFolder={() => void handlePickFolder()}
              onAddRoot={() => void handleAddRoot(rootInput)}
              onRemoveRoot={(root) => void handleRemoveRoot(root)}
              onRescan={() => void handleRescan()}
              onNewCategoryName={setNewCategoryName}
              onCreateCategory={() => void handleCreateCategory()}
              onDeleteCategory={(category) => void handleDeleteCategory(category)}
              onSetDefaultCategory={(category) => void handleSetDefaultCategory(category)}
              onReorderCategory={(category, direction) => void handleReorderCategory(category, direction)}
              onCreateRule={(input) => void handleCreateRule(input)}
              onUpdateRule={(id, input) => void handleUpdateRule(id, input)}
              onDeleteRule={(rule) => void handleDeleteRule(rule)}
              onResetRegexRules={() => void handleResetRegexRules()}
              onSaveAnilistClientId={(clientId) => void handleSaveAnilistClientId(clientId)}
              onLoginAnilist={() => void handleLoginAnilist()}
              onLogoutAnilist={() => void handleLogoutAnilist()}
              onPreferAnilistDisplayTitle={(enabled) => void handlePreferAnilistDisplayTitle(enabled)}
              onSkipOpEd={(enabled) => void handleSkipOpEd(enabled)}
              onCleanLocalData={() => void handleCleanLocalData()}
            />
          ) : null}
        </div>
      </section>

      <div className="screen-transition-overlay" data-state={screenTransition} aria-hidden />
      <ToastStack toasts={toasts} onDismiss={(id) => setToasts((current) => current.filter((toast) => toast.id !== id))} />
    </main>
  );
}

export default App;
