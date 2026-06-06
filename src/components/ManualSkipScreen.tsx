import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  deleteManualOpEdTemplate,
  listManualOpEdTemplates,
  mpvClearPreviewRect,
  mpvSetPreviewRect,
  probeVideoFps,
  saveManualOpEdTemplate,
  setMpvVolume,
  updateManualOpEdTemplate,
} from "../api";
import {
  readStoredManualSkipHideMatched,
  storeManualSkipHideMatched,
} from "../manualSkipPicker";
import { isOpEdSegmentMissing } from "../opEd";
import { clampVolume, loadVolume, saveVolume } from "../volume";
import type { AnimeSummary, Episode, ManualOpEdTemplate } from "../types";
import {
  errorMessage,
  formatEpisodeNumber,
  formatTime,
  isEpisodeNumberKnown,
} from "../utils";
import { CustomCheckbox } from "./CustomCheckbox";
import { ArrowLeftIcon } from "./Icons";
import {
  clampTemplateRange,
  defaultEditorRange,
  TemplateRangeScrubber,
} from "./TemplateRangeScrubber";
import { ViewHeader } from "./ViewHeader";
import { VolumeControl } from "./VolumeControl";

const HIDDEN_PLAYER_SIDEBAR_PX = 100_000;
const TEST_LEAD_SEC = 2;
const TEST_TAIL_SEC = 2;

type EditorPlayback =
  | {
      mode: "test";
      startSec: number;
      endSec: number;
      endPlaySec: number;
      phase: "pre" | "post-skip";
    }
  | {
      mode: "area";
      endSec: number;
    };

type ScreenView =
  | { kind: "list" }
  | { kind: "picker"; templateKind: "op" | "ed" }
  | {
      kind: "editor";
      templateKind: "op" | "ed";
      episode: Episode;
      templateId?: number;
      startSec: number;
      endSec: number;
      returnView: "list" | "picker";
    };

function templateListLabel(template: ManualOpEdTemplate): string {
  const kind = template.kind.toUpperCase();
  const endSec = template.startSec + template.durationSec;
  return `${kind} #${template.kindIndex} · ${formatTime(template.startSec)}–${formatTime(endSec)} · ${template.sourceEpisodeLabel}`;
}

function episodeRowLabel(episode: Episode, trackerOffset: number): string {
  if (!isEpisodeNumberKnown(episode.episode_number)) {
    return episode.file_name;
  }
  return formatEpisodeNumber(episode.episode_number - trackerOffset);
}

function initialEditorRange(
  kind: "op" | "ed",
  episode: Episode,
): { startSec: number; endSec: number } {
  const duration = episode.duration_seconds > 0 ? episode.duration_seconds : 600;
  const seg = episode.op_ed_segments.find((s) => s.kind === kind);
  if (
    seg?.status === "matched" &&
    seg.searchPass !== "manual" &&
    seg.startSec != null &&
    seg.endSec != null
  ) {
    return clampTemplateRange(seg.startSec, seg.endSec, duration);
  }
  return defaultEditorRange(kind, duration);
}

function MissingSegmentColumn(props: {
  title: string;
  episodes: Episode[];
  trackerOffset: number;
}) {
  const { title, episodes, trackerOffset } = props;
  return (
    <div className="manual-skip-missing-col">
      <h3 className="manual-skip-missing-col__title">{title}</h3>
      {episodes.length > 0 ?
        <ul className="manual-skip-missing-col__list">
          {episodes.map((episode) => (
            <li key={episode.id}>{episodeRowLabel(episode, trackerOffset)}</li>
          ))}
        </ul>
      : null}
    </div>
  );
}

