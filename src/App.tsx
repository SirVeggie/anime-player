import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { getCurrent, onOpenUrl } from "@tauri-apps/plugin-deep-link";
import { openUrl } from "@tauri-apps/plugin-opener";
import { type UnlistenFn } from "@tauri-apps/api/event";
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
  getAnilistAuthState,
  getAnilistLoginUrl,
  getAnilistMediaStatus,
  getLibraryState,
  getLocalDataStats,
  linkAnimeAnilist,
  listEpisodes,
  logoutAnilist,
  moveAnimeToCategory,
  openAnimeEpisodeFolder,
  removeRootFolder,
  rescanLibrary,
  searchAnilistAnime,
  setDefaultCategory,
  setAnilistClientId,
  setAnilistMediaScore,
  stopMpv,
  unlinkAnimeAnilist,
  updateRegexRule,
} from "./api";
import { AnimeGrid } from "./components/AnimeGrid";
import { BulkEditScreen } from "./components/BulkEditScreen";
import { CategoryScreen } from "./components/CategoryScreen";
import { EpisodeScreen } from "./components/EpisodeScreen";
import { BulkEditIcon, LibraryIcon, MissingIcon, RescanIcon, SearchIcon, SettingsIcon } from "./components/Icons";
import { MissingScreen } from "./components/MissingScreen";
import { PlayerView } from "./components/PlayerView";
import { SearchScreen } from "./components/SearchScreen";
import { SettingsScreen } from "./components/SettingsScreen";
import { type Toast, ToastStack } from "./components/ToastStack";
import { WindowTitleBar } from "./components/WindowTitleBar";
import { pickQuickPlayEpisode } from "./quickPlay";
import type {
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
  RootFolder,
} from "./types";
import { APP_WINDOW_TITLE, errorMessage, formatSize, isTextInputTarget, shortenForOsTitle } from "./utils";
import "./App.css";

type View = "categories" | "anime" | "search" | "bulkEdit" | "missing" | "episodes" | "settings" | "player";
type EpisodeReturnView = "anime" | "search" | "bulkEdit" | "categories";
type ScrollRestoration = "top" | "restore";

type AnilistProgressUpdate = {
  animeId: number;
  progress: number;
  updatedAt: number;
};

const VIDEO_OPEN_FADE_MS = 180;
const appWindow = getCurrentWindow();

function hasEpisodeProgress(episode: Episode): boolean {
  return episode.watched || episode.position_seconds > 0;
}

