/**
 * Full-page loading state shared by session restoration and route code loading.
 */

import { LoadingText } from '@/components/ui/loading-text';

/**
 * Render the full-page loading state shared by session restoration and route code loading.
 *
 * @returns Accessible full-page loading state.
 */
export function RouteLoadingFallback() {
  return (
    <main id="main-content" className="flex h-dvh items-center justify-center">
      <div role="status" className="text-muted-foreground">
        <LoadingText text="加载中…" />
      </div>
    </main>
  );
}
