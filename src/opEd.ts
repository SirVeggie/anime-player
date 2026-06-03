import type { AnimeOpEdAnalysisSummary, JobRecord, JobsSnapshot, OpEdSegmentInfo } from "./types";

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

export function opEdSegmentLabel(segments: OpEdSegmentInfo[], kind: "op" | "ed"): string | null {
  const seg = segments.find((s) => s.kind === kind);
  if (!seg) return null;
  switch (seg.status) {
    case "matched":
      if (seg.start_sec != null && seg.end_sec != null) {
        return `${formatShortTime(seg.start_sec)}–${formatShortTime(seg.end_sec)}`;
      }
      return "matched";
    case "not_found":
      return "none";
    case "analyzing":
      return "…";
    case "failed":
      return "err";
    case "pending":
      return "—";
    default:
      return seg.status;
  }
}

function formatShortTime(seconds: number): string {
  const m = Math.floor(seconds / 60);
  const s = Math.floor(seconds % 60);
  return `${m}:${s.toString().padStart(2, "0")}`;
}

export function analysisProgressLabel(summary: AnimeOpEdAnalysisSummary | null): string {
  if (!summary) return "";
  if (summary.no_op_ed) return "No OP/ED detected for this title";
  return `OP ${summary.op_matched}/${summary.episode_count} · ED ${summary.ed_matched}/${summary.episode_count}`;
}
