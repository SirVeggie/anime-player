import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { libraryOpsGetSnapshot } from "../api";
import type {
  LibraryOperationFinishedEvent,
  LibraryOpsSnapshot,
  LibraryUpdatedEvent,
} from "../types";

let snapshotListeners = new Set<(snapshot: LibraryOpsSnapshot) => void>();
let libraryUpdatedListeners = new Set<(event: LibraryUpdatedEvent) => void>();
let finishedListeners = new Set<(event: LibraryOperationFinishedEvent) => void>();
let started = false;
let cachedSnapshot: LibraryOpsSnapshot | null = null;
let snapshotFetchPromise: Promise<LibraryOpsSnapshot> | null = null;

function notifySnapshot(snapshot: LibraryOpsSnapshot) {
  cachedSnapshot = snapshot;
  for (const listener of snapshotListeners) {
    listener(snapshot);
  }
}

function ensureGlobalListeners() {
  if (started) return;
  started = true;
  void listen<LibraryOpsSnapshot>("library-ops://updated", (event) => {
    notifySnapshot(event.payload);
  });
  void listen<LibraryOperationFinishedEvent>("library-ops://finished", (event) => {
    for (const listener of finishedListeners) {
      listener(event.payload);
    }
  });
  void listen<LibraryUpdatedEvent>("library://updated", (event) => {
    for (const listener of libraryUpdatedListeners) {
      listener(event.payload);
    }
  });
}

function fetchSnapshotOnce(): Promise<LibraryOpsSnapshot> {
  if (!snapshotFetchPromise) {
    snapshotFetchPromise = libraryOpsGetSnapshot()
      .then((snapshot) => {
        notifySnapshot(snapshot);
        return snapshot;
      })
      .catch((err) => {
        snapshotFetchPromise = null;
        throw err;
      });
  }
  return snapshotFetchPromise;
}

export function subscribeLibraryOpsSnapshot(
  listener: (snapshot: LibraryOpsSnapshot) => void,
): () => void {
  ensureGlobalListeners();
  snapshotListeners.add(listener);
  if (cachedSnapshot) {
    listener(cachedSnapshot);
  } else {
    void fetchSnapshotOnce().then(listener).catch(() => {
      /* ignore */
    });
  }
  return () => {
    snapshotListeners.delete(listener);
  };
}

export function subscribeLibraryUpdated(listener: (event: LibraryUpdatedEvent) => void): () => void {
  ensureGlobalListeners();
  libraryUpdatedListeners.add(listener);
  return () => {
    libraryUpdatedListeners.delete(listener);
  };
}

export function subscribeLibraryOperationFinished(
  listener: (event: LibraryOperationFinishedEvent) => void,
): () => void {
  ensureGlobalListeners();
  finishedListeners.add(listener);
  return () => {
    finishedListeners.delete(listener);
  };
}

export function useLibraryOpsActiveCount(): number {
  const [count, setCount] = useState(() => cachedSnapshot?.activeCount ?? 0);
  useEffect(() => {
    return subscribeLibraryOpsSnapshot((snapshot) => {
      setCount((current) => (current === snapshot.activeCount ? current : snapshot.activeCount));
    });
  }, []);
  return count;
}

export function useLibraryOpsSnapshot(): LibraryOpsSnapshot | null {
  const [snapshot, setSnapshot] = useState<LibraryOpsSnapshot | null>(() => cachedSnapshot);
  useEffect(() => {
    return subscribeLibraryOpsSnapshot(setSnapshot);
  }, []);
  return snapshot;
}
