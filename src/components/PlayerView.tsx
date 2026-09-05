import { useCallback, useEffect, useMemo, useRef, useState, type MutableRefObject } from "react";
import { invoke } from "@tauri-apps/api/core";
import { LogicalSize } from "@tauri-apps/api/dpi";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { open } from "@tauri-apps/plugin-dialog";
import {
  addMpvSubtitleFile,
  applySavedTrackPrefs,
  getScrubSpriteIfReady,
  jobsEnqueueOpEdChromaForEpisode,
  jobsEnqueueScrubSprite,
  getMinPositionSecondsToPersist,
  getMpvTimePos,
  getMpvTracks,
  getMpvVideoGeometry,
  saveCurrentTrackPrefs,
  saveEpisodeProgress,
  selectMpvAudioTrack,
  selectMpvSubtitleTrack,
  setMpvVolume,
  syncAnilistEpisodeProgress,
} from "../api";
import { trackPrefFromTracks, trackPrefsEqual } from "../trackPrefs";
import { opEdSeekMarkers, type OpEdSeekMarker } from "../opEd";
import { animeDisplayTitle } from "../utils";
import type {
  AnilistProgressSyncResult,
  AnimeSummary,
  Episode,
  MpvTrack,
  MpvVideoGeometry,
  ScrubSpriteReady,
  ScrubSpriteStatus,
  TrackPref,
} from "../types";
import { SkipOpEdIcon } from "./Icons";
import {
  errorMessage,
  formatTime,
  isFirstEpisode,
  isTextInputTarget,
  mediaPathsEqual,
  playerNowPlayingLabel,
} from "../utils";
import { VolumeControl, VolumeSpeakerIcon } from "./VolumeControl";
import { HOTKEY_STEP, MAX_VOLUME, clampVolume, loadVolume, saveVolume } from "../volume";

const PLAYER_SIDEBAR_PX = 0;
const HIDDEN_PLAYER_SIDEBAR_PX = 100_000;
/** Same ratio as `persistProgress` marking an episode watched without EOF. */
const NEAR_END_PROGRESS_RATIO = 0.9;
/** Seeking to within this many seconds of EOF (or past it) advances to the next episode. */
const SEEK_NEAR_EOF_THRESHOLD_SECONDS = 1;
const END_ADVANCE_POLL_MS = 400;
const END_ADVANCE_MAX_POLLS = 30;
/** Retry if an advance was started but mpv/file-loaded never completed. */
const EOF_HANDLING_STALL_MS = 8000;
/** Brief revisits to an already-watched episode do not overwrite saved progress. */
const WATCHED_PEEK_MAX_MS = 5 * 60 * 1000;
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

const SCRUB_PREVIEW_MIN_INTERVAL_MS = 50;
const SCRUB_PREVIEW_MIN_DELTA_SECONDS = 0.25;
/** mpv may emit pause / playback-restart after `seek` returns; hold the scrub session until then. */
const SCRUB_SETTLE_MS = 200;

function effectivePlaybackDuration(duration: number, episodeDurationSeconds: number): number {
  if (duration > 0) return duration;
  if (episodeDurationSeconds > 0) return episodeDurationSeconds;
  return 0;
}

/** True when a seek target lands within the near-EOF window or past the file end. */
function seekTargetTriggersEpisodeEnd(targetSeconds: number, durationSeconds: number): boolean {
  if (durationSeconds <= 0) return false;
  return targetSeconds >= durationSeconds - SEEK_NEAR_EOF_THRESHOLD_SECONDS;
}

