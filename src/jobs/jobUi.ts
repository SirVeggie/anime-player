import type { JobRecord } from "../types";

export function jobStepProgressPercent(job: JobRecord): number {
  if (job.progress.totalSteps <= 0) return 0;
  return Math.round((job.progress.currentStep / job.progress.totalSteps) * 100);
}

export function jobPrerequisiteProgressPercent(job: JobRecord): number {
  if (job.prerequisiteProgressTotal <= 0) return 0;
  return Math.round(
    (job.prerequisiteProgressCurrent / job.prerequisiteProgressTotal) * 100,
  );
}

/** Progress bar fill: prerequisite completion while queued, then job steps when running. */
export function jobProgressBarPercent(job: JobRecord): number {
  if (job.status === "queued" && job.prerequisiteTotal > 0) {
    return jobPrerequisiteProgressPercent(job);
  }
  return jobStepProgressPercent(job);
}

export function shouldShowJobProgressBar(job: JobRecord): boolean {
  if (job.status === "running") return job.progress.totalSteps > 1;
  if (job.status === "queued") return job.prerequisiteTotal > 0;
  return false;
}
