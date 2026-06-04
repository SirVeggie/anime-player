import { startTransition, useEffect, useState } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { jobsGetSnapshot } from "../api";
import type { JobFinishedEvent, JobRecord, JobStatus, JobsSnapshot } from "../types";

type IdentityListener = (event: JobFinishedEvent) => void;

const identityListeners = new Map<string, Set<IdentityListener>>();

let snapshotListeners = new Set<(snapshot: JobsSnapshot) => void>();
let started = false;
/** Last payload from `jobs://updated` or `jobs_get_snapshot` — avoids IPC when opening the Jobs page. */
let cachedSnapshot: JobsSnapshot | null = null;
let snapshotFetchPromise: Promise<JobsSnapshot> | null = null;

function notifySnapshotListeners(snapshot: JobsSnapshot) {
  cachedSnapshot = snapshot;
  for (const cb of snapshotListeners) {
    cb(snapshot);
  }
}

function ensureGlobalListeners() {
  if (started) return;
  started = true;
  void listen<JobsSnapshot>("jobs://updated", (event) => {
    notifySnapshotListeners(event.payload);
  });
  void listen<JobFinishedEvent>("jobs://finished", (event) => {
    const payload = event.payload;
    const set = identityListeners.get(payload.identity);
    if (!set) return;
    for (const cb of set) {
      cb(payload);
    }
  });
}

function fetchSnapshotOnce(): Promise<JobsSnapshot> {
  if (!snapshotFetchPromise) {
    snapshotFetchPromise = jobsGetSnapshot()
      .then((snapshot) => {
        notifySnapshotListeners(snapshot);
        return snapshot;
      })
      .catch((err) => {
        snapshotFetchPromise = null;
        throw err;
      });
  }
  return snapshotFetchPromise;
}

export function getCachedJobsSnapshot(): JobsSnapshot | null {
  return cachedSnapshot;
}

export function subscribeJobsSnapshot(listener: (snapshot: JobsSnapshot) => void): () => void {
  ensureGlobalListeners();
  snapshotListeners.add(listener);
  if (cachedSnapshot) {
    listener(cachedSnapshot);
  } else {
    void fetchSnapshotOnce()
      .then(listener)
      .catch(() => {
        /* ignore */
      });
  }
  return () => {
    snapshotListeners.delete(listener);
  };
}

/** Sidebar badge only — avoids re-rendering the whole app on every job progress tick. */
export function useJobsActiveCount(): number {
  const [count, setCount] = useState(() => cachedSnapshot?.activeCount ?? 0);
  useEffect(() => {
    return subscribeJobsSnapshot((snapshot) => {
      setCount((current) => (current === snapshot.activeCount ? current : snapshot.activeCount));
    });
  }, []);
  return count;
}

export function useJobsSnapshot(): JobsSnapshot | null {
  const [snapshot, setSnapshot] = useState<JobsSnapshot | null>(() => cachedSnapshot);
  useEffect(() => {
    return subscribeJobsSnapshot((next) => {
      startTransition(() => setSnapshot(next));
    });
  }, []);
  return snapshot;
}

export function onJobIdentityFinished(identity: string, listener: IdentityListener): () => void {
  ensureGlobalListeners();
  let set = identityListeners.get(identity);
  if (!set) {
    set = new Set();
    identityListeners.set(identity, set);
  }
  set.add(listener);
  return () => {
    const current = identityListeners.get(identity);
    if (!current) return;
    current.delete(listener);
    if (current.size === 0) {
      identityListeners.delete(identity);
    }
  };
}

export function waitForJob(jobId: string): Promise<JobRecord> {
  ensureGlobalListeners();
  return new Promise((resolve, reject) => {
    let unlistenFinished: UnlistenFn | undefined;
    let unlistenUpdated: UnlistenFn | undefined;

    const cleanup = () => {
      void unlistenFinished?.();
      void unlistenUpdated?.();
    };

    const terminal = new Set<JobStatus>(["done", "failed", "canceled"]);

    const tryResolve = (snap: JobsSnapshot) => {
      const record =
        snap.history.find((j) => j.id === jobId) ?? snap.active.find((j) => j.id === jobId);
      if (record && terminal.has(record.status)) {
        cleanup();
        resolve(record);
      }
    };

    void listen<JobFinishedEvent>("jobs://finished", (event) => {
      if (event.payload.jobId !== jobId) return;
      cleanup();
      if (cachedSnapshot) {
        tryResolve(cachedSnapshot);
        return;
      }
      void fetchSnapshotOnce().then(tryResolve).catch(reject);
    }).then((fn) => {
      unlistenFinished = fn;
    });

    void listen<JobsSnapshot>("jobs://updated", (event) => {
      tryResolve(event.payload);
    }).then((fn) => {
      unlistenUpdated = fn;
    });

    if (cachedSnapshot) {
      tryResolve(cachedSnapshot);
    } else {
      void fetchSnapshotOnce().then(tryResolve).catch(() => {
        /* ignore */
      });
    }
  });
}
