/**
 * Media-query subscription for responsive composition decisions.
 */

import { useCallback, useSyncExternalStore } from 'react';

/** Minimum width where the article reader is docked beside the list. */
export const READER_DOCK_MEDIA_QUERY = '(min-width: 1280px)';

/**
 * Report whether a media query currently matches.
 *
 * @param query - CSS media query.
 * @returns Whether the query matches in the current browser.
 */
export function useMediaQuery(query: string): boolean {
  const subscribe = useCallback(
    (onChange: () => void) => {
      const mediaQueryList = window.matchMedia(query);
      mediaQueryList.addEventListener('change', onChange);
      return () => mediaQueryList.removeEventListener('change', onChange);
    },
    [query],
  );
  const getSnapshot = useCallback(() => window.matchMedia(query).matches, [query]);

  return useSyncExternalStore(subscribe, getSnapshot);
}
