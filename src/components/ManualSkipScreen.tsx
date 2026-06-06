import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useRef, useState } from "react";
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
import { clampVolume, loadVolume, saveVolume } from "../volume";
import type { AnimeSummary, Episode, ManualOpEdTemplate } from "../types";
import {
  errorMessage,
  formatEpisodeNumber,
  formatTime,
  isEpisodeNumberKnown,
} from "../utils";
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

type TestPlayback = {
  startSec: number;
  endSec: number;
  endPlaySec: number;
  phase: "pre" | "post-skip";
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
  const [testPlaying, setTestPlaying] = useState(false);
  const [volume, setVolume] = useState(loadVolume);
  const [muted, setMuted] = useState(false);
  const [volumePopupOpen, setVolumePopupOpen] = useState(false);
  const volumeHideTimerRef = useRef<number | null>(null);
  const previewRef = useRef<HTMLDivElement>(null);
  const mpvReadyRef = useRef(false);
  const layoutTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const testPlaybackRef = useRef<TestPlayback | null>(null);
  const onErrorRef = useRef(onError);
  onErrorRef.current = onError;

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
    if (!editorEpisode) return;
    void loadEditorEpisode(editorEpisode);
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
        const test = testPlaybackRef.current;
        if (!test || typeof e.payload !== "number") return;
        const pos = e.payload;
        if (test.phase === "pre" && pos >= test.startSec && pos < test.endSec) {
          test.phase = "post-skip";
          void invoke("mpv_seek", { seconds: test.endSec, keyframe: false }).catch((err) =>
            onError(errorMessage(err)),
          );
          return;
        }
        if (test.phase === "post-skip" && pos >= test.endSec && pos >= test.endPlaySec) {
          testPlaybackRef.current = null;
          setTestPlaying(false);
          void invoke("mpv_set_pause", { paused: true }).catch((err) =>
            onError(errorMessage(err)),
          );
        }
      });
    })();
    return () => {
      cancelled = true;
      testPlaybackRef.current = null;
      setTestPlaying(false);
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

  const seekPreview = useCallback(
    (seconds: number) => {
      testPlaybackRef.current = null;
      setTestPlaying(false);
      void invoke("mpv_seek", { seconds, keyframe: false })
        .then(() => invoke("mpv_set_pause", { paused: true }))
        .catch((e) => onError(errorMessage(e)));
    },
    [onError],
  );

  const handleTest = useCallback(async () => {
    if (view.kind !== "editor") return;
    const duration =
      view.episode.duration_seconds > 0 ? view.episode.duration_seconds : view.endSec + TEST_TAIL_SEC;
    const startSec = view.startSec;
    const endSec = view.endSec;
    setTestPlaying(true);
    try {
      const preStart = Math.max(0, startSec - TEST_LEAD_SEC);
      await invoke("mpv_seek", { seconds: preStart, keyframe: false });
      testPlaybackRef.current = {
        startSec,
        endSec,
        endPlaySec: Math.min(duration, endSec + TEST_TAIL_SEC),
        phase: "pre",
      };
      await invoke("mpv_set_pause", { paused: false });
    } catch (e) {
      testPlaybackRef.current = null;
      setTestPlaying(false);
      onError(errorMessage(e));
    }
  }, [onError, view]);

  const leaveEditor = useCallback(() => {
    if (view.kind !== "editor") return;
    testPlaybackRef.current = null;
    setTestPlaying(false);
    void teardownMpv();
    if (view.returnView === "picker") {
      setView({ kind: "picker", templateKind: view.templateKind });
      return;
    }
    setView({ kind: "list" });
  }, [teardownMpv, view]);

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
          </div>
        </>
      : view.kind === "picker" ?
        <>
          <ViewHeader
            title="Select source video"
            subtitle={`${view.templateKind.toUpperCase()} template`}
            onBack={() => setView({ kind: "list" })}
          />
          <ul className="manual-skip-episode-list manual-skip-screen__body">
            {episodes.map((episode) => (
              <li key={episode.id}>
                <button
                  type="button"
                  className="manual-skip-episode-list__row"
                  onClick={() => {
                    const range = defaultEditorRange(view.templateKind, episode.duration_seconds || 600);
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
                  {episodeRowLabel(episode, anime.tracker_offset)}
                </button>
              </li>
            ))}
          </ul>
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
                const next = clampTemplateRange(startSec, view.endSec, editorDuration);
                setView({ ...view, startSec: next.startSec, endSec: next.endSec });
              }}
              onEndChange={(endSec) => {
                const next = clampTemplateRange(view.startSec, endSec, editorDuration);
                setView({ ...view, startSec: next.startSec, endSec: next.endSec });
              }}
              onRangeChange={(startSec, endSec) => {
                const next = clampTemplateRange(startSec, endSec, editorDuration);
                setView({ ...view, startSec: next.startSec, endSec: next.endSec });
              }}
              onSeek={seekPreview}
              centerActions={
                <>
                  <button type="button" disabled={testPlaying} onClick={() => void handleTest()}>
                    {testPlaying ? "Testing…" : "Test"}
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
