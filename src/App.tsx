import { useCallback, useEffect, useMemo, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { getCurrent, onOpenUrl } from "@tauri-apps/plugin-deep-link";
import { openUrl } from "@tauri-apps/plugin-opener";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  addRootFolder,
  completeAnilistLogin,
  createCategory,
  createRegexRule,
  deleteCategory,
  deleteRegexRule,
  getAnilistAuthState,
  getAnilistLoginUrl,
  getAnilistMediaStatus,
  getLibraryState,
  linkAnimeAnilist,
  listEpisodes,
  logoutAnilist,
  moveAnimeToCategory,
  removeRootFolder,
  rescanLibrary,
  searchAnilistAnime,
  setDefaultCategory,
  setAnilistClientId,
  setAnilistMediaScore,
  unlinkAnimeAnilist,
  updateRegexRule,
} from "./api";
import { AnimeGrid } from "./components/AnimeGrid";
import { CategoryScreen } from "./components/CategoryScreen";
import { EpisodeScreen } from "./components/EpisodeScreen";
import { LibraryIcon, RescanIcon, SettingsIcon } from "./components/Icons";
import { PlayerView } from "./components/PlayerView";
import { SettingsScreen } from "./components/SettingsScreen";
import { type Toast, ToastStack } from "./components/ToastStack";
import { WindowTitleBar } from "./components/WindowTitleBar";
import { pickQuickPlayEpisode } from "./quickPlay";
import type {
  AnimeSummary,
  AnilistAuthState,
  AnilistMediaStatus,
  AnilistSearchResult,
  Category,
  Episode,
  LibraryState,
  RegexRule,
  RegexRuleInput,
  RootFolder,
} from "./types";
import { errorMessage, isTextInputTarget } from "./utils";
import "./App.css";

type View = "categories" | "anime" | "episodes" | "settings" | "player";

const VIDEO_OPEN_FADE_MS = 180;
const appWindow = getCurrentWindow();

