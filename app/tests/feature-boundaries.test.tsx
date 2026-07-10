/**
 * Focused coverage for extracted page feature boundaries.
 */

import { screen } from '@testing-library/react';
import { http, HttpResponse } from 'msw';
import { NuqsTestingAdapter } from 'nuqs/adapters/testing';
import { describe, expect, test } from 'vitest';

import { AdminOverviewCard } from '@/components/admin/overview-card';
import { FavoritesPageContent } from '@/components/favorites/favorites-page-content';
import { getFavoriteSelectionKey } from '@/components/favorites/use-favorites-page';
import { AccessTokensCard } from '@/components/settings/access-tokens-card';
import { server } from '@/tests/mocks/server';
import { renderWithQuery } from '@/tests/render';

/**
 * Ignore copy actions in a component that does not expose a token value.
 */
async function ignoreCopy(): Promise<void> {}

/**
 * Verify the extracted favorites hook retains URL-key identity and empty-folder rendering.
 */
async function rendersFavoritesBoundary(): Promise<void> {
  server.use(http.get('http://localhost/api/favorites/folders', () => HttpResponse.json([])));

  renderWithQuery(
    <NuqsTestingAdapter searchParams="?folder=7">
      <FavoritesPageContent userId={21} />
    </NuqsTestingAdapter>,
  );

  expect(await screen.findByText('暂无收藏夹，点击 + 创建')).toBeInTheDocument();
  expect(getFavoriteSelectionKey(7, 'article-1', 'fixture.sqlite')).toBe(
    '7:article-1:fixture.sqlite',
  );
}

/**
 * Verify the extracted settings token card owns its original query boundary.
 */
async function rendersAccessTokenBoundary(): Promise<void> {
  server.use(http.get('http://localhost/api/auth/tokens', () => HttpResponse.json([])));

  renderWithQuery(<AccessTokensCard copyFeedback={null} handleCopy={ignoreCopy} />);

  expect(await screen.findByText('暂无访问令牌')).toBeInTheDocument();
}

/**
 * Verify the extracted administrator overview maps the unchanged statistics payload.
 */
async function rendersAdminOverviewBoundary(): Promise<void> {
  server.use(
    http.get('http://localhost/api/admin/stats', () =>
      HttpResponse.json({
        auth: {
          total_users: 42,
          admin_count: 2,
          total_folders: 3,
          total_favorites: 4,
          total_invite_codes: 5,
          used_invite_codes: 1,
          unused_invite_codes: 4,
          active_tokens: 6,
          notification_subscribers: 7,
          scheduled_tasks: 8,
          active_announcements: 9,
        },
        index: { databases: [], total_articles: 10, total_journals: 11 },
        push: [],
      }),
    ),
  );

  renderWithQuery(<AdminOverviewCard isEnabled />);

  expect(await screen.findByText('42')).toBeInTheDocument();
  expect(screen.getByText('用户总数')).toBeInTheDocument();
}

describe('extracted page feature boundaries', () => {
  test('keeps favorites URL/query behavior in its feature hook', rendersFavoritesBoundary);
  test('keeps access-token querying in its settings card', rendersAccessTokenBoundary);
  test('keeps administrator statistics in its overview card', rendersAdminOverviewBoundary);
});
