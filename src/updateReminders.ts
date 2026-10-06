import type { LibraryState, UpdateStatus } from "./types";

export function shouldRemindAboutUpdate(
  library: LibraryState | null | undefined,
  status: UpdateStatus | null | undefined,
): boolean {
  if (!library || !status) return false;
  if (!status.available && !status.pending_apply) return false;
  const latest = status.latest_version?.trim();
  const skipped = library.skipped_update_version?.trim();
  if (latest && skipped && latest === skipped) return false;
  return true;
}

/** Collapse empty markdown sections into a short changelog for the update popup. */
export function summarizeUpdateNotes(notes: string, maxItems = 8): string {
  const lines = notes.replace(/\r\n/g, "\n").split("\n");
  const kept: string[] = [];
  let pendingHeading: string | null = null;

  for (const line of lines) {
    const trimmed = line.trim();
    if (!trimmed) continue;
    const heading = trimmed.match(/^#{1,6}\s+(.*)$/);
    if (heading) {
      pendingHeading = heading[1].trim();
      continue;
    }
    if (pendingHeading) {
      kept.push(pendingHeading);
      pendingHeading = null;
      if (kept.length >= maxItems) break;
    }
    kept.push(trimmed.replace(/^[-*]\s+/, "• "));
    if (kept.length >= maxItems) break;
  }

  return kept.join("\n");
}
