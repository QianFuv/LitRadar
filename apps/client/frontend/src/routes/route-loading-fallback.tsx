/**
 * Full-page loading state shared by session restoration and route code loading.
 */

import { LoadingText } from '@/components/ui/loading-text';
import * as stylex from '@stylexjs/stylex';
import { colors } from '@/lib/theme.stylex';

/**
 * Render the full-page loading state shared by session restoration and route code loading.
 *
 * @returns Accessible full-page loading state.
 */
export function RouteLoadingFallback() {
  return (
    <main id="main-content" {...stylex.props(styles.routeLoadingFallbackMain)}>
      <div role="status" {...stylex.props(styles.routeLoadingFallbackDiv)}>
        <LoadingText text="加载中…" />
      </div>
    </main>
  );
}

const styles = stylex.create({
  routeLoadingFallbackMain: {
    display: 'flex',
    height: '100dvh',
    alignItems: 'center',
    justifyContent: 'center',
  },
  routeLoadingFallbackDiv: {
    color: colors.mutedForeground,
  },
});