function App() {
  const [library, setLibrary] = useState<LibraryState | null>(null);
  const [view, setView] = useState<View>("categories");
  const [selectedCategoryId, setSelectedCategoryId] = useState<number | null>(null);
  const [selectedAnime, setSelectedAnime] = useState<AnimeSummary | null>(null);
  const [anilistAuth, setAnilistAuth] = useState<AnilistAuthState | null>(null);
  const [episodes, setEpisodes] = useState<Episode[]>([]);
  const [selectedEpisode, setSelectedEpisode] = useState<Episode | null>(null);
  const [rootInput, setRootInput] = useState("");
  const [newCategoryName, setNewCategoryName] = useState("");
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [fatalError, setFatalError] = useState<string | null>(null);
  const [toasts, setToasts] = useState<Toast[]>([]);
  const [videoOpening, setVideoOpening] = useState(false);
  const [playerControlsChromeVisible, setPlayerControlsChromeVisible] = useState(true);

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

  useEffect(() => {
    void (async () => {
      try {
        await Promise.all([reloadLibrary(), reloadAnilistAuth()]);
      } catch (e) {
        setFatalError(errorMessage(e));
      } finally {
        setLoading(false);
      }
    })();
  }, [reloadAnilistAuth, reloadLibrary]);

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
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.repeat) return;
      if (e.code !== "F11") return;
      if (isTextInputTarget(e.target)) return;
      e.preventDefault();
      void (async () => {
        try {
          const next = !(await appWindow.isFullscreen());
          await appWindow.setFullscreen(next);
        } catch {
          /* ignore */
        }
      })();
    };
    window.addEventListener("keydown", onKeyDown, true);
    return () => window.removeEventListener("keydown", onKeyDown, true);
  }, []);

  useEffect(() => {
    if (!(view === "player" && selectedEpisode)) {
      setPlayerControlsChromeVisible(true);
    }
  }, [view, selectedEpisode]);

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

  const openAnime = useCallback(
    async (anime: AnimeSummary) => {
      setSelectedAnime(anime);
      try {
        const nextEpisodes = await listEpisodes(anime.id);
        setEpisodes(nextEpisodes);
        setView("episodes");
      } catch (e) {
        showToast("error", errorMessage(e));
      }
    },
    [showToast],
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
        return "Root folder removed. Existing library entries are preserved until rescanned.";
      });
    },
    [reloadLibrary, runAction],
  );

  const handleRescan = useCallback(async () => {
    await runAction(async () => {
      const summary = await rescanLibrary();
      await reloadLibrary();
      return `Scanned ${summary.roots_scanned} root folder${summary.roots_scanned === 1 ? "" : "s"}: ${summary.episodes_imported} episode${summary.episodes_imported === 1 ? "" : "s"} imported, ${summary.unmatched_files} unmatched.`;
    });
  }, [reloadLibrary, runAction]);

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

  const handleLinkAnilist = useCallback(
    async (animeId: number, anilistId: number) => {
      await runAction(async () => {
        await linkAnimeAnilist(animeId, anilistId);
        const state = await reloadLibrary();
        const updated = state.anime.find((anime) => anime.id === animeId);
        if (updated) setSelectedAnime(updated);
        return "Anime linked to AniList.";
      });
    },
    [reloadLibrary, runAction],
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
    setView("anime");
  }, []);

  const openEpisode = useCallback((episode: Episode) => {
    setVideoOpening(true);
    window.setTimeout(() => {
      setSelectedEpisode(episode);
      setView("player");
      setVideoOpening(false);
    }, VIDEO_OPEN_FADE_MS);
  }, []);

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
        setView("player");
      } else {
        openEpisode(target);
      }
    };
    window.addEventListener("keydown", onKeyDown, true);
    return () => window.removeEventListener("keydown", onKeyDown, true);
  }, [episodes, openEpisode, selectedAnime, selectedEpisode, view]);

  if (loading) {
    return (
      <main className="app app--loading">
        <WindowTitleBar playerOpen={false} playerControlsChromeVisible />
        <div className="empty">
          <h2>Loading library...</h2>
        </div>
      </main>
    );
  }

  if (!library) {
    return (
      <main className="app app--loading">
        <WindowTitleBar playerOpen={false} playerControlsChromeVisible />
        <div className="empty">
          <h2>Library failed to load</h2>
          {fatalError ? <p className="muted">{fatalError}</p> : null}
        </div>
      </main>
    );
  }

  const showPlayer = view === "player" && selectedEpisode;
  const playerLoadedInBackground = selectedEpisode && !showPlayer;
  const libraryViewActive = view === "categories" || view === "anime" || view === "episodes";

  return (
    <main
      className={`app${showPlayer ? " app--player-open" : ""}${
        playerLoadedInBackground ? " app--player-background" : ""
      }${videoOpening ? " app--video-opening" : ""}`}
    >
      <WindowTitleBar
        playerOpen={Boolean(showPlayer)}
        playerControlsChromeVisible={playerControlsChromeVisible}
      />
      <aside className="sidebar">
        <nav className="nav-list" aria-label="Primary navigation">
          <button
            type="button"
            className={libraryViewActive ? "nav-item active" : "nav-item"}
            onClick={() => setView("categories")}
            aria-label="Library"
            title="Library"
          >
            <LibraryIcon />
            <span className="nav-label">Library</span>
          </button>
          <button
            type="button"
            className={view === "settings" ? "nav-item active" : "nav-item"}
            onClick={() => setView("settings")}
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
          onBack={() => setView("episodes")}
          onClose={() => {
            setSelectedEpisode(null);
            setView("episodes");
          }}
          onProgressSaved={handleProgressSaved}
          onError={(message) => showToast("error", message)}
          onControlsVisibilityChange={setPlayerControlsChromeVisible}
        />
      ) : null}

      <section className="content">
        <div className="content-inner">
          {view === "categories" ? (
            <CategoryScreen
              library={library}
              onOpenCategory={navigateToCategory}
              onOpenAnime={openAnime}
              onOpenSettings={() => setView("settings")}
            />
          ) : null}

          {view === "anime" ? (
            <AnimeGrid
              category={selectedCategory}
              anime={animeInCategory}
              onBack={() => setView("categories")}
              onOpenAnime={openAnime}
              onOpenSettings={() => setView("settings")}
            />
          ) : null}

          {view === "episodes" && selectedAnime ? (
            <EpisodeScreen
              anime={selectedAnime}
              episodes={episodes}
              categories={library.categories}
              onBack={() => setView("anime")}
              onPlay={openEpisode}
              onMoveAnime={(categoryId) => void handleMoveAnime(categoryId)}
              onSearchAnilist={handleSearchAnilist}
              onGetAnilistStatus={handleGetAnilistStatus}
              onSetAnilistScore={handleSetAnilistScore}
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
              anilistAuth={anilistAuth}
              onBack={() => setView("categories")}
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
