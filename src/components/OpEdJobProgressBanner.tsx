import { useCallback, useMemo, useState } from "react";
import { jobsCancel } from "../api";
import { useJobsSnapshot } from "../jobs/jobClient";
import { jobProgressBarPercent, shouldShowJobProgressBar } from "../jobs/jobUi";
import {
  findActiveOpEdJob,
  manualRematchBatchFraction,
  opEdDetectBatchFraction,
} from "../opEd";
import { errorMessage } from "../utils";

export function OpEdJobProgressBanner(props: {
  animeId: number;
  title: string;
  minEpisodes?: number;
  episodeCount?: number;
}) {
  const { animeId, title, minEpisodes = 0, episodeCount = 0 } = props;
  const jobsSnapshot = useJobsSnapshot();
  const [error, setError] = useState<string | null>(null);

  const activeOpEdJob = useMemo(
    () => findActiveOpEdJob(jobsSnapshot, animeId),
    [animeId, jobsSnapshot],
  );
  const activeOpEdBatch = activeOpEdJob
    ? (manualRematchBatchFraction(activeOpEdJob) ?? opEdDetectBatchFraction(activeOpEdJob.name))
    : null;

  const handleCancel = useCallback(async () => {
    if (!activeOpEdJob) return;
    try {
      await jobsCancel(activeOpEdJob.id);
      setError(null);
    } catch (e) {
      setError(errorMessage(e));
    }
  }, [activeOpEdJob]);

  if (episodeCount < minEpisodes && !activeOpEdJob && !error) return null;
  if (!activeOpEdJob && !error) return null;

  return (
    <section
      className="op-ed-analysis-banner"
      aria-live="polite"
      aria-label={title}
    >
      {activeOpEdJob ?
        <>
          <div className="op-ed-analysis-banner__header">
            <strong className="op-ed-analysis-banner__title">{title}</strong>
            {activeOpEdBatch ?
              <span className="muted op-ed-analysis-banner__batch">{activeOpEdBatch}</span>
            : null}
          </div>
          {activeOpEdJob.stepLabel ?
            <p className="muted op-ed-analysis-banner__step">{activeOpEdJob.stepLabel}</p>
          : null}
          {activeOpEdJob.prerequisitePending > 0 ?
            <div className="job-row-prerequisites">
              <span className="muted">Waiting for</span>
              {activeOpEdJob.waitingFor.map((prereq) => (
                <span key={prereq.jobId} className="job-prerequisite-pill">
                  #{prereq.shortId}
                </span>
              ))}
              {activeOpEdJob.prerequisitePending > activeOpEdJob.waitingFor.length ?
                <span className="job-prerequisite-pill">
                  +{activeOpEdJob.prerequisitePending - activeOpEdJob.waitingFor.length} more
                </span>
              : null}
            </div>
          : null}
          {shouldShowJobProgressBar(activeOpEdJob) ?
            <div className="job-progress-track" aria-hidden>
              <div
                className="job-progress-fill"
                style={{ width: `${jobProgressBarPercent(activeOpEdJob)}%` }}
              />
            </div>
          : null}
          {activeOpEdJob.cancelable ?
            <button type="button" onClick={() => void handleCancel()}>
              Cancel
            </button>
          : null}
        </>
      : null}
      {error ? <p className="error">{error}</p> : null}
    </section>
  );
}
