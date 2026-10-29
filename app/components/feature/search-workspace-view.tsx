'use client';

/**
 * Search composition rendered inside the shared reading workspace.
 */

import { AnnouncementsDialog } from '@/components/announcements-dialog';
import { ReaderPlaceholder } from '@/components/feature/reader-placeholder';
import { ResultsList } from '@/components/feature/results-list';
import { SearchBar } from '@/components/feature/search-bar';
import { SearchFilters } from '@/components/feature/search-filters';
import { WorkspaceShell } from '@/components/feature/workspace-shell';

/**
 * Render the article search field, filter pills, announcements, results, and reading desk.
 *
 * @returns Search view inside the shared workspace shell.
 */
export function SearchWorkspaceView() {
  return (
    <WorkspaceShell
      listLabel="检索结果"
      emphasis="list"
      shouldAutoSelectFirst
      header={
        <div className="space-y-3">
          <SearchBar hasSyntaxAssist />
          <SearchFilters />
        </div>
      }
      readerPlaceholder={
        <ReaderPlaceholder
          title="选择一篇文献开始阅读"
          description="点击左侧列表中的文献，摘要、访问入口与收藏操作会显示在这里。"
        />
      }
    >
      <AnnouncementsDialog />
      <ResultsList />
    </WorkspaceShell>
  );
}
