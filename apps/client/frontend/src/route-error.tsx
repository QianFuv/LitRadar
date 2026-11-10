/**
 * Route-level error fallback rendered inside the application providers.
 */

import { Link, type ErrorComponentProps } from '@tanstack/react-router';
import { useEffect } from 'react';

import { Button } from '@/components/ui/button';
import { RadarMark } from '@/components/feature/reader-placeholder';
import { reportClientError } from '@/lib/client-logger';
import * as stylex from '@stylexjs/stylex';
import { colors, lineHeights } from '@/lib/theme.stylex';

/**
 * Report a route failure safely and offer retry and home navigation.
 *
 * @param props - Captured error and boundary reset callback.
 * @returns Accessible route failure UI.
 */
export default function RouteError({ error, reset }: ErrorComponentProps) {
  useEffect(
    /**
     * Emit the route-boundary terminal event once for this error object.
     */
    function reportRouteError(): void {
      reportClientError('route_boundary', error);
    },
    [error],
  );

  return (
    <main id="main-content" {...stylex.props(styles.routeErrorMain)}>
      <RadarMark xstyle={styles.routeErrorRadarMark} />
      <div role="alert" {...stylex.props(styles.routeErrorDiv)}>
        <h1 {...stylex.props(styles.routeErrorH1)}>页面加载失败</h1>
        <p {...stylex.props(styles.routeErrorP)}>
          发生了意外错误。你可以重试，或返回首页继续使用。
        </p>
      </div>
      <div {...stylex.props(styles.routeErrorDiv2)}>
        <Button type="button" xstyle={styles.routeErrorButton} onClick={reset}>
          重试
        </Button>
        <Button variant="outline" xstyle={styles.routeErrorButton} asChild>
          <Link to="/">返回首页</Link>
        </Button>
      </div>
    </main>
  );
}

const styles = stylex.create({
  routeErrorMain: {
    display: 'flex',
    minHeight: '100dvh',
    flexDirection: 'column',
    alignItems: 'center',
    justifyContent: 'center',
    backgroundColor: colors.background,
    paddingInline: '1.5rem',
    paddingBlock: '4rem',
    textAlign: 'center',
  },
  routeErrorRadarMark: {
    width: '10rem',
    height: '10rem',
    color: colors.mutedForeground,
  },
  routeErrorDiv: {
    marginTop: '2rem',
  },
  routeErrorH1: {
    fontSize: '1.5rem',
    lineHeight: lineHeights.text2xl,
    fontWeight: 700,
    letterSpacing: '-0.025em',
  },
  routeErrorP: {
    marginTop: '0.5rem',
    maxWidth: '24rem',
    fontSize: '0.875rem',
    lineHeight: '1.5rem',
    color: colors.mutedForeground,
  },
  routeErrorDiv2: {
    marginTop: '2rem',
    display: 'flex',
    flexDirection: {
      default: 'column',
      '@media (min-width: 40rem)': 'row',
    },
    gap: '0.75rem',
  },
  routeErrorButton: {
    height: '2.5rem',
    borderRadius: '12px',
    paddingInline: {
      default: '1.25rem',
      ':has( > svg)': '0.75rem',
    },
  },
});
