import { useEffect, useRef, useState } from "react";
import { MAX_VOLUME, clampVolume } from "../volume";

export function VolumeSpeakerIcon(props: { volume: number; muted: boolean }) {
  const { volume, muted } = props;
  if (muted) {
    return (
      <svg viewBox="0 0 24 24" fill="currentColor" aria-hidden>
        <path d="M16.5 12A4.5 4.5 0 0 0 14 7.97v2.21l2.45 2.45c.03-.2.05-.41.05-.63zm2.5 0c0 .94-.2 1.82-.54 2.64l1.51 1.51A8.796 8.796 0 0 0 21 12c0-4.28-2.99-7.86-7-8.77v2.06c2.89.86 5 3.54 5 6.71zM4.27 3 3 4.27 7.73 9H3v6h4l5 5v-6.73l4.25 4.25c-.67.52-1.42.93-2.25 1.18v2.06a8.99 8.99 0 0 0 3.69-1.81L19.73 21 21 19.73l-9-9L4.27 3zM12 4 9.91 6.09 12 8.18V4z" />
      </svg>
    );
  }
  if (volume === 0) {
    return (
      <svg viewBox="0 0 24 24" fill="currentColor" aria-hidden>
        <path d="M7 9v6h4l5 5V4l-5 5H7z" />
      </svg>
    );
  }
  if (volume <= 33) {
    return (
      <svg viewBox="0 0 24 24" fill="currentColor" aria-hidden>
        <path d="M7 9v6h4l5 5V4l-5 5H7z" />
      </svg>
    );
  }
  if (volume <= 66) {
    return (
      <svg viewBox="0 0 24 24" fill="currentColor" aria-hidden>
        <path d="M18.5 12A4.5 4.5 0 0 0 16 7.97v8.05c1.48-.73 2.5-2.25 2.5-4.02zM5 9v6h4l5 5V4L9 9H5z" />
      </svg>
    );
  }
  return (
    <svg viewBox="0 0 24 24" fill="currentColor" aria-hidden>
      <path d="M3 9v6h4l5 5V4L7 9H3zm13.5 3A4.5 4.5 0 0 0 14 7.97v8.05c1.48-.73 2.5-2.25 2.5-4.02zM14 3.23v2.06c2.89.86 5 3.54 5 6.71s-2.11 5.85-5 6.71v2.06c4.01-.91 7-4.49 7-8.77s-2.99-7.86-7-8.77z" />
    </svg>
  );
}

export function VolumeControl(props: {
  volume: number;
  muted: boolean;
  popupOpen: boolean;
  buttonId?: string;
  onApplyVolume: (volume: number) => void;
  onToggleMute: () => void;
  onOpenPopup: () => void;
  onScheduleHidePopup: () => void;
}) {
  const {
    volume,
    muted,
    popupOpen,
    buttonId = "volume-control-button",
    onApplyVolume,
    onToggleMute,
    onOpenPopup,
    onScheduleHidePopup,
  } = props;
  const popupRef = useRef<HTMLDivElement>(null);
  const trackRef = useRef<HTMLDivElement>(null);
  const draggingRef = useRef(false);
  const activePointerRef = useRef<number | null>(null);
  const dragCleanupRef = useRef<(() => void) | null>(null);

  const [isDragging, setIsDragging] = useState(false);

  const volumeFromClientY = (clientY: number) => {
    const track = trackRef.current;
    if (!track) return volume;
    const rect = track.getBoundingClientRect();
    const offset = rect.width / 2;
    const ratio = 1 - Math.max(0, Math.min(1, (clientY - rect.top - offset + 1) / (rect.height - rect.width)));
    return clampVolume(Math.round(ratio * MAX_VOLUME));
  };

  useEffect(() => () => dragCleanupRef.current?.(), []);

  const onPointerDown = (e: React.PointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return;
    e.preventDefault();
    draggingRef.current = true;
    setIsDragging(true);
    activePointerRef.current = e.pointerId;
    popupRef.current?.setPointerCapture(e.pointerId);
    onApplyVolume(volumeFromClientY(e.clientY));

    const onMove = (ev: PointerEvent) => {
      if (!draggingRef.current || ev.pointerId !== activePointerRef.current) return;
      onApplyVolume(volumeFromClientY(ev.clientY));
    };
    const stopDrag = (ev?: PointerEvent) => {
      if (ev && ev.pointerId !== activePointerRef.current) return;
      if (ev && popupRef.current?.hasPointerCapture(ev.pointerId)) {
        popupRef.current.releasePointerCapture(ev.pointerId);
      }
      draggingRef.current = false;
      setIsDragging(false);
      activePointerRef.current = null;
      dragCleanupRef.current?.();
      dragCleanupRef.current = null;
    };
    const onUp = (ev: PointerEvent) => {
      if (ev.pointerId !== activePointerRef.current) return;
      onApplyVolume(volumeFromClientY(ev.clientY));
      stopDrag(ev);
    };

    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
    window.addEventListener("pointercancel", stopDrag);
    dragCleanupRef.current = () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
      window.removeEventListener("pointercancel", stopDrag);
    };
  };

  const trackWidth = trackRef.current?.clientWidth ?? 6;
  const fillOffset = Math.min(MAX_VOLUME + trackWidth, volume + trackWidth);
  const handleOffset = MAX_VOLUME - Math.min(MAX_VOLUME, volume) + trackWidth / 2;

  return (
    <div
      className="volume-control"
      onMouseEnter={onOpenPopup}
      onMouseLeave={onScheduleHidePopup}
    >
      <button
        type="button"
        id={buttonId}
        className="icon-button icon-button--player icon-button--lg"
        title={muted ? "Unmute" : "Mute"}
        aria-pressed={muted}
        onClick={(e) => {
          e.preventDefault();
          e.stopPropagation();
          onToggleMute();
        }}
      >
        <VolumeSpeakerIcon volume={volume} muted={muted} />
      </button>
      <div
        className={`volume-popup-wrap${popupOpen ? " volume-popup-wrap--open" : ""}${isDragging ? " is-dragging" : ""}`}
        aria-hidden={!popupOpen}
      >
        <div
          ref={popupRef}
          className={`volume-popup${isDragging ? " is-dragging" : ""}`}
          onPointerDown={onPointerDown}
        >
          <div
            ref={trackRef}
            className={`volume-slider-track${muted ? " volume-slider-track--muted" : ""}`}
            style={{ height: `${MAX_VOLUME + trackWidth}px` }}
          >
            <div
              className={`volume-slider-fill${!muted && volume > 100 ? " volume-slider-fill--high" : ""}`}
              style={{ height: `${fillOffset}px` }}
            />
            <div className="volume-slider-handle" style={{ top: `${handleOffset}px` }} />
          </div>
        </div>
        <div
          className={`volume-label${!muted && volume > 100 ? " volume-label--high" : ""}`}
          style={{ top: `${12 + handleOffset}px` }}
        >
          {volume}
        </div>
      </div>
    </div>
  );
}
