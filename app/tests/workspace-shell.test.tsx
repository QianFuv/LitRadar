/**
 * Shared article-workspace shell coverage.
 */

import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, test } from 'vitest';

import { WorkspaceShell } from '@/components/feature/workspace-shell';

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

describe('WorkspaceShell', () => {
  test('renders stable workspace regions and safe-area scrolling', rendersWorkspaceRegions);
  test('opens an accessible mobile sidebar and restores focus', opensAccessibleSidebarDialog);
});
