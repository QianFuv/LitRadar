/** Database navigation and unadapted-journal behavior for CFP tracking. */

import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { http, HttpResponse } from 'msw';
import { NuqsTestingAdapter } from 'nuqs/adapters/testing';
import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest';

import { CfpTrackingView } from '@/components/cfp/cfp-tracking-view';
import type { CfpJournalSummary, CfpNoticePage } from '@/lib/api';
import { CFP_FIXTURE_PAGES } from '@/tests/fixtures/cfp-pages';
import { SELECTED_DATABASE_KEY } from '@/lib/api/client';
import { server } from '@/tests/mocks/server';
import { renderWithQuery } from '@/tests/render';

vi.mock('@/lib/auth-context', () => ({
  useAuth: () => ({ user: { id: 31, username: 'cfp_user', is_admin: false } }),
}));

const DATABASES = [
  'ccf_computer_journals.sqlite',
  'chinese_journals.sqlite',
  'english_journals.sqlite',
];

/** Build an API journal summary, keeping unknown sources explicitly unadapted. */
function journal(catalogId: string, title: string): CfpJournalSummary {
  return (
    CFP_FIXTURE_PAGES[catalogId]?.journal ?? {
      catalogId,
      catalogAliases: [],
      title,
      allIssns: [],
      titleAliases: [],
      coverage: 'unadapted',
      noticeCount: 0,
      currentCount: 0,
      stateCounts: {},
      canRefresh: false,
      refreshStatus: 'unadapted',
    }
  );
}

const JOURNALS: Record<string, CfpJournalSummary[]> = {
  [DATABASES[0]]: [
    journal('issn-1949-3045', 'IEEE Transactions on Affective Computing'),
    journal('issn-unadapted', '未适配示例期刊'),
  ],
  [DATABASES[1]]: [journal('issn-1004-4833', '审计与经济研究')],
  [DATABASES[2]]: [journal('issn-0022-2429', 'Journal of Marketing')],
};

/** Return one complete lightweight catalog response. */
function catalog(items: CfpJournalSummary[], database: string) {
  return {
    database,
    evaluatedAt: 1789473600,
    items,
    summary: {
      journals: items.length,
      adaptedJournals: items.filter((item) => item.coverage === 'adapted').length,
      notices: items.reduce((count, item) => count + item.noticeCount, 0),
      currentNotices: items.reduce((count, item) => count + item.currentCount, 0),
    },
  };
}

/** Provide an explicitly server-filtered notice page for the request. */
function noticePage(catalogId: string, includeClosed: boolean): CfpNoticePage {
  const base = CFP_FIXTURE_PAGES[catalogId];
  const selected = base?.journal ?? journal(catalogId, '未适配示例期刊');
  const items = (base?.items ?? []).filter(
    (notice) => includeClosed || !['closed', 'historical'].includes(notice.state),
  );
  return {
    journal: selected,
    evaluatedAt: 1789473600,
    items,
    page: { total: items.length, limit: 50, offset: 0, next_cursor: null, has_more: false },
  };
}

/** Use a single metadata entry to exercise one source state. */
function mockSingleJournal(catalogId: string, title: string) {
  server.use(
    http.get('http://localhost/api/cfp/journals', ({ request }) =>
      HttpResponse.json(
        catalog([journal(catalogId, title)], new URL(request.url).searchParams.get('db')!),
      ),
    ),
  );
}

/** Render the CFP page under isolated URL and query state. */
function renderCfp(searchParams = '?view=cfp-tracking') {
  return renderWithQuery(
    <NuqsTestingAdapter searchParams={searchParams} hasMemory>
      <CfpTrackingView />
    </NuqsTestingAdapter>,
  );
}

beforeEach(() => {
  window.localStorage.clear();
  server.use(
    http.get('http://localhost/api/meta/databases', () => HttpResponse.json(DATABASES)),
    http.get('http://localhost/api/cfp/journals', ({ request }) => {
      const database = new URL(request.url).searchParams.get('db')!;
      return HttpResponse.json(catalog(JOURNALS[database] ?? [], database));
    }),
    http.get('http://localhost/api/cfp/journals/:catalogId/notices', ({ request, params }) =>
      HttpResponse.json(
        noticePage(
          String(params.catalogId),
          new URL(request.url).searchParams.get('include_closed') === 'true',
        ),
      ),
    ),
  );
});

afterEach(() => vi.useRealTimers());

