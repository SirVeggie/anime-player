import { type ButtonHTMLAttributes, useCallback, useEffect, useId, useMemo, useState } from "react";
import { isTextInputTarget } from "./utils";

const ARROW_KEYS = new Set(["ArrowDown", "ArrowLeft", "ArrowRight", "ArrowUp"]);

type RovingItemProps = ButtonHTMLAttributes<HTMLButtonElement> & {
  "data-roving-list-item": string;
};

function getItemCenter(rect: DOMRect) {
  return {
    x: rect.left + rect.width / 2,
    y: rect.top + rect.height / 2,
  };
}

function isVisibleItem(item: HTMLElement) {
  return item.offsetParent !== null && !item.hasAttribute("disabled");
}

function getScrollContainer(item: HTMLElement) {
  return item.closest<HTMLElement>(".content");
}

function isInTopRow(items: HTMLElement[], item: HTMLElement) {
  const firstItemTop = items[0]?.getBoundingClientRect().top;
  if (firstItemTop === undefined) return false;
  const itemTop = item.getBoundingClientRect().top;
  return Math.abs(itemTop - firstItemTop) <= 4;
}

function findVerticalTargetIndex(items: HTMLElement[], currentIndex: number, direction: "up" | "down") {
  const currentRect = items[currentIndex].getBoundingClientRect();
  const currentCenter = getItemCenter(currentRect);
  let bestIndex = currentIndex;
  let bestScore = Number.POSITIVE_INFINITY;

  items.forEach((item, index) => {
    if (index === currentIndex) return;
    const rect = item.getBoundingClientRect();
    const center = getItemCenter(rect);
    const primaryDistance = direction === "down" ? center.y - currentCenter.y : currentCenter.y - center.y;
    if (primaryDistance <= 4) return;

    const perpendicularDistance = Math.abs(center.x - currentCenter.x);
    const score = primaryDistance * 4 + perpendicularDistance;
    if (score < bestScore) {
      bestIndex = index;
      bestScore = score;
    }
  });

  return bestIndex;
}

function findNextIndex(items: HTMLElement[], currentIndex: number, key: string) {
  switch (key) {
    case "ArrowLeft":
      return Math.max(0, currentIndex - 1);
    case "ArrowRight":
      return Math.min(items.length - 1, currentIndex + 1);
    case "ArrowUp":
      return findVerticalTargetIndex(items, currentIndex, "up");
    case "ArrowDown":
      return findVerticalTargetIndex(items, currentIndex, "down");
    default:
      return currentIndex;
  }
}

export function useRovingListNavigation(itemCount: number, options: { enabled?: boolean } = {}) {
  const { enabled = true } = options;
  const generatedId = useId();
  const listId = useMemo(() => `roving-${generatedId.replace(/[^a-zA-Z0-9_-]/g, "")}`, [generatedId]);
  const [activeIndex, setActiveIndex] = useState(0);

  const getItems = useCallback(() => {
    return Array.from(document.querySelectorAll<HTMLElement>(`[data-roving-list-item="${listId}"]`)).filter(
      isVisibleItem,
    );
  }, [listId]);

  useEffect(() => {
    setActiveIndex((current) => Math.max(0, Math.min(current, itemCount - 1)));
  }, [itemCount]);

  useEffect(() => {
    if (!enabled) return;

    const onKeyDown = (event: KeyboardEvent) => {
      if (!ARROW_KEYS.has(event.key)) return;
      if (isTextInputTarget(event.target)) return;

      const items = getItems();
      if (items.length === 0) return;

      event.preventDefault();
      const focusedIndex = items.findIndex((item) => item === document.activeElement);
      const currentIndex =
        focusedIndex >= 0 ? focusedIndex : Math.max(0, Math.min(activeIndex, items.length - 1));
      const nextIndex = findNextIndex(items, currentIndex, event.key);
      const nextItem = items[nextIndex];

      setActiveIndex(nextIndex);
      nextItem.focus({ preventScroll: true });
      if (isInTopRow(items, nextItem)) {
        getScrollContainer(nextItem)?.scrollTo({ top: 0, left: 0, behavior: "auto" });
      } else {
        nextItem.scrollIntoView({ block: "nearest", inline: "nearest" });
      }
    };

    window.addEventListener("keydown", onKeyDown, true);
    return () => window.removeEventListener("keydown", onKeyDown, true);
  }, [activeIndex, enabled, getItems]);

  const getRovingItemProps = useCallback(
    (index: number): RovingItemProps => ({
      "data-roving-list-item": listId,
      tabIndex: index === activeIndex ? 0 : -1,
      onFocus: () => setActiveIndex(index),
    }),
    [activeIndex, listId],
  );

  return getRovingItemProps;
}
