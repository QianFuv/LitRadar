/**
 * Toolbar placed above administrator lists.
 */

import type { ReactNode } from 'react';

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
    <div className="flex flex-wrap items-center justify-between gap-3">
      <p className="text-xs text-muted-foreground tabular-nums">{summary}</p>
      {children && <div className="flex flex-wrap items-center gap-2">{children}</div>}
    </div>
  );
}
