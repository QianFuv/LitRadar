'use client';

/**
 * Hydration-safe media-query subscription for responsive composition decisions.
 */

import { useCallback, useSyncExternalStore } from 'react';

/** Minimum width where the article reader is docked beside the list. */
export const READER_DOCK_MEDIA_QUERY = '(min-width: 1280px)';

/** Minimum width where filters and pickers use anchored popovers instead of bottom sheets. */
export const POPOVER_MEDIA_QUERY = '(min-width: 768px)';

/**
 * Report whether a media query currently matches, defaulting to false before hydration.
 *
 * @param query - CSS media query.
 * @returns Whether the query matches in the current browser.
 */
export function useMediaQuery(query: string): boolean {
  const subscribe = useCallback(
    (onChange: () => void) => {
      if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') {
        return () => undefined;
      }
      const mediaQueryList = window.matchMedia(query);
      if (typeof mediaQueryList.addEventListener === 'function') {
        mediaQueryList.addEventListener('change', onChange);
        return () => mediaQueryList.removeEventListener('change', onChange);
      }
      mediaQueryList.addListener?.(onChange);
      return () => mediaQueryList.removeListener?.(onChange);
    },
    [query],
  );
  const getSnapshot = useCallback(
    () =>
      typeof window !== 'undefined' &&
      typeof window.matchMedia === 'function' &&
      window.matchMedia(query).matches,
    [query],
  );

  return useSyncExternalStore(subscribe, getSnapshot, () => false);
}
