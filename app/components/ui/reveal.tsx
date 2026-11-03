'use client';

/**
 * Reading-order reveal for loaded content.
 *
 * Placeholders appear at once. Within one surface, real content replaces its placeholder only after
 * every block before it in document order (top to bottom, left to right) has been revealed, so
 * fast requests never surface content below or beside a block that is still loading.
 */

import {
  createContext,
  useContext,
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  useSyncExternalStore,
  type ReactNode,
} from 'react';

import { MotionDiv, MotionPresence, PANEL_VARIANTS } from '@/components/ui/motion';

/** Longest time a block that is still loading may hold back the ready blocks after it. */
export const REVEAL_WAIT_LIMIT_MS = 1500;

type RevealPhase = 'unresolved' | 'placeholder' | 'content';

type RevealEntry = {
  getElement: () => HTMLElement | null;
  isReady: boolean;
  isResolved: boolean;
  isRevealed: boolean;
  pendingSince: number;
};

/**
 * Order the entries of one surface by their position in the document.
 *
 * @param entries - Registered entries.
 * @returns Entries sorted top to bottom, left to right; detached entries last.
 */
function sortByDocumentOrder(entries: RevealEntry[]): RevealEntry[] {
  return entries
    .map((entry) => ({ entry, element: entry.getElement() }))
    .sort((first, second) => {
      if (!first.element || !second.element) {
        return Number(!first.element) - Number(!second.element);
      }
      if (first.element === second.element) return 0;
      return first.element.compareDocumentPosition(second.element) &
        Node.DOCUMENT_POSITION_FOLLOWING
        ? -1
        : 1;
    })
    .map(({ entry }) => entry);
}

/** Registry that decides which blocks of one surface may show their content. */
class RevealSequence {
  private readonly entries = new Map<string, RevealEntry>();
  private readonly listeners = new Set<() => void>();
  private releaseTimer: ReturnType<typeof setTimeout> | null = null;

  /**
   * Subscribe to phase changes.
   *
   * @param listener - Callback run after every recomputation.
   * @returns Unsubscribe function.
   */
  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };

  /**
   * Read the current phase of one block.
   *
   * @param id - Block identifier.
   * @returns Whether the block is unresolved, holding its placeholder, or showing content.
   */
  phase(id: string): RevealPhase {
    const entry = this.entries.get(id);
    if (!entry?.isResolved) return 'unresolved';
    return entry.isRevealed ? 'content' : 'placeholder';
  }

  /**
   * Track a newly mounted block as still loading; its readiness follows through `update`.
   *
   * @param id - Block identifier.
   * @param getElement - Reader for the block's current DOM element.
   */
  register(id: string, getElement: () => HTMLElement | null): void {
    this.entries.set(id, {
      getElement,
      isReady: false,
      isResolved: false,
      isRevealed: false,
      pendingSince: performance.now(),
    });
    this.recompute();
  }

  /**
   * Record a readiness change; a block that loads again returns to its placeholder.
   *
   * @param id - Block identifier.
   * @param isReady - Whether the block's data has settled.
   */
  update(id: string, isReady: boolean): void {
    const entry = this.entries.get(id);
    if (!entry || entry.isReady === isReady) return;
    entry.isReady = isReady;
    if (!isReady) {
      entry.isRevealed = false;
      entry.pendingSince = performance.now();
    }
    this.recompute();
  }

  /**
   * Stop tracking an unmounted block.
   *
   * @param id - Block identifier.
   */
  unregister(id: string): void {
    if (this.entries.delete(id)) this.recompute();
  }

  /** Cancel the pending release timer. */
  dispose(): void {
    if (this.releaseTimer !== null) clearTimeout(this.releaseTimer);
    this.releaseTimer = null;
  }

  /** Reveal every ready block that no earlier, still-waiting block holds back. */
  private recompute(): void {
    this.dispose();
    const now = performance.now();
    let isBlocked = false;
    let nextReleaseDelay = Number.POSITIVE_INFINITY;

    for (const entry of sortByDocumentOrder([...this.entries.values()])) {
      entry.isResolved = true;
      if (entry.isRevealed) continue;
      if (entry.isReady) {
        entry.isRevealed = !isBlocked;
        continue;
      }
      const remainingWait = REVEAL_WAIT_LIMIT_MS - (now - entry.pendingSince);
      if (remainingWait > 0) {
        isBlocked = true;
        nextReleaseDelay = Math.min(nextReleaseDelay, remainingWait);
      }
    }

    if (Number.isFinite(nextReleaseDelay)) {
      this.releaseTimer = setTimeout(() => this.recompute(), nextReleaseDelay);
    }
    for (const listener of this.listeners) listener();
  }
}

