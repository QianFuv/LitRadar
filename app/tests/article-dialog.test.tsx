/**
 * Article card selection, dialog accessibility, copy actions, and safe-link coverage.
 */

import { fireEvent, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { http, HttpResponse } from 'msw';
import { describe, expect, test, vi } from 'vitest';

import { ArticleDialogCard } from '@/components/feature/article-dialog-card';
import type { Article } from '@/lib/api';
import { AuthProvider, useAuth } from '@/lib/auth-context';
import { server } from '@/tests/mocks/server';
import { renderWithQuery } from '@/tests/render';

const navigationMocks = vi.hoisted(() => ({
  pathname: '/',
  searchParams: new URLSearchParams('view=favorites&folder=4'),
}));

vi.mock('next/navigation', () => ({
  usePathname: () => navigationMocks.pathname,
  useSearchParams: () => navigationMocks.searchParams,
}));

/**
 * Prevent jsdom from attempting document navigation after React handlers finish.
 *
 * @param event - Bubbling browser click event.
 */
function preventDocumentNavigation(event: MouseEvent): void {
  if (event.target instanceof HTMLAnchorElement) {
    event.preventDefault();
  }
}

const SAFE_ARTICLE: Article = {
  article_id: 'article-1',
  journal_id: 'journal-1',
  title: 'Selectable title',
  abstract: 'Selectable abstract text',
  authors: ['Ada Lovelace'],
  journal_title: 'Journal of Tests',
  date: '2024-05-17',
  doi: '10.1000/example',
};

/**
 * Return an authenticated test user.
 *
 * @returns Current-user response.
 */
function currentUserResponse(): Response {
  return HttpResponse.json({ id: 21, username: 'article_user', is_admin: false });
}

/**
 * Return all online article actions without exposing an upstream destination.
 *
 * @returns Article access response.
 */
function articleAccessResponse(): Response {
  return HttpResponse.json({
    abstract_page: {
      available: true,
      label: '查看摘要页',
      requires_login: false,
      message: null,
    },
    fulltext: {
      available: true,
      label: '获取全文',
      requires_login: false,
      message: null,
    },
  });
}

/**
 * Return article access that requires the user to configure CNKI.
 *
 * @returns Login-required article access response.
 */
function articleLoginRequiredResponse(): Response {
  return HttpResponse.json({
    abstract_page: {
      available: true,
      label: '查看摘要页',
      requires_login: false,
      message: null,
    },
    fulltext: {
      available: false,
      label: '获取全文',
      requires_login: true,
      message: '需要登录',
    },
  });
}

/**
 * Register API handlers required while the real article dialog is mounted.
 */
function registerArticleDialogHandlers(): void {
  server.use(
    http.get('http://localhost/api/auth/me', currentUserResponse),
    http.get('http://localhost/api/articles/:articleId/access', articleAccessResponse),
  );
}

/**
 * Render one production article card inside authentication and query providers.
 *
 * @param article - Article fixture to render.
 */
async function renderArticleCard(article: Article): Promise<void> {
  renderWithQuery(
    <AuthProvider>
      <AuthenticatedArticleFixture article={article} />
    </AuthProvider>,
  );
  await screen.findByRole('button', { name: /^查看文章详情：/ });
}

/** Mount the private article fixture only after server authentication is resolved. */
function AuthenticatedArticleFixture({ article }: { article: Article }) {
  const { user, loading } = useAuth();
  return loading || !user ? null : <ArticleDialogCard article={article} dbName="fixture.sqlite" />;
}

/**
 * Verify the whole card opens with pointer or keyboard input and regains focus on close.
 */
async function opensAndClosesAccessibleDialog(): Promise<void> {
  registerArticleDialogHandlers();
  const user = userEvent.setup();
  await renderArticleCard(SAFE_ARTICLE);

  expect(screen.getByText('Selectable title').closest('button')).toBeNull();
  expect(screen.getByText('Selectable abstract text').closest('button')).toBeNull();
  const card = screen.getByRole('button', { name: '查看文章详情：Selectable title' });
  expect(card).toHaveAttribute('tabindex', '0');
  expect(card.querySelector('[data-slot="card-footer"]')).toBeNull();
  expect(screen.queryByText('查看详情')).not.toBeInTheDocument();

  await user.click(screen.getByText('Selectable abstract text'));
  expect(await screen.findByRole('dialog')).toBeInTheDocument();
  await user.click(screen.getByRole('button', { name: '关闭' }));
  await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
  expect(card).toHaveFocus();

  for (const key of ['{Enter}', ' ']) {
    await user.keyboard(key);
    expect(await screen.findByRole('dialog')).toBeInTheDocument();
    await user.keyboard('{Escape}');
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    expect(card).toHaveFocus();
  }
}

/** Verify text selection and a sibling selection checkbox do not open article details. */
async function keepsSelectionSeparateFromOpening(): Promise<void> {
  registerArticleDialogHandlers();
  const user = userEvent.setup();
  renderWithQuery(
    <AuthProvider>
      <ArticleDialogCard
        article={SAFE_ARTICLE}
        dbName="fixture.sqlite"
        leading={<input type="checkbox" aria-label="选择文章" />}
      />
    </AuthProvider>,
  );

  const selection = window.getSelection();
  const range = document.createRange();
  range.selectNodeContents(screen.getByText('Selectable title'));
  selection?.addRange(range);
  try {
    fireEvent.click(screen.getByText('Selectable title'), { detail: 1 });
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  } finally {
    selection?.removeAllRanges();
  }
  const checkbox = screen.getByRole('checkbox', { name: '选择文章' });
  await user.click(checkbox);
  expect(checkbox).toBeChecked();
  expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
}

/**
 * Verify surviving copy actions and all online actions use stable LitRadar routes.
 */
async function copiesArticleValuesAndUsesStableActionRoutes(): Promise<void> {
  registerArticleDialogHandlers();
  const user = userEvent.setup();
  const writeText = vi.fn().mockResolvedValue(undefined);
  Object.defineProperty(navigator, 'clipboard', {
    configurable: true,
    value: { writeText },
  });
  await renderArticleCard(SAFE_ARTICLE);

  await user.click(screen.getByRole('button', { name: /^查看文章详情：/ }));
  expect(await screen.findByRole('dialog')).toBeInTheDocument();

  expect(screen.queryByRole('link', { name: '查看详情' })).not.toBeInTheDocument();
  const abstractLink = await screen.findByRole('link', { name: '查看摘要页' });
  expect(abstractLink).toHaveAttribute(
    'href',
    'http://localhost/api/articles/article-1/abstract?db=fixture.sqlite',
  );
  const fulltextLink = screen.getByRole('link', { name: '获取全文' });
  expect(fulltextLink).toHaveAttribute(
    'href',
    'http://localhost/api/articles/article-1/fulltext?db=fixture.sqlite',
  );
  for (const link of [abstractLink, fulltextLink]) {
    expect(link).toHaveAttribute('target', '_blank');
    expect(link).toHaveAttribute('rel', 'noreferrer');
  }

  expect(screen.queryByText('引用与链接')).not.toBeInTheDocument();
  expect(screen.queryByRole('button', { name: '复制 GB/T 7714' })).not.toBeInTheDocument();
  expect(screen.queryByRole('button', { name: '复制 BibTeX' })).not.toBeInTheDocument();
  expect(screen.queryByRole('button', { name: '复制 DOI' })).not.toBeInTheDocument();

  await user.click(screen.getByRole('button', { name: '复制文章标题' }));
  expect(writeText).toHaveBeenLastCalledWith('Selectable title');
  expect(
    screen
      .getByRole('button', { name: '复制文章标题' })
      .querySelector('[data-copy-state="copied"]'),
  ).not.toBeNull();
  expect(screen.queryByText('文章标题已复制。')).not.toBeInTheDocument();
  expect(screen.queryByRole('status')).not.toBeInTheDocument();

  await user.click(screen.getByRole('button', { name: '复制信息' }));
  expect(writeText).toHaveBeenLastCalledWith(
    [
      '标题：Selectable title',
      '作者：Ada Lovelace',
      '期刊：Journal of Tests',
      '日期：2024-05-17',
      'DOI: 10.1000/example',
      'DOI 链接：https://doi.org/10.1000/example',
    ].join('\n'),
  );
  expect(screen.getByRole('button', { name: '已复制' })).toBeInTheDocument();
  expect(
    screen.getByRole('button', { name: '已复制' }).querySelector('[data-copy-state="copied"]'),
  ).not.toBeNull();
  expect(document.querySelector('[data-article-access-state="ready"]')).not.toBeNull();
  expect(document.querySelector('[data-article-favorite-state="ready"]')).not.toBeNull();
  expect(screen.queryByText('文章信息已复制。')).not.toBeInTheDocument();
  expect(screen.queryByRole('status')).not.toBeInTheDocument();
}

/**
 * Verify stored metadata is never exposed as a direct external-link action.
 */
async function doesNotExposeStoredOrDirectExternalLinks(): Promise<void> {
  registerArticleDialogHandlers();
  const user = userEvent.setup();
  await renderArticleCard({
    ...SAFE_ARTICLE,
    article_id: 'unsafe-article',
    doi: 'javascript:alert(1)',
  });

  await user.click(screen.getByRole('button', { name: /^查看文章详情：/ }));
  expect(await screen.findByRole('dialog')).toBeInTheDocument();
  expect(screen.queryByRole('button', { name: '复制 DOI' })).not.toBeInTheDocument();
  expect(screen.queryByRole('link', { name: '打开 DOI' })).not.toBeInTheDocument();
  expect(screen.queryByRole('link', { name: '打开永久链接' })).not.toBeInTheDocument();
  expect(screen.queryByRole('button', { name: '复制永久链接' })).not.toBeInTheDocument();
}

/** Verify clipboard rejection remains visible as an accessible error. */
async function reportsCopyFailure(): Promise<void> {
  registerArticleDialogHandlers();
  const user = userEvent.setup();
  Object.defineProperty(navigator, 'clipboard', {
    configurable: true,
    value: { writeText: vi.fn().mockRejectedValue(new Error('clipboard denied')) },
  });
  await renderArticleCard(SAFE_ARTICLE);

  await user.click(screen.getByRole('button', { name: /^查看文章详情：/ }));
  await user.click(screen.getByRole('button', { name: '复制文章标题' }));

  expect(await screen.findByRole('alert')).toHaveTextContent('复制失败，请手动选择文本复制。');
  expect(screen.queryByRole('status')).not.toBeInTheDocument();
}

/** Verify the CNKI setup action preserves route state and closes article details first. */
async function opensDataSourceSettingsWithoutDialogStacking(): Promise<void> {
  registerArticleDialogHandlers();
  server.use(
    http.get('http://localhost/api/articles/:articleId/access', articleLoginRequiredResponse),
  );
  const user = userEvent.setup();
  await renderArticleCard(SAFE_ARTICLE);

  await user.click(screen.getByRole('button', { name: /^查看文章详情：/ }));
  const settingsLink = await screen.findByRole('link', { name: '去设置登录' });
  expect(settingsLink).toHaveAttribute('href', '/?view=favorites&folder=4&settings=data-sources');
  window.addEventListener('click', preventDocumentNavigation, { once: true });

  await user.click(settingsLink);
  await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
}

/**
 * Verify a failed access lookup is visible and a later dialog mount recovers through refetch.
 */
async function recoversArticleAccessAfterReopening(): Promise<void> {
  registerArticleDialogHandlers();
  let requestCount = 0;
  server.use(
    http.get('http://localhost/api/articles/:articleId/access', () => {
      requestCount += 1;
      if (requestCount === 1) {
        return HttpResponse.json({ detail: 'temporary access failure' }, { status: 503 });
      }
      return articleAccessResponse();
    }),
  );
  const user = userEvent.setup();
  await renderArticleCard(SAFE_ARTICLE);

  await user.click(screen.getByRole('button', { name: /^查看文章详情：/ }));
  const failedAccess = await screen.findByRole('button', { name: '访问状态失败' });
  expect(failedAccess).toHaveAttribute('title', 'temporary access failure');
  expect(document.querySelector('[data-article-access-state="error"]')).not.toBeNull();
  expect(screen.queryByRole('link', { name: '获取全文' })).not.toBeInTheDocument();

  await user.click(screen.getByRole('button', { name: '关闭' }));
  await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
  await user.click(screen.getByRole('button', { name: /^查看文章详情：/ }));

  expect(await screen.findByRole('link', { name: '获取全文' })).toBeInTheDocument();
  expect(requestCount).toBe(2);
}

describe('article dialog workflow', () => {
  test(
    'keeps text and checkbox selection separate from opening details',
    keepsSelectionSeparateFromOpening,
  );
  test(
    'keeps card text selectable and supports named open and close controls',
    opensAndClosesAccessibleDialog,
  );
  test(
    'copies article values and uses stable online action routes',
    copiesArticleValuesAndUsesStableActionRoutes,
  );
  test('does not expose stored or direct external links', doesNotExposeStoredOrDirectExternalLinks);
  test('reports clipboard rejection as an accessible error', reportsCopyFailure);
  test(
    'opens data-source settings without stacking dialogs',
    opensDataSourceSettingsWithoutDialogStacking,
  );
  test('recovers article access after reopening', recoversArticleAccessAfterReopening);
});
