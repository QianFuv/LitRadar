/**
 * Custom not-found route and document metadata.
 */

import { Link } from '@tanstack/react-router';

import { Button } from '@/components/ui/button';
import { RadarMark } from '@/components/feature/reader-placeholder';
import * as stylex from '@stylexjs/stylex';
import { colors, lineHeights } from '@/lib/theme.stylex';

/** Document title rendered by the not-found route. */
export const NOT_FOUND_PAGE_TITLE = '页面未找到 | LitRadar';

/** Document description rendered by the not-found route. */
export const NOT_FOUND_PAGE_DESCRIPTION = '所请求的 LitRadar 页面不存在。';

/**
 * Render the custom application not-found page.
 *
 * @returns Accessible not-found content with home navigation.
 */
export default function NotFound() {
  return (
    <main id="main-content" {...stylex.props(styles.notFoundMain)}>
      <title>{NOT_FOUND_PAGE_TITLE}</title>
      <meta name="description" content={NOT_FOUND_PAGE_DESCRIPTION} />
      <RadarMark xstyle={styles.notFoundRadarMark} />
      <p {...stylex.props(styles.notFoundP)}>404 · 信号丢失</p>
      <h1 {...stylex.props(styles.notFoundH1)}>页面未找到</h1>
      <p {...stylex.props(styles.notFoundP2)}>你访问的页面不存在，可能已被移动或删除。</p>
      <Button asChild xstyle={styles.notFoundButton}>
        <Link to="/">返回首页</Link>
      </Button>
    </main>
  );
}

const styles = stylex.create({
  notFoundMain: {
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
  notFoundRadarMark: {
    width: '10rem',
    height: '10rem',
    color: colors.mutedForeground,
  },
  notFoundP: {
    marginTop: '2rem',
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    fontWeight: 700,
    letterSpacing: '0.2em',
    color: colors.primaryText,
  },
  notFoundH1: {
    marginTop: '0.75rem',
    fontSize: '1.5rem',
    lineHeight: lineHeights.text2xl,
    fontWeight: 700,
    letterSpacing: '-0.025em',
  },
  notFoundP2: {
    marginTop: '0.5rem',
    maxWidth: '24rem',
    fontSize: '0.875rem',
    lineHeight: '1.5rem',
    color: colors.mutedForeground,
  },
  notFoundButton: {
    marginTop: '2rem',
    height: '2.5rem',
    borderRadius: '12px',
    paddingInline: {
      default: '1.25rem',
      ':has( > svg)': '0.75rem',
    },
  },
});