function SeekBar(props: {
  duration: number;
  position: number;
  onScrubStart?: () => void;
  onScrubPreview: (seconds: number) => void;
  onScrubEnd: (seconds: number) => void;
  onInteractionChange?: (active: boolean) => void;
  sprite?: ScrubSpriteReady | null;
  opEdMarkers?: OpEdSeekMarker[];
}) {
  const { duration, position, onScrubStart, onScrubPreview, onScrubEnd, onInteractionChange, sprite, opEdMarkers } =
    props;
  const areaRef = useRef<HTMLDivElement>(null);
  const durationRef = useRef(duration);
  const onScrubStartRef = useRef(onScrubStart);
  const onScrubPreviewRef = useRef(onScrubPreview);
  const onScrubEndRef = useRef(onScrubEnd);
  durationRef.current = duration;
  onScrubStartRef.current = onScrubStart;
  onScrubPreviewRef.current = onScrubPreview;
  onScrubEndRef.current = onScrubEnd;

  const [isDragging, setIsDragging] = useState(false);
  const isDraggingRef = useRef(false);
  const activePointerId = useRef<number | null>(null);
  const [dragRatio, setDragRatio] = useState<number | null>(null);
  const [showHoverTime, setShowHoverTime] = useState(false);
  const [hoverRatio, setHoverRatio] = useState(0);
  const [hoverTime, setHoverTime] = useState(0);
  const dragListenersCleanup = useRef<(() => void) | null>(null);
  const previewRafRef = useRef<number | null>(null);
  const pendingPreviewSecondsRef = useRef<number | null>(null);
  const lastPreviewSecondsRef = useRef<number | null>(null);
  const lastPreviewAtMsRef = useRef(0);
  const lastScrubSecondsRef = useRef(0);

  const clampRatio = (v: number) => Math.min(1, Math.max(0, v));

  const getRatioFromClientX = (clientX: number) => {
    const container = areaRef.current;
    if (!container) return 0;
    const rect = container.getBoundingClientRect();
    if (rect.width <= 0) return 0;
    return clampRatio((clientX - rect.left) / rect.width);
  };

  const cancelPreviewRaf = () => {
    if (previewRafRef.current !== null) {
      cancelAnimationFrame(previewRafRef.current);
      previewRafRef.current = null;
    }
    pendingPreviewSecondsRef.current = null;
  };

  const emitPreviewSeek = (seconds: number, force: boolean) => {
    const now = performance.now();
    const lastSeconds = lastPreviewSecondsRef.current;
    const elapsed = now - lastPreviewAtMsRef.current;
    if (
      !force &&
      lastSeconds !== null &&
      elapsed < SCRUB_PREVIEW_MIN_INTERVAL_MS &&
      Math.abs(seconds - lastSeconds) < SCRUB_PREVIEW_MIN_DELTA_SECONDS
    ) {
      return;
    }
    lastPreviewSecondsRef.current = seconds;
    lastPreviewAtMsRef.current = now;
    onScrubPreviewRef.current(seconds);
  };

  const flushPendingPreview = (force: boolean) => {
    const pending = pendingPreviewSecondsRef.current;
    if (pending === null) return;
    pendingPreviewSecondsRef.current = null;
    emitPreviewSeek(pending, force);
  };

  const schedulePreviewSeek = (seconds: number, force: boolean) => {
    if (force) {
      cancelPreviewRaf();
      emitPreviewSeek(seconds, true);
      return;
    }
    pendingPreviewSecondsRef.current = seconds;
    if (previewRafRef.current !== null) return;
    previewRafRef.current = requestAnimationFrame(() => {
      previewRafRef.current = null;
      flushPendingPreview(false);
    });
  };

  const detachDragListeners = () => {
    cancelPreviewRaf();
    lastPreviewSecondsRef.current = null;
    lastPreviewAtMsRef.current = 0;
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

    const initialSeconds = ratio * durationRef.current;
    lastScrubSecondsRef.current = initialSeconds;
    onScrubStartRef.current?.();
    schedulePreviewSeek(initialSeconds, true);

    const onMove = (ev: PointerEvent) => {
      if (!isDraggingRef.current || ev.pointerId !== activePointerId.current) return;
      const r = getRatioFromClientX(ev.clientX);
      const seconds = r * durationRef.current;
      lastScrubSecondsRef.current = seconds;
      setDragRatio(r);
      setHoverRatio(r);
      setHoverTime(seconds);
      schedulePreviewSeek(seconds, false);
    };
    const onUp = (ev: PointerEvent) => {
      if (!isDraggingRef.current || ev.pointerId !== activePointerId.current) return;
      cancelPreviewRaf();
      const r = getRatioFromClientX(ev.clientX);
      onScrubEndRef.current(r * durationRef.current);
      stopDragging(ev);
      setShowHoverTime(false);
    };
    const onCancel = (ev: PointerEvent) => {
      if (ev.pointerId !== activePointerId.current) return;
      cancelPreviewRaf();
      onScrubEndRef.current(lastScrubSecondsRef.current);
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

  const previewIndex =
    sprite && sprite.thumbCount > 0
      ? Math.min(sprite.thumbCount - 1, Math.floor(hoverRatio * sprite.thumbCount))
      : 0;
  const previewCol = sprite ? previewIndex % sprite.cols : 0;
  const previewRow = sprite ? Math.floor(previewIndex / sprite.cols) : 0;
  const previewFrameStyle =
    sprite ?
      {
        backgroundImage: `url(${sprite.dataUrl})`,
        backgroundSize: `${sprite.cols * sprite.thumbWidth}px ${sprite.rows * sprite.thumbHeight}px`,
        backgroundPosition: `-${previewCol * sprite.thumbWidth}px -${previewRow * sprite.thumbHeight}px`,
      }
    : undefined;

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
        <div className="scrub-preview" style={{ left: `${hoverRatio * 100}%` }}>
          {sprite && !isDragging ?
            <div className="scrub-preview-frame" style={previewFrameStyle} aria-hidden />
          : null}
          <div className="time-tooltip">{formatTime(hoverTime)}</div>
        </div>
      ) : null}
      <div className="progress-bg">
        {duration > 0 && opEdMarkers?.length ?
          opEdMarkers.map((marker) => (
            <div
              key={`${marker.kind}-${marker.startSec}`}
              className={`seek-op-ed-marker seek-op-ed-marker--${marker.kind}`}
              style={{
                left: `${(marker.startSec / duration) * 100}%`,
                width: `${((marker.endSec - marker.startSec) / duration) * 100}%`,
              }}
              aria-hidden
            />
          ))
        : null}
        <div className="progress-current" style={{ width: `${displayProgressPercent}%` }} />
      </div>
      <div className="scrubber-head" style={{ left: `${displayProgressPercent}%` }} />
    </div>
  );
}

