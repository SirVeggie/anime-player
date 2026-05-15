import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { LogicalSize } from "@tauri-apps/api/dpi";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { open } from "@tauri-apps/plugin-dialog";
import {
  addMpvSubtitleFile,
  getMpvTracks,
  getMpvVideoGeometry,
  saveEpisodeProgress,
  selectMpvAudioTrack,
  selectMpvSubtitleTrack,
  setMpvVolume,
  syncAnilistEpisodeProgress,
} from "../api";
import type { AnilistProgressSyncResult, Episode, MpvTrack, MpvVideoGeometry } from "../types";
import { errorMessage, formatTime, isTextInputTarget } from "../utils";
import { HOTKEY_STEP, MAX_VOLUME, clampVolume, loadVolume, saveVolume } from "../volume";

const PLAYER_SIDEBAR_PX = 0;
const HIDDEN_PLAYER_SIDEBAR_PX = 100_000;
/** Same ratio as `persistProgress` marking an episode watched without EOF. */
const NEAR_END_PROGRESS_RATIO = 0.9;
const appWindow = getCurrentWindow();

function sidebarPxForVisibility(visible: boolean) {
  return visible ? PLAYER_SIDEBAR_PX : HIDDEN_PLAYER_SIDEBAR_PX;
}

function parentDirFromPath(path: string) {
  const lastForwardSlash = path.lastIndexOf("/");
  const lastBackSlash = path.lastIndexOf("\\");
  const separatorIndex = Math.max(lastForwardSlash, lastBackSlash);
  return separatorIndex >= 0 ? path.slice(0, separatorIndex) : null;
}

