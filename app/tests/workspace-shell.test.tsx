/**
 * Shared article-workspace shell coverage.
 */

import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { NuqsTestingAdapter } from 'nuqs/adapters/testing';
import { describe, expect, test, vi } from 'vitest';

import Home from '@/app/(protected)/page';
import { WorkspaceShell } from '@/components/feature/workspace-shell';

vi.mock('@/lib/auth-context', () => ({
  useAuth: () => ({ user: { id: 21, username: 'workspace_user', is_admin: false } }),
}));

vi.mock('@/components/feature/search-workspace-view', () => ({
  SearchWorkspaceView: () => <section>检索工作区</section>,
}));

vi.mock('@/components/favorites/favorites-page-content', () => ({
  FavoritesPageContent: ({ userId }: { userId: number }) => (
    <section>收藏工作区用户 {userId}</section>
  ),
}));

vi.mock('@/components/weekly/weekly-updates-view', () => ({
  WeeklyUpdatesView: () => <section>每周更新工作区</section>,
}));

/**
 * Render a deterministic workspace with visible slot labels.
 *
 * @returns Workspace sidebar trigger.
 */
function renderWorkspaceShell(): HTMLElement {
  render(
    <WorkspaceShell
      sidebar={
        <aside aria-label="测试侧栏">
          <button type="button">侧栏操作</button>
        </aside>
      }
      sidebarOpenLabel="打开测试侧栏"
      sidebarDialogTitle="测试侧栏"
      sidebarDialogDescription="测试移动侧栏内容。"
      toolbar={<div>工作区工具栏</div>}
    >
      <article>工作区文章</article>
    </WorkspaceShell>,
  );

  return screen.getByRole('button', { name: '打开测试侧栏' });
}

/**
 * Verify stable landmarks, slots, scrolling, and safe-area spacing.
 */
function rendersWorkspaceRegions(): void {
  renderWorkspaceShell();

  const main = screen.getByRole('main');
  const scrollContainer = document.getElementById('results-scroll-container');

  expect(main).toHaveAttribute('id', 'main-content');
  expect(screen.getByRole('complementary', { name: '测试侧栏' })).toBeInTheDocument();
  expect(screen.getByText('工作区工具栏')).toBeInTheDocument();
  expect(screen.getByText('工作区文章')).toBeInTheDocument();
  expect(scrollContainer).toHaveClass('overflow-y-auto');
  expect(scrollContainer?.style.paddingBottom).toContain('var(--safe-area-inset-bottom');
}

/**
 * Verify the mobile sidebar has an accessible name and restores trigger focus after closing.
 */
async function opensAccessibleSidebarDialog(): Promise<void> {
  const user = userEvent.setup();
  const trigger = renderWorkspaceShell();

  await user.click(trigger);
  const dialog = screen.getByRole('dialog', { name: '测试侧栏' });
  expect(within(dialog).getByText('测试移动侧栏内容。')).toBeInTheDocument();
  expect(within(dialog).getByRole('button', { name: '侧栏操作' })).toBeInTheDocument();

  await user.click(within(dialog).getByRole('button', { name: '关闭' }));
  await waitFor(() => expect(screen.queryByRole('dialog', { name: '测试侧栏' })).toBeNull());
  expect(trigger).toHaveFocus();
}

/**
 * Render the protected root dispatcher with one query-string fixture.
 *
 * @param searchParams - Query string supplied to the nuqs adapter.
 */
function renderHome(searchParams = ''): void {
  render(
    <NuqsTestingAdapter searchParams={searchParams}>
      <Home />
    </NuqsTestingAdapter>,
  );
}

/**
 * Verify legal workspace views dispatch to their reusable root content.
 */
function dispatchesLegalWorkspaceViews(): void {
  const { unmount } = render(
    <NuqsTestingAdapter searchParams="?view=favorites">
      <Home />
    </NuqsTestingAdapter>,
  );
  expect(screen.getByText('收藏工作区用户 21')).toBeInTheDocument();

  unmount();
  renderHome('?view=weekly-updates');
  expect(screen.getByText('每周更新工作区')).toBeInTheDocument();
}

/**
 * Verify absent or unknown workspace values safely select search.
 */
function fallsBackToSearchWorkspace(): void {
  const { unmount } = render(
    <NuqsTestingAdapter>
      <Home />
    </NuqsTestingAdapter>,
  );
  expect(screen.getByText('检索工作区')).toBeInTheDocument();

  unmount();
  renderHome('?view=unknown');
  expect(screen.getByText('检索工作区')).toBeInTheDocument();
  expect(screen.queryByText(/收藏工作区|每周更新工作区/)).toBeNull();
}

describe('WorkspaceShell', () => {
  test('renders stable workspace regions and safe-area scrolling', rendersWorkspaceRegions);
  test('opens an accessible mobile sidebar and restores focus', opensAccessibleSidebarDialog);
  test('dispatches legal root workspace views', dispatchesLegalWorkspaceViews);
  test('falls back to search for absent or unknown views', fallsBackToSearchWorkspace);
});
