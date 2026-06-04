import { useCallback, useEffect, useMemo, useState } from "react";
import { jobsCancel, jobsCancelAll, jobsSetMaxParallel, jobsSetTypeMaxParallel } from "../api";
import { useJobsSnapshot } from "../jobs/jobClient";
import { jobProgressBarPercent, shouldShowJobProgressBar } from "../jobs/jobUi";
import type { JobPriority, JobRecord, JobResourceType } from "../types";
import { errorMessage, formatDurationMs } from "../utils";
import { ViewHeader } from "./ViewHeader";

type JobsTab = "active" | "history";

const PARALLEL_LIMIT_DEBOUNCE_MS = 500;
/** Collapse many identical chroma queue rows (typical after Detect OP/ED). */
const CHROMA_QUEUE_COLLAPSE_THRESHOLD = 6;

function bucketActiveJobs(jobs: JobRecord[]) {
  const running: JobRecord[] = [];
  const queuedOther: JobRecord[] = [];
  const queuedChroma: JobRecord[] = [];
  for (const job of jobs) {
    if (job.status === "running") {
      running.push(job);
    } else if (job.status === "queued" && job.jobType === "op_ed_chroma") {
      queuedChroma.push(job);
    } else if (job.status === "queued") {
      queuedOther.push(job);
    }
  }
  return { running, queuedOther, queuedChroma };
}

function ChromaQueueSummary(props: { jobs: JobRecord[] }) {
  const { jobs } = props;
  const oldest = jobs.reduce(
    (min, job) => (job.createdAt < min ? job.createdAt : min),
    jobs[0]?.createdAt ?? 0,
  );
  const priority = jobs[0]?.priority ?? "medium";
  return (
    <div className="job-row job-row--group" aria-label={`${jobs.length} fingerprint jobs queued`}>
      <div className="job-row-body">
        <div className="job-row-header">
          <div className="job-row-name-line">
            <strong>Fingerprint episodes</strong>
            <span className="job-resource-type-pill job-resource-type-pill--chroma">Chroma</span>
            <span className={`job-priority-pill job-priority-pill--${priority}`}>
              {priorityLabel(priority)}
            </span>
          </div>
          <span className="muted job-row-desc">
            {jobs.length} queued — runs when chroma slots and disk load allow
          </span>
        </div>
        <p className="muted job-row-meta">Queued {formatDurationMs(Date.now() - oldest)} ago (oldest)</p>
      </div>
      <div className="job-row-actions">
        <span className="job-status job-status--queued">queued</span>
      </div>
    </div>
  );
}

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
        max={20}
        value={value}
        disabled={disabled}
        aria-label={ariaLabel}
        onChange={(e) => {
          const parsed = Number(e.currentTarget.value);
          if (!Number.isFinite(parsed)) return;
          onChange(Math.min(20, Math.max(1, Math.trunc(parsed))));
        }}
      />
      <div className="score-stepper-buttons">
        <button
          type="button"
          className="score-stepper-button score-stepper-button--up"
          aria-label={`Increase ${ariaLabel}`}
          disabled={disabled || value >= 20}
          onClick={() => onChange(Math.min(20, value + 1))}
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
  return jobProgressBarPercent(job);
}

function sortJobsOldestFirst(
  jobs: JobRecord[],
  sortKey: (job: JobRecord) => number,
): JobRecord[] {
  return [...jobs].sort((a, b) => sortKey(a) - sortKey(b));
}

function sortJobsNewestFirst(
  jobs: JobRecord[],
  sortKey: (job: JobRecord) => number,
): JobRecord[] {
  return [...jobs].sort((a, b) => sortKey(b) - sortKey(a));
}

