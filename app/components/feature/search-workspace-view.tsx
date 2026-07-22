'use client';

/**
 * Search composition rendered inside the shared article workspace.
 */

import { AnnouncementsDialog } from '@/components/announcements-dialog';
import { ActiveFilterChips } from '@/components/feature/active-filter-chips';
import { ResultsList } from '@/components/feature/results-list';
import { SearchBar } from '@/components/feature/search-bar';
import { Sidebar } from '@/components/feature/sidebar';
import { WorkspaceShell } from '@/components/feature/workspace-shell';

/**
 * Render the article search controls, filters, announcements, and result list.
 *
 * @returns Search view inside the shared workspace shell.
 */
export function SearchWorkspaceView() {
  return (
    <WorkspaceShell
      sidebar={<Sidebar />}
      sidebarOpenLabel="打开筛选器"
      sidebarDialogTitle="筛选器"
      sidebarDialogDescription="选择数据库、领域、期刊和发表时间筛选文章。"
      toolbar={<SearchBar className="min-w-0 flex-1 md:mx-auto md:max-w-4xl" />}
    >
      <AnnouncementsDialog />
      <ResultsList filterSummary={<ActiveFilterChips />} />
    </WorkspaceShell>
  );
}