function App() {
  const [library, setLibrary] = useState<LibraryState | null>(null);
  const [view, setView] = useState<View>("categories");
  const [selectedCategoryId, setSelectedCategoryId] = useState<number | null>(null);
  const [selectedAnime, setSelectedAnime] = useState<AnimeSummary | null>(null);
  const [episodeReturnView, setEpisodeReturnView] = useState<EpisodeReturnView>("anime");
  const [anilistAuth, setAnilistAuth] = useState<AnilistAuthState | null>(null);
  const [localDataStats, setLocalDataStats] = useState<LocalDataStats | null>(null);
  const [episodes, setEpisodes] = useState<Episode[]>([]);
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
  const [videoOpening, setVideoOpening] = useState(false);
  const [playerControlsChromeVisible, setPlayerControlsChromeVisible] = useState(true);
  const [windowFullscreen, setWindowFullscreen] = useState(false);
  const [playerPaused, setPlayerPaused] = useState(true);
  const contentRef = useRef<HTMLElement | null>(null);
  const selectedAnimeIdRef = useRef<number | null>(null);
  const currentPageKeyRef = useRef("categories");
  const pendingScrollRestorationRef = useRef<ScrollRestoration>("top");
  const scrollPositionsRef = useRef(new Map<string, number>());

  const showToast = useCallback((kind: Toast["kind"], message: string) => {
    const id = Date.now() + Math.random();
    setToasts((current) => [...current, { id, kind, message }]);
    window.setTimeout(() => {
      setToasts((current) => current.filter((toast) => toast.id !== id));
    }, 4200);
  }, []);

  const reloadLibrary = useCallback(async () => {
    const state = await getLibraryState();
    setLibrary(state);
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
        const state = await reloadLibrary();
        await reloadAnilistAuth();
        await reloadLocalDataStats();
        if (state.root_folders.length > 0) {
          try {
            await rescanLibrary();
            await reloadLibrary();
            await reloadLocalDataStats();
          } catch (e) {
            showToast("error", errorMessage(e));
          }
        }
      } catch (e) {
        setFatalError(errorMessage(e));
      } finally {
        setLoading(false);
      }
    })();
  }, [reloadAnilistAuth, reloadLibrary, reloadLocalDataStats, showToast]);

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

  const refreshAnimePageData = useCallback(
    async (animeId: number) => {
      const [state, nextEpisodes] = await Promise.all([reloadLibrary(), listEpisodes(animeId)]);
      if (selectedAnimeIdRef.current === animeId) {
        setEpisodes(nextEpisodes);
        const updated = state.anime.find((anime) => anime.id === animeId);
        if (updated) setSelectedAnime(updated);
      }
      return state;
    },
    [reloadLibrary],
  );

  const openAnime = useCallback(
    async (anime: AnimeSummary, returnView: EpisodeReturnView = "anime") => {
      selectedAnimeIdRef.current = anime.id;
      setSelectedAnime(anime);
      setEpisodeReturnView(returnView);
      try {
        const nextEpisodes = await listEpisodes(anime.id);
        setEpisodes(nextEpisodes);
        navigateToView("episodes");
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
        await reloadLibrary();
        return "Root folder added.";
      });
    },
    [reloadLibrary, runAction, showToast],
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
        await reloadLibrary();
        return "Root folder removed; episodes from that folder were dropped from the library.";
      });
    },
    [reloadLibrary, runAction],
  );

  const handleRescan = useCallback(async () => {
    await runAction(async () => {
      const summary = await rescanLibrary();
      const state = await reloadLibrary();
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
  }, [navigateToView, reloadLibrary, reloadLocalDataStats, runAction, view]);

  const handleCleanLocalData = useCallback(async () => {
    const confirmed = window.confirm(
      "Clean local data now? This removes database entries for episodes that are missing or no longer match your current detection rules, plus unused saved thumbnails.",
    );
    if (!confirmed) return;

    await runAction(async () => {
      const summary = await cleanLocalData();
      await reloadLibrary();
      await reloadLocalDataStats();
      const staleEpisodes = `${summary.stale_episodes_removed} stale episode${summary.stale_episodes_removed === 1 ? "" : "s"}`;
      const emptyAnime = `${summary.empty_anime_removed} empty anime entr${summary.empty_anime_removed === 1 ? "y" : "ies"}`;
      const thumbnails = `${summary.thumbnails_removed} unused thumbnail${summary.thumbnails_removed === 1 ? "" : "s"}`;
      return `Cleaned local data: removed ${staleEpisodes}, ${emptyAnime}, and ${thumbnails}.`;
    });
  }, [reloadLibrary, reloadLocalDataStats, runAction]);

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
        return "Category deleted. Anime were moved to the default category.";
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

  const handleMoveAnime = useCallback(
    async (categoryId: number) => {
      if (!selectedAnime) return;
      await runAction(async () => {
        await moveAnimeToCategory(selectedAnime.id, categoryId);
        const state = await reloadLibrary();
        const updated = state.anime.find((anime) => anime.id === selectedAnime.id);
        if (updated) setSelectedAnime(updated);
        return "Anime moved.";
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
      await reloadLibrary();
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
    reloadLibrary,
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
        return `Moved ${animeIds.length} anime to a new category.`;
      });
    },
    [reloadLibrary, runAction],
  );

  const handleSaveAnilistClientId = useCallback(
    async (clientId: string) => {
      await runAction(async () => {
        const state = await setAnilistClientId(clientId);
        setAnilistAuth(state);
        return clientId.trim() ? "AniList client ID saved." : "AniList client ID cleared.";
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

  const handleSearchAnilist = useCallback((query: string): Promise<AnilistSearchResult[]> => {
    return searchAnilistAnime(query);
  }, []);

  const handleGetAnilistStatus = useCallback((animeId: number): Promise<AnilistMediaStatus | null> => {
    return getAnilistMediaStatus(animeId);
  }, []);

  const handleSetAnilistScore = useCallback((animeId: number, score: number | null): Promise<AnilistMediaStatus> => {
    return setAnilistMediaScore(animeId, score);
  }, []);

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
        } else {
          const state = await reloadLibrary();
          const updated = state.anime.find((anime) => anime.id === animeId);
          if (updated) setSelectedAnime(updated);
        }
        return "Anime linked to AniList.";
      });
    },
    [applyLocalAnilistProgress, refreshAnimePageData, reloadLibrary, runAction],
  );

  const handleUnlinkAnilist = useCallback(
    async (animeId: number) => {
      await runAction(async () => {
        await unlinkAnimeAnilist(animeId);
        const state = await reloadLibrary();
        const updated = state.anime.find((anime) => anime.id === animeId);
        if (updated) setSelectedAnime(updated);
        return "Anime unlinked from AniList.";
      });
    },
    [reloadLibrary, runAction],
  );

  const handleOpenAnilist = useCallback(async (url: string) => {
    await openUrl(url);
  }, []);

  const handleProgressSaved = useCallback(
    (saved: Episode) => {
      setEpisodes((current) => {
        const next = current.map((episode) => (episode.id === saved.id ? saved : episode));
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

  const openEpisode = useCallback((episode: Episode) => {
    setVideoOpening(true);
    window.setTimeout(() => {
      setSelectedEpisode(episode);
      navigateToView("player");
      setVideoOpening(false);
    }, VIDEO_OPEN_FADE_MS);
  }, [navigateToView]);

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
        // Same file already loaded in the hidden player; reveal it without
        // the open-fade. PlayerView's `visible` effect handles the unpause.
        navigateToView("player");
      } else {
        openEpisode(target);
      }
    };
    window.addEventListener("keydown", onKeyDown, true);
    return () => window.removeEventListener("keydown", onKeyDown, true);
  }, [episodes, openEpisode, selectedAnime, selectedEpisode, view]);

  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.repeat || e.code !== "Escape") return;
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
      if (view === "settings") {
        navigateToView("categories", "restore");
      }
    };
    window.addEventListener("keydown", onKeyDown, true);
    return () => window.removeEventListener("keydown", onKeyDown, true);
  }, [episodeReturnView, navigateToView, selectedAnime, view]);

  const showPlayer = view === "player" && Boolean(selectedEpisode);

  const osTitleAnimeLabel = useMemo(() => {
    if (!selectedEpisode || !library) return null;
    const anime =
      selectedAnime?.id === selectedEpisode.anime_id
        ? selectedAnime
        : library.anime.find((a) => a.id === selectedEpisode.anime_id);
    const label = anime?.anilist_title?.trim() || anime?.title?.trim();
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
    const raw = osTitleAnimeLabel ?? selectedEpisode?.file_name ?? "Anime";
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
      }${videoOpening ? " app--video-opening" : ""}`}
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
          playlist={episodes}
          visible={Boolean(showPlayer)}
          onSelectEpisode={setSelectedEpisode}
          onBack={() => navigateToView("episodes", "restore")}
          onClose={() => {
            setSelectedEpisode(null);
            navigateToView("episodes", "restore");
          }}
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
              onBack={() => navigateToView("categories", "restore")}
              onOpenAnime={(anime) => void openAnime(anime, "anime")}
              onOpenSettings={() => navigateToView("settings")}
            />
          ) : null}

          {view === "search" ? (
            <SearchScreen
              anime={library.anime}
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
                onMoveAnime={(animeIds, categoryId) => void handleBulkMoveAnime(animeIds, categoryId)}
              />
            </div>
          ) : null}

          {view === "missing" ? (
            <MissingScreen anime={library.missing_anime} />
          ) : null}

          {view === "episodes" && selectedAnime ? (
            <EpisodeScreen
              anime={selectedAnime}
              episodes={episodes}
              categories={library.categories}
              onBack={() => navigateToView(episodeReturnView, "restore")}
              onPlay={openEpisode}
              onMoveAnime={(categoryId) => void handleMoveAnime(categoryId)}
              onOpenEpisodeFolder={() => void handleOpenEpisodeFolder()}
              onDeleteAnime={() => void handleDeleteSelectedAnimeFiles()}
              onSearchAnilist={handleSearchAnilist}
              onGetAnilistStatus={handleGetAnilistStatus}
              onSetAnilistScore={handleSetAnilistScore}
              anilistProgressUpdate={anilistProgressUpdate}
              onLinkAnilist={(animeId, anilistId) => void handleLinkAnilist(animeId, anilistId)}
              onUnlinkAnilist={(animeId) => void handleUnlinkAnilist(animeId)}
              onOpenAnilist={(url) => void handleOpenAnilist(url)}
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
              onCreateRule={(input) => void handleCreateRule(input)}
              onUpdateRule={(id, input) => void handleUpdateRule(id, input)}
              onDeleteRule={(rule) => void handleDeleteRule(rule)}
              onSaveAnilistClientId={(clientId) => void handleSaveAnilistClientId(clientId)}
              onLoginAnilist={() => void handleLoginAnilist()}
              onLogoutAnilist={() => void handleLogoutAnilist()}
              onCleanLocalData={() => void handleCleanLocalData()}
            />
          ) : null}
        </div>
      </section>

      {videoOpening ? <div className="video-open-overlay" /> : null}
      <ToastStack toasts={toasts} onDismiss={(id) => setToasts((current) => current.filter((toast) => toast.id !== id))} />
    </main>
  );
}

export default App;
