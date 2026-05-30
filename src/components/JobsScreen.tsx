import { useCallback, useEffect, useMemo, useState } from "react";
import { jobsCancel, jobsCancelAll, jobsSetMaxParallel, jobsSetTypeMaxParallel } from "../api";
import { subscribeJobsSnapshot } from "../jobs/jobClient";
import type { JobPriority, JobRecord, JobResourceType, JobsSnapshot } from "../types";
import { errorMessage, formatDurationMs } from "../utils";
import { ViewHeader } from "./ViewHeader";

type JobsTab = "active" | "history";

const PARALLEL_LIMIT_DEBOUNCE_MS = 500;

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

function resourceTypeLabel(resourceType: JobResourceType) {
  return resourceType.charAt(0).toUpperCase() + resourceType.slice(1);
}

function ParallelLimitStepper(props: {
  value: number;
  disabled: boolean;
  ariaLabel: string;
  onChange: (value: number) => void;
}) {
  const { value, disabled, ariaLabel, onChange } = props;
  return (
    <div className="score-stepper">
      <input
        type="number"
        min={1}
        max={8}
        value={value}
        disabled={disabled}
        aria-label={ariaLabel}
        onChange={(e) => {
          const parsed = Number(e.currentTarget.value);
          if (!Number.isFinite(parsed)) return;
          onChange(Math.min(8, Math.max(1, Math.trunc(parsed))));
        }}
      />
      <div className="score-stepper-buttons">
        <button
          type="button"
          className="score-stepper-button score-stepper-button--up"
          aria-label={`Increase ${ariaLabel}`}
          disabled={disabled || value >= 8}
          onClick={() => onChange(Math.min(8, value + 1))}
        />
        <button
          type="button"
          className="score-stepper-button score-stepper-button--down"
          aria-label={`Decrease ${ariaLabel}`}
          disabled={disabled || value <= 1}
          onClick={() => onChange(Math.max(1, value - 1))}
        />
      </div>
    </div>
  );
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
            {job.resourceType !== "none" ?
              <span className={`job-resource-type-pill job-resource-type-pill--${job.resourceType}`}>
                {resourceTypeLabel(job.resourceType)}
              </span>
            : null}
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
  const [typeMaxParallelDraft, setTypeMaxParallelDraft] = useState<Record<string, number>>({});
  const [cancelBusy, setCancelBusy] = useState(false);
  const [limitsSaving, setLimitsSaving] = useState(false);

  const maxParallelSaved = snapshot?.maxParallel;
  const typeMaxParallelSaved = snapshot?.typeMaxParallel;
  const typeMaxParallelSavedKey =
    typeMaxParallelSaved?.map((entry) => `${entry.resourceType}:${entry.maxParallel}`).join("|") ?? "";

  const savedLimits = useMemo(() => {
    if (maxParallelSaved == null || !typeMaxParallelSaved) return null;
    return {
      maxParallel: maxParallelSaved,
      typeMaxParallel: Object.fromEntries(
        typeMaxParallelSaved.map((entry) => [entry.resourceType, entry.maxParallel]),
      ),
      entries: typeMaxParallelSaved,
    };
  }, [maxParallelSaved, typeMaxParallelSavedKey]);

  useEffect(() => {
    if (!savedLimits) return;
    setMaxParallelDraft(savedLimits.maxParallel);
    setTypeMaxParallelDraft({ ...savedLimits.typeMaxParallel });
  }, [savedLimits]);

  useEffect(() => {
    if (!savedLimits) return;

    const globalChanged = maxParallelDraft !== savedLimits.maxParallel;
    const typeChanged = savedLimits.entries.some(
      (entry) => (typeMaxParallelDraft[entry.resourceType] ?? entry.maxParallel) !== entry.maxParallel,
    );
    if (!globalChanged && !typeChanged) return;

    const timeoutId = window.setTimeout(() => {
      if (!Number.isInteger(maxParallelDraft) || maxParallelDraft < 1 || maxParallelDraft > 8) {
        onError("Max parallel jobs must be an integer from 1 to 8.");
        return;
      }
      for (const entry of savedLimits.entries) {
        const value = typeMaxParallelDraft[entry.resourceType] ?? entry.maxParallel;
        if (!Number.isInteger(value) || value < 1 || value > 8) {
          onError(`Max parallel ${entry.resourceType} jobs must be an integer from 1 to 8.`);
          return;
        }
      }

      void (async () => {
        setLimitsSaving(true);
        try {
          if (globalChanged) {
            await jobsSetMaxParallel(maxParallelDraft);
          }
          for (const entry of savedLimits.entries) {
            const value = typeMaxParallelDraft[entry.resourceType] ?? entry.maxParallel;
            if (value !== entry.maxParallel) {
              await jobsSetTypeMaxParallel(entry.resourceType, value);
            }
          }
        } catch (e) {
          onError(errorMessage(e));
        } finally {
          setLimitsSaving(false);
        }
      })();
    }, PARALLEL_LIMIT_DEBOUNCE_MS);

    return () => window.clearTimeout(timeoutId);
  }, [maxParallelDraft, onError, savedLimits, typeMaxParallelDraft]);

  const activeJobs = useMemo(
    () =>
      (snapshot?.active ?? []).filter((j) => j.status === "queued" || j.status === "running"),
    [snapshot?.active],
  );

  const tickNow = activeTab === "active" && activeJobs.length > 0;
  const nowMs = useNowMs(tickNow);

  const handleCancel = useCallback(
    async (jobId: string) => {
      setCancelBusy(true);
      try {
        await jobsCancel(jobId);
      } catch (e) {
        onError(errorMessage(e));
      } finally {
        setCancelBusy(false);
      }
    },
    [onError],
  );

  const handleCancelAll = useCallback(async () => {
    setCancelBusy(true);
    try {
      await jobsCancelAll();
    } catch (e) {
      onError(errorMessage(e));
    } finally {
      setCancelBusy(false);
    }
  }, [onError]);

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
            <div className="jobs-parallel-limits">
              <label className="stacked-field">
                <span>Max parallel jobs</span>
                <ParallelLimitStepper
                  value={maxParallelDraft}
                  disabled={limitsSaving}
                  ariaLabel="max parallel jobs"
                  onChange={setMaxParallelDraft}
                />
              </label>
              {(snapshot?.typeMaxParallel ?? []).map((entry) => (
                <label key={entry.resourceType} className="stacked-field">
                  <span>Max parallel {entry.resourceType}</span>
                  <ParallelLimitStepper
                    value={typeMaxParallelDraft[entry.resourceType] ?? entry.maxParallel}
                    disabled={limitsSaving}
                    ariaLabel={`max parallel ${entry.resourceType} jobs`}
                    onChange={(value) => {
                      setTypeMaxParallelDraft((current) => ({
                        ...current,
                        [entry.resourceType]: value,
                      }));
                    }}
                  />
                </label>
              ))}
            </div>
            <div className="jobs-toolbar-actions">
              <button
                type="button"
                className="jobs-cancel-all"
                disabled={cancelBusy || activeJobs.length === 0}
                onClick={() => void handleCancelAll()}
              >
                Cancel all
              </button>
            </div>
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
