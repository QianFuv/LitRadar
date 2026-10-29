/**
 * Shared reading-workspace shell and application frame coverage.
 */

import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { NuqsTestingAdapter } from 'nuqs/adapters/testing';
import { createPortal } from 'react-dom';
import { useState } from 'react';
import { afterEach, describe, expect, test, vi } from 'vitest';

import Home from '@/app/(protected)/page';
import {
  READER_ITEM_ATTRIBUTE,
  WORKSPACE_SEARCH_ATTRIBUTE,
  useArticleReader,
} from '@/components/feature/article-reader-context';
import { WorkspaceShell } from '@/components/feature/workspace-shell';
import { AppFrame } from '@/components/shell/app-frame';
import { READER_DOCK_MEDIA_QUERY } from '@/lib/use-media-query';

const shellMocks = vi.hoisted(() => ({
  searchParams: new URLSearchParams('view=favorites&folder=4'),
}));

vi.mock('next/navigation', () => ({
  usePathname: () => '/',
  useRouter: () => ({ push: vi.fn(), replace: vi.fn(), refresh: vi.fn() }),
  useSearchParams: () => shellMocks.searchParams,
}));

vi.mock('@/lib/auth-context', () => ({
  useAuth: () => ({ user: { id: 21, username: 'workspace_user', is_admin: false } }),
}));

