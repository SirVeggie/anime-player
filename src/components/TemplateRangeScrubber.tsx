import { type ReactNode, useCallback, useEffect, useRef } from "react";
import { formatTime } from "../utils";

const MIN_DURATION_SEC = 5;
const MAX_DURATION_SEC = 180;
const REPEAT_MS = 80;
const REPEAT_DELAY_MS = 200;
const COARSE_FRAME_MULTIPLIER = 5;

type DragMode = "left" | "right" | "move" | null;

type FrameStepConfig = {
  frameMultiplier: number;
  ariaStepLabel: string;
  glyph: string;
  coarse?: boolean;
};

export function clampTemplateRange(
  startSec: number,
  endSec: number,
  duration: number,
): { startSec: number; endSec: number } {
  let start = Math.max(0, startSec);
  let end = Math.min(duration, endSec);
  if (end - start < MIN_DURATION_SEC) {
    end = Math.min(duration, start + MIN_DURATION_SEC);
    if (end - start < MIN_DURATION_SEC) {
      start = Math.max(0, end - MIN_DURATION_SEC);
    }
  }
  if (end - start > MAX_DURATION_SEC) {
    end = start + MAX_DURATION_SEC;
  }
  return { startSec: start, endSec: end };
}

const FRAME_STEP_CONFIGS: FrameStepConfig[] = [
  { frameMultiplier: -COARSE_FRAME_MULTIPLIER, ariaStepLabel: "5 frames earlier", glyph: "«", coarse: true },
  { frameMultiplier: -1, ariaStepLabel: "one frame earlier", glyph: "◀" },
  { frameMultiplier: 1, ariaStepLabel: "one frame later", glyph: "▶" },
  { frameMultiplier: COARSE_FRAME_MULTIPLIER, ariaStepLabel: "5 frames later", glyph: "»", coarse: true },
];

function FrameStepButton(props: {
  label: string;
  config: FrameStepConfig;
  frameStepSec: number;
  onStep: (delta: number) => void;
}) {
  const { label, config, frameStepSec, onStep } = props;
  const delayRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const repeatRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const holdingRef = useRef(false);
  const onStepRef = useRef(onStep);
  onStepRef.current = onStep;
  const deltaRef = useRef(config.frameMultiplier * frameStepSec);
  deltaRef.current = config.frameMultiplier * frameStepSec;

  const clearRepeat = useCallback(() => {
    holdingRef.current = false;
    if (delayRef.current !== null) {
      window.clearTimeout(delayRef.current);
      delayRef.current = null;
    }
    if (repeatRef.current !== null) {
      window.clearInterval(repeatRef.current);
      repeatRef.current = null;
    }
  }, []);

  const stepOnce = useCallback(() => {
    onStepRef.current(deltaRef.current);
  }, []);

  const startRepeat = useCallback(
    (e: React.PointerEvent<HTMLButtonElement>) => {
      if (e.button !== 0 || holdingRef.current) return;
      e.preventDefault();
      e.stopPropagation();
      holdingRef.current = true;
      stepOnce();
      delayRef.current = window.setTimeout(() => {
        delayRef.current = null;
        repeatRef.current = window.setInterval(stepOnce, REPEAT_MS);
      }, REPEAT_DELAY_MS);
    },
    [stepOnce],
  );

  useEffect(() => {
    const stop = () => clearRepeat();
    window.addEventListener("pointerup", stop);
    window.addEventListener("pointercancel", stop);
    return () => {
      window.removeEventListener("pointerup", stop);
      window.removeEventListener("pointercancel", stop);
      clearRepeat();
    };
  }, [clearRepeat]);

  return (
    <button
      type="button"
      className={`template-range-stepper__btn${config.coarse ? " template-range-stepper__btn--coarse" : ""}`}
      aria-label={`${label} ${config.ariaStepLabel}`}
      onPointerDown={startRepeat}
      onClick={(e) => e.preventDefault()}
    >
      {config.glyph}
    </button>
  );
}

