/**
 * Element visibility tracking on the native IntersectionObserver.
 */

import { useCallback, useLayoutEffect, useRef, useState } from 'react';

type UseInViewOptions = {
  /** Margin around the viewport that still counts as visible, in CSS margin syntax. */
  rootMargin?: string;
  /** Share of the element that must be visible. */
  threshold?: number;
  /** Stop observing once the element has been visible. */
  isOnce?: boolean;
  /** Called with every visibility change the observer reports. */
  onChange?: (isInView: boolean) => void;
};

/**
 * Track whether an element intersects the viewport. Attach the returned callback ref to the
 * element; the observer follows the element and disconnects when it unmounts.
 *
 * @param options - Viewport margin, threshold, one-shot flag, and change callback.
 * @returns Callback ref and the latest visibility.
 */
export function useInView({
  rootMargin,
  threshold = 0,
  isOnce = false,
  onChange,
}: UseInViewOptions = {}) {
  const [isInView, setIsInView] = useState(false);
  const onChangeRef = useRef(onChange);
  useLayoutEffect(() => {
    onChangeRef.current = onChange;
  });

  const ref = useCallback(
    (element: Element | null) => {
      if (!element) return;
      const observer = new IntersectionObserver(
        (entries) => {
          for (const entry of entries) {
            setIsInView(entry.isIntersecting);
            onChangeRef.current?.(entry.isIntersecting);
            if (entry.isIntersecting && isOnce) observer.disconnect();
          }
        },
        { rootMargin, threshold },
      );
      observer.observe(element);
      return () => observer.disconnect();
    },
    [rootMargin, threshold, isOnce],
  );

  return { ref, isInView };
}