vi.mock('@/components/feature/user-menu', () => ({
  UserMenu: ({ placement }: { placement?: string }) => <div>账号菜单 {placement}</div>,
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

vi.mock('@/components/cfp/cfp-tracking-view', () => ({
  CfpTrackingView: () => <section>征稿追踪工作区</section>,
}));

/**
 * Make the reader-dock media query match for one test.
 */
function enableDockedReader(): void {
  Object.defineProperty(window, 'matchMedia', {
    configurable: true,
    writable: true,
    value: (query: string): MediaQueryList => ({
      matches: query === READER_DOCK_MEDIA_QUERY || query === '(prefers-reduced-motion: reduce)',
      media: query,
      onchange: null,
      addEventListener: () => undefined,
      addListener: () => undefined,
      dispatchEvent: () => false,
      removeEventListener: () => undefined,
      removeListener: () => undefined,
    }),
  });
}

const originalMatchMedia = window.matchMedia;

afterEach(() => {
  Object.defineProperty(window, 'matchMedia', {
    configurable: true,
    writable: true,
    value: originalMatchMedia,
  });
});

/**
 * Render one selectable row that portals its reader content into the docked desk.
 *
 * @param props - Row key and label.
 * @returns Selectable test row.
 */
function ReaderRow({ itemKey, label }: { itemKey: string; label: string }) {
  const reader = useArticleReader();
  const isSelected = reader?.selectedKey === itemKey;
  return (
    <>
      <button
        type="button"
        {...{ [READER_ITEM_ATTRIBUTE]: '' }}
        data-selected={isSelected ? 'true' : 'false'}
        onClick={() => reader?.select(itemKey)}
      >
        {label}
      </button>
      {isSelected &&
        reader?.readerNode &&
        createPortal(
          <article aria-label={`阅读 ${label}`}>{label} 正文</article>,
          reader.readerNode,
        )}
    </>
  );
}

/**
 * Render a deterministic workspace with visible slot labels.
 */
function renderArticleWorkspace(): void {
  render(
    <WorkspaceShell
      listLabel="测试列表"
      header={<input aria-label="测试搜索" {...{ [WORKSPACE_SEARCH_ATTRIBUTE]: '' }} />}
      readerPlaceholder={<p>请选择条目</p>}
    >
      <ReaderRow itemKey="first" label="第一篇" />
      <ReaderRow itemKey="second" label="第二篇" />
    </WorkspaceShell>,
  );
}

/**
 * Verify the list column, header slot, scroll container, and bottom spacing.
 */
function rendersWorkspaceRegions(): void {
  renderArticleWorkspace();

  const list = screen.getByRole('region', { name: '测试列表' });
  const scrollContainer = document.getElementById('results-scroll-container');

  expect(within(list).getByRole('textbox', { name: '测试搜索' })).toBeInTheDocument();
  expect(within(list).getByRole('button', { name: '第一篇' })).toBeInTheDocument();
  expect(scrollContainer).toHaveClass('overflow-y-auto', 'workspace-bottom-space');
  expect(screen.queryByRole('region', { name: '阅读台' })).toBeNull();
}

/**
 * Verify docked reading, keyboard row navigation, and the slash search shortcut.
 */
async function docksReaderAndSupportsShortcuts(): Promise<void> {
  enableDockedReader();
  const user = userEvent.setup();
  renderArticleWorkspace();

  const desk = screen.getByRole('region', { name: '阅读台' });
  expect(within(desk).getByText('请选择条目')).toBeInTheDocument();

  await user.click(screen.getByRole('button', { name: '第一篇' }));
  expect(within(desk).getByRole('article', { name: '阅读 第一篇' })).toBeInTheDocument();
  expect(within(desk).queryByText('请选择条目')).toBeNull();

  await user.keyboard('j');
  expect(screen.getByRole('button', { name: '第二篇' })).toHaveFocus();
  expect(within(desk).getByRole('article', { name: '阅读 第二篇' })).toBeInTheDocument();

  await user.keyboard('k');
  expect(screen.getByRole('button', { name: '第一篇' })).toHaveFocus();

  await user.keyboard('/');
  expect(screen.getByRole('textbox', { name: '测试搜索' })).toHaveFocus();
  await user.keyboard('j');
  expect(screen.getByRole('textbox', { name: '测试搜索' })).toHaveValue('j');
}

/**
 * Verify list-emphasized workspaces fill the side panel with the first item automatically.
 */
async function autoSelectsFirstItemInSidePanel(): Promise<void> {
  enableDockedReader();
  render(
    <WorkspaceShell
      listLabel="测试列表"
      header={<p>列表头部</p>}
      emphasis="list"
      shouldAutoSelectFirst
      readerPlaceholder={<p>请选择条目</p>}
    >
      <ReaderRow itemKey="first" label="第一篇" />
      <ReaderRow itemKey="second" label="第二篇" />
    </WorkspaceShell>,
  );

  const desk = screen.getByRole('region', { name: '阅读台' });
  expect(desk).toHaveAttribute('data-emphasis', 'list');
  expect(await within(desk).findByRole('article', { name: '阅读 第一篇' })).toBeInTheDocument();
  expect(within(desk).queryByText('请选择条目')).toBeNull();
}

/**
 * Render a view-owned reader that opens as a sheet on narrow screens.
 *
 * @returns Explicit reader workspace.
 */
function ExplicitReaderWorkspace() {
  const [isSheetOpen, setIsSheetOpen] = useState(false);
  return (
    <WorkspaceShell
      listLabel="期刊目录"
      header={<p>目录头部</p>}
      isListSelfScrolling
      reader={{
        content: <p>征稿正文</p>,
        description: '期刊征稿需求与时间安排',
        isSheetOpen,
        onSheetOpenChange: setIsSheetOpen,
        title: '测试期刊',
      }}
      readerPlaceholder={null}
    >
      <button type="button" onClick={() => setIsSheetOpen(true)}>
        选择测试期刊
      </button>
    </WorkspaceShell>
  );
}

/**
 * Verify explicit reader content opens as an accessible sheet and restores focus.
 */
async function opensExplicitReaderSheet(): Promise<void> {
  const user = userEvent.setup();
  render(<ExplicitReaderWorkspace />);

  expect(document.getElementById('results-scroll-container')).toBeNull();
  const trigger = screen.getByRole('button', { name: '选择测试期刊' });
  await user.click(trigger);
  const sheet = screen.getByRole('dialog', { name: '测试期刊' });
  expect(sheet).toHaveAttribute('data-motion-placement', 'right');
  expect(within(sheet).getByText('征稿正文')).toBeInTheDocument();

  await user.click(within(sheet).getByRole('button', { name: '关闭' }));
  await waitFor(() => expect(screen.queryByRole('dialog', { name: '测试期刊' })).toBeNull());
  expect(trigger).toHaveFocus();
}

/**
 * Verify the application frame exposes the main landmark, navigation, and shortcuts.
 */
function rendersApplicationFrame(): void {
  render(
    <AppFrame>
      <p>页面内容</p>
    </AppFrame>,
  );

  expect(screen.getByRole('main')).toHaveAttribute('id', 'main-content');
  expect(within(screen.getByRole('main')).getByText('页面内容')).toBeInTheDocument();
  const navigations = screen.getAllByRole('navigation', { name: '页面导航' });
  expect(navigations).toHaveLength(2);
  for (const navigation of navigations) {
    expect(within(navigation).getAllByRole('link')).toHaveLength(5);
    expect(within(navigation).getByRole('link', { name: '文献检索' })).toHaveAttribute('href', '/');
    expect(within(navigation).getByRole('link', { name: '每周更新' })).toHaveAttribute(
      'href',
      '/?view=weekly-updates',
    );
    expect(within(navigation).getByRole('link', { name: '征稿追踪' })).toHaveAttribute(
      'href',
      '/?view=cfp-tracking',
    );
    expect(within(navigation).getByRole('link', { name: '文献关系图' })).toHaveAttribute(
      'href',
      '/?view=relations',
    );
    for (const shortLabel of ['检索', '收藏', '周报', '征稿', '关系']) {
      expect(within(navigation).getByText(shortLabel)).toBeVisible();
    }
    expect(within(navigation).getByRole('link', { name: '我的收藏' })).toHaveAttribute(
      'href',
      '/?view=favorites',
    );
    expect(within(navigation).getByRole('link', { name: '我的收藏' })).toHaveAttribute(
      'aria-current',
      'page',
    );
    expect(within(navigation).getByRole('link', { name: '文献检索' })).not.toHaveAttribute(
      'aria-current',
    );
  }
  expect(screen.getByRole('link', { name: '设置中心' })).toHaveAttribute(
    'href',
    '/?view=favorites&folder=4&settings=general',
  );
  expect(screen.queryByRole('link', { name: '管理面板' })).toBeNull();
  expect(screen.getByText('账号菜单 rail')).toBeInTheDocument();
  expect(screen.getByText('账号菜单 tab')).toBeInTheDocument();
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
  test('renders the list column and scroll container', rendersWorkspaceRegions);
  test('docks the reader and supports keyboard shortcuts', docksReaderAndSupportsShortcuts);
  test(
    'auto-selects the first item in a list-emphasized side panel',
    autoSelectsFirstItemInSidePanel,
  );
  test('opens explicit reader content as a sheet and restores focus', opensExplicitReaderSheet);
  test('renders the application frame landmarks and navigation', rendersApplicationFrame);
  test('dispatches legal root workspace views', dispatchesLegalWorkspaceViews);
  test('dispatches the CFP workspace from its canonical URL', () => {
    renderHome('?view=cfp-tracking');
    expect(screen.getByText('征稿追踪工作区')).toBeInTheDocument();
  });
  test('falls back to search for absent or unknown views', fallsBackToSearchWorkspace);
});