function SeekBar(props: {
  duration: number;
  position: number;
  onSeek: (seconds: number) => void;
  onInteractionChange?: (active: boolean) => void;
}) {
  const { duration, position, onSeek, onInteractionChange } = props;
  const areaRef = useRef<HTMLDivElement>(null);
  const durationRef = useRef(duration);
  const onSeekRef = useRef(onSeek);
  durationRef.current = duration;
  onSeekRef.current = onSeek;

  const [isDragging, setIsDragging] = useState(false);
  const isDraggingRef = useRef(false);
  const activePointerId = useRef<number | null>(null);
  const [dragRatio, setDragRatio] = useState<number | null>(null);
  const [showHoverTime, setShowHoverTime] = useState(false);
  const [hoverRatio, setHoverRatio] = useState(0);
  const [hoverTime, setHoverTime] = useState(0);
  const dragListenersCleanup = useRef<(() => void) | null>(null);

  const clampRatio = (v: number) => Math.min(1, Math.max(0, v));

  const getRatioFromClientX = (clientX: number) => {
    const container = areaRef.current;
    if (!container) return 0;
    const rect = container.getBoundingClientRect();
    if (rect.width <= 0) return 0;
    return clampRatio((clientX - rect.left) / rect.width);
  };

  const detachDragListeners = () => {
    dragListenersCleanup.current?.();
    dragListenersCleanup.current = null;
  };

  useEffect(() => () => detachDragListeners(), []);

  useEffect(() => {
    const onKeyDown = () => {
      if (!isDraggingRef.current) setShowHoverTime(false);
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);

  const stopDragging = (event?: PointerEvent) => {
    if (event && areaRef.current?.hasPointerCapture(event.pointerId)) {
      areaRef.current.releasePointerCapture(event.pointerId);
    }
    activePointerId.current = null;
    isDraggingRef.current = false;
    setIsDragging(false);
    setDragRatio(null);
    detachDragListeners();
    onInteractionChange?.(false);
  };

  const onPointerDown = (e: React.PointerEvent<HTMLDivElement>) => {
    if (e.button !== 0 || duration <= 0) return;
    e.preventDefault();
    onInteractionChange?.(true);
    detachDragListeners();
    activePointerId.current = e.pointerId;
    isDraggingRef.current = true;
    setIsDragging(true);
    areaRef.current?.setPointerCapture(e.pointerId);
    const ratio = getRatioFromClientX(e.clientX);
    setDragRatio(ratio);
    setHoverRatio(ratio);
    setHoverTime(ratio * durationRef.current);
    setShowHoverTime(true);

    const onMove = (ev: PointerEvent) => {
      if (!isDraggingRef.current || ev.pointerId !== activePointerId.current) return;
      const r = getRatioFromClientX(ev.clientX);
      setDragRatio(r);
      setHoverRatio(r);
      setHoverTime(r * durationRef.current);
    };
    const onUp = (ev: PointerEvent) => {
      if (!isDraggingRef.current || ev.pointerId !== activePointerId.current) return;
      const r = getRatioFromClientX(ev.clientX);
      onSeekRef.current(r * durationRef.current);
      stopDragging(ev);
      setShowHoverTime(false);
    };
    const onCancel = (ev: PointerEvent) => {
      if (ev.pointerId !== activePointerId.current) return;
      stopDragging(ev);
      setShowHoverTime(false);
    };

    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
    window.addEventListener("pointercancel", onCancel);
    dragListenersCleanup.current = () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
      window.removeEventListener("pointercancel", onCancel);
    };
  };

  const updateHoverTime = (e: React.MouseEvent) => {
    if (isDraggingRef.current) return;
    if (duration <= 0) {
      setShowHoverTime(false);
      onInteractionChange?.(false);
      return;
    }
    onInteractionChange?.(true);
    const ratio = getRatioFromClientX(e.clientX);
    setHoverRatio(ratio);
    setHoverTime(ratio * duration);
    setShowHoverTime(true);
  };

  const hideHoverTime = () => {
    if (isDraggingRef.current) return;
    setShowHoverTime(false);
    onInteractionChange?.(false);
  };

  const progressPercent = duration > 0 ? (position / duration) * 100 : 0;
  const displayProgressPercent =
    isDragging && dragRatio !== null ? dragRatio * 100 : progressPercent;

  return (
    <div
      ref={areaRef}
      className={`progress-area${isDragging ? " is-dragging" : ""}`}
      role="slider"
      aria-label="Seek"
      aria-valuemin={0}
      aria-valuemax={Math.max(duration, 0)}
      aria-valuenow={Math.min(position, duration)}
      onPointerDown={onPointerDown}
      onMouseMove={updateHoverTime}
      onMouseLeave={hideHoverTime}
    >
      {showHoverTime ? (
        <div className="time-tooltip" style={{ left: `${hoverRatio * 100}%` }}>
          {formatTime(hoverTime)}
        </div>
      ) : null}
      <div className="progress-bg">
        <div className="progress-current" style={{ width: `${displayProgressPercent}%` }} />
      </div>
      <div className="scrubber-head" style={{ left: `${displayProgressPercent}%` }} />
    </div>
  );
}

export function PlayerView(props: {
  episode: Episode;
  playlist: Episode[];
  visible: boolean;
  onSelectEpisode: (episode: Episode) => void;
  onBack: () => void;
  onClose: () => void;
  onProgressSaved: (episode: Episode) => void;
  onAnilistProgressSynced?: (animeId: number, result: AnilistProgressSyncResult) => void;
  onError: (message: string) => void;
  onControlsVisibilityChange?: (visible: boolean) => void;
  onPausedStateChange?: (paused: boolean) => void;
}) {
  const {
    episode,
    playlist,
    visible,
    onSelectEpisode,
    onBack,
    onClose,
    onProgressSaved,
    onAnilistProgressSynced,
    onError,
    onControlsVisibilityChange,
    onPausedStateChange,
  } = props;
  const [paused, setPaused] = useState(true);
  const [position, setPosition] = useState(episode.position_seconds || 0);
  const [duration, setDuration] = useState(episode.duration_seconds || 0);
  const [fullscreen, setFullscreen] = useState(false);
  const [videoCompositorRevealed, setVideoCompositorRevealed] = useState(false);
  const [controlsVisible, setControlsVisible] = useState(true);
  const [seekInteracting, setSeekInteracting] = useState(false);
  const [activeTrackMenu, setActiveTrackMenu] = useState<"audio" | "sub" | null>(null);
  const [tracks, setTracks] = useState<MpvTrack[]>([]);
  const [videoGeometry, setVideoGeometry] = useState<MpvVideoGeometry | null>(null);
  const [volume, setVolume] = useState(loadVolume);
  const [muted, setMuted] = useState(false);
  const [volumePopupOpen, setVolumePopupOpen] = useState(false);
  const [volumeOsdVisible, setVolumeOsdVisible] = useState(false);
  const volumeHideTimerRef = useRef<number | null>(null);
  const volumeOsdTimerRef = useRef<number | null>(null);
  const volumeRef = useRef(volume);
  const mpvReadyRef = useRef(false);
  const loadedPathRef = useRef<string | null>(null);
  const playbackRef = useRef({ episode, position, duration });
  const pendingResumeSecondsRef = useRef<number | null>(null);
  const controlsHideTimerRef = useRef<number | null>(null);
  const handlingEofRef = useRef(false);
  const eventListenersReadyRef = useRef(false);
  const [eventListenersReadyVersion, setEventListenersReadyVersion] = useState(0);
  // Set by handlePlaybackFinished's auto-advance branch so the [episode.id]
  // reset effect skips clearing videoCompositorRevealed: an EOF -> next-file
  // transition stays on the previous frame instead of flashing through the
  // black load-fade. Manual prev/next via loadSibling intentionally leaves
  // the flag false so it still gets the load-fade.
  const seamlessAdvanceRef = useRef(false);
  // Tracks the previous `visible` so the visible-effect only unpauses on a
  // real false -> true transition (returning to the player), not on every
  // re-run from unrelated state changes.
  const wasVisibleRef = useRef(false);

  // App passes inline-arrow handlers that change identity every render
  // (`onError`, `onClose`, etc.). If the listener-setup useEffect depends on
  // them directly it tears down + reattaches mpv listeners on every App
  // render — which (a) bumps eventListenersReadyVersion, retriggering the
  // visible-effect's mpv_set_pause(false) and undoing user pauses, and
  // (b) leaves a window where the one-shot mpv://file-loaded event can be
  // dropped, leaving videoGeometry null and the fit-window button disabled.
  // Mirroring the latest props into a ref lets the callbacks below stay
  // referentially stable without going stale.
  const propsRef = useRef({
    onError,
    onClose,
    onProgressSaved,
    onSelectEpisode,
    onAnilistProgressSynced,
  });
  propsRef.current = {
    onError,
    onClose,
    onProgressSaved,
    onSelectEpisode,
    onAnilistProgressSynced,
  };

  const selectedIndex = playlist.findIndex((item) => item.id === episode.id);
  const canPrev = selectedIndex > 0;
  const canNext = selectedIndex >= 0 && selectedIndex < playlist.length - 1;
  const controlsPinned = seekInteracting || activeTrackMenu !== null || volumePopupOpen;
  const audioTracks = tracks.filter((track) => track.kind === "audio");
  const subtitleTracks = tracks.filter((track) => track.kind === "sub");
  const selectedAudioTrack = audioTracks.find((track) => track.selected) ?? null;
  const selectedSubtitleTrack = subtitleTracks.find((track) => track.selected) ?? null;

  useEffect(() => {
    playbackRef.current = { episode, position, duration };
  }, [duration, episode, position]);

  useEffect(() => {
    onPausedStateChange?.(paused);
  }, [onPausedStateChange, paused]);

  // Synchronous render-time refs so handlePlaybackFinished/loadSibling can
  // read the latest values without depending on `playlist` or `selectedIndex`
  // (and re-creating, which would churn the listener-setup useEffect).
  const playlistRef = useRef(playlist);
  playlistRef.current = playlist;
  const selectedIndexRef = useRef(selectedIndex);
  selectedIndexRef.current = selectedIndex;

  const clearControlsHideTimer = useCallback(() => {
    if (controlsHideTimerRef.current !== null) {
      window.clearTimeout(controlsHideTimerRef.current);
      controlsHideTimerRef.current = null;
    }
  }, []);

  const scheduleControlsHide = useCallback(() => {
    clearControlsHideTimer();
    if (!visible || controlsPinned) return;
    controlsHideTimerRef.current = window.setTimeout(() => {
      setControlsVisible(false);
      controlsHideTimerRef.current = null;
    }, 2200);
  }, [clearControlsHideTimer, controlsPinned, visible]);

  const revealControlsFromPointer = useCallback(() => {
    if (!visible) return;
    setControlsVisible(true);
    scheduleControlsHide();
  }, [scheduleControlsHide, visible]);

  const closeTrackMenu = useCallback(() => setActiveTrackMenu(null), []);

  useEffect(() => {
    if (visible) {
      setControlsVisible(true);
      scheduleControlsHide();
    } else {
      clearControlsHideTimer();
    }
    return clearControlsHideTimer;
  }, [clearControlsHideTimer, scheduleControlsHide, visible]);

  useEffect(() => {
    if (controlsPinned) {
      setControlsVisible(true);
      clearControlsHideTimer();
    } else {
      scheduleControlsHide();
    }
  }, [clearControlsHideTimer, controlsPinned, scheduleControlsHide]);

  useEffect(() => {
    onControlsVisibilityChange?.(controlsVisible);
  }, [controlsVisible, onControlsVisibilityChange]);

  useEffect(() => {
    const root = document.documentElement;
    const onPointerLeaveWindow = () => {
      if (!visible || controlsPinned) return;
      clearControlsHideTimer();
      setControlsVisible(false);
    };
    root.addEventListener("pointerleave", onPointerLeaveWindow);
    return () => root.removeEventListener("pointerleave", onPointerLeaveWindow);
  }, [clearControlsHideTimer, controlsPinned, visible]);

  const persistProgress = useCallback(async (forceWatched = false) => {
    const current = playbackRef.current;
    const watched =
      forceWatched ||
      (current.duration > 0 &&
        current.position / current.duration >= NEAR_END_PROGRESS_RATIO);
    const saved = await saveEpisodeProgress(
      current.episode.id,
      current.position,
      current.duration,
      watched,
    );
    propsRef.current.onProgressSaved(saved);
    if (watched) {
      void syncAnilistEpisodeProgress(saved.id)
        .then((result) => propsRef.current.onAnilistProgressSynced?.(saved.anime_id, result))
        .catch((err) => propsRef.current.onError(errorMessage(err)));
    }
    return saved;
  }, []);

  const refreshTracks = useCallback(async () => {
    try {
      setTracks(await getMpvTracks());
    } catch (e) {
      propsRef.current.onError(errorMessage(e));
    }
  }, []);

  const refreshVideoGeometry = useCallback(async () => {
    try {
      setVideoGeometry(await getMpvVideoGeometry());
    } catch (e) {
      propsRef.current.onError(errorMessage(e));
    }
  }, []);

  const handlePlaybackFinished = useCallback(() => {
    if (handlingEofRef.current) return;
    handlingEofRef.current = true;
    const next = playlistRef.current[selectedIndexRef.current + 1];
    void persistProgress(true)
      .catch((err) => propsRef.current.onError(errorMessage(err)))
      .then(async () => {
        if (next) {
          // Skip the load-fade between episodes — the previous episode's
          // outro/credits already ended on a clean note, so flashing to
          // black is more disruptive than just transitioning straight into
          // the next file.
          seamlessAdvanceRef.current = true;
          try {
            const saved = await saveEpisodeProgress(
              next.id,
              0,
              next.duration_seconds,
              false,
            );
            propsRef.current.onProgressSaved(saved);
            propsRef.current.onSelectEpisode(saved);
          } catch (err) {
            propsRef.current.onError(errorMessage(err));
            propsRef.current.onSelectEpisode(next);
          }
          return;
        }

        void invoke("mpv_stop")
          .catch((err) => propsRef.current.onError(errorMessage(err)))
          .finally(() => {
            loadedPathRef.current = null;
            setPosition(0);
            setDuration(0);
            setPaused(true);
            propsRef.current.onClose();
          });
      });
  }, [persistProgress]);

  useEffect(() => {
    const unlisteners: UnlistenFn[] = [];
    let cancelled = false;
    eventListenersReadyRef.current = false;

    (async () => {
      const subs: Array<[string, (e: { payload: unknown }) => void]> = [
        [
          "mpv://time-pos",
          (e) => {
            if (typeof e.payload === "number") setPosition(e.payload);
          },
        ],
        [
          "mpv://duration",
          (e) => {
            if (typeof e.payload === "number") setDuration(e.payload);
          },
        ],
        [
          "mpv://pause",
          (e) => {
            if (typeof e.payload === "boolean") setPaused(e.payload);
          },
        ],
        [
          "mpv://eof-reached",
          (e) => {
            if (e.payload === true) {
              setPaused(true);
              handlePlaybackFinished();
            }
          },
        ],
        [
          "mpv://file-loaded",
          () => {
            void refreshTracks();
            void refreshVideoGeometry();
            const seconds = pendingResumeSecondsRef.current;
            pendingResumeSecondsRef.current = null;
            if (seconds === null) return;
            window.setTimeout(() => {
              void invoke("mpv_seek", { seconds }).catch((err) =>
                propsRef.current.onError(errorMessage(err)),
              );
            }, 0);
          },
        ],
        [
          "mpv://playback-restart",
          () => {
            setPaused(false);
            setVideoCompositorRevealed(true);
            void refreshVideoGeometry();
          },
        ],
      ];

      for (const [name, handler] of subs) {
        const fn = await listen(name, handler);
        if (cancelled) {
          fn();
          continue;
        }
        unlisteners.push(fn);
      }

      if (!cancelled) {
        eventListenersReadyRef.current = true;
        setEventListenersReadyVersion((version) => version + 1);
      }
    })();

    return () => {
      cancelled = true;
      eventListenersReadyRef.current = false;
      unlisteners.forEach((fn) => fn());
    };
    // The deps below are now all stable (refs / empty-dep useCallbacks), so
    // this effect runs exactly once per mount. That avoids the listener
    // detach/re-attach storm that used to drop file-loaded events and
    // re-trigger the visible-effect's auto-unpause on every App render.
  }, [handlePlaybackFinished, refreshTracks, refreshVideoGeometry]);

  // Only reset when switching episodes. Progress saves refresh `position_seconds` on the same
  // `episode.id`; doing that here cleared `videoCompositorRevealed` and left the pane opaque
  // while mpv still had the file loaded, which broke reopening the same episode from the list.
  useEffect(() => {
    handlingEofRef.current = false;
    setPosition(episode.position_seconds || 0);
    setDuration(episode.duration_seconds || 0);
    setTracks([]);
    setVideoGeometry(null);
    setActiveTrackMenu(null);
    if (seamlessAdvanceRef.current) {
      // EOF auto-advance: keep videoCompositorRevealed=true so the
      // .player-load-fade stays hidden across the file switch. The user
      // sees the last frame of the previous episode (mpv keep-open) until
      // the next file's first frame composes — much smoother than a black
      // flash between back-to-back episodes.
      seamlessAdvanceRef.current = false;
    } else {
      setVideoCompositorRevealed(false);
    }
  }, [episode.id]);

  useEffect(() => {
    let cancelled = false;
    // Only auto-unpause on a real visibility transition (false -> true).
    // Without this, any spurious re-run of this effect with visible=true
    // (e.g. a stale dep change) would resume playback the user just paused.
    const becameVisible = visible && !wasVisibleRef.current;
    wasVisibleRef.current = visible;
    (async () => {
      try {
        if (!eventListenersReadyRef.current) return;
        const sidebarPx = sidebarPxForVisibility(visible);
        if (!mpvReadyRef.current) {
          await invoke("mpv_init", {
            windowWidth: window.innerWidth,
            sidebarPx,
          });
          mpvReadyRef.current = true;
          void setMpvVolume(muted ? 0 : volume).catch(() => {});
        } else {
          await invoke("mpv_set_layout", {
            windowWidth: window.innerWidth,
            sidebarPx,
          });
        }
        if (cancelled) return;
        if (loadedPathRef.current === episode.path) {
          if (becameVisible) {
            try {
              await invoke("mpv_set_pause", { paused: false });
              setPaused(false);
              setVideoCompositorRevealed(true);
            } catch (e) {
              if (!cancelled) propsRef.current.onError(errorMessage(e));
            }
          }
          return;
        }
        pendingResumeSecondsRef.current =
          episode.position_seconds > 1 && !episode.watched ? episode.position_seconds : null;
        setPaused(false);
        await invoke("mpv_load", { path: episode.path });
        loadedPathRef.current = episode.path;
      } catch (e) {
        if (!cancelled) propsRef.current.onError(errorMessage(e));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [episode.path, eventListenersReadyVersion, visible]);

  useEffect(() => {
    if (!mpvReadyRef.current) return;
    void setMpvVolume(muted ? 0 : volume).catch((e) => onError(errorMessage(e)));
  }, [muted, onError, volume]);

  useEffect(() => {
    if (!mpvReadyRef.current) return;
    void invoke("mpv_set_layout", {
      windowWidth: window.innerWidth,
      sidebarPx: sidebarPxForVisibility(visible),
    }).catch((err) => onError(errorMessage(err)));
  }, [onError, visible]);

  useEffect(() => {
    let unlisten: UnlistenFn | undefined;
    let cancelled = false;
    (async () => {
      try {
        setFullscreen(await appWindow.isFullscreen());
      } catch {
        /* ignore */
      }
      if (cancelled) return;
      unlisten = await appWindow.onResized(async () => {
        try {
          setFullscreen(await appWindow.isFullscreen());
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

  const onTogglePause = useCallback(() => {
    void invoke("mpv_cycle_pause").catch((e) => onError(errorMessage(e)));
  }, [onError]);

  const onSeekCommit = useCallback(
    (seconds: number) => {
      setPosition(seconds);
      void invoke("mpv_seek", { seconds }).catch((e) => onError(errorMessage(e)));
    },
    [onError],
  );

  const selectAudioTrack = useCallback(
    async (trackId: number) => {
      try {
        await selectMpvAudioTrack(trackId);
        await refreshTracks();
        setActiveTrackMenu(null);
      } catch (e) {
        onError(errorMessage(e));
      }
    },
    [onError, refreshTracks],
  );

  const selectSubtitleTrack = useCallback(
    async (trackId: number | null) => {
      try {
        await selectMpvSubtitleTrack(trackId);
        await refreshTracks();
        setActiveTrackMenu(null);
      } catch (e) {
        onError(errorMessage(e));
      }
    },
    [onError, refreshTracks],
  );

  const browseSubtitleFile = useCallback(async () => {
    try {
      const defaultPath = parentDirFromPath(episode.path);
      const picked = await open({
        directory: false,
        multiple: false,
        ...(defaultPath ? { defaultPath } : {}),
        filters: [
          {
            name: "Subtitle files",
            extensions: ["srt", "ass", "ssa", "sub", "vtt", "sup", "idx"],
          },
        ],
      });
      if (typeof picked !== "string" || !picked) return;
      await addMpvSubtitleFile(picked);
      await refreshTracks();
      setActiveTrackMenu(null);
    } catch (e) {
      onError(errorMessage(e));
    }
  }, [episode.path, onError, refreshTracks]);

  const toggleFullscreen = useCallback(async () => {
    try {
      const next = !(await appWindow.isFullscreen());
      await appWindow.setFullscreen(next);
      setFullscreen(next);
    } catch (e) {
      onError(errorMessage(e));
    }
  }, [onError]);

  const fitWindowToAspect = useCallback(async () => {
    if (!videoGeometry || videoGeometry.width <= 0 || videoGeometry.height <= 0) {
      onError("Video dimensions are not available yet.");
      return;
    }

    try {
      const aspect = videoGeometry.width / videoGeometry.height;
      const currentWidth = window.innerWidth;
      const currentHeight = window.innerHeight;
      const currentAspect = currentWidth / currentHeight;
      const nextWidth = currentAspect > aspect ? Math.round(currentHeight * aspect) : currentWidth;
      const nextHeight = currentAspect > aspect ? currentHeight : Math.round(currentWidth / aspect);
      await appWindow.setSize(new LogicalSize(nextWidth, nextHeight));
    } catch (e) {
      onError(errorMessage(e));
    }
  }, [onError, videoGeometry]);

  const applyVolume = useCallback((next: number) => {
    const clamped = clampVolume(next);
    setVolume(clamped);
    volumeRef.current = clamped;
    saveVolume(clamped);
    setMuted(false);
  }, []);

  const flashVolumeOsd = useCallback(() => {
    setVolumeOsdVisible(true);
    if (volumeOsdTimerRef.current !== null) window.clearTimeout(volumeOsdTimerRef.current);
    volumeOsdTimerRef.current = window.setTimeout(() => {
      setVolumeOsdVisible(false);
      volumeOsdTimerRef.current = null;
    }, 1200);
  }, []);

  const toggleMuteUi = useCallback(() => {
    setMuted((m) => !m);
  }, []);

  const toggleMuteHotkey = useCallback(() => {
    setMuted((m) => !m);
    flashVolumeOsd();
  }, [flashVolumeOsd]);

  const adjustVolumeWithOsd = useCallback(
    (delta: number) => {
      const snapped = Math.round(volumeRef.current / HOTKEY_STEP) * HOTKEY_STEP;
      applyVolume(snapped + delta);
      flashVolumeOsd();
    },
    [applyVolume, flashVolumeOsd],
  );

  const openVolumePopup = useCallback(() => {
    if (volumeHideTimerRef.current !== null) {
      window.clearTimeout(volumeHideTimerRef.current);
      volumeHideTimerRef.current = null;
    }
    setVolumePopupOpen(true);
  }, []);

  const scheduleVolumePopupHide = useCallback(() => {
    if (volumeHideTimerRef.current !== null) window.clearTimeout(volumeHideTimerRef.current);
    volumeHideTimerRef.current = window.setTimeout(() => {
      setVolumePopupOpen(false);
      volumeHideTimerRef.current = null;
    }, 500);
  }, []);

  useEffect(() => {
    return () => {
      if (volumeHideTimerRef.current !== null) window.clearTimeout(volumeHideTimerRef.current);
      if (volumeOsdTimerRef.current !== null) window.clearTimeout(volumeOsdTimerRef.current);
    };
  }, []);

  const hidePlayer = useCallback(async () => {
    try {
      await invoke("mpv_set_pause", { paused: true });
      setPaused(true);
      await persistProgress();
      onBack();
    } catch (e) {
      onError(errorMessage(e));
    }
  }, [onBack, onError, persistProgress]);

  const loadSibling = useCallback(
    (delta: number) => {
      const next = playlist[selectedIndex + delta];
      if (!next) return;
      void persistProgress()
        .catch((e) => onError(errorMessage(e)))
        .then(async () => {
          if (delta === 1) {
            const cur = playbackRef.current;
            const nearEnd =
              cur.duration > 0 && cur.position / cur.duration >= NEAR_END_PROGRESS_RATIO;
            if (nearEnd) {
              try {
                const saved = await saveEpisodeProgress(
                  next.id,
                  0,
                  next.duration_seconds,
                  false,
                );
                onProgressSaved(saved);
                onSelectEpisode(saved);
                return;
              } catch (e) {
                onError(errorMessage(e));
              }
            }
          }
          onSelectEpisode(next);
        });
    },
    [onError, onProgressSaved, onSelectEpisode, persistProgress, playlist, selectedIndex],
  );

  const onCanvasMouseDown = useCallback(
    (e: React.MouseEvent<HTMLDivElement>) => {
      if (e.button !== 0) return;
      if (e.detail === 2) {
        void toggleFullscreen();
        return;
      }
      void appWindow.startDragging();
    },
    [toggleFullscreen],
  );

  const onCanvasContextMenu = useCallback(
    (e: React.MouseEvent<HTMLDivElement>) => {
      e.preventDefault();
      onTogglePause();
    },
    [onTogglePause],
  );

  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.repeat) return;
      if (isTextInputTarget(e.target)) return;

      if (e.code === "Space") {
        e.preventDefault();
        onTogglePause();
        return;
      }
      if (e.ctrlKey && e.code === "ArrowLeft") {
        if (!visible) return;
        e.preventDefault();
        if (canPrev) loadSibling(-1);
        return;
      }
      if (e.ctrlKey && e.code === "ArrowRight") {
        if (!visible) return;
        e.preventDefault();
        if (canNext) loadSibling(1);
        return;
      }
      if (e.code === "ArrowLeft") {
        e.preventDefault();
        void invoke("mpv_seek_relative", { delta: -5 }).catch((err) =>
          onError(errorMessage(err)),
        );
        return;
      }
      if (e.code === "ArrowRight") {
        e.preventDefault();
        void invoke("mpv_seek_relative", { delta: 5 }).catch((err) =>
          onError(errorMessage(err)),
        );
        return;
      }
      if (e.code === "Numpad4") {
        e.preventDefault();
        void invoke("mpv_seek_relative", { delta: -28 }).catch((err) =>
          onError(errorMessage(err)),
        );
        return;
      }
      if (e.code === "Numpad6") {
        e.preventDefault();
        void invoke("mpv_seek_relative", { delta: 28 }).catch((err) =>
          onError(errorMessage(err)),
        );
        return;
      }
      if (e.code === "Numpad7") {
        e.preventDefault();
        void invoke("mpv_seek_relative", { delta: -85 }).catch((err) =>
          onError(errorMessage(err)),
        );
        return;
      }
      if (e.code === "Numpad9") {
        e.preventDefault();
        void invoke("mpv_seek_relative", { delta: 85 }).catch((err) =>
          onError(errorMessage(err)),
        );
        return;
      }
      if (e.code === "KeyW") {
        if (!visible) return;
        e.preventDefault();
        adjustVolumeWithOsd(HOTKEY_STEP);
        return;
      }
      if (e.code === "KeyS") {
        if (!visible) return;
        e.preventDefault();
        adjustVolumeWithOsd(-HOTKEY_STEP);
        return;
      }
      if (e.code === "KeyM") {
        if (!visible) return;
        e.preventDefault();
        toggleMuteHotkey();
        return;
      }
      if (e.code === "KeyF") {
        e.preventDefault();
        void toggleFullscreen();
        return;
      }
      if (e.code === "KeyC") {
        if (!visible) return;
        if (e.ctrlKey || e.metaKey || e.altKey) return;
        e.preventDefault();
        setControlsVisible((prev) => {
          const next = !prev;
          if (next) {
            queueMicrotask(() => scheduleControlsHide());
          } else {
            clearControlsHideTimer();
          }
          return next;
        });
        return;
      }
      if (e.code === "KeyQ") {
        // Q outside the player is owned by App.tsx (it picks an episode for
        // the current anime); here we only close the playback view.
        if (!visible) return;
        e.preventDefault();
        void hidePlayer();
        return;
      }
      if (e.code === "Escape") {
        if (!visible) return;
        e.preventDefault();
        void hidePlayer();
      }
    };
    window.addEventListener("keydown", onKeyDown, true);
    return () => window.removeEventListener("keydown", onKeyDown, true);
  }, [
    adjustVolumeWithOsd,
    toggleMuteHotkey,
    canNext,
    canPrev,
    clearControlsHideTimer,
    hidePlayer,
    loadSibling,
    onError,
    onTogglePause,
    scheduleControlsHide,
    toggleFullscreen,
    visible,
  ]);

  useEffect(() => {
    if (!visible) return;
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      adjustVolumeWithOsd(e.deltaY < 0 ? HOTKEY_STEP : -HOTKEY_STEP);
    };
    window.addEventListener("wheel", onWheel, { passive: false });
    return () => window.removeEventListener("wheel", onWheel);
  }, [adjustVolumeWithOsd, visible]);

  const safeDuration = duration > 0 ? duration : 0;

  return (
    <section
      className={
        `${videoCompositorRevealed ? "player player--playback" : "player player--playback-pending"}${
          visible ? "" : " player--hidden"
        }${controlsVisible ? " player--controls-visible" : " player--controls-hidden"}`
      }
      onPointerMove={revealControlsFromPointer}
    >
      <div
        className="player-canvas"
        onMouseDown={onCanvasMouseDown}
        onContextMenu={onCanvasContextMenu}
      />
      <div
        className={
          videoCompositorRevealed
            ? "player-load-fade player-load-fade--hidden"
            : "player-load-fade"
        }
      />
      <div className={`volume-osd${volumeOsdVisible && !volumePopupOpen ? "" : " volume-osd--hidden"}`}>
        <div className="volume-osd-bar">
          <div
            className={`volume-osd-fill${!muted && volume > 100 ? " volume-osd-fill--high" : ""}`}
            style={{ height: `${muted ? 0 : Math.min(100, (volume / MAX_VOLUME) * 100)}%` }}
          />
        </div>
        <span
          className={`volume-osd-percent${!muted && volume > 100 ? " volume-osd-percent--high" : ""}${
            muted ? " volume-osd-percent--muted" : ""
          }`}
        >
          {muted ? "Muted" : volume}
        </span>
      </div>
      <button type="button" className="player-back back-button" onClick={() => void hidePlayer()} aria-label="Back">
        <ArrowLeftIcon />
      </button>
      <div className="now-playing" title={episode.path}>
        {episode.file_name}
      </div>
      <div className="player-controls ui-surface">
        <div className="player-controls-content">
          <SeekBar
            duration={safeDuration}
            position={Math.min(position, safeDuration || position)}
            onSeek={onSeekCommit}
            onInteractionChange={setSeekInteracting}
          />
          <div className="controls-main-viewport">
            <div className="controls-main">
              <div className="controls-left">
                <button
                  type="button"
                  className={`icon-button icon-button--player icon-button--lg${canPrev ? "" : " icon-button--disabled"}`}
                  disabled={!canPrev}
                  onClick={() => loadSibling(-1)}
                  title="Previous"
                >
                  <svg viewBox="0 0 24 24" fill="currentColor" aria-hidden>
                    <path d="M6 18V6h2v12H6zm3.5-6 8.5 6V6l-8.5 6z" />
                  </svg>
                </button>
                <button
                  type="button"
                  className="icon-button icon-button--player icon-button--lg"
                  onClick={onTogglePause}
                  title={paused ? "Play" : "Pause"}
                >
                  {paused ? (
                    <svg viewBox="0 0 24 24" fill="currentColor" aria-hidden>
                      <path d="M8,5.14V19.14L19,12.14L8,5.14Z" />
                    </svg>
                  ) : (
                    <svg viewBox="0 0 24 24" fill="currentColor" aria-hidden>
                      <path d="M14,19H18V5H14M6,19H10V5H6V19Z" />
                    </svg>
                  )}
                </button>
                <button
                  type="button"
                  className={`icon-button icon-button--player icon-button--lg${canNext ? "" : " icon-button--disabled"}`}
                  disabled={!canNext}
                  onClick={() => loadSibling(1)}
                  title="Next"
                >
                  <svg viewBox="0 0 24 24" fill="currentColor" aria-hidden>
                    <path d="M16 6v12h2V6h-2zm-1.5 6L6 18V6l8.5 6z" />
                  </svg>
                </button>
                <div className="time-display">
                  <span>{formatTime(Math.min(position, safeDuration || position))}</span>
                  <span className="separator">/</span>
                  <span>{formatTime(safeDuration)}</span>
                </div>
              </div>
              <div className="controls-right">
                <TrackMenu
                  kind="audio"
                  label={selectedAudioTrack ? trackLabel(selectedAudioTrack) : "Audio"}
                  tracks={audioTracks}
                  selectedTrackId={selectedAudioTrack?.id ?? null}
                  open={activeTrackMenu === "audio"}
                  onToggle={() => setActiveTrackMenu((current) => (current === "audio" ? null : "audio"))}
                  onSelect={(trackId) => void selectAudioTrack(trackId)}
                  onDismiss={closeTrackMenu}
                />
                <TrackMenu
                  kind="sub"
                  label={selectedSubtitleTrack ? trackLabel(selectedSubtitleTrack) : "Subs"}
                  tracks={subtitleTracks}
                  selectedTrackId={selectedSubtitleTrack?.id ?? null}
                  open={activeTrackMenu === "sub"}
                  onToggle={() => setActiveTrackMenu((current) => (current === "sub" ? null : "sub"))}
                  onSelect={(trackId) => void selectSubtitleTrack(trackId)}
                  onDisable={() => void selectSubtitleTrack(null)}
                  onBrowse={() => void browseSubtitleFile()}
                  browseLabel="Select file..."
                  onDismiss={closeTrackMenu}
                />
                <VolumeControl
                  volume={volume}
                  muted={muted}
                  popupOpen={volumePopupOpen}
                  onApplyVolume={applyVolume}
                  onToggleMute={toggleMuteUi}
                  onOpenPopup={openVolumePopup}
                  onScheduleHidePopup={scheduleVolumePopupHide}
                />
                <button
                  type="button"
                  className="icon-button icon-button--player icon-button--lg"
                  onClick={() => void fitWindowToAspect()}
                  title="Fit window to video aspect"
                  disabled={!videoGeometry}
                >
                  <svg viewBox="0 0 24 24" fill="currentColor" aria-hidden>
                    <path d="M4 7h16v10H4V7zm2 2v6h12V9H6zm3-6h6v2H9V3zm0 16h6v2H9v-2z" />
                  </svg>
                </button>
                <button
                  type="button"
                  className="icon-button icon-button--player icon-button--lg"
                  onClick={() => void toggleFullscreen()}
                  title={fullscreen ? "Exit fullscreen (F, F11)" : "Fullscreen (F, F11)"}
                >
                  {fullscreen ? (
                    <svg viewBox="0 0 24 24" fill="currentColor" aria-hidden>
                      <path d="M5 16h3v3h2v-5H5v2zm3-8H5v2h5V5H8v3zm6 11h2v-3h3v-2h-5v5zm2-11V5h-2v5h5V8h-3z" />
                    </svg>
                  ) : (
                    <svg viewBox="0 0 24 24" fill="currentColor" aria-hidden>
                      <path d="M7 14H5v5h5v-2H7v-3zm-2-4h2V7h3V5H5v5zm12 7h-3v2h5v-5h-2v3zM14 5v2h3v3h2V5h-5z" />
                    </svg>
                  )}
                </button>
              </div>
            </div>
          </div>
        </div>
      </div>
    </section>
  );
}

function TrackMenu(props: {
  kind: "audio" | "sub";
  label: string;
  tracks: MpvTrack[];
  selectedTrackId: number | null;
  open: boolean;
  onToggle: () => void;
  onSelect: (trackId: number) => void;
  onDisable?: () => void;
  onBrowse?: () => void;
  browseLabel?: string;
  onDismiss?: () => void;
}) {
  const {
    kind,
    label,
    tracks,
    selectedTrackId,
    open,
    onToggle,
    onSelect,
    onDisable,
    onBrowse,
    browseLabel,
    onDismiss,
  } = props;
  const emptyLabel = kind === "audio" ? "No audio tracks" : "No subtitles";
  const rootRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open || !onDismiss) return;
    const onPointerDownCapture = (e: PointerEvent) => {
      const root = rootRef.current;
      if (!root || root.contains(e.target as Node)) return;
      onDismiss();
    };
    document.addEventListener("pointerdown", onPointerDownCapture, true);
    return () => document.removeEventListener("pointerdown", onPointerDownCapture, true);
  }, [open, onDismiss]);

  return (
    <div ref={rootRef} className="track-menu">
      <button type="button" className="track-menu-trigger" onClick={onToggle}>
        {label}
      </button>
      {open ? (
        <div className="track-menu-popover">
          {onDisable ? (
            <button
              type="button"
              className={selectedTrackId === null ? "track-menu-option active" : "track-menu-option"}
              onClick={onDisable}
            >
              Off
            </button>
          ) : null}
          {tracks.map((track) => (
            <button
              type="button"
              key={track.id}
              className={track.id === selectedTrackId ? "track-menu-option active" : "track-menu-option"}
              onClick={() => onSelect(track.id)}
            >
              {trackLabel(track)}
            </button>
          ))}
          {onBrowse ? (
            <button type="button" className="track-menu-option" onClick={onBrowse}>
              {browseLabel ?? "Browse..."}
            </button>
          ) : null}
          {tracks.length === 0 ? <div className="track-menu-empty">{emptyLabel}</div> : null}
        </div>
      ) : null}
    </div>
  );
}

function trackLabel(track: MpvTrack) {
  const parts = [track.lang?.toUpperCase(), track.title].filter(Boolean);
  return parts.length > 0 ? parts.join(" - ") : `Track ${track.id}`;
}

function VolumeControl(props: {
  volume: number;
  muted: boolean;
  popupOpen: boolean;
  onApplyVolume: (volume: number) => void;
  onToggleMute: () => void;
  onOpenPopup: () => void;
  onScheduleHidePopup: () => void;
}) {
  const { volume, muted, popupOpen, onApplyVolume, onToggleMute, onOpenPopup, onScheduleHidePopup } = props;
  const popupRef = useRef<HTMLDivElement>(null);
  const trackRef = useRef<HTMLDivElement>(null);
  const draggingRef = useRef(false);
  const activePointerRef = useRef<number | null>(null);
  const dragCleanupRef = useRef<(() => void) | null>(null);

  const [isDragging, setIsDragging] = useState(false);

  const volumeFromClientY = (clientY: number) => {
    const track = trackRef.current;
    if (!track) return volume;
    const rect = track.getBoundingClientRect();
    const offset = rect.width / 2;
    const ratio = 1 - Math.max(0, Math.min(1, (clientY - rect.top - offset + 1) / (rect.height - rect.width)));
    return clampVolume(Math.round(ratio * MAX_VOLUME));
  };

  useEffect(() => () => dragCleanupRef.current?.(), []);

  const onPointerDown = (e: React.PointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return;
    e.preventDefault();
    draggingRef.current = true;
    setIsDragging(true);
    activePointerRef.current = e.pointerId;
    popupRef.current?.setPointerCapture(e.pointerId);
    onApplyVolume(volumeFromClientY(e.clientY));

    const onMove = (ev: PointerEvent) => {
      if (!draggingRef.current || ev.pointerId !== activePointerRef.current) return;
      onApplyVolume(volumeFromClientY(ev.clientY));
    };
    const stopDrag = (ev?: PointerEvent) => {
      if (ev && ev.pointerId !== activePointerRef.current) return;
      if (ev && popupRef.current?.hasPointerCapture(ev.pointerId)) {
        popupRef.current.releasePointerCapture(ev.pointerId);
      }
      draggingRef.current = false;
      setIsDragging(false);
      activePointerRef.current = null;
      dragCleanupRef.current?.();
      dragCleanupRef.current = null;
    };
    const onUp = (ev: PointerEvent) => {
      if (ev.pointerId !== activePointerRef.current) return;
      onApplyVolume(volumeFromClientY(ev.clientY));
      stopDrag(ev);
    };

    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
    window.addEventListener("pointercancel", stopDrag);
    dragCleanupRef.current = () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
      window.removeEventListener("pointercancel", stopDrag);
    };
  };

  const trackWidth = trackRef.current?.clientWidth ?? 6;
  const fillOffset = Math.min(MAX_VOLUME + trackWidth, volume + trackWidth);
  const handleOffset = MAX_VOLUME - Math.min(MAX_VOLUME, volume) + trackWidth / 2;

  const volumeIcon =
    muted ? (
      <svg viewBox="0 0 24 24" fill="currentColor" aria-hidden>
        <path d="M16.5 12A4.5 4.5 0 0 0 14 7.97v2.21l2.45 2.45c.03-.2.05-.41.05-.63zm2.5 0c0 .94-.2 1.82-.54 2.64l1.51 1.51A8.796 8.796 0 0 0 21 12c0-4.28-2.99-7.86-7-8.77v2.06c2.89.86 5 3.54 5 6.71zM4.27 3 3 4.27 7.73 9H3v6h4l5 5v-6.73l4.25 4.25c-.67.52-1.42.93-2.25 1.18v2.06a8.99 8.99 0 0 0 3.69-1.81L19.73 21 21 19.73l-9-9L4.27 3zM12 4 9.91 6.09 12 8.18V4z" />
      </svg>
    ) : volume === 0 ? (
      <svg viewBox="0 0 24 24" fill="currentColor" aria-hidden>
        <path d="M7 9v6h4l5 5V4l-5 5H7z" />
      </svg>
    ) : volume <= 33 ? (
      <svg viewBox="0 0 24 24" fill="currentColor" aria-hidden>
        <path d="M7 9v6h4l5 5V4l-5 5H7z" />
      </svg>
    ) : volume <= 66 ? (
      <svg viewBox="0 0 24 24" fill="currentColor" aria-hidden>
        <path d="M18.5 12A4.5 4.5 0 0 0 16 7.97v8.05c1.48-.73 2.5-2.25 2.5-4.02zM5 9v6h4l5 5V4L9 9H5z" />
      </svg>
    ) : (
      <svg viewBox="0 0 24 24" fill="currentColor" aria-hidden>
        <path d="M3 9v6h4l5 5V4L7 9H3zm13.5 3A4.5 4.5 0 0 0 14 7.97v8.05c1.48-.73 2.5-2.25 2.5-4.02zM14 3.23v2.06c2.89.86 5 3.54 5 6.71s-2.11 5.85-5 6.71v2.06c4.01-.91 7-4.49 7-8.77s-2.99-7.86-7-8.77z" />
      </svg>
    );

  return (
    <div
      className="volume-control"
      onMouseEnter={onOpenPopup}
      onMouseLeave={onScheduleHidePopup}
    >
      <button
        type="button"
        id="volume-control-button"
        className="icon-button icon-button--player icon-button--lg"
        title={muted ? `Unmute (M)` : `Mute (M)`}
        aria-pressed={muted}
        onClick={(e) => {
          e.preventDefault();
          e.stopPropagation();
          onToggleMute();
        }}
      >
        {volumeIcon}
      </button>
      {popupOpen ? (
        <div className={`volume-popup-wrap${isDragging ? " is-dragging" : ""}`}>
          <div
            ref={popupRef}
            className={`volume-popup${isDragging ? " is-dragging" : ""}`}
            onPointerDown={onPointerDown}
          >
            <div
              ref={trackRef}
              className={`volume-slider-track${muted ? " volume-slider-track--muted" : ""}`}
              style={{ height: `${MAX_VOLUME + trackWidth}px` }}
            >
              <div
                className={`volume-slider-fill${!muted && volume > 100 ? " volume-slider-fill--high" : ""}`}
                style={{ height: `${fillOffset}px` }}
              />
              <div className="volume-slider-handle" style={{ top: `${handleOffset}px` }} />
            </div>
          </div>
          <div
            className={`volume-label${!muted && volume > 100 ? " volume-label--high" : ""}`}
            style={{ top: `${12 + handleOffset}px` }}
          >
            {volume}
          </div>
        </div>
      ) : null}
    </div>
  );
}

function ArrowLeftIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="currentColor" aria-hidden>
      <path d="M20 11H7.83l5.59-5.59L12 4l-8 8 8 8 1.42-1.41L7.83 13H20v-2z" />
    </svg>
  );
}
