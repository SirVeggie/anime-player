import { useCallback, useEffect, useMemo, useState } from "react";
import { jobsCancel, jobsCancelAll, jobsSetMaxParallel } from "../api";
import { subscribeJobsSnapshot } from "../jobs/jobClient";
import type { JobPriority, JobRecord, JobsSnapshot } from "../types";
import { errorMessage, formatDurationMs } from "../utils";
import { ViewHeader } from "./ViewHeader";

type JobsTab = "active" | "history";

function useNowMs(tick: boolean) {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!tick) return;
    const id = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(id);
  }, [tick]);
  return now;
}

function jobDurationLabel(job: JobRecord, nowMs: number, history: boolean): string | null {
  if (history) {
    if (job.finishedAt == null) return null;
    const start = job.startedAt ?? job.createdAt;
    return `Ran for ${formatDurationMs(job.finishedAt - start)}`;
  }
  if (job.status === "running") {
    const start = job.startedAt ?? job.createdAt;
    return `Running for ${formatDurationMs(nowMs - start)}`;
  }
  if (job.status === "queued") {
    return `Queued ${formatDurationMs(nowMs - job.createdAt)} ago`;
  }
  return null;
}

function priorityLabel(priority: JobPriority) {
  return priority.charAt(0).toUpperCase() + priority.slice(1);
}

function progressPercent(job: JobRecord) {
  if (job.progress.totalSteps <= 0) return 0;
  return Math.round((job.progress.currentStep / job.progress.totalSteps) * 100);
}

function JobRow(props: {
  job: JobRecord;
  nowMs: number;
  history?: boolean;
  onCancel?: (jobId: string) => void;
  showProgress?: boolean;
}) {
  const { job, nowMs, history = false, onCancel, showProgress = true } = props;
  const canCancel =
    onCancel &&
    job.cancelable &&
    (job.status === "queued" || job.status === "running");
  const durationLabel = jobDurationLabel(job, nowMs, history);

  return (
    <div className="job-row">
      <div className="job-row-header">
        <div className="job-row-titles">
          <div className="job-row-name-line">
            <strong>{job.name}</strong>
            <span className={`job-priority-pill job-priority-pill--${job.priority}`}>
              {priorityLabel(job.priority)}
            </span>
          </div>
          <span className="muted job-row-desc">{job.desc}</span>
        </div>
        <span className={`job-status job-status--${job.status}`}>{job.status}</span>
      </div>
      {durationLabel ?
        <span className="muted job-row-duration">{durationLabel}</span>
      : null}
      {showProgress && (job.status === "queued" || job.status === "running") ?
        <>
          <div className="job-progress-track" aria-hidden>
            <div className="job-progress-fill" style={{ width: `${progressPercent(job)}%` }} />
          </div>
          <span className="muted job-row-step">{job.stepLabel}</span>
        </>
      : null}
      {job.completionMessage ?
        <span className="muted job-row-completion">{job.completionMessage}</span>
      : null}
      {canCancel ?
        <button type="button" className="job-row-cancel" onClick={() => onCancel(job.id)}>
          Cancel
        </button>
      : null}
    </div>
  );
}

