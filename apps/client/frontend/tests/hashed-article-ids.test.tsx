/**
 * Hashed 63-bit content identifiers exceed `Number.MAX_SAFE_INTEGER`; every request must carry
 * them as the exact decimal strings the server sent.
 */

import { screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { http, HttpResponse, type JsonBodyType } from 'msw';
import { beforeEach, describe, expect, test, vi } from 'vite-plus/test';

import { ArticleDialogCard } from '@/components/feature/article-dialog-card';
import type { Article } from '@/lib/api';
import { AuthProvider, useAuth } from '@/lib/auth-context';
import { createAuthUserScenario } from '@/tests/mocks/scenarios';
import { server } from '@/tests/mocks/server';
import { renderWithQuery } from '@/tests/render';

vi.mock('@/lib/use-in-view', () => ({
  useInView: () => ({ ref: () => undefined, isInView: true }),
}));

/** Article id above 2^53, where `Number` rounds to a different integer. */
const LARGE_ARTICLE_ID = '9223361829786370983';
const LARGE_RELATED_ARTICLE_ID = '9007199254740993';
const LARGE_JOURNAL_ID = '9223372036854775783';
const LARGE_ISSUE_ID = '18446744073709551557';

const SOURCE: Article = {
  article_id: LARGE_ARTICLE_ID,
  journal_id: LARGE_JOURNAL_ID,
  issue_id: LARGE_ISSUE_ID,
  title: 'Hashed identifier article',
  abstract: 'An article whose identifier needs all 63 bits.',
  authors: ['Ada Lovelace'],
  journal_title: 'Journal of Large Identifiers',
  date: '2024-05-17',
};

const RELATED: Article = {
  article_id: LARGE_RELATED_ARTICLE_ID,
  journal_id: LARGE_JOURNAL_ID,
  issue_id: null,
  title: 'Related hashed article',
  abstract: '',
  authors: ['Someone Else'],
  journal_title: 'Journal of Large Identifiers',
  date: '2024-01-01',
};

/** Mount the article card once the server session is known. */
function AuthenticatedCard() {
  const { user, loading } = useAuth();
  return loading || !user ? null : <ArticleDialogCard article={SOURCE} dbName="fixture.sqlite" />;
}

/**
 * Answer like the server: an unknown identifier is a 404, as a rounded one would be.
 *
 * @param articleId - Path identifier received by the handler.
 * @param body - Response for the known article.
 * @returns Mock response.
 */
function respondForKnownArticle(articleId: unknown, body: JsonBodyType) {
  return articleId === LARGE_ARTICLE_ID
    ? HttpResponse.json(body)
    : HttpResponse.json({ detail: 'Article not found' }, { status: 404 });
}

/** Verify detail, related articles, access state, and journal lookups keep the exact ids. */
async function keepsHashedIdsInRequests(): Promise<void> {
  const requestUrls: URL[] = [];
  server.events.on('request:start', ({ request }) => {
    requestUrls.push(new URL(request.url));
  });
  server.use(
    http.get('http://localhost/api/auth/me', () =>
      HttpResponse.json(createAuthUserScenario({ id: 21, username: 'reader', is_admin: false })),
    ),
    http.get('http://localhost/api/articles/:articleId/access', ({ params }) =>
      respondForKnownArticle(params.articleId, {
        abstract_page: {
          available: true,
          label: '查看摘要页',
          requires_login: false,
          message: null,
        },
        fulltext: { available: true, label: '获取全文', requires_login: false, message: null },
      }),
    ),
    http.get('http://localhost/api/articles/:articleId/related', ({ params }) =>
      respondForKnownArticle(params.articleId, [RELATED]),
    ),
  );
  const user = userEvent.setup();
  await renderWithQuery(
    <AuthProvider>
      <AuthenticatedCard />
    </AuthProvider>,
  );

  await user.click(await screen.findByRole('button', { name: /^查看文章详情：/ }));
  const dialog = await screen.findByRole('dialog');
  expect(
    await within(dialog).findByRole('heading', { name: /Hashed identifier article/ }),
  ).toBeInTheDocument();
  const related = await within(dialog).findByRole('region', { name: '相关文章' });
  expect(await within(related).findByText('Related hashed article')).toBeInTheDocument();
  expect(await within(dialog).findByRole('link', { name: /获取全文/ })).toBeInTheDocument();
  expect(dialog).not.toHaveTextContent('访问状态失败');

  const articlePaths = requestUrls
    .map((url) => url.pathname)
    .filter((pathname) => pathname.startsWith('/api/articles/'));
  expect(articlePaths).toEqual(
    expect.arrayContaining([
      `/api/articles/${LARGE_ARTICLE_ID}/access`,
      `/api/articles/${LARGE_ARTICLE_ID}/related`,
    ]),
  );
  expect(articlePaths.every((pathname) => pathname.includes(`/${LARGE_ARTICLE_ID}/`))).toBe(true);
  expect(requestUrls.map((url) => url.pathname)).toContain(`/api/journals/${LARGE_JOURNAL_ID}`);
}

beforeEach(() => {
  Object.defineProperty(HTMLElement.prototype, 'scrollIntoView', {
    configurable: true,
    value: () => undefined,
  });
});

describe('hashed content identifiers', () => {
  test('keeps 63-bit article ids exact in every request', keepsHashedIdsInRequests, 15000);
});