export function PlayerView(props: {
  episode: Episode;
  anime: Pick<AnimeSummary, "title" | "anilist_id" | "anilist_title" | "tracker_offset"> | null;
  preferAnilistDisplayTitle: boolean;
  skipOpEdEnabled: boolean;
  dontSkipFirstEpisodeOpEd: boolean;
  onSkipOpEdEnabledChange: (enabled: boolean) => void;
  playlist: Episode[];
  visible: boolean;
  /** When true, PlayerView must not touch mpv (e.g. manual skip editor owns it). */
  playbackSuspended?: boolean;
  playbackProgressFlushRef: MutableRefObject<(() => Promise<void>) | null>;
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
    anime,
    preferAnilistDisplayTitle,
    skipOpEdEnabled,
    dontSkipFirstEpisodeOpEd,
    onSkipOpEdEnabledChange,
    playlist,
    visible,
    playbackSuspended = false,
    playbackProgressFlushRef,
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
  const seekInteractingRef = useRef(false);
  const scrubSessionRef = useRef<{ resumeAfter: boolean } | null>(null);
  const scrubSeekEpochRef = useRef(0);
  const skippedOpEdRef = useRef(new Set<string>());
  const skipOpEdEnabledRef = useRef(skipOpEdEnabled);
  skipOpEdEnabledRef.current = skipOpEdEnabled;
  const dontSkipFirstEpisodeOpEdRef = useRef(dontSkipFirstEpisodeOpEd);
  dontSkipFirstEpisodeOpEdRef.current = dontSkipFirstEpisodeOpEd;
  const scrubEndIdRef = useRef(0);
  const pausedRef = useRef(true);
  const [scrubSprite, setScrubSprite] = useState<ScrubSpriteReady | null>(null);
  const scrubSpritePathRef = useRef<string | null>(null);
  const [activeTrackMenu, setActiveTrackMenu] = useState<"audio" | "sub" | null>(null);
  const [tracks, setTracks] = useState<MpvTrack[]>([]);
  const appliedTrackPrefRef = useRef<TrackPref | null>(null);
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
  const pendingTrackPrefTargetRef = useRef<Pick<Episode, "id" | "anime_id"> | null>(null);
  const playbackRef = useRef({ episode, position, duration });
  const pendingResumeSecondsRef = useRef<number | null>(null);
  const controlsHideTimerRef = useRef<number | null>(null);
  const handlingEofRef = useRef(false);
  const handlingEofStartedAtMsRef = useRef(0);
  const advancingFromEpisodeIdRef = useRef<number | null>(null);
  const endAdvancePollRef = useRef<number | null>(null);
  const endAdvancePollCountRef = useRef(0);
  const endAdvanceArmedRef = useRef(false);
  const seekHotkeyEpochRef = useRef(0);
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
  const visibleRef = useRef(visible);
  const playbackSuspendedRef = useRef(playbackSuspended);
  playbackSuspendedRef.current = playbackSuspended;
  const prevPlaybackSuspendedRef = useRef(playbackSuspended);
  const minPositionSecondsToPersistRef = useRef(60);
  const sessionOpenedAtMsRef = useRef(Date.now());
  const sessionOpenedAsWatchedRef = useRef(episode.watched);
  const sessionEpisodeSnapshotRef = useRef(episode);
  const userRequestedStartResetRef = useRef(false);
  const lastPersistEpisodeIdRef = useRef<number | null>(null);
  const lastPersistAtMsRef = useRef(0);
  const wasVisibleForAutoPersistRef = useRef(visible);

  useEffect(() => {
    pausedRef.current = paused;
  }, [paused]);

  useEffect(() => {
    visibleRef.current = visible;
  }, [visible]);

  useEffect(() => {
    void getMinPositionSecondsToPersist().then((seconds) => {
      minPositionSecondsToPersistRef.current = seconds;
    });
  }, []);

  useEffect(() => {
    const path = episode.path;
    let cancelled = false;
    scrubSpritePathRef.current = path;
    setScrubSprite(null);

    const applyReadyFromCache = () => {
      void getScrubSpriteIfReady(path)
        .then((ready) => {
          if (cancelled || scrubSpritePathRef.current !== path || !ready) return;
          setScrubSprite(ready);
        })
        .catch(() => {
          /* keep time-only tooltip */
        });
    };

    let unlisten: UnlistenFn | undefined;

    void (async () => {
      applyReadyFromCache();

      unlisten = await listen<ScrubSpriteStatus>("scrub-sprite-ready", (event) => {
        if (scrubSpritePathRef.current !== path) return;
        if (event.payload.status === "ready") {
          // Reload from cache: emitted paths are canonicalized in Rust and may not
          // strictly equal `episode.path`, and the payload is not needed here.
          applyReadyFromCache();
        } else if (event.payload.status === "unavailable" && mediaPathsEqual(event.payload.path, path)) {
          setScrubSprite(null);
        }
      });

      if (cancelled) {
        void unlisten();
        return;
      }

      try {
        await Promise.all([
          jobsEnqueueScrubSprite({
            path,
            priority: "high",
            episodeLabel: episode.file_name,
          }),
          jobsEnqueueOpEdChromaForEpisode({
            episodeId: episode.id,
            priority: "high",
            animeTitle: anime ? animeDisplayTitle(anime, preferAnilistDisplayTitle) : null,
          }),
        ]);
      } catch {
        /* keep time-only tooltip */
      }

      if (!cancelled) {
        applyReadyFromCache();
      }
    })();

    return () => {
      cancelled = true;
      scrubSpritePathRef.current = null;
      void unlisten?.();
    };
  }, [anime, episode.file_name, episode.id, episode.path, preferAnilistDisplayTitle]);

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
    if (!visible) {
      skippedOpEdRef.current.clear();
    }
  }, [visible]);

  useEffect(() => {
    skippedOpEdRef.current.clear();
  }, [episode.id]);

  const maybeSkipOpEdAtPositionRef = useRef<(seconds: number) => boolean>(() => false);
  useEffect(() => {
    maybeSkipOpEdAtPositionRef.current = (seconds: number) => {
      if (!skipOpEdEnabledRef.current) return false;
      if (
        dontSkipFirstEpisodeOpEdRef.current &&
        anime &&
        isFirstEpisode(episode, anime)
      ) {
        return false;
      }
      for (const seg of episode.op_ed_segments) {
        if (seg.status !== "matched" || seg.startSec == null || seg.endSec == null) continue;
        const key = `${episode.id}:${seg.kind}`;
        if (skippedOpEdRef.current.has(key)) continue;
        if (seconds >= seg.startSec && seconds < seg.endSec - 0.25) {
          skippedOpEdRef.current.add(key);
          void invoke("mpv_seek", { seconds: seg.endSec }).catch((err) =>
            propsRef.current.onError(errorMessage(err)),
          );
          setPosition(seg.endSec);
          return true;
        }
      }
      return false;
    };
  }, [anime, episode]);

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
    const compositing = visible && videoCompositorRevealed;
    document.documentElement.classList.toggle("compositor-active", compositing);
    return () => {
      document.documentElement.classList.remove("compositor-active");
    };
  }, [visible, videoCompositorRevealed]);

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

  const syncAnilistProgressInBackground = useCallback((episodeId: number, animeId: number) => {
    void syncAnilistEpisodeProgress(episodeId)
      .then((result) => {
        propsRef.current.onAnilistProgressSynced?.(animeId, result);
      })
      .catch((err) => {
        propsRef.current.onError(errorMessage(err));
      });
  }, []);

  const resolvePlaybackSnapshot = useCallback(async () => {
    const current = playbackRef.current;
    let position = current.position;
    let duration = current.duration;
    if (loadedPathRef.current && mediaPathsEqual(loadedPathRef.current, current.episode.path)) {
      try {
        const actualSeconds = await getMpvTimePos();
        if (Number.isFinite(actualSeconds) && actualSeconds >= 0) {
          position = actualSeconds;
        }
      } catch {
        /* keep React-tracked position */
      }
    }
    if (duration <= 0 && current.episode.duration_seconds > 0) {
      duration = current.episode.duration_seconds;
    }
    return { episode: current.episode, position, duration };
  }, []);

  const isNearEndPlayback = useCallback(
    (position: number, duration: number) =>
      duration > 0 && position / duration >= NEAR_END_PROGRESS_RATIO,
    [],
  );

  const shouldSkipAutoPersist = useCallback((episodeId: number) => {
    return lastPersistEpisodeIdRef.current === episodeId && Date.now() - lastPersistAtMsRef.current < 1000;
  }, []);

  const persistProgress = useCallback(
    async (forceWatched = false, options?: { deferAnilistSync?: boolean }) => {
      const current = await resolvePlaybackSnapshot();
      playbackRef.current = current;
      const nearEnd = isNearEndPlayback(current.position, current.duration);
      const shouldMarkWatched = forceWatched || nearEnd;

      if (userRequestedStartResetRef.current && !shouldMarkWatched) {
        const saved = await saveEpisodeProgress(
          current.episode.id,
          current.position,
          current.duration,
          false,
        );
        lastPersistEpisodeIdRef.current = saved.id;
        lastPersistAtMsRef.current = Date.now();
        propsRef.current.onProgressSaved(saved);
        return saved;
      }

      if (
        !forceWatched &&
        !shouldMarkWatched &&
        !userRequestedStartResetRef.current &&
        sessionOpenedAsWatchedRef.current &&
        Date.now() - sessionOpenedAtMsRef.current < WATCHED_PEEK_MAX_MS
      ) {
        return sessionEpisodeSnapshotRef.current;
      }

      const watched = shouldMarkWatched;
      const saved = await saveEpisodeProgress(
        current.episode.id,
        current.position,
        current.duration,
        watched,
      );
      lastPersistEpisodeIdRef.current = saved.id;
      lastPersistAtMsRef.current = Date.now();
      propsRef.current.onProgressSaved(saved);
      if (watched) {
        if (options?.deferAnilistSync) {
          syncAnilistProgressInBackground(saved.id, saved.anime_id);
        } else {
          try {
            const result = await syncAnilistEpisodeProgress(saved.id);
            propsRef.current.onAnilistProgressSynced?.(saved.anime_id, result);
          } catch (err) {
            propsRef.current.onError(errorMessage(err));
          }
        }
      }
      return saved;
    },
    [isNearEndPlayback, resolvePlaybackSnapshot, syncAnilistProgressInBackground],
  );

  const persistProgressRef = useRef(persistProgress);
  persistProgressRef.current = persistProgress;

  const persistTrackPrefsIfChanged = useCallback(async (
    target?: Pick<Episode, "id" | "anime_id" | "path">,
  ) => {
    const current = target ?? playbackRef.current.episode;
    if (appliedTrackPrefRef.current === null) return;
    if (loadedPathRef.current && !mediaPathsEqual(loadedPathRef.current, current.path)) return;
    try {
      const identity = trackPrefFromTracks(await getMpvTracks());
      if (trackPrefsEqual(identity, appliedTrackPrefRef.current)) return;
      const saved = await saveCurrentTrackPrefs(current.anime_id, current.id);
      if (playbackRef.current.episode.id === current.id) {
        appliedTrackPrefRef.current = saved;
      }
    } catch (e) {
      propsRef.current.onError(errorMessage(e));
    }
  }, []);

  const persistTrackPrefsIfChangedRef = useRef(persistTrackPrefsIfChanged);
  persistTrackPrefsIfChangedRef.current = persistTrackPrefsIfChanged;

  useEffect(() => {
    const wasVisible = wasVisibleForAutoPersistRef.current;
    wasVisibleForAutoPersistRef.current = visible;
    if (!wasVisible || visible) return;
    const current = playbackRef.current.episode;
    if (shouldSkipAutoPersist(current.id)) return;
    void persistTrackPrefsIfChangedRef.current(current);
    void persistProgressRef.current().catch((err) => propsRef.current.onError(errorMessage(err)));
  }, [shouldSkipAutoPersist, visible]);

  useEffect(() => {
    const r = playbackProgressFlushRef;
    r.current = async () => {
      await persistTrackPrefsIfChangedRef.current();
      await persistProgressRef.current();
    };
    return () => {
      r.current = null;
    };
  }, [playbackProgressFlushRef]);

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

  const clearEndAdvancePolling = useCallback(() => {
    if (endAdvancePollRef.current !== null) {
      window.clearInterval(endAdvancePollRef.current);
      endAdvancePollRef.current = null;
    }
    endAdvancePollCountRef.current = 0;
    endAdvanceArmedRef.current = false;
  }, []);

  const handlePlaybackFinished = useCallback(() => {
    const currentEpisodeId = playbackRef.current.episode.id;
    if (advancingFromEpisodeIdRef.current === currentEpisodeId) return;
    if (handlingEofRef.current) {
      if (Date.now() - handlingEofStartedAtMsRef.current < EOF_HANDLING_STALL_MS) return;
      handlingEofRef.current = false;
    }

    handlingEofRef.current = true;
    handlingEofStartedAtMsRef.current = Date.now();
    advancingFromEpisodeIdRef.current = currentEpisodeId;

    const next = playlistRef.current[selectedIndexRef.current + 1];
    void (async () => {
      try {
        if (next) {
          await persistTrackPrefsIfChangedRef.current();
          seamlessAdvanceRef.current = true;
          pendingResumeSecondsRef.current = null;
          pendingTrackPrefTargetRef.current = { id: next.id, anime_id: next.anime_id };
          await invoke("mpv_load", { path: next.path });
          loadedPathRef.current = next.path;
          handlingEofRef.current = false;
          advancingFromEpisodeIdRef.current = null;
          clearEndAdvancePolling();
          void persistProgress(true, { deferAnilistSync: true }).catch((err) =>
            propsRef.current.onError(errorMessage(err)),
          );
          try {
            const saved = await saveEpisodeProgress(next.id, 0, next.duration_seconds, false);
            propsRef.current.onProgressSaved(saved);
            propsRef.current.onSelectEpisode(saved);
          } catch (err) {
            propsRef.current.onError(errorMessage(err));
            propsRef.current.onSelectEpisode(next);
          }
          return;
        }

        await persistTrackPrefsIfChangedRef.current();
        await persistProgress(true, { deferAnilistSync: true });
        handlingEofRef.current = false;
        advancingFromEpisodeIdRef.current = null;
        clearEndAdvancePolling();
        // Keep mpv loaded so the last frame stays composited until App's screen
        // cover is opaque; stopping here flashes the transparent window.
        propsRef.current.onClose();
      } catch (err) {
        handlingEofRef.current = false;
        advancingFromEpisodeIdRef.current = null;
        propsRef.current.onError(errorMessage(err));
      }
    })();
  }, [clearEndAdvancePolling, persistProgress]);

  const pollForEpisodeEndAdvance = useCallback(async () => {
    if (playbackSuspendedRef.current || seekInteractingRef.current) return;
    if (!endAdvanceArmedRef.current) {
      clearEndAdvancePolling();
      return;
    }

    if (
      handlingEofRef.current &&
      Date.now() - handlingEofStartedAtMsRef.current > EOF_HANDLING_STALL_MS
    ) {
      handlingEofRef.current = false;
      advancingFromEpisodeIdRef.current = null;
    }

    endAdvancePollCountRef.current += 1;
    if (endAdvancePollCountRef.current > END_ADVANCE_MAX_POLLS) {
      clearEndAdvancePolling();
      handlingEofRef.current = false;
      advancingFromEpisodeIdRef.current = null;
      return;
    }

    handlePlaybackFinished();
  }, [clearEndAdvancePolling, handlePlaybackFinished]);

  const startEndAdvancePolling = useCallback(() => {
    if (endAdvancePollRef.current !== null) return;
    endAdvancePollCountRef.current = 0;
    endAdvancePollRef.current = window.setInterval(() => {
      void pollForEpisodeEndAdvance();
    }, END_ADVANCE_POLL_MS);
  }, [pollForEpisodeEndAdvance]);

  const armAndAdvanceFromEof = useCallback(() => {
    endAdvanceArmedRef.current = true;
    startEndAdvancePolling();
    handlePlaybackFinished();
  }, [handlePlaybackFinished, startEndAdvancePolling]);

  const maybeAdvanceFromSeekTarget = useCallback((targetSeconds: number): boolean => {
    const { duration, episode } = playbackRef.current;
    const effectiveDuration = effectivePlaybackDuration(duration, episode.duration_seconds);
    if (!seekTargetTriggersEpisodeEnd(targetSeconds, effectiveDuration)) return false;
    handlePlaybackFinished();
    return true;
  }, [handlePlaybackFinished]);

  const armAndAdvanceFromEofRef = useRef(armAndAdvanceFromEof);
  armAndAdvanceFromEofRef.current = armAndAdvanceFromEof;
  const maybeAdvanceFromSeekTargetRef = useRef(maybeAdvanceFromSeekTarget);
  maybeAdvanceFromSeekTargetRef.current = maybeAdvanceFromSeekTarget;
  const clearEndAdvancePollingRef = useRef(clearEndAdvancePolling);
  clearEndAdvancePollingRef.current = clearEndAdvancePolling;

  const seekRelativeFromHotkey = useCallback((delta: number) => {
    const { position, duration, episode } = playbackRef.current;
    const effectiveDuration = effectivePlaybackDuration(duration, episode.duration_seconds);
    const targetSeconds = position + delta;
    if (seekTargetTriggersEpisodeEnd(targetSeconds, effectiveDuration)) {
      seekHotkeyEpochRef.current += 1;
      maybeAdvanceFromSeekTargetRef.current(targetSeconds);
      return;
    }
    seekHotkeyEpochRef.current += 1;
    void invoke("mpv_seek_relative", { delta }).catch((err) =>
      propsRef.current.onError(errorMessage(err)),
    );
  }, []);

  useEffect(() => {
    const unlisteners: UnlistenFn[] = [];
    let cancelled = false;
    eventListenersReadyRef.current = false;

    (async () => {
      const subs: Array<[string, (e: { payload: unknown }) => void]> = [
        [
          "mpv://time-pos",
          (e) => {
            if (playbackSuspendedRef.current) return;
            if (typeof e.payload === "number" && !seekInteractingRef.current) {
              if (!maybeSkipOpEdAtPositionRef.current(e.payload)) {
                setPosition(e.payload);
              }
            }
          },
        ],
        [
          "mpv://duration",
          (e) => {
            if (playbackSuspendedRef.current) return;
            if (typeof e.payload === "number") setDuration(e.payload);
          },
        ],
        [
          "mpv://pause",
          (e) => {
            if (playbackSuspendedRef.current || scrubSessionRef.current) return;
            if (typeof e.payload === "boolean") {
              setPaused(e.payload);
              if (!e.payload) {
                clearEndAdvancePollingRef.current();
              }
            }
          },
        ],
        [
          "mpv://eof-reached",
          (e) => {
            if (playbackSuspendedRef.current) return;
            if (e.payload === true) {
              setPaused(true);
              armAndAdvanceFromEofRef.current();
            }
          },
        ],
        [
          "mpv://file-loaded",
          () => {
            if (playbackSuspendedRef.current) return;
            handlingEofRef.current = false;
            void (async () => {
              try {
                const current = playbackRef.current.episode;
                const target = pendingTrackPrefTargetRef.current ?? current;
                pendingTrackPrefTargetRef.current = null;
                appliedTrackPrefRef.current = await applySavedTrackPrefs(
                  target.anime_id,
                  target.id,
                );
                await refreshTracks();
              } catch (err) {
                propsRef.current.onError(errorMessage(err));
                void refreshTracks();
              }
            })();
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
            if (playbackSuspendedRef.current) return;
            if (scrubSessionRef.current) {
              setVideoCompositorRevealed(true);
              void refreshVideoGeometry();
              return;
            }
            if (!visibleRef.current) return;
            setPaused(false);
            setVideoCompositorRevealed(true);
            void refreshVideoGeometry();
            maybeSkipOpEdAtPositionRef.current(playbackRef.current.position);
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
    const episodeForPrefs = {
      id: episode.id,
      anime_id: episode.anime_id,
      path: episode.path,
    };
    return () => {
      void persistTrackPrefsIfChangedRef.current(episodeForPrefs);
      const episodeId = playbackRef.current.episode.id;
      if (shouldSkipAutoPersist(episodeId)) return;
      void persistProgressRef.current().catch((err) => propsRef.current.onError(errorMessage(err)));
    };
  }, [episode.anime_id, episode.id, episode.path, shouldSkipAutoPersist]);

  useEffect(() => {
    sessionOpenedAtMsRef.current = Date.now();
    sessionOpenedAsWatchedRef.current = episode.watched;
    sessionEpisodeSnapshotRef.current = episode;
    userRequestedStartResetRef.current = false;
    advancingFromEpisodeIdRef.current = null;
    clearEndAdvancePolling();
    setPosition(episode.position_seconds || 0);
    setDuration(episode.duration_seconds || 0);
    appliedTrackPrefRef.current = null;
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
  }, [clearEndAdvancePolling, episode.id]);

  useEffect(() => {
    if (!visible) clearEndAdvancePolling();
    return clearEndAdvancePolling;
  }, [clearEndAdvancePolling, visible]);

  // Manual skip (or any suspended owner) may load a different file or leave mpv
  // paused without unloading. Clear the cached path so the visible effect reloads.
  useEffect(() => {
    const wasSuspended = prevPlaybackSuspendedRef.current;
    prevPlaybackSuspendedRef.current = playbackSuspended;
    if (wasSuspended && !playbackSuspended) {
      loadedPathRef.current = null;
    }
  }, [playbackSuspended]);

  useEffect(() => {
    let cancelled = false;
    // Only auto-unpause on a real visibility transition (false -> true).
    // Without this, any spurious re-run of this effect with visible=true
    // (e.g. a stale dep change) would resume playback the user just paused.
    const becameVisible = visible && !wasVisibleRef.current;
    wasVisibleRef.current = visible;
    (async () => {
      try {
        if (playbackSuspended || !eventListenersReadyRef.current) return;
        // Manual skip may load another file while the player stays hidden in the
        // background. Clearing loadedPathRef on suspension end defers reload until
        // the user opens the player again — do not load or unpause here.
        if (!visible) return;
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
        pendingTrackPrefTargetRef.current = { id: episode.id, anime_id: episode.anime_id };
        await invoke("mpv_load", { path: episode.path });
        loadedPathRef.current = episode.path;
      } catch (e) {
        handlingEofRef.current = false;
        if (!cancelled) propsRef.current.onError(errorMessage(e));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [episode.path, eventListenersReadyVersion, playbackSuspended, visible]);

  useEffect(() => {
    if (playbackSuspended || !mpvReadyRef.current) return;
    void setMpvVolume(muted ? 0 : volume).catch((e) => onError(errorMessage(e)));
  }, [muted, onError, playbackSuspended, volume]);

  useEffect(() => {
    if (playbackSuspended || !mpvReadyRef.current) return;
    void invoke("mpv_set_layout", {
      windowWidth: window.innerWidth,
      sidebarPx: sidebarPxForVisibility(visible),
    }).catch((err) => onError(errorMessage(err)));
  }, [onError, playbackSuspended, visible]);

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

  const cancelScrubSession = useCallback(() => {
    scrubEndIdRef.current += 1;
    scrubSeekEpochRef.current += 1;
    scrubSessionRef.current = null;
    seekInteractingRef.current = false;
    setSeekInteracting(false);
  }, []);

  useEffect(() => {
    if (!visible) cancelScrubSession();
  }, [cancelScrubSession, visible]);

  const onScrubStart = useCallback(() => {
    scrubEndIdRef.current += 1;
    scrubSeekEpochRef.current += 1;
    const resumeAfter = !pausedRef.current;
    scrubSessionRef.current = { resumeAfter };
    seekInteractingRef.current = true;
    void invoke("mpv_set_pause", { paused: true }).catch((e) => onError(errorMessage(e)));
    setPaused(true);
  }, [onError]);

  const onScrubPreview = useCallback(
    (seconds: number) => {
      const epoch = scrubSeekEpochRef.current;
      setPosition(seconds);
      void (async () => {
        try {
          await invoke("mpv_seek", { seconds, keyframe: true });
          const session = scrubSessionRef.current;
          if (scrubSeekEpochRef.current !== epoch || !session) return;
          if (!session.resumeAfter) {
            await invoke("mpv_set_pause", { paused: true });
          }
        } catch (e) {
          onError(errorMessage(e));
        }
      })();
    },
    [onError],
  );

  const onScrubEnd = useCallback(
    (seconds: number) => {
      userRequestedStartResetRef.current = seconds < minPositionSecondsToPersistRef.current;
      scrubSeekEpochRef.current += 1;
      const endId = (scrubEndIdRef.current += 1);
      const resume = scrubSessionRef.current?.resumeAfter ?? false;
      if (maybeAdvanceFromSeekTarget(seconds)) {
        if (scrubEndIdRef.current === endId) {
          scrubSessionRef.current = null;
          seekInteractingRef.current = false;
          setSeekInteracting(false);
        }
        return;
      }
      void (async () => {
        try {
          await invoke("mpv_seek", { seconds, keyframe: true });
          if (scrubEndIdRef.current !== endId) return;
          await invoke("mpv_set_pause", { paused: !resume });
          setPaused(!resume);
          await new Promise((resolve) => window.setTimeout(resolve, SCRUB_SETTLE_MS));
          if (scrubEndIdRef.current !== endId) return;
          await invoke("mpv_set_pause", { paused: !resume });
          setPaused(!resume);
          const actualSeconds = await getMpvTimePos();
          if (scrubEndIdRef.current !== endId) return;
          if (Number.isFinite(actualSeconds) && actualSeconds >= 0) {
            setPosition(actualSeconds);
          } else {
            setPosition(seconds);
          }
        } catch (e) {
          onError(errorMessage(e));
        } finally {
          if (scrubEndIdRef.current === endId) {
            scrubSessionRef.current = null;
            seekInteractingRef.current = false;
          }
        }
      })();
    },
    [maybeAdvanceFromSeekTarget, onError],
  );

  const selectAudioTrack = useCallback(
    async (trackId: number) => {
      try {
        await selectMpvAudioTrack(trackId);
        const current = playbackRef.current.episode;
        appliedTrackPrefRef.current = await saveCurrentTrackPrefs(current.anime_id, current.id);
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
        const current = playbackRef.current.episode;
        appliedTrackPrefRef.current = await saveCurrentTrackPrefs(current.anime_id, current.id);
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
      const current = playbackRef.current.episode;
      appliedTrackPrefRef.current = await saveCurrentTrackPrefs(current.anime_id, current.id);
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
    cancelScrubSession();
    try {
      await invoke("mpv_set_pause", { paused: true });
      setPaused(true);
      await persistTrackPrefsIfChanged();
      await persistProgress();
      onBack();
    } catch (e) {
      onError(errorMessage(e));
    }
  }, [cancelScrubSession, onBack, onError, persistProgress, persistTrackPrefsIfChanged]);

  const loadSibling = useCallback(
    (delta: number) => {
      const next = playlist[selectedIndex + delta];
      if (!next) return;
      void persistTrackPrefsIfChanged()
        .then(() => persistProgress(false, { deferAnilistSync: true }))
        .catch((e) => onError(errorMessage(e)))
        .then(async (saved) => {
          if (delta === 1 && saved?.watched) {
            pendingResumeSecondsRef.current = null;
            try {
              pendingTrackPrefTargetRef.current = { id: next.id, anime_id: next.anime_id };
              const [nextSaved] = await Promise.all([
                saveEpisodeProgress(next.id, 0, next.duration_seconds, false),
                invoke("mpv_load", { path: next.path }),
              ]);
              loadedPathRef.current = next.path;
              onProgressSaved(nextSaved);
              onSelectEpisode(nextSaved);
              return;
            } catch (e) {
              onError(errorMessage(e));
            }
          }
          onSelectEpisode(next);
        });
    },
    [
      onError,
      onProgressSaved,
      onSelectEpisode,
      persistProgress,
      persistTrackPrefsIfChanged,
      playlist,
      selectedIndex,
    ],
  );

  const onCanvasMouseDown = useCallback(
    (e: React.MouseEvent<HTMLDivElement>) => {
      if (e.button !== 0) return;
      if (e.detail === 2) {
        void toggleFullscreen();
        return;
      }
      void (async () => {
        try {
          const [fs, max] = await Promise.all([appWindow.isFullscreen(), appWindow.isMaximized()]);
          if (fs || max) return;
          await appWindow.startDragging();
        } catch {
          /* ignore */
        }
      })();
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
    if (!visible) return;

    const onKeyDown = (e: KeyboardEvent) => {
      if (e.repeat) return;
      if (isTextInputTarget(e.target)) return;

      if (e.code === "Space") {
        e.preventDefault();
        onTogglePause();
        return;
      }
      if (e.ctrlKey && e.code === "ArrowLeft") {
        e.preventDefault();
        if (canPrev) loadSibling(-1);
        return;
      }
      if (e.ctrlKey && e.code === "ArrowRight") {
        e.preventDefault();
        if (canNext) loadSibling(1);
        return;
      }
      if (e.code === "ArrowLeft") {
        e.preventDefault();
        seekRelativeFromHotkey(-5);
        return;
      }
      if (e.code === "ArrowRight") {
        e.preventDefault();
        seekRelativeFromHotkey(5);
        return;
      }
      if (e.code === "Numpad4") {
        e.preventDefault();
        seekRelativeFromHotkey(-28);
        return;
      }
      if (e.code === "Numpad6") {
        e.preventDefault();
        seekRelativeFromHotkey(28);
        return;
      }
      if (e.code === "Numpad7") {
        e.preventDefault();
        seekRelativeFromHotkey(-85);
        return;
      }
      if (e.code === "Numpad9") {
        e.preventDefault();
        seekRelativeFromHotkey(85);
        return;
      }
      if (e.code === "KeyW") {
        e.preventDefault();
        adjustVolumeWithOsd(HOTKEY_STEP);
        return;
      }
      if (e.code === "KeyS") {
        e.preventDefault();
        adjustVolumeWithOsd(-HOTKEY_STEP);
        return;
      }
      if (e.code === "KeyM") {
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
        e.preventDefault();
        void hidePlayer();
        return;
      }
      if (e.code === "Escape") {
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
    onTogglePause,
    scheduleControlsHide,
    seekRelativeFromHotkey,
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
  const seekOpEdMarkers = useMemo(
    () => opEdSeekMarkers(episode.op_ed_segments, safeDuration),
    [episode.op_ed_segments, safeDuration],
  );

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
      <div
        className={`volume-osd${volumeOsdVisible && !volumePopupOpen ? "" : " volume-osd--hidden"}`}
        role="status"
        aria-live="polite"
        aria-label={muted ? "Muted" : undefined}
      >
        <div
          className={`volume-osd-fill${!muted && volume > 100 ? " volume-osd-fill--high" : ""}`}
          style={{ height: `${muted ? 0 : Math.min(100, (volume / MAX_VOLUME) * 100)}%` }}
        />
        {muted || volume === 0 ? (
          <div className="volume-osd-icon-wrap" aria-hidden>
            <VolumeSpeakerIcon volume={volume} muted />
          </div>
        ) : (
          <span
            className={`volume-osd-value${volume > 100 ? " volume-osd-value--high" : ""}`}
          >
            {volume}
          </span>
        )}
      </div>
      <button type="button" className="player-back back-button" onClick={() => void hidePlayer()} aria-label="Back">
        <ArrowLeftIcon />
      </button>
      <div className="now-playing" title={episode.path}>
        {anime
          ? playerNowPlayingLabel(episode, anime, preferAnilistDisplayTitle)
          : episode.file_name}
      </div>
      <div className="player-controls ui-surface">
        <div className="player-controls-content">
          <SeekBar
            duration={safeDuration}
            position={Math.min(position, safeDuration || position)}
            onScrubStart={onScrubStart}
            onScrubPreview={onScrubPreview}
            onScrubEnd={onScrubEnd}
            onInteractionChange={setSeekInteracting}
            sprite={scrubSprite}
            opEdMarkers={seekOpEdMarkers}
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
                  className={`icon-button icon-button--player icon-button--skip-op-ed${skipOpEdEnabled ? " icon-button--skip-op-ed-on" : ""}`}
                  title={
                    skipOpEdEnabled
                      ? "Skip detected OP/ED (on)"
                      : "Skip detected OP/ED (off)"
                  }
                  aria-pressed={skipOpEdEnabled}
                  onClick={() => onSkipOpEdEnabledChange(!skipOpEdEnabled)}
                >
                  <SkipOpEdIcon enabled={skipOpEdEnabled} />
                </button>
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

function ArrowLeftIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="currentColor" aria-hidden>
      <path d="M20 11H7.83l5.59-5.59L12 4l-8 8 8 8 1.42-1.41L7.83 13H20v-2z" />
    </svg>
  );
}