function JobList(props: {
  jobs: JobRecord[];
  nowMs: number;
  onCancel?: (jobId: string) => void;
  history?: boolean;
}) {
  const { jobs, nowMs, onCancel, history = false } = props;
  if (jobs.length === 0) return null;
  return (
    <div className="job-list">
      {jobs.map((job) => (
        <JobRow
          key={job.id}
          job={job}
          nowMs={nowMs}
          history={history}
          showProgress={!history}
          onCancel={onCancel}
        />
      ))}
    </div>
  );
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
  const showProgressBar =
    showProgress && (job.status === "queued" || job.status === "running") && shouldShowJobProgressBar(job);

  return (
    <div className="job-row">
      <div className="job-row-body">
        <div className="job-row-header">
          <div className="job-row-name-line">
            <span className="job-short-id">#{job.shortId}</span>
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
        {showProgress && (job.status === "queued" || job.status === "running") ?
          <>
            {durationLabel || job.stepLabel ?
              <p className="muted job-row-meta">
                {durationLabel}
                {durationLabel && job.stepLabel ?
                  <span className="job-row-meta-sep" aria-hidden="true">
                    {" · "}
                  </span>
                : null}
                {job.stepLabel ? <span>{job.stepLabel}</span> : null}
              </p>
            : null}
            {showProgressBar ?
              <div className="job-progress-track" aria-hidden>
                <div className="job-progress-fill" style={{ width: `${progressPercent(job)}%` }} />
              </div>
            : null}
          </>
        : durationLabel || job.completionMessage ?
          <p className="muted job-row-meta">
            {durationLabel}
            {durationLabel && job.completionMessage ?
              <span className="job-row-meta-sep" aria-hidden="true">
                {" · "}
              </span>
            : null}
            {job.completionMessage ? <span>{job.completionMessage}</span> : null}
          </p>
        : null}
        {job.prerequisitePending > 0 ?
          <div className="job-row-prerequisites">
            <span className="muted">Waiting for</span>
            {job.waitingFor.map((prereq) => (
              <span key={prereq.jobId} className="job-prerequisite-pill">
                #{prereq.shortId}
              </span>
            ))}
            {job.prerequisitePending > job.waitingFor.length ?
              <span className="job-prerequisite-pill">
                +{job.prerequisitePending - job.waitingFor.length} more
              </span>
            : null}
          </div>
        : null}
      </div>
      <div className="job-row-actions">
        <span className={`job-status job-status--${job.status}`}>{job.status}</span>
        {canCancel ?
          <button type="button" className="job-row-cancel" onClick={() => onCancel!(job.id)}>
            Cancel
          </button>
        : null}
      </div>
    </div>
  );
}

export function JobsScreen(props: {
  onBack: () => void;
  onError: (message: string) => void;
}) {
  const snapshot = useJobsSnapshot();
  const { onBack, onError } = props;
  const [activeTab, setActiveTab] = useState<JobsTab>("active");
  const [maxParallelDraft, setMaxParallelDraft] = useState(5);
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
      if (!Number.isInteger(maxParallelDraft) || maxParallelDraft < 1 || maxParallelDraft > 20) {
        onError("Max parallel jobs must be an integer from 1 to 20.");
        return;
      }
      for (const entry of savedLimits.entries) {
        const value = typeMaxParallelDraft[entry.resourceType] ?? entry.maxParallel;
        if (!Number.isInteger(value) || value < 1 || value > 20) {
          onError(`Max parallel ${entry.resourceType} jobs must be an integer from 1 to 20.`);
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

  const activeBuckets = useMemo(() => bucketActiveJobs(activeJobs), [activeJobs]);

  const runningJobs = useMemo(
    () => sortJobsOldestFirst(activeBuckets.running, (j) => j.startedAt ?? j.createdAt),
    [activeBuckets.running],
  );

  const queuedOtherJobs = useMemo(
    () => sortJobsOldestFirst(activeBuckets.queuedOther, (j) => j.createdAt),
    [activeBuckets.queuedOther],
  );

  const queuedChromaJobs = useMemo(
    () => sortJobsOldestFirst(activeBuckets.queuedChroma, (j) => j.createdAt),
    [activeBuckets.queuedChroma],
  );

  const collapseChromaQueue = queuedChromaJobs.length >= CHROMA_QUEUE_COLLAPSE_THRESHOLD;

  const historyJobs = useMemo(
    () =>
      sortJobsNewestFirst(
        snapshot?.history ?? [],
        (j) => j.finishedAt ?? j.startedAt ?? j.createdAt,
      ),
    [snapshot?.history],
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
            <div className="jobs-active-sections">
              <JobList
                jobs={runningJobs}
                nowMs={nowMs}
                onCancel={(id) => void handleCancel(id)}
              />
              {queuedOtherJobs.length > 0 || queuedChromaJobs.length > 0 ?
                <section className="jobs-active-section" aria-label="Job queue">
                  <h3 className="jobs-section-title">Queue</h3>
                  <JobList
                    jobs={queuedOtherJobs}
                    nowMs={nowMs}
                    onCancel={(id) => void handleCancel(id)}
                  />
                  {collapseChromaQueue ?
                    <ChromaQueueSummary jobs={queuedChromaJobs} />
                  : queuedChromaJobs.length > 0 ?
                    <JobList
                      jobs={queuedChromaJobs}
                      nowMs={nowMs}
                      onCancel={(id) => void handleCancel(id)}
                    />
                  : null}
                </section>
              : null}
            </div>
          )}
        </section>
      : (
        <section className="panel bulk-edit-panel jobs-panel">
          {(snapshot?.history.length ?? 0) === 0 ?
            <p className="muted">No completed jobs yet.</p>
          : (
            <JobList jobs={historyJobs} nowMs={nowMs} history />
          )}
        </section>
      )}
    </>
  );
}