const RevealSequenceContext = createContext<RevealSequence | null>(null);

/**
 * Subscribe placeholder for blocks rendered outside any surface.
 *
 * @returns No-op unsubscribe function.
 */
function subscribeNothing(): () => void {
  return () => undefined;
}

/**
 * Give one surface (the page, a dialog, a popover, or a menu) its own reading-order sequence.
 *
 * @param props - Surface children.
 * @returns Children sharing one reveal sequence.
 */
export function RevealScope({ children }: { children: ReactNode }) {
  const [sequence] = useState(() => new RevealSequence());
  useEffect(() => () => sequence.dispose(), [sequence]);
  return <RevealSequenceContext value={sequence}>{children}</RevealSequenceContext>;
}

type RevealItemProps = {
  /** Loaded content; only rendered once the block is revealed. */
  children: ReactNode;
  /** Classes for the content element. */
  className?: string;
  /** Whether the block's first data for its current subject has settled, including failures. */
  isReady: boolean;
  /** Placeholder shown immediately and until the block is revealed. */
  placeholder: ReactNode;
  /** Classes for the placeholder element; defaults to the content classes. */
  placeholderClassName?: string;
};

/**
 * Swap one block's placeholder for its content in reading order within the current surface.
 *
 * Content that is already available when the surface mounts shows without animation; later swaps
 * remove the outgoing element at once and blur-fade the incoming one in, so a swap never waits on
 * an exit animation.
 *
 * @param props - Readiness, placeholder, content, and element classes.
 * @returns Animated placeholder or content element.
 */
export function RevealItem({
  children,
  className,
  isReady,
  placeholder,
  placeholderClassName,
}: RevealItemProps) {
  const sequence = useContext(RevealSequenceContext);
  const id = useId();
  const elementRef = useRef<HTMLDivElement | null>(null);
  const standalonePhase: RevealPhase = isReady ? 'content' : 'placeholder';
  const phase = useSyncExternalStore(
    sequence?.subscribe ?? subscribeNothing,
    () => (sequence ? sequence.phase(id) : standalonePhase),
    () => (sequence ? 'unresolved' : standalonePhase),
  );

  useLayoutEffect(() => {
    if (!sequence) return undefined;
    sequence.register(id, () => elementRef.current);
    return () => sequence.unregister(id);
  }, [sequence, id]);

  useLayoutEffect(() => {
    sequence?.update(id, isReady);
  }, [sequence, id, isReady]);

  /**
   * Track the mounted element so exiting elements keep the block's place in the order.
   *
   * @param element - Mounted element, or null while unmounting.
   */
  const trackElement = (element: HTMLDivElement | null) => {
    if (element) elementRef.current = element;
  };
  const isShowingContent = phase === 'content' && isReady;

  return (
    <MotionPresence key={phase === 'unresolved' ? 'unresolved' : 'resolved'}>
      {isShowingContent ? (
        <MotionDiv
          key="content"
          ref={trackElement}
          className={className}
          variants={PANEL_VARIANTS}
          initial="hidden"
          animate="visible"
        >
          {children}
        </MotionDiv>
      ) : (
        <MotionDiv
          key="placeholder"
          ref={trackElement}
          className={placeholderClassName ?? className}
          variants={PANEL_VARIANTS}
          initial="hidden"
          animate="visible"
        >
          {placeholder}
        </MotionDiv>
      )}
    </MotionPresence>
  );
}
