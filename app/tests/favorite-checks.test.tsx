/**
 * Shared batch favorite-check cache and request behavior coverage.
 */

import { QueryClientProvider, type QueryClient } from '@tanstack/react-query';
import { renderHook, waitFor } from '@testing-library/react';
import { http, HttpResponse } from 'msw';
import type { ReactNode } from 'react';
import { describe, expect, test } from 'vitest';

import {
  useFavoriteChecks,
  type FavoriteChecksResult,
} from '@/components/feature/use-favorite-checks';
import type { ArticleId, FavoriteCheck } from '@/lib/api';
import { createTestQueryClient } from '@/tests/render';
import { server } from '@/tests/mocks/server';

type BatchRequest = {
  article_ids: ArticleId[];
  db_name: string;
};

type FavoriteChecksHarnessProps = {
  articleIds: ArticleId[];
  dbName: string;
  userId?: number | null;
};

const batchRequests: BatchRequest[] = [];

/**
 * Return deterministic folder membership for every requested article.
 *
 * @param context - MSW request context.
 * @returns Batch favorite-check response.
 */
async function favoriteBatchResponse(context: { request: Request }): Promise<Response> {
  const requestBody = (await context.request.json()) as BatchRequest;
  batchRequests.push(requestBody);
  return HttpResponse.json(
    requestBody.article_ids.map((articleId, index) => ({
      article_id: articleId,
      folders: [
        {
          folder_id: index + 1,
          folder_name: `${requestBody.db_name}:${articleId}`,
        },
      ],
    })),
  );
}

/**
 * Create a provider wrapper for one isolated query client.
 *
 * @param queryClient - Query client owned by the hook test.
 * @returns React provider wrapper.
 */
function createQueryWrapper(queryClient: QueryClient) {
  /**
   * Provide the isolated query client to the hook.
   *
   * @param props - Wrapper children.
   * @returns Query client provider tree.
   */
  function QueryWrapper({ children }: { children: ReactNode }) {
    return <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>;
  }

  return QueryWrapper;
}

/**
 * Invoke the production hook from a renderHook callback.
 *
 * @param props - Hook inputs.
 * @returns Favorite-check result.
 */
function useFavoriteChecksHarness(props: FavoriteChecksHarnessProps): FavoriteChecksResult {
  return useFavoriteChecks(props.articleIds, props.dbName, props.userId);
}

/**
 * Verify cached ids are merged and only sorted unique missing ids are requested.
 */
async function requestsOnlyMissingIds(): Promise<void> {
  batchRequests.length = 0;
  server.use(http.post('http://localhost/api/favorites/check/batch', favoriteBatchResponse));
  const queryClient = createTestQueryClient();
  queryClient.setQueryData<Record<ArticleId, FavoriteCheck[]>>(
    ['fav-check-batch', 21, 'fixture.sqlite', 'missing', 'article-1'],
    {
      'article-1': [{ folder_id: 9, folder_name: 'Cached' }],
    },
  );

  const { result, rerender } = renderHook(useFavoriteChecksHarness, {
    initialProps: {
      articleIds: ['article-3', 'article-1', 'article-2', 'article-2'],
      dbName: 'fixture.sqlite',
      userId: 21,
    },
    wrapper: createQueryWrapper(queryClient),
  });

  await waitFor(() => expect(result.current.favoriteChecksByArticle['article-3']).toHaveLength(1));
  expect(batchRequests).toEqual([
    {
      article_ids: ['article-2', 'article-3'],
      db_name: 'fixture.sqlite',
    },
  ]);
  expect(result.current.favoriteChecksByArticle['article-1']).toEqual([
    { folder_id: 9, folder_name: 'Cached' },
  ]);
  expect(result.current.isFavoriteStatePending).toBe(false);

  rerender({
    articleIds: ['article-2', 'article-3', 'article-1'],
    dbName: 'fixture.sqlite',
    userId: 21,
  });
  expect(batchRequests).toHaveLength(1);
}

/**
 * Verify changing the user or database cannot reuse another cache scope.
 */
async function isolatesUserAndDatabaseScopes(): Promise<void> {
  batchRequests.length = 0;
  server.use(http.post('http://localhost/api/favorites/check/batch', favoriteBatchResponse));
  const queryClient = createTestQueryClient();
  const { result, rerender } = renderHook(useFavoriteChecksHarness, {
    initialProps: {
      articleIds: ['shared-article'],
      dbName: 'first.sqlite',
      userId: 21,
    },
    wrapper: createQueryWrapper(queryClient),
  });

  await waitFor(() =>
    expect(result.current.favoriteChecksByArticle['shared-article']?.[0]?.folder_name).toBe(
      'first.sqlite:shared-article',
    ),
  );

  rerender({
    articleIds: ['shared-article'],
    dbName: 'second.sqlite',
    userId: 22,
  });
  await waitFor(() =>
    expect(result.current.favoriteChecksByArticle['shared-article']?.[0]?.folder_name).toBe(
      'second.sqlite:shared-article',
    ),
  );
  expect(batchRequests).toEqual([
    { article_ids: ['shared-article'], db_name: 'first.sqlite' },
    { article_ids: ['shared-article'], db_name: 'second.sqlite' },
  ]);
}

/**
 * Verify anonymous, empty-id, and empty-database inputs never request or report pending state.
 */
function disablesInvalidScopes(): void {
  batchRequests.length = 0;
  server.use(http.post('http://localhost/api/favorites/check/batch', favoriteBatchResponse));
  const queryClient = createTestQueryClient();
  const { result, rerender } = renderHook(useFavoriteChecksHarness, {
    initialProps: {
      articleIds: ['article-1'],
      dbName: 'fixture.sqlite',
      userId: null,
    } as FavoriteChecksHarnessProps,
    wrapper: createQueryWrapper(queryClient),
  });

  expect(result.current).toEqual({
    favoriteChecksByArticle: {},
    isFavoriteStatePending: false,
  });

  rerender({ articleIds: ['article-1'], dbName: '', userId: 21 });
  expect(result.current).toEqual({
    favoriteChecksByArticle: {},
    isFavoriteStatePending: false,
  });

  rerender({ articleIds: [], dbName: 'fixture.sqlite', userId: 21 });
  expect(result.current).toEqual({
    favoriteChecksByArticle: {},
    isFavoriteStatePending: false,
  });
  expect(batchRequests).toEqual([]);
}

describe('useFavoriteChecks', () => {
  test('requests only unique missing article ids', requestsOnlyMissingIds);
  test('isolates favorite caches by user and database', isolatesUserAndDatabaseScopes);
  test('disables anonymous and empty scopes', disablesInvalidScopes);
});
