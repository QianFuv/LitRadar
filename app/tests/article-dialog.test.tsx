/**
 * Article card selection, dialog accessibility, citation, and safe-link coverage.
 */

import { screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { http, HttpResponse } from 'msw';
import { describe, expect, test, vi } from 'vitest';

import { ArticleDialogCard } from '@/components/feature/article-dialog-card';
import type { Article } from '@/lib/api';
import { AuthProvider } from '@/lib/auth-context';
import { generateArticleCitation } from '@/lib/citation';
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
    detail: {
      available: true,
      label: '查看详情',
      requires_login: false,
      message: null,
    },
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
    detail: {
      available: true,
      label: '查看详情',
      requires_login: false,
      message: null,
    },
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
function renderArticleCard(article: Article): void {
  renderWithQuery(
    <AuthProvider>
      <ArticleDialogCard article={article} dbName="fixture.sqlite" />
    </AuthProvider>,
  );
}

/**
 * Verify card text remains selectable and the named trigger opens an accessible dialog.
 */
async function opensAndClosesAccessibleDialog(): Promise<void> {
  registerArticleDialogHandlers();
  const user = userEvent.setup();
  renderArticleCard(SAFE_ARTICLE);

  expect(screen.getByText('Selectable title').closest('button')).toBeNull();
  expect(screen.getByText('Selectable abstract text').closest('button')).toBeNull();

  await user.click(screen.getByRole('button', { name: '查看详情' }));
  expect(await screen.findByRole('dialog')).toBeInTheDocument();
  await user.click(screen.getByRole('button', { name: '关闭' }));
  await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());

  await user.click(screen.getByRole('button', { name: '查看详情' }));
  expect(await screen.findByRole('dialog')).toBeInTheDocument();
  await user.keyboard('{Escape}');
  await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
}

/**
 * Verify citations copy and all online actions use stable LitRadar routes.
 */
async function copiesCitationsAndUsesStableActionRoutes(): Promise<void> {
  registerArticleDialogHandlers();
  const user = userEvent.setup();
  const writeText = vi.fn().mockResolvedValue(undefined);
  Object.defineProperty(navigator, 'clipboard', {
    configurable: true,
    value: { writeText },
  });
  renderArticleCard(SAFE_ARTICLE);

  await user.click(screen.getByRole('button', { name: '查看详情' }));
  expect(await screen.findByRole('dialog')).toBeInTheDocument();

  const detailLink = await screen.findByRole('link', { name: '查看详情' });
  expect(detailLink).toHaveAttribute(
    'href',
    'http://localhost/api/articles/article-1/detail?db=fixture.sqlite',
  );
  const abstractLink = screen.getByRole('link', { name: '查看摘要页' });
  expect(abstractLink).toHaveAttribute(
    'href',
    'http://localhost/api/articles/article-1/abstract?db=fixture.sqlite',
  );
  const fulltextLink = screen.getByRole('link', { name: '获取全文' });
  expect(fulltextLink).toHaveAttribute(
    'href',
    'http://localhost/api/articles/article-1/fulltext?db=fixture.sqlite',
  );
  for (const link of [detailLink, abstractLink, fulltextLink]) {
    expect(link).toHaveAttribute('target', '_blank');
    expect(link).toHaveAttribute('rel', 'noreferrer');
  }

  await user.click(screen.getByRole('button', { name: '复制 GB/T 7714' }));
  expect(writeText).toHaveBeenLastCalledWith(generateArticleCitation(SAFE_ARTICLE, 'gb-t-7714'));
  await user.click(screen.getByRole('button', { name: '复制 BibTeX' }));
  expect(writeText).toHaveBeenLastCalledWith(generateArticleCitation(SAFE_ARTICLE, 'bibtex'));
  await user.click(screen.getByRole('button', { name: '复制 DOI' }));
  expect(writeText).toHaveBeenLastCalledWith('10.1000/example');
}

/**
 * Verify a DOI remains copyable metadata and is never used as a direct action destination.
 */
async function doesNotExposeStoredOrDirectExternalLinks(): Promise<void> {
  registerArticleDialogHandlers();
  const user = userEvent.setup();
  renderArticleCard({
    ...SAFE_ARTICLE,
    article_id: 'unsafe-article',
    doi: 'javascript:alert(1)',
  });

  await user.click(screen.getByRole('button', { name: '查看详情' }));
  expect(await screen.findByRole('dialog')).toBeInTheDocument();
  expect(screen.getByRole('button', { name: '复制 DOI' })).toBeInTheDocument();
  expect(screen.queryByRole('link', { name: '打开 DOI' })).not.toBeInTheDocument();
  expect(screen.queryByRole('link', { name: '打开永久链接' })).not.toBeInTheDocument();
  expect(screen.queryByRole('button', { name: '复制永久链接' })).not.toBeInTheDocument();
}

/** Verify the CNKI setup action preserves route state and closes article details first. */
async function opensDataSourceSettingsWithoutDialogStacking(): Promise<void> {
  registerArticleDialogHandlers();
  server.use(
    http.get('http://localhost/api/articles/:articleId/access', articleLoginRequiredResponse),
  );
  const user = userEvent.setup();
  renderArticleCard(SAFE_ARTICLE);

  await user.click(screen.getByRole('button', { name: '查看详情' }));
  const settingsLink = await screen.findByRole('link', { name: '去设置登录' });
  expect(settingsLink).toHaveAttribute('href', '/?view=favorites&folder=4&settings=data-sources');
  window.addEventListener('click', preventDocumentNavigation, { once: true });

  await user.click(settingsLink);
  await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
}

describe('article dialog workflow', () => {
  test(
    'keeps card text selectable and supports named open and close controls',
    opensAndClosesAccessibleDialog,
  );
  test(
    'copies citations and uses stable online action routes',
    copiesCitationsAndUsesStableActionRoutes,
  );
  test('does not expose stored or direct external links', doesNotExposeStoredOrDirectExternalLinks);
  test(
    'opens data-source settings without stacking dialogs',
    opensDataSourceSettingsWithoutDialogStacking,
  );
});