export function JobsScreen(props: {
  snapshot: JobsSnapshot | null;
  onBack: () => void;
  onError: (message: string) => void;
}) {
  const { snapshot, onBack, onError } = props;
  const [activeTab, setActiveTab] = useState<JobsTab>("active");
  const [maxParallelDraft, setMaxParallelDraft] = useState(2);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (snapshot) {
      setMaxParallelDraft(snapshot.maxParallel);
    }
  }, [snapshot?.maxParallel]);

  const activeJobs = useMemo(
    () =>
      (snapshot?.active ?? []).filter((j) => j.status === "queued" || j.status === "running"),
    [snapshot?.active],
  );

  const tickNow = activeTab === "active" && activeJobs.length > 0;
  const nowMs = useNowMs(tickNow);

  const handleCancel = useCallback(
    async (jobId: string) => {
      setBusy(true);
      try {
        await jobsCancel(jobId);
      } catch (e) {
        onError(errorMessage(e));
      } finally {
        setBusy(false);
      }
    },
    [onError],
  );

  const handleCancelAll = useCallback(async () => {
    setBusy(true);
    try {
      await jobsCancelAll();
    } catch (e) {
      onError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  }, [onError]);

  const handleSaveMaxParallel = useCallback(async () => {
    if (!Number.isInteger(maxParallelDraft) || maxParallelDraft < 1 || maxParallelDraft > 8) {
      onError("Max parallel jobs must be an integer from 1 to 8.");
      return;
    }
    setBusy(true);
    try {
      await jobsSetMaxParallel(maxParallelDraft);
    } catch (e) {
      onError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  }, [maxParallelDraft, onError]);

  return (
    <>
      <ViewHeader title="Background jobs" subtitle="Queued and completed work" onBack={onBack} />

      <div className="bulk-edit-tabs" role="tablist" aria-label="Job views">
        <button
          type="button"
          role="tab"
          aria-selected={activeTab === "active"}
          className={activeTab === "active" ? "bulk-edit-tab bulk-edit-tab--active" : "bulk-edit-tab"}
          onClick={() => setActiveTab("active")}
        >
          Active ({activeJobs.length})
        </button>
        <button
          type="button"
          role="tab"
          aria-selected={activeTab === "history"}
          className={activeTab === "history" ? "bulk-edit-tab bulk-edit-tab--active" : "bulk-edit-tab"}
          onClick={() => setActiveTab("history")}
        >
          History ({snapshot?.history.length ?? 0})
        </button>
      </div>

      {activeTab === "active" ?
        <section className="panel bulk-edit-panel jobs-panel">
          <div className="jobs-toolbar">
            <label className="stacked-field">
              <span>Max parallel jobs</span>
              <div className="score-stepper">
                <input
                  type="number"
                  min={1}
                  max={8}
                  value={maxParallelDraft}
                  disabled={busy}
                  onChange={(e) => {
                    const parsed = Number(e.currentTarget.value);
                    if (!Number.isFinite(parsed)) return;
                    setMaxParallelDraft(Math.min(8, Math.max(1, Math.trunc(parsed))));
                  }}
                />
                <div className="score-stepper-buttons">
                  <button
                    type="button"
                    className="score-stepper-button score-stepper-button--up"
                    aria-label="Increase max parallel jobs"
                    disabled={busy || maxParallelDraft >= 8}
                    onClick={() => setMaxParallelDraft((value) => Math.min(8, value + 1))}
                  />
                  <button
                    type="button"
                    className="score-stepper-button score-stepper-button--down"
                    aria-label="Decrease max parallel jobs"
                    disabled={busy || maxParallelDraft <= 1}
                    onClick={() => setMaxParallelDraft((value) => Math.max(1, value - 1))}
                  />
                </div>
              </div>
            </label>
            <button type="button" disabled={busy} onClick={() => void handleSaveMaxParallel()}>
              Apply
            </button>
            <button
              type="button"
              className="jobs-cancel-all"
              disabled={busy || activeJobs.length === 0}
              onClick={() => void handleCancelAll()}
            >
              Cancel all
            </button>
          </div>

          {activeJobs.length === 0 ?
            <p className="muted">No queued or running jobs.</p>
          : (
            <div className="job-list">
              {activeJobs.map((job) => (
                <JobRow
                  key={job.id}
                  job={job}
                  nowMs={nowMs}
                  onCancel={(id) => void handleCancel(id)}
                />
              ))}
            </div>
          )}
        </section>
      : (
        <section className="panel bulk-edit-panel jobs-panel">
          {(snapshot?.history.length ?? 0) === 0 ?
            <p className="muted">No completed jobs yet.</p>
          : (
            <div className="job-list">
              {snapshot?.history.map((job) => (
                <JobRow key={job.id} job={job} nowMs={nowMs} history showProgress={false} />
              ))}
            </div>
          )}
        </section>
      )}
    </>
  );
}

export function useJobsSnapshot() {
  const [snapshot, setSnapshot] = useState<JobsSnapshot | null>(null);
  useEffect(() => subscribeJobsSnapshot(setSnapshot), []);
  return snapshot;
}
