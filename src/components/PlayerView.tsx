import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { saveEpisodeProgress } from "../api";
import type { Episode } from "../types";
import { errorMessage, formatTime } from "../utils";

const APP_SIDEBAR_PX = 280;
const appWindow = getCurrentWindow();

function isTextInputTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  const tag = target.tagName;
  if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") return true;
  return target.isContentEditable;
}

function SeekBar(props: {
  duration: number;
  position: number;
  onSeek: (seconds: number) => void;
}) {
  const { duration, position, onSeek } = props;
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
  };

  const onPointerDown = (e: React.PointerEvent<HTMLDivElement>) => {
    if (e.button !== 0 || duration <= 0) return;
    e.preventDefault();
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
      return;
    }
    const ratio = getRatioFromClientX(e.clientX);
    setHoverRatio(ratio);
    setHoverTime(ratio * duration);
    setShowHoverTime(true);
  };

  const hideHoverTime = () => {
    if (isDraggingRef.current) return;
    setShowHoverTime(false);
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
  onSelectEpisode: (episode: Episode) => void;
  onBack: () => void;
  onProgressSaved: (episode: Episode) => void;
  onError: (message: string) => void;
}) {
  const { episode, playlist, onSelectEpisode, onBack, onProgressSaved, onError } = props;
  const [paused, setPaused] = useState(true);
  const [position, setPosition] = useState(episode.position_seconds || 0);
  const [duration, setDuration] = useState(episode.duration_seconds || 0);
  const [fullscreen, setFullscreen] = useState(false);
  const [videoCompositorRevealed, setVideoCompositorRevealed] = useState(false);
  const mpvReadyRef = useRef(false);
  const playbackRef = useRef({ episode, position, duration });
  const pendingResumeSecondsRef = useRef<number | null>(null);

  const selectedIndex = playlist.findIndex((item) => item.id === episode.id);
  const canPrev = selectedIndex > 0;
  const canNext = selectedIndex >= 0 && selectedIndex < playlist.length - 1;

  useEffect(() => {
    playbackRef.current = { episode, position, duration };
  }, [duration, episode, position]);

  const persistProgress = useCallback(
    async (forceWatched = false) => {
      const current = playbackRef.current;
      const watched =
        forceWatched ||
        (current.duration > 0 && current.position / current.duration >= 0.9);
      const saved = await saveEpisodeProgress(
        current.episode.id,
        current.position,
        current.duration,
        watched,
      );
      onProgressSaved(saved);
      return saved;
    },
    [onProgressSaved],
  );

  useEffect(() => {
    setPosition(episode.position_seconds || 0);
    setDuration(episode.duration_seconds || 0);
    setVideoCompositorRevealed(false);
  }, [episode.id, episode.duration_seconds, episode.position_seconds]);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        if (!mpvReadyRef.current) {
          await invoke("mpv_init", {
            windowWidth: window.innerWidth,
            sidebarPx: APP_SIDEBAR_PX,
          });
          mpvReadyRef.current = true;
        } else {
          await invoke("mpv_set_layout", {
            windowWidth: window.innerWidth,
            sidebarPx: APP_SIDEBAR_PX,
          });
        }
        if (cancelled) return;
        pendingResumeSecondsRef.current =
          episode.position_seconds > 1 && !episode.watched ? episode.position_seconds : null;
        setPaused(false);
        await invoke("mpv_load", { path: episode.path });
      } catch (e) {
        if (!cancelled) onError(errorMessage(e));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [episode.path, episode.position_seconds, episode.watched, onError]);

  useEffect(() => {
    const unlisteners: UnlistenFn[] = [];
    let cancelled = false;

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
              void persistProgress(true).catch((err) => onError(errorMessage(err)));
            }
          },
        ],
        [
          "mpv://file-loaded",
          () => {
            const seconds = pendingResumeSecondsRef.current;
            pendingResumeSecondsRef.current = null;
            if (seconds === null) return;
            window.setTimeout(() => {
              void invoke("mpv_seek", { seconds }).catch((err) => onError(errorMessage(err)));
            }, 0);
          },
        ],
        [
          "mpv://playback-restart",
          () => {
            setPaused(false);
            setVideoCompositorRevealed(true);
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
    })();

    return () => {
      cancelled = true;
      unlisteners.forEach((fn) => fn());
    };
  }, [onError, persistProgress]);

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

  const toggleFullscreen = useCallback(async () => {
    try {
      const next = !(await appWindow.isFullscreen());
      await appWindow.setFullscreen(next);
      setFullscreen(next);
    } catch (e) {
      onError(errorMessage(e));
    }
  }, [onError]);

  const closeVideo = useCallback(async () => {
    try {
      await persistProgress();
      await invoke("mpv_stop");
    } catch (e) {
      onError(errorMessage(e));
    } finally {
      setPosition(0);
      setDuration(0);
      setPaused(true);
      onBack();
    }
  }, [onBack, onError, persistProgress]);

  const loadSibling = useCallback(
    (delta: number) => {
      const next = playlist[selectedIndex + delta];
      if (!next) return;
      void persistProgress()
        .catch((e) => onError(errorMessage(e)))
        .finally(() => onSelectEpisode(next));
    },
    [onError, onSelectEpisode, persistProgress, playlist, selectedIndex],
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
      if (e.code === "KeyF") {
        e.preventDefault();
        void toggleFullscreen();
      }
    };
    window.addEventListener("keydown", onKeyDown, true);
    return () => window.removeEventListener("keydown", onKeyDown, true);
  }, [onError, onTogglePause, toggleFullscreen]);

  const safeDuration = duration > 0 ? duration : 0;

  return (
    <section
      className={
        videoCompositorRevealed
          ? "player player--playback"
          : "player player--playback-pending"
      }
    >
      <div
        className="player-canvas"
        onMouseDown={onCanvasMouseDown}
        onContextMenu={onCanvasContextMenu}
      />
      <button type="button" className="player-back back-button" onClick={() => void closeVideo()} aria-label="Back">
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
                <button
                  type="button"
                  className="icon-button icon-button--player icon-button--lg"
                  onClick={() => void toggleFullscreen()}
                  title={fullscreen ? "Exit fullscreen (F)" : "Fullscreen (F)"}
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
                <button
                  type="button"
                  className="icon-button icon-button--player icon-button--lg"
                  onClick={() => void closeVideo()}
                  title="Close video"
                  aria-label="Close video"
                >
                  <svg viewBox="0 0 24 24" fill="currentColor" aria-hidden>
                    <path d="M19 6.41L17.59 5 12 10.59 6.41 5 5 6.41 10.59 12 5 17.59 6.41 19 12 13.41 17.59 19 19 17.59 13.41 12z" />
                  </svg>
                </button>
              </div>
            </div>
          </div>
        </div>
      </div>
    </section>
  );
}

function ArrowLeftIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="currentColor" aria-hidden>
      <path d="M20 11H7.83l5.59-5.59L12 4l-8 8 8 8 1.42-1.41L7.83 13H20v-2z" />
    </svg>
  );
}
