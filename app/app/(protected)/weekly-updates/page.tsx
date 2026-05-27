import { Suspense } from 'react';
import { WeeklyWorkspace } from '@/components/desktop/weekly-workspace';

/**
 * Render the weekly updates route.
 *
 * @returns Weekly updates page.
 */
export default function WeeklyUpdatesPage() {
  return (
    <Suspense fallback={<div className="desktop-loading">正在打开每周更新...</div>}>
      <WeeklyWorkspace />
    </Suspense>
  );
}
