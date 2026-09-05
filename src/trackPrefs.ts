import type { MpvTrack, TrackPref } from "./types";

function norm(value: string | null | undefined): string {
  return (value ?? "").trim();
}

export function trackPrefFromTracks(tracks: MpvTrack[]): TrackPref {
  const audio = tracks.find((track) => track.kind === "audio" && track.selected) ?? null;
  const sub = tracks.find((track) => track.kind === "sub" && track.selected) ?? null;
  return {
    audio_lang: audio?.lang ?? null,
    audio_title: audio?.title ?? null,
    subtitle_off: sub === null,
    subtitle_lang: sub?.lang ?? null,
    subtitle_title: sub?.title ?? null,
    subtitle_external_path: sub?.external ? (sub.external_filename ?? null) : null,
  };
}

export function trackPrefsEqual(left: TrackPref | null, right: TrackPref | null): boolean {
  if (left === right) return true;
  if (!left || !right) return false;
  if (norm(left.audio_lang) !== norm(right.audio_lang)) return false;
  if (norm(left.audio_title) !== norm(right.audio_title)) return false;
  if (left.subtitle_off !== right.subtitle_off) return false;
  if (left.subtitle_off) return true;
  return (
    norm(left.subtitle_lang) === norm(right.subtitle_lang) &&
    norm(left.subtitle_title) === norm(right.subtitle_title) &&
    norm(left.subtitle_external_path) === norm(right.subtitle_external_path)
  );
}
