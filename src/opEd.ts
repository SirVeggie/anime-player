import type { JobRecord, JobsSnapshot, OpEdSegmentInfo } from "./types";
import { formatTime } from "./utils";

export function opEdJobIdentity(animeId: number): string {
  return `op_ed_detect:${animeId}`;
}

export function findActiveOpEdJob(
  snapshot: JobsSnapshot | null | undefined,
  animeId: number,
): JobRecord | null {
  if (!snapshot) return null;
  const identity = opEdJobIdentity(animeId);
  return (
    snapshot.active.find(
      (job) =>
        job.identity === identity &&
        (job.status === "queued" || job.status === "running"),
    ) ?? null
  );
}

export function jobProgressPercent(job: JobRecord): number {
  if (job.progress.totalSteps <= 0) return 0;
  return Math.round((job.progress.currentStep / job.progress.totalSteps) * 100);
}

/** Time range for episode list OP/ED pills; null hides the pill for that kind. */
export function opEdSegmentLabel(segments: OpEdSegmentInfo[], kind: "op" | "ed"): string | null {
  const seg = segments.find((s) => s.kind === kind);
  if (!seg) return null;

  if (seg.startSec != null && seg.endSec != null) {
    return `${formatTime(seg.startSec)}–${formatTime(seg.endSec)}`;
  }

  switch (seg.status) {
    case "analyzing":
      return "…";
    case "not_found":
      return "none";
    case "failed":
      return "err";
    case "pending":
      return "—";
    default:
      return null;
  }
}

export type OpEdSeekMarker = {
  kind: "op" | "ed";
  startSec: number;
  endSec: number;
};

export function opEdSeekMarkers(segments: OpEdSegmentInfo[], duration: number): OpEdSeekMarker[] {
  if (!Number.isFinite(duration) || duration <= 0) return [];
  return segments.flatMap((seg) => {
    if (seg.status !== "matched" || seg.startSec == null || seg.endSec == null) return [];
    if (seg.endSec <= seg.startSec) return [];
    return [{ kind: seg.kind as "op" | "ed", startSec: seg.startSec, endSec: seg.endSec }];
  });
}
