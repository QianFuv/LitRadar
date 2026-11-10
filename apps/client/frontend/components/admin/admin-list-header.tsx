/**
 * Toolbar placed above administrator lists.
 */

import type { ReactNode } from 'react';
import * as stylex from '@stylexjs/stylex';
import { colors, lineHeights } from '@/lib/theme.stylex';

/**
 * Render a list summary on the leading edge and optional actions on the trailing edge.
 *
 * @param props - Summary text and optional toolbar actions.
 * @returns Administrator list toolbar.
 */
export function AdminListHeader({
  children,
  summary,
}: {
  children?: ReactNode;
  summary: ReactNode;
}) {
  return (
    <div {...stylex.props(styles.adminListHeaderDiv)}>
      <p {...stylex.props(styles.adminListHeaderP)}>{summary}</p>
      {children && <div {...stylex.props(styles.adminListHeaderDiv2)}>{children}</div>}
    </div>
  );
}

const styles = stylex.create({
  adminListHeaderDiv: {
    display: 'flex',
    flexWrap: 'wrap',
    alignItems: 'center',
    justifyContent: 'space-between',
    gap: '0.75rem',
  },
  adminListHeaderP: {
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    color: colors.mutedForeground,
    fontVariantNumeric: 'tabular-nums',
  },
  adminListHeaderDiv2: {
    display: 'flex',
    flexWrap: 'wrap',
    alignItems: 'center',
    gap: '0.5rem',
  },
});
