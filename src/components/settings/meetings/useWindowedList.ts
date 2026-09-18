import { useCallback, useEffect, useMemo, useRef, useState } from "react";

/**
 * Minimal windowed list: render only the rows near the viewport.
 *
 * Written by hand rather than pulled in as a dependency — `@tanstack/react-virtual`
 * is not in `package.json` and an hour-long transcript does not justify adding
 * one. It is variable-height by necessity: a speaker turn is one line or twenty,
 * so heights are measured after paint and cached, with an estimate standing in
 * for rows that have never been on screen. Offsets are a prefix sum over that
 * cache, which is what lets the scrollbar be the right length before the whole
 * transcript has ever been rendered.
 */
export interface WindowedListOptions {
  /** Total rows. */
  count: number;
  /** Height assumed for a row that has not been measured yet. */
  estimatedItemHeight: number;
  /** Extra pixels rendered above and below the viewport. */
  overscanPx?: number;
  /**
   * Changes when the rows describe a different list.
   *
   * Rows are addressed by index, so a cache built for one transcript would
   * describe the wrong rows for the next one — same indexes, different content.
   */
  resetKey?: string | number;
}

export interface WindowedItem {
  index: number;
  /** Distance from the top of the scrolled content, in pixels. */
  offset: number;
}

export interface WindowedList {
  /** Attach to the scrolling element. */
  containerRef: React.RefObject<HTMLDivElement>;
  /** Height of the spacer that holds the scrollbar open. */
  totalHeight: number;
  /** The rows to render right now. */
  items: WindowedItem[];
  /** Ref callback for a rendered row, so its real height replaces the estimate. */
  measure: (index: number) => (node: HTMLElement | null) => void;
}

export const useWindowedList = ({
  count,
  estimatedItemHeight,
  overscanPx = 600,
  resetKey,
}: WindowedListOptions): WindowedList => {
  const containerRef = useRef<HTMLDivElement>(null);
  const heights = useRef<Map<number, number>>(new Map());
  const nodes = useRef<Map<number, HTMLElement>>(new Map());
  const callbacks = useRef<Map<number, (node: HTMLElement | null) => void>>(
    new Map(),
  );
  const observer = useRef<ResizeObserver | null>(null);
  // Bumped whenever a measurement changes, which is the only thing that can
  // invalidate the offsets without the row count changing.
  const [measured, setMeasured] = useState(0);
  const [viewport, setViewport] = useState({ scrollTop: 0, height: 0 });

  // One observer for every row, so a turn that reflows (window resize, a font
  // change) corrects its own offset instead of leaving a gap.
  useEffect(() => {
    if (typeof ResizeObserver === "undefined") return;
    const instance = new ResizeObserver((entries) => {
      let changed = false;
      for (const entry of entries) {
        const target = entry.target as HTMLElement;
        const index = Number(target.dataset.windowIndex);
        if (!Number.isInteger(index)) continue;
        const height = target.offsetHeight;
        // A sub-pixel tolerance: without it a fractional layout height can
        // oscillate against the cached value and re-render forever.
        if (
          height > 0 &&
          Math.abs((heights.current.get(index) ?? 0) - height) > 0.5
        ) {
          heights.current.set(index, height);
          changed = true;
        }
      }
      if (changed) setMeasured((value) => value + 1);
    });
    observer.current = instance;
    return () => {
      instance.disconnect();
      observer.current = null;
    };
  }, []);

  // Rows are keyed by index, and the cache would otherwise describe the
  // previous list after the transcript is replaced (opening another meeting).
  useEffect(() => {
    heights.current.clear();
    nodes.current.clear();
    callbacks.current.clear();
    setMeasured((value) => value + 1);
  }, [estimatedItemHeight, resetKey]);

  const measure = useCallback((index: number) => {
    const existing = callbacks.current.get(index);
    if (existing) return existing;
    // Cached so the ref identity is stable across renders; a fresh closure each
    // render would detach and re-observe every visible row every time.
    const callback = (node: HTMLElement | null) => {
      const previous = nodes.current.get(index);
      if (previous && previous !== node) {
        observer.current?.unobserve(previous);
        nodes.current.delete(index);
      }
      if (!node) return;
      node.dataset.windowIndex = String(index);
      nodes.current.set(index, node);
      const height = node.offsetHeight;
      if (
        height > 0 &&
        Math.abs((heights.current.get(index) ?? 0) - height) > 0.5
      ) {
        heights.current.set(index, height);
        setMeasured((value) => value + 1);
      }
      observer.current?.observe(node);
    };
    callbacks.current.set(index, callback);
    return callback;
  }, []);

  const { offsets, totalHeight } = useMemo(() => {
    const prefix = new Array<number>(count + 1);
    prefix[0] = 0;
    for (let index = 0; index < count; index += 1) {
      prefix[index + 1] =
        prefix[index] + (heights.current.get(index) ?? estimatedItemHeight);
    }
    return { offsets: prefix, totalHeight: prefix[count] ?? 0 };
    // `measured` is the dependency that matters; it stands in for the mutable
    // height cache, which React cannot observe.
  }, [count, estimatedItemHeight, measured]);

  // `count` is in the deps because the scroll container is only mounted once
  // there is something to scroll, so the first run can find no element at all.
  useEffect(() => {
    const element = containerRef.current;
    if (!element) return;
    let frame = 0;
    const sync = () => {
      frame = 0;
      setViewport({
        scrollTop: element.scrollTop,
        height: element.clientHeight,
      });
    };
    const onScroll = () => {
      if (!frame) frame = requestAnimationFrame(sync);
    };
    sync();
    element.addEventListener("scroll", onScroll, { passive: true });
    const resize =
      typeof ResizeObserver === "undefined" ? null : new ResizeObserver(sync);
    resize?.observe(element);
    return () => {
      element.removeEventListener("scroll", onScroll);
      resize?.disconnect();
      if (frame) cancelAnimationFrame(frame);
    };
  }, [count]);

  const items = useMemo<WindowedItem[]>(() => {
    if (count === 0) return [];
    const top = viewport.scrollTop - overscanPx;
    // Before the first measurement the container reports no height; assuming a
    // screenful means the first paint is not a single row.
    const bottom = viewport.scrollTop + (viewport.height || 600) + overscanPx;

    // First row whose bottom edge is still below the top of the window. Starting
    // at the last row covers the case where every row ends above it, which
    // happens for a moment after the list shrinks under a deep scroll position.
    let low = 0;
    let high = count - 1;
    let start = count - 1;
    while (low <= high) {
      const mid = (low + high) >> 1;
      if (offsets[mid + 1] <= top) {
        low = mid + 1;
      } else {
        start = mid;
        high = mid - 1;
      }
    }

    const visible: WindowedItem[] = [];
    for (let index = start; index < count; index += 1) {
      if (offsets[index] > bottom) break;
      visible.push({ index, offset: offsets[index] });
    }
    return visible;
  }, [count, offsets, viewport, overscanPx]);

  return { containerRef, totalHeight, items, measure };
};