export function ManualSkipScreen(props: {
  anime: AnimeSummary;
  episodes: Episode[];
  onBack: () => void;
  onDirtyClose: () => void;
  onError: (message: string) => void;
}) {
  const { anime, episodes, onBack, onDirtyClose, onError } = props;
  const [view, setView] = useState<ScreenView>({ kind: "list" });
  const [templates, setTemplates] = useState<ManualOpEdTemplate[]>([]);
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [dirty, setDirty] = useState(false);
  const [frameStepSec, setFrameStepSec] = useState(1 / 24);
  const [previewCompositorRevealed, setPreviewCompositorRevealed] = useState(false);
  const [playbackMode, setPlaybackMode] = useState<"test" | "area" | null>(null);
  const [volume, setVolume] = useState(loadVolume);
  const [muted, setMuted] = useState(false);
  const [volumePopupOpen, setVolumePopupOpen] = useState(false);
  const [hideMatched, setHideMatched] = useState(readStoredManualSkipHideMatched);
  const volumeHideTimerRef = useRef<number | null>(null);
  const previewRef = useRef<HTMLDivElement>(null);
  const mpvReadyRef = useRef(false);
  const layoutTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const playbackRef = useRef<EditorPlayback | null>(null);
  const onErrorRef = useRef(onError);
  onErrorRef.current = onError;

  const episodesMissingOp = useMemo(
    () => episodes.filter((episode) => isOpEdSegmentMissing(episode.op_ed_segments, "op")),
    [episodes],
  );
  const episodesMissingEd = useMemo(
    () => episodes.filter((episode) => isOpEdSegmentMissing(episode.op_ed_segments, "ed")),
    [episodes],
  );

  const pickerEpisodes = useMemo(() => {
    if (view.kind !== "picker") return [];
    if (!hideMatched) return episodes;
    return episodes.filter((episode) =>
      isOpEdSegmentMissing(episode.op_ed_segments, view.templateKind),
    );
  }, [episodes, hideMatched, view]);

  const reloadTemplates = useCallback(async () => {
    setLoading(true);
    try {
      const rows = await listManualOpEdTemplates(anime.id);
      setTemplates(rows);
    } catch (e) {
      onErrorRef.current(errorMessage(e));
    } finally {
      setLoading(false);
    }
  }, [anime.id]);

  useEffect(() => {
    setView({ kind: "list" });
    setDirty(false);
    void reloadTemplates();
  }, [anime.id, reloadTemplates]);

  const syncPreviewRect = useCallback(() => {
    const el = previewRef.current;
    if (!el) return;
    const rect = el.getBoundingClientRect();
    if (rect.width < 1 || rect.height < 1) return;
    void mpvSetPreviewRect({
      x: rect.left,
      y: rect.top,
      width: rect.width,
      height: rect.height,
      windowWidth: window.innerWidth,
      windowHeight: window.innerHeight,
    }).catch((e) => onError(errorMessage(e)));
  }, [onError]);

  const schedulePreviewRect = useCallback(() => {
    if (layoutTimerRef.current !== null) {
      window.clearTimeout(layoutTimerRef.current);
    }
    layoutTimerRef.current = window.setTimeout(() => {
      layoutTimerRef.current = null;
      syncPreviewRect();
    }, 16);
  }, [syncPreviewRect]);

  const teardownMpv = useCallback(async () => {
    try {
      await invoke("mpv_stop");
      await mpvClearPreviewRect(window.innerWidth, HIDDEN_PLAYER_SIDEBAR_PX);
    } catch {
      /* ignore teardown errors */
    }
    mpvReadyRef.current = false;
  }, []);

  const loadEditorEpisode = useCallback(
    async (episode: Episode) => {
      try {
        if (!mpvReadyRef.current) {
          await invoke("mpv_init", {
            windowWidth: window.innerWidth,
            sidebarPx: HIDDEN_PLAYER_SIDEBAR_PX,
          });
          mpvReadyRef.current = true;
        }
        await invoke("mpv_load", { path: episode.path });
        await invoke("mpv_set_pause", { paused: true });
        const fps = await probeVideoFps(episode.path);
        setFrameStepSec(fps > 0 ? 1 / fps : 1 / 24);
        schedulePreviewRect();
      } catch (e) {
        onError(errorMessage(e));
      }
    },
    [onError, schedulePreviewRect],
  );

  const editorEpisode = view.kind === "editor" ? view.episode : null;
  const editorCompositing = view.kind === "editor" && previewCompositorRevealed;

  const applyVolume = useCallback((next: number) => {
    const clamped = clampVolume(next);
    setVolume(clamped);
    saveVolume(clamped);
    setMuted(false);
  }, []);

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
    if (view.kind !== "editor") return;
    void setMpvVolume(muted ? 0 : volume).catch((e) => onError(errorMessage(e)));
  }, [muted, onError, view.kind, volume]);

  useEffect(() => {
    return () => {
      if (volumeHideTimerRef.current !== null) window.clearTimeout(volumeHideTimerRef.current);
    };
  }, []);

  useEffect(() => {
    setPreviewCompositorRevealed(false);
  }, [view.kind, editorEpisode?.id]);

  useEffect(() => {
    const root = document.documentElement;
    root.classList.toggle("compositor-active", editorCompositing);
    return () => {
      root.classList.remove("compositor-active");
    };
  }, [editorCompositing]);

  useEffect(() => {
    return () => {
      void teardownMpv();
    };
  }, [teardownMpv]);

  useEffect(() => {
    if (!editorEpisode || view.kind !== "editor") return;
    const startSec = view.startSec;
    void (async () => {
      await loadEditorEpisode(editorEpisode);
      try {
        await invoke("mpv_seek", { seconds: startSec, keyframe: false });
        await invoke("mpv_set_pause", { paused: true });
      } catch (e) {
        onError(errorMessage(e));
      }
    })();
    const el = previewRef.current;
    if (!el) return;
    let cancelled = false;
    const observer = new ResizeObserver(() => schedulePreviewRect());
    observer.observe(el);
    const onResize = () => schedulePreviewRect();
    window.addEventListener("resize", onResize);
    let unlistenFileLoaded: (() => void) | undefined;
    let unlistenPlaybackRestart: (() => void) | undefined;
    let unlistenTimePos: (() => void) | undefined;
    void (async () => {
      unlistenFileLoaded = await listen("mpv://file-loaded", () => schedulePreviewRect());
      unlistenPlaybackRestart = await listen("mpv://playback-restart", () => {
        if (cancelled) return;
        setPreviewCompositorRevealed(true);
        schedulePreviewRect();
      });
      unlistenTimePos = await listen("mpv://time-pos", (e) => {
        if (cancelled) return;
        const playback = playbackRef.current;
        if (!playback || typeof e.payload !== "number") return;
        const pos = e.payload;
        if (playback.mode === "area") {
          if (pos >= playback.endSec) {
            playbackRef.current = null;
            setPlaybackMode(null);
            void invoke("mpv_set_pause", { paused: true }).catch((err) =>
              onError(errorMessage(err)),
            );
          }
          return;
        }
        if (playback.phase === "pre" && pos >= playback.startSec && pos < playback.endSec) {
          playback.phase = "post-skip";
          void invoke("mpv_seek", { seconds: playback.endSec, keyframe: false }).catch((err) =>
            onError(errorMessage(err)),
          );
          return;
        }
        if (playback.phase === "post-skip" && pos >= playback.endSec && pos >= playback.endPlaySec) {
          playbackRef.current = null;
          setPlaybackMode(null);
          void invoke("mpv_set_pause", { paused: true }).catch((err) =>
            onError(errorMessage(err)),
          );
        }
      });
    })();
    return () => {
      cancelled = true;
      playbackRef.current = null;
      setPlaybackMode(null);
      observer.disconnect();
      window.removeEventListener("resize", onResize);
      unlistenFileLoaded?.();
      unlistenPlaybackRestart?.();
      unlistenTimePos?.();
      void teardownMpv();
    };
  }, [editorEpisode?.id, loadEditorEpisode, onError, schedulePreviewRect, teardownMpv]);

  const exitScreen = useCallback(() => {
    void teardownMpv().finally(() => {
      if (dirty) onDirtyClose();
      onBack();
    });
  }, [dirty, onBack, onDirtyClose, teardownMpv]);

  const stopEditorPlayback = useCallback(async () => {
    playbackRef.current = null;
    setPlaybackMode(null);
    try {
      await invoke("mpv_set_pause", { paused: true });
    } catch {
      /* ignore */
    }
  }, []);

  const seekPreview = useCallback(
    (seconds: number) => {
      void stopEditorPlayback().then(() =>
        invoke("mpv_seek", { seconds, keyframe: false })
          .then(() => invoke("mpv_set_pause", { paused: true }))
          .catch((e) => onError(errorMessage(e))),
      );
    },
    [onError, stopEditorPlayback],
  );

  const stopPlaybackOnRangeChange = useCallback(() => {
    void stopEditorPlayback();
  }, [stopEditorPlayback]);

  const handlePlayArea = useCallback(async () => {
    if (view.kind !== "editor") return;
    if (playbackMode === "area") {
      await stopEditorPlayback();
      return;
    }
    const { startSec, endSec } = view;
    try {
      await stopEditorPlayback();
      await invoke("mpv_seek", { seconds: startSec, keyframe: false });
      playbackRef.current = { mode: "area", endSec };
      setPlaybackMode("area");
      await invoke("mpv_set_pause", { paused: false });
    } catch (e) {
      playbackRef.current = null;
      setPlaybackMode(null);
      onError(errorMessage(e));
    }
  }, [onError, playbackMode, stopEditorPlayback, view]);

  const handleTest = useCallback(async () => {
    if (view.kind !== "editor") return;
    const duration =
      view.episode.duration_seconds > 0 ? view.episode.duration_seconds : view.endSec + TEST_TAIL_SEC;
    const startSec = view.startSec;
    const endSec = view.endSec;
    try {
      await stopEditorPlayback();
      const preStart = Math.max(0, startSec - TEST_LEAD_SEC);
      await invoke("mpv_seek", { seconds: preStart, keyframe: false });
      playbackRef.current = {
        mode: "test",
        startSec,
        endSec,
        endPlaySec: Math.min(duration, endSec + TEST_TAIL_SEC),
        phase: "pre",
      };
      setPlaybackMode("test");
      await invoke("mpv_set_pause", { paused: false });
    } catch (e) {
      playbackRef.current = null;
      setPlaybackMode(null);
      onError(errorMessage(e));
    }
  }, [onError, stopEditorPlayback, view]);

  const leaveEditor = useCallback(() => {
    if (view.kind !== "editor") return;
    void stopEditorPlayback();
    void teardownMpv();
    if (view.returnView === "picker") {
      setView({ kind: "picker", templateKind: view.templateKind });
      return;
    }
    setView({ kind: "list" });
  }, [stopEditorPlayback, teardownMpv, view]);

  const handleBackStep = useCallback(() => {
    if (view.kind === "editor") {
      leaveEditor();
      return;
    }
    if (view.kind === "picker") {
      setView({ kind: "list" });
      return;
    }
    exitScreen();
  }, [exitScreen, leaveEditor, view.kind]);

  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.repeat || e.code !== "Escape") return;
      e.preventDefault();
      e.stopPropagation();
      handleBackStep();
    };
    window.addEventListener("keydown", onKeyDown, true);
    return () => window.removeEventListener("keydown", onKeyDown, true);
  }, [handleBackStep]);

  const handleDelete = useCallback(
    async (template: ManualOpEdTemplate) => {
      if (!window.confirm(`Delete ${templateListLabel(template)}?`)) return;
      try {
        await deleteManualOpEdTemplate(template.id);
        setDirty(true);
        await reloadTemplates();
      } catch (e) {
        onError(errorMessage(e));
      }
    },
    [onError, reloadTemplates],
  );

  const handleSave = useCallback(async () => {
    if (view.kind !== "editor") return;
    const durationSec = view.endSec - view.startSec;
    setSaving(true);
    try {
      if (view.templateId != null) {
        await updateManualOpEdTemplate({
          templateId: view.templateId,
          startSec: view.startSec,
          durationSec,
        });
      } else {
        await saveManualOpEdTemplate({
          animeId: anime.id,
          kind: view.templateKind,
          episodeId: view.episode.id,
          startSec: view.startSec,
          durationSec,
        });
      }
      setDirty(true);
      await teardownMpv();
      await reloadTemplates();
      setView({ kind: "list" });
    } catch (e) {
      onError(errorMessage(e));
    } finally {
      setSaving(false);
    }
  }, [anime.id, onError, reloadTemplates, teardownMpv, view]);

  const editorDuration =
    view.kind === "editor" ?
      view.episode.duration_seconds > 0 ?
        view.episode.duration_seconds
      : 600
    : 0;

  return (
    <div
      className={`manual-skip-screen${
        view.kind === "editor" ? " manual-skip-screen--editor" : ""
      }${previewCompositorRevealed ? " manual-skip-screen--editor-revealed" : ""}`}
    >
      {view.kind === "list" ?
        <>
          <ViewHeader
            title="Manual skip areas"
            subtitle={anime.title}
            onBack={() => exitScreen()}
            action={
              <>
                <button type="button" onClick={() => setView({ kind: "picker", templateKind: "op" })}>
                  Add OP
                </button>
                <button type="button" onClick={() => setView({ kind: "picker", templateKind: "ed" })}>
                  Add ED
                </button>
              </>
            }
          />
          <div className="manual-skip-screen__body">
            <h2 className="manual-skip-section-heading">Custom templates</h2>
            {loading ?
              <p className="muted">Loading templates…</p>
            : templates.length === 0 ?
              <p className="muted">
                No manual skip areas yet. Add an OP or ED template from a source episode.
              </p>
            : <ul className="manual-skip-list">
                {templates.map((template) => (
                  <li key={template.id} className="manual-skip-list__item">
                    <span>{templateListLabel(template)}</span>
                    <div className="manual-skip-list__actions">
                      <button
                        type="button"
                        onClick={() => {
                          const ep = episodes.find((e) => e.id === template.sourceEpisodeId);
                          if (!ep) {
                            onError("Source episode is no longer available.");
                            return;
                          }
                          setView({
                            kind: "editor",
                            templateKind: template.kind as "op" | "ed",
                            episode: ep,
                            templateId: template.id,
                            startSec: template.startSec,
                            endSec: template.startSec + template.durationSec,
                            returnView: "list",
                          });
                        }}
                      >
                        Edit
                      </button>
                      <button
                        type="button"
                        className="button-danger"
                        onClick={() => void handleDelete(template)}
                      >
                        Delete
                      </button>
                    </div>
                  </li>
                ))}
              </ul>
            }

            {episodesMissingOp.length > 0 || episodesMissingEd.length > 0 ?
              <>
                <h2 className="manual-skip-section-heading">Missing segments</h2>
                <div className="manual-skip-missing-grid">
                  <MissingSegmentColumn
                    title="Openings"
                    episodes={episodesMissingOp}
                    trackerOffset={anime.tracker_offset}
                  />
                  <MissingSegmentColumn
                    title="Endings"
                    episodes={episodesMissingEd}
                    trackerOffset={anime.tracker_offset}
                  />
                </div>
              </>
            : null}
          </div>
        </>
      : view.kind === "picker" ?
        <>
          <ViewHeader
            title="Select source video"
            subtitle={`${view.templateKind.toUpperCase()} template`}
            onBack={() => setView({ kind: "list" })}
          />
          <div className="manual-skip-screen__body manual-skip-screen__body--picker">
            <div className="manual-skip-picker-options">
              <CustomCheckbox
                checked={hideMatched}
                onChange={(checked) => {
                  setHideMatched(checked);
                  storeManualSkipHideMatched(checked);
                }}
                label="Hide matched"
              />
            </div>
            {pickerEpisodes.length === 0 ?
              <p className="muted manual-skip-picker-empty">All episodes already have a match.</p>
            : <ul className="manual-skip-episode-list">
                {pickerEpisodes.map((episode) => (
                  <li key={episode.id}>
                    <button
                      type="button"
                      className="manual-skip-episode-list__row"
                      onClick={() => {
                        const range = initialEditorRange(view.templateKind, episode);
                        setView({
                          kind: "editor",
                          templateKind: view.templateKind,
                          episode,
                          startSec: range.startSec,
                          endSec: range.endSec,
                          returnView: "picker",
                        });
                      }}
                    >
                      <span>{episodeRowLabel(episode, anime.tracker_offset)}</span>
                      {isOpEdSegmentMissing(episode.op_ed_segments, view.templateKind) ?
                        <span className="manual-skip-episode-list__missing">Missing</span>
                      : null}
                    </button>
                  </li>
                ))}
              </ul>
            }
          </div>
        </>
      : <>
          <header className="manual-skip-screen__header manual-skip-screen__header--editor">
            <div className="view-title-row">
              <button
                type="button"
                className="back-button"
                aria-label="Back"
                onClick={() => leaveEditor()}
              >
                <ArrowLeftIcon />
              </button>
              <div>
                <h1>
                  {view.templateKind.toUpperCase()} · {episodeRowLabel(view.episode, anime.tracker_offset)}
                </h1>
              </div>
            </div>
          </header>
          <div ref={previewRef} className="manual-skip-preview" aria-hidden />
          <div className="manual-skip-screen__editor-panel">
            <TemplateRangeScrubber
              duration={editorDuration}
              startSec={view.startSec}
              endSec={view.endSec}
              frameStepSec={frameStepSec}
              onStartChange={(startSec) => {
                stopPlaybackOnRangeChange();
                setView((current) => {
                  if (current.kind !== "editor") return current;
                  const next = clampTemplateRange(startSec, current.endSec, editorDuration);
                  return { ...current, startSec: next.startSec, endSec: next.endSec };
                });
              }}
              onEndChange={(endSec) => {
                stopPlaybackOnRangeChange();
                setView((current) => {
                  if (current.kind !== "editor") return current;
                  const next = clampTemplateRange(current.startSec, endSec, editorDuration);
                  return { ...current, startSec: next.startSec, endSec: next.endSec };
                });
              }}
              onRangeChange={(startSec, endSec) => {
                stopPlaybackOnRangeChange();
                setView((current) => {
                  if (current.kind !== "editor") return current;
                  const next = clampTemplateRange(startSec, endSec, editorDuration);
                  return { ...current, startSec: next.startSec, endSec: next.endSec };
                });
              }}
              onSeek={seekPreview}
              centerActions={
                <>
                  <button
                    type="button"
                    aria-pressed={playbackMode === "area"}
                    onClick={() => void handlePlayArea()}
                  >
                    Play
                  </button>
                  <button
                    type="button"
                    aria-pressed={playbackMode === "test"}
                    onClick={() => void handleTest()}
                  >
                    Test
                  </button>
                  <button type="button" disabled={saving} onClick={() => void handleSave()}>
                    {saving ? "Saving…" : "Save"}
                  </button>
                </>
              }
              trailingActions={
                <VolumeControl
                  volume={volume}
                  muted={muted}
                  popupOpen={volumePopupOpen}
                  buttonId="manual-skip-volume-button"
                  onApplyVolume={applyVolume}
                  onToggleMute={() => setMuted((current) => !current)}
                  onOpenPopup={openVolumePopup}
                  onScheduleHidePopup={scheduleVolumePopupHide}
                />
              }
            />
          </div>
        </>
      }
    </div>
  );
}
