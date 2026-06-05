import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useRef, useState } from "react";
import {
  deleteManualOpEdTemplate,
  listManualOpEdTemplates,
  mpvClearPreviewRect,
  mpvSetPreviewRect,
  probeVideoFps,
  saveManualOpEdTemplate,
  updateManualOpEdTemplate,
} from "../api";
import type { AnimeSummary, Episode, ManualOpEdTemplate } from "../types";
import {
  errorMessage,
  formatEpisodeNumber,
  formatTime,
  isEpisodeNumberKnown,
} from "../utils";
import {
  clampTemplateRange,
  defaultEditorRange,
  TemplateRangeScrubber,
} from "./TemplateRangeScrubber";

const HIDDEN_PLAYER_SIDEBAR_PX = 100_000;

type ModalView =
  | { kind: "list" }
  | { kind: "picker"; templateKind: "op" | "ed" }
  | {
      kind: "editor";
      templateKind: "op" | "ed";
      episode: Episode;
      templateId?: number;
      startSec: number;
      endSec: number;
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

export function ManualSkipAreasModal(props: {
  open: boolean;
  anime: AnimeSummary;
  episodes: Episode[];
  onClose: () => void;
  onDirtyClose: () => void;
  onError: (message: string) => void;
}) {
  const { open, anime, episodes, onClose, onDirtyClose, onError } = props;
  const [view, setView] = useState<ModalView>({ kind: "list" });
  const [templates, setTemplates] = useState<ManualOpEdTemplate[]>([]);
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [dirty, setDirty] = useState(false);
  const [frameStepSec, setFrameStepSec] = useState(1 / 24);
  const previewRef = useRef<HTMLDivElement>(null);
  const mpvReadyRef = useRef(false);
  const layoutTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const reloadTemplates = useCallback(async () => {
    setLoading(true);
    try {
      const rows = await listManualOpEdTemplates(anime.id);
      setTemplates(rows);
    } catch (e) {
      onError(errorMessage(e));
    } finally {
      setLoading(false);
    }
  }, [anime.id, onError]);

  useEffect(() => {
    if (!open) return;
    setView({ kind: "list" });
    setDirty(false);
    void reloadTemplates();
  }, [open, reloadTemplates]);

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

  useEffect(() => {
    if (!open || view.kind !== "editor") return;
    void loadEditorEpisode(view.episode);
    const el = previewRef.current;
    if (!el) return;
    const observer = new ResizeObserver(() => schedulePreviewRect());
    observer.observe(el);
    const onResize = () => schedulePreviewRect();
    window.addEventListener("resize", onResize);
    return () => {
      observer.disconnect();
      window.removeEventListener("resize", onResize);
      void teardownMpv();
    };
  }, [loadEditorEpisode, open, schedulePreviewRect, teardownMpv, view]);

  const handleClose = useCallback(() => {
    void teardownMpv().finally(() => {
      if (dirty) onDirtyClose();
      onClose();
    });
  }, [dirty, onClose, onDirtyClose, teardownMpv]);

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

  if (!open) return null;

  const editorDuration =
    view.kind === "editor" ?
      view.episode.duration_seconds > 0 ?
        view.episode.duration_seconds
      : 600
    : 0;

  return (
    <div
      className="modal-backdrop manual-skip-backdrop"
      role="presentation"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget && view.kind === "list") handleClose();
      }}
    >
      <section
        className="modal manual-skip-modal"
        role="dialog"
        aria-modal="true"
        aria-labelledby="manual-skip-title"
      >
        {view.kind === "list" ?
          <>
            <header className="modal-heading manual-skip-modal__header">
              <h2 id="manual-skip-title">Manual skip areas</h2>
              <div className="manual-skip-modal__header-actions">
                <button type="button" onClick={() => setView({ kind: "picker", templateKind: "op" })}>
                  Add OP
                </button>
                <button type="button" onClick={() => setView({ kind: "picker", templateKind: "ed" })}>
                  Add ED
                </button>
                <button type="button" className="header-icon-button" onClick={handleClose} aria-label="Close">
                  ×
                </button>
              </div>
            </header>
            <div className="manual-skip-modal__body">
              {loading ?
                <p className="muted">Loading templates…</p>
              : templates.length === 0 ?
                <p className="muted">No manual skip areas yet. Add an OP or ED template from a source episode.</p>
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
                            });
                          }}
                        >
                          Edit
                        </button>
                        <button type="button" className="button-danger" onClick={() => void handleDelete(template)}>
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
            <header className="modal-heading">
              <h2 id="manual-skip-title">Select source video</h2>
              <button type="button" onClick={() => setView({ kind: "list" })}>
                Back
              </button>
            </header>
            <ul className="manual-skip-episode-list">
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
            <header className="modal-heading">
              <h2 id="manual-skip-title">
                {view.templateKind.toUpperCase()} · {episodeRowLabel(view.episode, anime.tracker_offset)}
              </h2>
              <button
                type="button"
                onClick={() => {
                  void teardownMpv();
                  setView({ kind: "list" });
                }}
              >
                Back
              </button>
            </header>
            <div
              ref={previewRef}
              className="manual-skip-preview"
              aria-hidden
            />
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
              onSeek={(seconds) => {
                void invoke("mpv_seek", { seconds, keyframe: true }).catch((e) =>
                  onError(errorMessage(e)),
                );
              }}
            />
            <footer className="modal-actions">
              <button
                type="button"
                onClick={() => {
                  void teardownMpv();
                  setView({ kind: "list" });
                }}
              >
                Cancel
              </button>
              <button type="button" disabled={saving} onClick={() => void handleSave()}>
                {saving ? "Saving…" : "Save"}
              </button>
            </footer>
          </>
        }
      </section>
    </div>
  );
}
