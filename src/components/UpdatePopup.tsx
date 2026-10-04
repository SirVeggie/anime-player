import { useEffect } from "react";
import type { UpdateStatus } from "../types";
import { formatSize } from "../utils";
import { summarizeUpdateNotes } from "../updateReminders";

export function UpdatePopup(props: {
  status: UpdateStatus;
  busy?: boolean;
  onUpdate: () => void;
  onRestart: () => void;
  onRemindLater: () => void;
  onSkipVersion: () => void;
}) {
  const { status, busy = false, onUpdate, onRestart, onRemindLater, onSkipVersion } = props;
  const current = formatVersion(status.current_version) ?? "this install";
  const latest = formatVersion(status.latest_version) ?? "a newer release";
  const notes = summarizeUpdateNotes(status.notes);
  const downloading = status.downloading;
  const pendingApply = status.pending_apply;
  const updateDisabled = busy || downloading || !status.updates_supported;

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !downloading) {
        event.preventDefault();
        onRemindLater();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [downloading, onRemindLater]);

  return (
    <div
      className="modal-backdrop"
      role="presentation"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget && !downloading) onRemindLater();
      }}
    >
      <section
        className="modal update-popup"
        role="dialog"
        aria-modal="true"
        aria-labelledby="update-popup-title"
        aria-describedby="update-popup-description"
      >
        <div className="modal-heading">
          <div>
            <h2 id="update-popup-title">{pendingApply ? "Restart to finish update" : "Update available"}</h2>
            <p className="muted" id="update-popup-description">
              {pendingApply
                ? `Version ${latest} is downloaded. Restart Anime Player to apply it.`
                : `A new version is available: ${current} → ${latest}.`}
            </p>
          </div>
        </div>
        {notes ? <pre className="update-popup-notes">{notes}</pre> : null}
        {downloading ? (
          <div className="update-progress">
            <p className="muted">{updateProgressLabel(status)}</p>
            <div className="job-progress-track" aria-hidden>
              <div className="job-progress-fill" style={{ width: `${updateProgressPercent(status)}%` }} />
            </div>
          </div>
        ) : null}
        {status.last_error ? <p className="error">{status.last_error}</p> : null}
        {!status.updates_supported ? (
          <p className="muted">In-app updates are disabled in development builds.</p>
        ) : null}
        <div className="modal-actions update-popup-actions">
          <button type="button" onClick={onSkipVersion} disabled={busy || downloading}>
            Skip version
          </button>
          <button type="button" onClick={onRemindLater} disabled={downloading}>
            Remind me later
          </button>
          {pendingApply ? (
            <button type="button" className="button-primary" onClick={onRestart} disabled={busy || downloading}>
              Restart now
            </button>
          ) : (
            <button type="button" className="button-primary" onClick={onUpdate} disabled={updateDisabled}>
              {downloading ? "Downloading…" : "Update"}
            </button>
          )}
        </div>
      </section>
    </div>
  );
}

function formatVersion(version: string | null | undefined): string | null {
  const trimmed = version?.trim();
  return trimmed ? trimmed : null;
}

function formatUpdateBytes(bytes: number): string {
  return bytes > 0 ? formatSize(bytes) : "0 B";
}

function updateProgressLabel(status: UpdateStatus): string {
  const file = status.progress.file ? ` ${status.progress.file}` : "";
  return `Downloading${file} (${formatUpdateBytes(status.progress.bytes_downloaded)} / ${formatUpdateBytes(status.progress.bytes_total)})`;
}

function updateProgressPercent(status: UpdateStatus): number {
  const total = status.progress.bytes_total;
  if (total <= 0) {
    return 0;
  }
  return Math.min(100, Math.round((status.progress.bytes_downloaded / total) * 100));
}
