import { useEffect, useRef, useState } from "react";
import { AlertCircleIcon, CheckCircleIcon } from "./Icons";

export type Toast = { id: number; kind: "success" | "error"; message: string };

const TOAST_DURATION_MS = 4500;
const TOAST_EXIT_MS = 240;

export function ToastStack(props: { toasts: Toast[]; onDismiss: (id: number) => void }) {
  const { toasts, onDismiss } = props;
  return (
    <div className="toast-stack" aria-live="polite" aria-atomic="false">
      {toasts.map((toast) => (
        <ToastItem key={toast.id} toast={toast} onDismiss={onDismiss} />
      ))}
    </div>
  );
}

function ToastItem(props: { toast: Toast; onDismiss: (id: number) => void }) {
  const { toast, onDismiss } = props;
  const [open, setOpen] = useState(false);
  const [paused, setPaused] = useState(false);
  const onDismissRef = useRef(onDismiss);
  onDismissRef.current = onDismiss;

  // Mount in the closed state, then flip to open on the next frame so the CSS
  // transition runs from the initial transform/opacity to the resting state.
  useEffect(() => {
    const id = requestAnimationFrame(() => setOpen(true));
    return () => cancelAnimationFrame(id);
  }, []);

  // Auto-dismiss timer; pauses while the user hovers so they have time to read.
  useEffect(() => {
    if (paused || !open) return;
    const id = window.setTimeout(() => setOpen(false), TOAST_DURATION_MS);
    return () => window.clearTimeout(id);
  }, [open, paused]);

  // Once the exit transition finishes, drop the toast from App's state.
  useEffect(() => {
    if (open) return;
    const id = window.setTimeout(() => onDismissRef.current(toast.id), TOAST_EXIT_MS);
    return () => window.clearTimeout(id);
  }, [open, toast.id]);

  const Icon = toast.kind === "success" ? CheckCircleIcon : AlertCircleIcon;

  return (
    <div
      className={`toast toast--${toast.kind}`}
      data-state={open ? "open" : "closed"}
      role={toast.kind === "error" ? "alert" : "status"}
      onMouseEnter={() => setPaused(true)}
      onMouseLeave={() => setPaused(false)}
      onClick={() => setOpen(false)}
    >
      <span className="toast-icon" aria-hidden>
        <Icon />
      </span>
      <span className="toast-message">{toast.message}</span>
      <span
        className="toast-progress"
        aria-hidden
        data-paused={paused ? "true" : "false"}
        data-state={open ? "open" : "closed"}
      />
    </div>
  );
}
