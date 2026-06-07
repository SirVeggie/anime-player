import type { JobRecord, JobsSnapshot, OpEdSegmentInfo } from "./types";
import { jobStepProgressPercent } from "./jobs/jobUi";
import { formatTime } from "./utils";

export function opEdJobIdentity(animeId: number): string {
  return `op_ed_detect:${animeId}`;
}

export function manualOpEdRematchJobIdentity(animeId: number): string {
  return `manual_op_ed_rematch:${animeId}`;
}

export function manualOpEdRematchEpisodeJobIdentity(animeId: number, episodeId: number): string {
  return `manual_op_ed_rematch:${animeId}:${episodeId}`;
}

export function manualOpEdRematchJobIdentityPrefix(animeId: number): string {
  return `manual_op_ed_rematch:${animeId}:`;
}

export function opEdChromaJobIdentity(episodeId: number): string {
  return `op_ed_chroma:${episodeId}`;
}

export function activeOpEdChromaJobs(
  snapshot: JobsSnapshot | null | undefined,
  episodeIds: number[],
): JobRecord[] {
  if (!snapshot || episodeIds.length === 0) return [];
  const identities = new Set(episodeIds.map((id) => opEdChromaJobIdentity(id)));
  return snapshot.active.filter(
    (job) =>
      identities.has(job.identity) &&
      (job.status === "queued" || job.status === "running"),
  );
}

/** e.g. `Detect OP/ED (2/5)` → `(2/5)`. */
export function opEdDetectBatchFraction(jobName: string): string | null {
  const match = jobName.match(/\((\d+\/\d+)\)\s*$/);
  return match ? `(${match[1]})` : null;
}

export function findActiveOpEdJob(
  snapshot: JobsSnapshot | null | undefined,
  animeId: number,
): JobRecord | null {
  if (!snapshot) return null;
  const legacy = opEdJobIdentity(animeId);
  const batchPrefix = `${legacy}:`;
  const manualRematchPrefix = manualOpEdRematchJobIdentityPrefix(animeId);
  const legacyManualRematch = manualOpEdRematchJobIdentity(animeId);
  return (
    snapshot.active.find(
      (job) =>
        (job.identity === legacy ||
          job.identity.startsWith(batchPrefix) ||
          job.identity.startsWith(manualRematchPrefix) ||
          job.identity === legacyManualRematch) &&
        (job.status === "queued" || job.status === "running"),
    ) ?? null
  );
}

export function jobProgressPercent(job: JobRecord): number {
  return jobStepProgressPercent(job);
}

export function isOpEdSegmentMissing(segments: OpEdSegmentInfo[], kind: "op" | "ed"): boolean {
  const seg = segments.find((s) => s.kind === kind);
  return seg?.status !== "matched";
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