function FrameStepButtons(props: {
  label: string;
  onStep: (delta: number) => void;
  frameStepSec: number;
}) {
  return (
    <div className="template-range-stepper">
      <span className="template-range-stepper__label">{props.label}</span>
      <div className="template-range-stepper__buttons">
        {FRAME_STEP_CONFIGS.map((config) => (
          <FrameStepButton
            key={`${props.label}-${config.frameMultiplier}`}
            label={props.label}
            config={config}
            frameStepSec={props.frameStepSec}
            onStep={props.onStep}
          />
        ))}
      </div>
    </div>
  );
}

export function TemplateRangeScrubber(props: {
  duration: number;
  startSec: number;
  endSec: number;
  frameStepSec: number;
  onStartChange: (startSec: number) => void;
  onEndChange: (endSec: number) => void;
  onRangeChange?: (startSec: number, endSec: number) => void;
  onSeek: (seconds: number) => void;
  centerActions?: ReactNode;
  trailingActions?: ReactNode;
}) {
  const {
    duration,
    startSec,
    endSec,
    frameStepSec,
    onStartChange,
    onEndChange,
    onRangeChange,
    onSeek,
    centerActions,
    trailingActions,
  } = props;
  const trackRef = useRef<HTMLDivElement>(null);
  const dragRef = useRef<{
    mode: DragMode;
    pointerStartX: number;
    rangeStart: number;
    rangeEnd: number;
  } | null>(null);
  const startSecRef = useRef(startSec);
  const endSecRef = useRef(endSec);
  const durationRef = useRef(duration);
  const onStartChangeRef = useRef(onStartChange);
  const onEndChangeRef = useRef(onEndChange);
  const onSeekRef = useRef(onSeek);
  startSecRef.current = startSec;
  endSecRef.current = endSec;
  durationRef.current = duration;
  onStartChangeRef.current = onStartChange;
  onEndChangeRef.current = onEndChange;
  onSeekRef.current = onSeek;

  const ratioAt = (clientX: number) => {
    const track = trackRef.current;
    if (!track || duration <= 0) return 0;
    const rect = track.getBoundingClientRect();
    const x = clientX - rect.left;
    return Math.min(1, Math.max(0, x / rect.width));
  };

  const applyPointer = useCallback(
    (clientX: number) => {
      const drag = dragRef.current;
      if (!drag || duration <= 0) return;
      const deltaRatio = ratioAt(clientX) - ratioAt(drag.pointerStartX);
      const deltaSec = deltaRatio * duration;
      if (drag.mode === "left") {
        const next = clampTemplateRange(drag.rangeStart + deltaSec, drag.rangeEnd, duration);
        onStartChange(next.startSec);
        onSeek(next.startSec);
      } else if (drag.mode === "right") {
        const next = clampTemplateRange(drag.rangeStart, drag.rangeEnd + deltaSec, duration);
        onEndChange(next.endSec);
        onSeek(next.endSec);
      } else if (drag.mode === "move") {
        const len = drag.rangeEnd - drag.rangeStart;
        let nextStart = drag.rangeStart + deltaSec;
        nextStart = Math.max(0, Math.min(duration - len, nextStart));
        const nextEnd = nextStart + len;
        if (onRangeChange) {
          onRangeChange(nextStart, nextEnd);
        } else {
          onStartChange(nextStart);
          onEndChange(nextEnd);
        }
        onSeek(nextStart);
      }
    },
    [duration, onEndChange, onRangeChange, onSeek, onStartChange],
  );

  useEffect(() => {
    const onMove = (e: PointerEvent) => {
      if (!dragRef.current) return;
      applyPointer(e.clientX);
    };
    const onUp = () => {
      dragRef.current = null;
    };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
    return () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
    };
  }, [applyPointer]);

  const startPct = duration > 0 ? (startSec / duration) * 100 : 0;
  const endPct = duration > 0 ? (endSec / duration) * 100 : 0;

  const beginDrag = (mode: DragMode, e: React.PointerEvent) => {
    if (e.button !== 0 || duration <= 0) return;
    e.preventDefault();
    e.stopPropagation();
    dragRef.current = {
      mode,
      pointerStartX: e.clientX,
      rangeStart: startSec,
      rangeEnd: endSec,
    };
    if (mode === "left") onSeek(startSec);
    else if (mode === "right") onSeek(endSec);
    else onSeek(startSec);
  };

  const stepStart = useCallback((delta: number) => {
    const next = clampTemplateRange(
      startSecRef.current + delta,
      endSecRef.current,
      durationRef.current,
    );
    onStartChangeRef.current(next.startSec);
    onSeekRef.current(next.startSec);
  }, []);

  const stepEnd = useCallback((delta: number) => {
    const next = clampTemplateRange(
      startSecRef.current,
      endSecRef.current + delta,
      durationRef.current,
    );
    onEndChangeRef.current(next.endSec);
    onSeekRef.current(next.endSec);
  }, []);

  const frameStepSecRef = useRef(frameStepSec);
  frameStepSecRef.current = frameStepSec;

  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.code !== "ArrowLeft" && e.code !== "ArrowRight") return;
      const target = e.target;
      if (
        target instanceof HTMLElement &&
        (target.isContentEditable ||
          target.tagName === "INPUT" ||
          target.tagName === "TEXTAREA" ||
          target.tagName === "SELECT")
      ) {
        return;
      }
      if (durationRef.current <= 0) return;

      const direction = e.code === "ArrowLeft" ? -1 : 1;
      const frames = e.shiftKey ? COARSE_FRAME_MULTIPLIER : 1;
      const delta = direction * frames * frameStepSecRef.current;

      e.preventDefault();
      e.stopPropagation();

      if (e.ctrlKey) {
        stepEnd(delta);
      } else {
        stepStart(delta);
      }
    };
    window.addEventListener("keydown", onKeyDown, true);
    return () => window.removeEventListener("keydown", onKeyDown, true);
  }, [stepEnd, stepStart]);

  return (
    <div className="template-range-scrubber">
      <div
        ref={trackRef}
        className="template-range-scrubber__track"
        role="group"
        aria-label="Skip area range"
      >
        <div className="template-range-scrubber__dim" style={{ width: `${startPct}%` }} />
        <div
          className="template-range-scrubber__selection"
          style={{ left: `${startPct}%`, width: `${Math.max(0, endPct - startPct)}%` }}
          onPointerDown={(e) => beginDrag("move", e)}
        />
        <div
          className="template-range-scrubber__dim template-range-scrubber__dim--tail"
          style={{ left: `${endPct}%`, width: `${Math.max(0, 100 - endPct)}%` }}
        />
        <button
          type="button"
          className="template-range-scrubber__handle template-range-scrubber__handle--left"
          style={{ left: `${startPct}%` }}
          aria-label={`Start ${formatTime(startSec)}`}
          onPointerDown={(e) => beginDrag("left", e)}
        >
          <span className="template-range-scrubber__handle-time">{formatTime(startSec)}</span>
        </button>
        <button
          type="button"
          className="template-range-scrubber__handle template-range-scrubber__handle--right"
          style={{ left: `${endPct}%` }}
          aria-label={`End ${formatTime(endSec)}`}
          onPointerDown={(e) => beginDrag("right", e)}
        >
          <span className="template-range-scrubber__handle-time">{formatTime(endSec)}</span>
        </button>
      </div>
      <div className="template-range-scrubber__steppers">
        <FrameStepButtons label="Start" frameStepSec={frameStepSec} onStep={stepStart} />
        {centerActions ? <div className="template-range-scrubber__center-actions">{centerActions}</div> : null}
        <FrameStepButtons label="End" frameStepSec={frameStepSec} onStep={stepEnd} />
        {trailingActions ? (
          <div className="template-range-scrubber__trailing-actions">{trailingActions}</div>
        ) : null}
      </div>
    </div>
  );
}

export function defaultEditorRange(kind: "op" | "ed", duration: number): { startSec: number; endSec: number } {
  const len = Math.min(90, MAX_DURATION_SEC, Math.max(MIN_DURATION_SEC, duration));
  if (kind === "ed" && duration > len) {
    const start = Math.max(0, duration - len);
    return { startSec: start, endSec: Math.min(duration, start + len) };
  }
  return { startSec: 0, endSec: Math.min(duration, len) };
}