describe('CFP tracking', () => {
  test('shows structured requirements for a maintained journal with no articles', async () => {
    renderCfp();
    expect(
      await screen.findByRole(
        'heading',
        { name: /When Affective Computing Meets Multimodal/ },
        { timeout: 3000 },
      ),
    ).toBeVisible();
    expect(screen.getByText('2026/12/30')).toBeVisible();
    expect(screen.getByText('论文截止')).toBeVisible();
    expect(screen.getByRole('region', { name: '投稿要求' })).toBeVisible();
    expect(screen.getByText(/Manuscripts should not be published/)).toBeVisible();
    expect(screen.getByRole('link', { name: '查看征稿原文' })).toHaveAttribute('target', '_blank');
  });

  test('renders English source excerpts when browsing the English database', async () => {
    renderCfp('?view=cfp-tracking&cfp_db=english_journals.sqlite');
    expect(
      await screen.findByRole('heading', {
        name: 'Call for Papers | Journal of Marketing: Special Issue on Organic Marketing Theory',
      }),
    ).toBeVisible();
    expect(screen.getByText(/What is material is that the theory/)).toBeVisible();
    expect(screen.getByText(/All submissions will go through Journal of Marketing/)).toBeVisible();
    expect(screen.queryByText('营销领域原创理论')).toBeNull();
  });

  test('leaves unavailable excerpts absent while keeping a confirmed call and its dates', async () => {
    mockSingleJournal('issn-0022-2380', 'Journal of Management Studies');
    renderCfp('?view=cfp-tracking&cfp_db=english_journals.sqlite');
    const heading = await screen.findByRole('heading', {
      name: 'Special Issue Call for Papers: The Dark Sides of Digital Communication',
    });
    const card = within(heading.closest('article')!);
    expect(heading).toBeVisible();
    expect(card.queryByRole('region', { name: '投稿范围' })).toBeNull();
    expect(card.queryByRole('region', { name: '投稿要求' })).toBeNull();
    expect(card.getByRole('link', { name: '查看征稿原文' })).toBeVisible();
  });

  test('shows invitation restrictions for newly adapted collection calls', async () => {
    mockSingleJournal('issn-0001-5903', 'Acta Informatica');
    renderCfp();
    const heading = await screen.findByRole('heading', { name: 'By Invite Only - RF70' });
    expect(within(heading.closest('article')!).getByText('仅限受邀投稿')).toBeVisible();
  });

  test('shows a verified empty source without implying that regular submissions are closed', async () => {
    mockSingleJournal('issn-1067-5027', 'Journal of the American Medical Informatics Association');
    renderCfp('?view=cfp-tracking&cfp_db=english_journals.sqlite');
    expect(await screen.findByRole('heading', { name: '暂无已收录的征稿公告' })).toBeVisible();
    expect(screen.getByText(/General submissions are still open/)).toBeVisible();
    expect(screen.queryByRole('heading', { name: '暂未适配' })).toBeNull();
    expect(screen.getByRole('link', { name: '查看期刊说明' })).toHaveAttribute('target', '_blank');
    expect(document.querySelectorAll('[data-slot="cfp-notice"]')).toHaveLength(0);
  });

  test('keeps archived calls hidden until the user includes historical results', async () => {
    const user = userEvent.setup();
    mockSingleJournal('issn-1004-3306', '保险研究');
    renderCfp('?view=cfp-tracking&cfp_db=chinese_journals.sqlite');
    expect(await screen.findByRole('heading', { name: '暂无已收录的进行中征稿' })).toBeVisible();
    await user.click(screen.getByRole('checkbox', { name: '显示已结束及历史' }));
    expect(
      await screen.findByRole('heading', { name: '“保险在普惠金融中的作用”选题讨论会征稿启事' }),
    ).toBeVisible();
    expect(screen.getByText('已结束')).toBeVisible();
  });

  test('retains conflicting source timelines for inspection without inventing a cutoff', async () => {
    const user = userEvent.setup();
    mockSingleJournal('issn-0024-6301', 'Long Range Planning');
    renderCfp('?view=cfp-tracking&cfp_db=english_journals.sqlite');
    expect(await screen.findByText('投稿时间待确认')).toBeVisible();
    expect(screen.queryByText('征稿中')).toBeNull();
    const summary = screen.getByText('查看原文时间说明');
    await user.click(summary);
    expect(summary.closest('details')).toHaveAttribute('open');
    expect(summary.closest('details')).toHaveTextContent('October 1st 2026');
    expect(summary.closest('details')).toHaveTextContent('November 1, 2026');
  });

  test('leaves an unadapted journal empty and does not show guessed calls or external links', async () => {
    const user = userEvent.setup();
    renderCfp();
    await user.click(await screen.findByRole('button', { name: /未适配示例期刊/ }));
    expect(await screen.findByRole('heading', { name: '暂未适配' })).toBeVisible();
    expect(document.querySelectorAll('[data-slot="cfp-notice"]')).toHaveLength(0);
    expect(screen.queryByRole('link', { name: '查看征稿原文' })).toBeNull();
  });

  test('uses database-scoped journal selection without changing article-search preferences', async () => {
    window.localStorage.setItem(SELECTED_DATABASE_KEY, 'article-search.sqlite');
    renderCfp('?view=cfp-tracking&cfp_db=chinese_journals.sqlite&cfp_journal=issn-1949-3045');
    expect(await screen.findByRole('heading', { name: '审计与经济研究' })).toBeVisible();
    expect(
      screen.queryByRole('heading', { name: /When Affective Computing Meets Multimodal/ }),
    ).toBeNull();
    expect(window.localStorage.getItem(SELECTED_DATABASE_KEY)).toBe('article-search.sqlite');
  });

  test('supports journal search and recovers invalid database and journal URL values', async () => {
    const user = userEvent.setup();
    renderCfp('?view=cfp-tracking&cfp_db=missing.sqlite&cfp_journal=missing');
    await screen.findByRole('heading', { name: /When Affective Computing Meets Multimodal/ });
    await user.type(screen.getByRole('textbox', { name: '搜索征稿期刊' }), '未适配');
    const sidebar = screen.getByRole('region', { name: '征稿期刊' });
    expect(within(sidebar).queryByRole('button', { name: /Affective/ })).toBeNull();
    expect(within(sidebar).getByRole('button', { name: /未适配示例期刊/ })).toBeVisible();
  });

  test('reports metadata errors separately from unsupported adaptation', async () => {
    server.use(
      http.get('http://localhost/api/cfp/journals', () =>
        HttpResponse.json({ detail: '目录读取失败' }, { status: 503 }),
      ),
    );
    renderCfp();
    expect(await screen.findByRole('heading', { name: '加载征稿期刊失败' })).toBeVisible();
    expect(screen.queryByRole('heading', { name: '暂未适配' })).toBeNull();
    expect(screen.getByRole('button', { name: '重试' })).toBeVisible();
  });

  test('renders server state even when the browser clock is wrong', async () => {
    vi.useFakeTimers({ toFake: ['Date'] });
    vi.setSystemTime(new Date('2100-01-01T12:00:00Z'));
    renderCfp();
    expect(await screen.findByText('征稿中')).toBeVisible();
  });

  test('loads more original notices and uses server totals instead of loaded item counts', async () => {
    const user = userEvent.setup();
    const base = noticePage('issn-1949-3045', false);
    const requests: (string | null)[] = [];
    server.use(
      http.get('http://localhost/api/cfp/journals/:catalogId/notices', ({ request }) => {
        const cursor = new URL(request.url).searchParams.get('cursor');
        requests.push(cursor);
        return HttpResponse.json({
          ...base,
          journal: { ...base.journal, currentCount: 17 },
          items: cursor
            ? [{ ...base.items[0], id: 'second', title: 'Second original notice' }]
            : base.items,
          page: {
            total: 17,
            limit: 1,
            offset: cursor ? 1 : 0,
            next_cursor: cursor ? null : 'next-page',
            has_more: !cursor,
          },
        });
      }),
    );
    renderCfp();
    expect(await screen.findByText('17 条已收录征稿')).toBeVisible();
    await user.click(screen.getByRole('button', { name: '加载更多征稿' }));
    expect(await screen.findByRole('heading', { name: 'Second original notice' })).toBeVisible();
    expect(document.querySelectorAll('[data-slot="cfp-notice"]')).toHaveLength(2);
    expect(requests).toEqual([null, 'next-page']);
  });

  test('discards stale cursor pages and reads the new first page after a 409', async () => {
    const user = userEvent.setup();
    const base = noticePage('issn-1949-3045', false);
    let hasChanged = false;
    const requests: (string | null)[] = [];
    server.use(
      http.get('http://localhost/api/cfp/journals/:catalogId/notices', ({ request }) => {
        const cursor = new URL(request.url).searchParams.get('cursor');
        requests.push(cursor);
        if (cursor) {
          hasChanged = true;
          return HttpResponse.json({ detail: 'CFP page changed' }, { status: 409 });
        }
        return HttpResponse.json({
          ...base,
          items: [
            {
              ...base.items[0],
              id: hasChanged ? 'new' : 'old',
              title: hasChanged ? 'New server snapshot' : 'Old server snapshot',
            },
          ],
          page: {
            total: hasChanged ? 1 : 2,
            limit: 1,
            offset: 0,
            next_cursor: hasChanged ? null : 'stale-page',
            has_more: !hasChanged,
          },
        });
      }),
    );
    renderCfp();
    await user.click(await screen.findByRole('button', { name: '加载更多征稿' }));
    expect(await screen.findByRole('heading', { name: 'New server snapshot' })).toBeVisible();
    expect(screen.queryByRole('heading', { name: 'Old server snapshot' })).toBeNull();
    expect(requests).toEqual([null, 'stale-page', null]);
  });

  test('shows notice API failures without bundled data and preserves journal navigation', async () => {
    server.use(
      http.get('http://localhost/api/cfp/journals/:catalogId/notices', () =>
        HttpResponse.json({ detail: 'Stored notice read failed' }, { status: 503 }),
      ),
    );
    renderCfp();
    expect(await screen.findByRole('heading', { name: '加载征稿需求失败' })).toBeVisible();
    expect(document.querySelectorAll('[data-slot="cfp-notice"]')).toHaveLength(0);
    expect(screen.getByRole('button', { name: /未适配示例期刊/ })).toBeVisible();
  });

  test('starts from the first cursor when the historical filter changes', async () => {
    const user = userEvent.setup();
    const base = noticePage('issn-1949-3045', false);
    const requests: [boolean, string | null][] = [];
    server.use(
      http.get('http://localhost/api/cfp/journals/:catalogId/notices', ({ request }) => {
        const url = new URL(request.url);
        const includeClosed = url.searchParams.get('include_closed') === 'true';
        const cursor = url.searchParams.get('cursor');
        requests.push([includeClosed, cursor]);
        return HttpResponse.json({
          ...base,
          items: [
            {
              ...base.items[0],
              id: includeClosed ? 'history' : (cursor ?? 'first'),
              title: includeClosed
                ? 'Historical response'
                : cursor
                  ? 'Second current response'
                  : 'First current response',
            },
          ],
          page: {
            total: includeClosed ? 1 : 2,
            limit: 1,
            offset: cursor ? 1 : 0,
            next_cursor: includeClosed || cursor ? null : 'next',
            has_more: !includeClosed && !cursor,
          },
        });
      }),
    );
    renderCfp();
    await user.click(await screen.findByRole('button', { name: '加载更多征稿' }));
    await screen.findByRole('heading', { name: 'Second current response' });
    await user.click(screen.getByRole('checkbox', { name: '显示已结束及历史' }));
    expect(await screen.findByRole('heading', { name: 'Historical response' })).toBeVisible();
    expect(screen.queryByRole('heading', { name: 'Second current response' })).toBeNull();
    expect(requests).toEqual([
      [false, null],
      [false, 'next'],
      [true, null],
    ]);
  });

  test('ignores a late response for a previously selected journal', async () => {
    const user = userEvent.setup();
    let releaseResponse = () => {};
    let markStarted = () => {};
    let didFinish = false;
    const delayed = new Promise<void>((resolve) => {
      releaseResponse = resolve;
    });
    const started = new Promise<void>((resolve) => {
      markStarted = resolve;
    });
    server.use(
      http.get('http://localhost/api/cfp/journals/issn-1949-3045/notices', async () => {
        markStarted();
        await delayed;
        didFinish = true;
        return HttpResponse.json(noticePage('issn-1949-3045', false));
      }),
    );
    renderCfp();
    await started;
    await user.click(screen.getByRole('button', { name: /未适配示例期刊/ }));
    expect(await screen.findByRole('heading', { name: '暂未适配' })).toBeVisible();
    releaseResponse();
    await waitFor(() => expect(didFinish).toBe(true));
    expect(
      screen.queryByRole('heading', { name: /When Affective Computing Meets Multimodal/ }),
    ).toBeNull();
    expect(screen.getByRole('button', { name: /未适配示例期刊/ })).toHaveAttribute(
      'aria-pressed',
      'true',
    );
  });
});
