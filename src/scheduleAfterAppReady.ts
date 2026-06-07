/** Wait for first paint, then run when the main thread is idle (or after a timeout). */
export function scheduleAfterAppReady(
  task: () => void,
  options?: { idleTimeoutMs?: number; fallbackDelayMs?: number },
): () => void {
  const idleTimeoutMs = options?.idleTimeoutMs ?? 3000;
  const fallbackDelayMs = options?.fallbackDelayMs ?? 1500;
  let cancelled = false;
  let idleId: number | undefined;
  let timeoutId: number | undefined;
  let raf1 = 0;
  let raf2 = 0;

  const cancel = () => {
    cancelled = true;
    cancelAnimationFrame(raf1);
    cancelAnimationFrame(raf2);
    if (idleId !== undefined && typeof cancelIdleCallback === "function") {
      cancelIdleCallback(idleId);
    }
    if (timeoutId !== undefined) {
      window.clearTimeout(timeoutId);
    }
  };

  const run = () => {
    if (cancelled) return;
    task();
  };

  raf1 = requestAnimationFrame(() => {
    raf2 = requestAnimationFrame(() => {
      if (cancelled) return;
      if (typeof requestIdleCallback === "function") {
        idleId = requestIdleCallback(run, { timeout: idleTimeoutMs });
      } else {
        timeoutId = window.setTimeout(run, fallbackDelayMs);
      }
    });
  });

  return cancel;
}
