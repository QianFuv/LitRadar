/**
 * Shared batch favorite-check cache and request behavior coverage.
 */

import { QueryClientProvider, type QueryClient } from '@tanstack/react-query';
import { act, renderHook, waitFor } from '@testing-library/react';
import { http, HttpResponse } from 'msw';
import type { ReactNode } from 'react';
import { describe, expect, test, vi } from 'vitest';

import {
  useFavoriteChecks,
  invalidateFavoriteMemberships,
  type FavoriteChecksResult,
} from '@/components/feature/use-favorite-checks';
import { checkFavorite, checkFavoritesBatch, type ArticleId, type FavoriteCheck } from '@/lib/api';
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
  queryClient.setQueryData<FavoriteCheck[]>(
    ['fav-check', 21, 'fixture.sqlite', 'article-1'],
    [{ folder_id: 9, folder_name: 'Cached' }],
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
    favoriteStateError: null,
    retryFavoriteChecks: expect.any(Function),
  });

  rerender({ articleIds: ['article-1'], dbName: '', userId: 21 });
  expect(result.current).toEqual({
    favoriteChecksByArticle: {},
    isFavoriteStatePending: false,
    favoriteStateError: null,
    retryFavoriteChecks: expect.any(Function),
  });

  rerender({ articleIds: [], dbName: 'fixture.sqlite', userId: 21 });
  expect(result.current).toEqual({
    favoriteChecksByArticle: {},
    isFavoriteStatePending: false,
    favoriteStateError: null,
    retryFavoriteChecks: expect.any(Function),
  });
  expect(batchRequests).toEqual([]);
}

/** Verify membership lookup failures remain distinguishable from known empty results. */
async function propagatesFavoriteLookupFailure(): Promise<void> {
  server.use(
    http.get('http://localhost/api/favorites/check', () =>
      HttpResponse.json({ detail: 'Favorite lookup unavailable' }, { status: 503 }),
    ),
  );
  await expect(checkFavorite('101', 'fixture.sqlite')).rejects.toMatchObject({ status: 503 });
}

/** Verify failed batch lookups cannot poison the cache with an empty successful record. */
async function propagatesFavoriteBatchFailure(): Promise<void> {
  server.use(
    http.post('http://localhost/api/favorites/check/batch', () =>
      HttpResponse.json({ detail: 'Favorite lookup unavailable' }, { status: 503 }),
    ),
  );
  await expect(checkFavoritesBatch(['101'], 'fixture.sqlite')).rejects.toMatchObject({
    status: 503,
  });
}

/** Verify the batch hook exposes a failure and retries it into authoritative empty membership. */
async function retriesUnavailableBatchMembership(): Promise<void> {
  let shouldFail = true;
  server.use(
    http.post('http://localhost/api/favorites/check/batch', () =>
      shouldFail
        ? HttpResponse.json({ detail: 'Favorite lookup unavailable' }, { status: 503 })
        : HttpResponse.json([{ article_id: '101', folders: [] }]),
    ),
  );
  const queryClient = createTestQueryClient();
  const { result } = renderHook(() => useFavoriteChecks(['101'], 'fixture.sqlite', 21), {
    wrapper: createQueryWrapper(queryClient),
  });
  await waitFor(() => expect(result.current.favoriteStateError).toMatchObject({ status: 503 }));
  expect(result.current.favoriteChecksByArticle['101']).toBeUndefined();
  shouldFail = false;
  act(() => result.current.retryFavoriteChecks());
  await waitFor(() => expect(result.current.favoriteChecksByArticle['101']).toEqual([]));
  expect(result.current.favoriteStateError).toBeNull();
}

/** Verify an already resolved list remains subscribed when membership is invalidated. */
async function refreshesResolvedMembershipAfterInvalidation(): Promise<void> {
  let isFavorited = true;
  server.use(
    http.post('http://localhost/api/favorites/check/batch', () =>
      HttpResponse.json([
        {
          article_id: '101',
          folders: isFavorited ? [{ folder_id: 3, folder_name: 'Reading' }] : [],
        },
      ]),
    ),
  );
  const queryClient = createTestQueryClient();
  const { result } = renderHook(() => useFavoriteChecks(['101'], 'fixture.sqlite', 21), {
    wrapper: createQueryWrapper(queryClient),
  });
  await waitFor(() => expect(result.current.favoriteChecksByArticle['101']).toHaveLength(1));
  isFavorited = false;
  await act(async () => {
    await queryClient.invalidateQueries({ queryKey: ['fav-check-batch', 21, 'fixture.sqlite'] });
  });
  await waitFor(() => expect(result.current.favoriteChecksByArticle['101']).toEqual([]));
}

/** Verify invalidating a long visible list respects the server's five-hundred-item bound. */
async function boundsLargeMembershipRefreshes(): Promise<void> {
  const requestedSizes: number[] = [];
  server.use(
    http.post('http://localhost/api/favorites/check/batch', async ({ request }) => {
      const payload = (await request.json()) as { article_ids: string[] };
      requestedSizes.push(payload.article_ids.length);
      expect(payload.article_ids.length).toBeLessThanOrEqual(500);
      return HttpResponse.json(
        payload.article_ids.map((article_id) => ({ article_id, folders: [] })),
      );
    }),
  );
  const queryClient = createTestQueryClient();
  const articleIds = Array.from({ length: 600 }, (_, index) => String(index + 1));
  const { result } = renderHook(() => useFavoriteChecks(articleIds, 'fixture.sqlite', 21), {
    wrapper: createQueryWrapper(queryClient),
  });
  await waitFor(() =>
    expect(Object.keys(result.current.favoriteChecksByArticle)).toHaveLength(600),
  );
  expect(requestedSizes).toEqual([500, 100]);
  await act(async () => {
    await queryClient.invalidateQueries({ queryKey: ['fav-check-batch', 21, 'fixture.sqlite'] });
  });
  expect(requestedSizes).toEqual([500, 100, 500, 100]);
}

/** Verify retrying a later failed batch preserves earlier memberships and their freshness. */
async function retriesOnlyFailedMembershipBatch(): Promise<void> {
  const requestedIds: ArticleId[][] = [];
  let shouldFailLastBatch = true;
  server.use(
    http.post('http://localhost/api/favorites/check/batch', async ({ request }) => {
      const payload = (await request.json()) as BatchRequest;
      requestedIds.push(payload.article_ids);
      if (shouldFailLastBatch && payload.article_ids.length === 100) {
        return HttpResponse.json({ detail: 'Last batch unavailable' }, { status: 503 });
      }
      return HttpResponse.json(
        payload.article_ids.map((article_id) => ({ article_id, folders: [] })),
      );
    }),
  );
  const queryClient = createTestQueryClient();
  const articleIds = Array.from({ length: 600 }, (_, index) => String(index + 1));
  const { result } = renderHook(() => useFavoriteChecks(articleIds, 'fixture.sqlite', 21), {
    wrapper: createQueryWrapper(queryClient),
  });
  await waitFor(() => expect(result.current.favoriteStateError).toMatchObject({ status: 503 }));
  expect(Object.keys(result.current.favoriteChecksByArticle)).toHaveLength(500);
  const successfulUpdatedAt = requestedIds[0].map((articleId) => ({
    queryKey: ['fav-check', 21, 'fixture.sqlite', articleId],
    updatedAt: queryClient.getQueryState(['fav-check', 21, 'fixture.sqlite', articleId])
      ?.dataUpdatedAt,
  }));

  shouldFailLastBatch = false;
  vi.spyOn(Date, 'now').mockReturnValue(Date.now() + 1000);
  act(() => result.current.retryFavoriteChecks());
  await waitFor(() =>
    expect(Object.keys(result.current.favoriteChecksByArticle)).toHaveLength(600),
  );
  expect(result.current.favoriteStateError).toBeNull();
  expect(requestedIds.map((ids) => ids.length)).toEqual([500, 100, 100]);
  for (const { queryKey, updatedAt } of successfulUpdatedAt) {
    expect(queryClient.getQueryState(queryKey)?.dataUpdatedAt).toBe(updatedAt);
  }

  shouldFailLastBatch = true;
  await act(async () => {
    await queryClient.invalidateQueries({ queryKey: ['fav-check-batch', 21, 'fixture.sqlite'] });
  });
  await waitFor(() => expect(result.current.favoriteStateError).toMatchObject({ status: 503 }));
  expect(Object.keys(result.current.favoriteChecksByArticle)).toHaveLength(500);
  shouldFailLastBatch = false;
  act(() => result.current.retryFavoriteChecks());
  await waitFor(() =>
    expect(Object.keys(result.current.favoriteChecksByArticle)).toHaveLength(600),
  );
  expect(result.current.favoriteStateError).toBeNull();
  expect(requestedIds.map((ids) => ids.length)).toEqual([500, 100, 100, 500, 100, 100]);
}

/** Verify loading more pages retains one membership value per unique article. */
async function retainsLinearMembershipStorage(): Promise<void> {
  server.use(
    http.post('http://localhost/api/favorites/check/batch', async ({ request }) => {
      const payload = (await request.json()) as BatchRequest;
      return HttpResponse.json(
        payload.article_ids.map((article_id) => ({ article_id, folders: [] })),
      );
    }),
  );
  const queryClient = createTestQueryClient();
  const { result, rerender } = renderHook(useFavoriteChecksHarness, {
    initialProps: { articleIds: [] as ArticleId[], dbName: 'fixture.sqlite', userId: 21 },
    wrapper: createQueryWrapper(queryClient),
  });

  for (let page = 1; page <= 20; page += 1) {
    const articleIds = Array.from(Array<number>(page * 50).keys(), (index) => String(index + 1));
    rerender({ articleIds, dbName: 'fixture.sqlite', userId: 21 });
    await waitFor(() =>
      expect(Object.keys(result.current.favoriteChecksByArticle)).toHaveLength(articleIds.length),
    );
  }

  const membershipCount = queryClient
    .getQueryCache()
    .getAll()
    .reduce((count, query) => {
      const data: unknown = query.state.data;
      if (Array.isArray(data)) return count + 1;
      return data && typeof data === 'object' ? count + Object.keys(data).length : count;
    }, 0);
  expect(membershipCount).toBe(1000);
  await waitFor(() =>
    expect(
      queryClient.getQueryCache().findAll({ queryKey: ['fav-check-batch', 21, 'fixture.sqlite'] }),
    ).toHaveLength(1),
  );
}

/** Verify adding fresh articles never extends an older article's membership lifetime. */
async function preservesIndividualFreshness(): Promise<void> {
  const requestedIds: string[][] = [];
  server.use(
    http.post('http://localhost/api/favorites/check/batch', async ({ request }) => {
      const payload = (await request.json()) as BatchRequest;
      requestedIds.push(payload.article_ids);
      return HttpResponse.json(
        payload.article_ids.map((article_id) => ({ article_id, folders: [] })),
      );
    }),
  );
  const queryClient = createTestQueryClient();
  const currentTime = Date.now();
  const originalUpdatedAt = currentTime - 4 * 60 * 1000;
  const firstArticleKey = ['fav-check', 21, 'fixture.sqlite', '101'];
  queryClient.setQueryData(firstArticleKey, [], { updatedAt: originalUpdatedAt });
  const { result, rerender } = renderHook(useFavoriteChecksHarness, {
    initialProps: { articleIds: ['101', '102'], dbName: 'fixture.sqlite', userId: 21 },
    wrapper: createQueryWrapper(queryClient),
  });
  await waitFor(() => expect(result.current.favoriteChecksByArticle['102']).toEqual([]));
  expect(requestedIds).toEqual([['102']]);
  expect(queryClient.getQueryState(firstArticleKey)?.dataUpdatedAt).toBe(originalUpdatedAt);

  vi.spyOn(Date, 'now').mockReturnValue(currentTime + 2 * 60 * 1000);
  rerender({ articleIds: ['101', '102', '103'], dbName: 'fixture.sqlite', userId: 21 });
  await waitFor(() => expect(result.current.favoriteChecksByArticle['103']).toEqual([]));
  expect(requestedIds).toEqual([['102'], ['101', '103']]);
}

/** Verify overlapping consumers share mutations and survive user-wide invalidation. */
async function synchronizesOverlappingConsumers(): Promise<void> {
  const requestedIds: string[][] = [];
  server.use(
    http.post('http://localhost/api/favorites/check/batch', async ({ request }) => {
      const payload = (await request.json()) as BatchRequest;
      requestedIds.push(payload.article_ids);
      return HttpResponse.json(
        payload.article_ids.map((article_id) => ({ article_id, folders: [] })),
      );
    }),
  );
  const queryClient = createTestQueryClient();
  const wrapper = createQueryWrapper(queryClient);
  const first = renderHook(() => useFavoriteChecks(['101', '102'], 'fixture.sqlite', 21), {
    wrapper,
  });
  await waitFor(() => expect(first.result.current.favoriteChecksByArticle['102']).toEqual([]));
  const second = renderHook(() => useFavoriteChecks(['102', '103'], 'fixture.sqlite', 21), {
    wrapper,
  });
  await waitFor(() => expect(second.result.current.favoriteChecksByArticle['103']).toEqual([]));
  expect(requestedIds).toEqual([['101', '102'], ['103']]);

  const changedMembership = [{ folder_id: 9, folder_name: 'Reading' }];
  act(() =>
    queryClient.setQueryData(['fav-check', 21, 'fixture.sqlite', '102'], changedMembership),
  );
  await waitFor(() => {
    expect(first.result.current.favoriteChecksByArticle['102']).toEqual(changedMembership);
    expect(second.result.current.favoriteChecksByArticle['102']).toEqual(changedMembership);
  });

  await act(async () => invalidateFavoriteMemberships(queryClient, 21));
  await waitFor(() => {
    expect(first.result.current.favoriteChecksByArticle['102']).toEqual([]);
    expect(second.result.current.favoriteChecksByArticle['102']).toEqual([]);
  });
}

/** Verify an aborted lookup cannot overwrite a membership published after cancellation. */
async function preventsCancelledMembershipWrites(): Promise<void> {
  let releaseResponse = (): void => undefined;
  const responseGate = new Promise<void>((resolve) => {
    releaseResponse = resolve;
  });
  let requestSignal: AbortSignal | undefined;
  let completeResponse = (): void => undefined;
  const responseCompleted = new Promise<void>((resolve) => {
    completeResponse = resolve;
  });
  server.use(
    http.post('http://localhost/api/favorites/check/batch', async ({ request }) => {
      requestSignal = request.signal;
      await responseGate;
      completeResponse();
      return HttpResponse.json([{ article_id: '101', folders: [] }]);
    }),
  );
  const queryClient = createTestQueryClient();
  const { result } = renderHook(() => useFavoriteChecks(['101'], 'fixture.sqlite', 21), {
    wrapper: createQueryWrapper(queryClient),
  });
  await waitFor(() => expect(requestSignal).toBeDefined());
  const changedMembership = [{ folder_id: 9, folder_name: 'Reading' }];
  await act(async () => {
    await queryClient.cancelQueries({ queryKey: ['fav-check-batch', 21, 'fixture.sqlite'] });
    queryClient.setQueryData(['fav-check', 21, 'fixture.sqlite', '101'], changedMembership);
    releaseResponse();
    await responseCompleted;
  });
  expect(requestSignal?.aborted).toBe(true);
  await waitFor(() =>
    expect(result.current.favoriteChecksByArticle['101']).toEqual(changedMembership),
  );
  expect(queryClient.getQueryData(['fav-check', 21, 'fixture.sqlite', '101'])).toEqual(
    changedMembership,
  );
}

describe('useFavoriteChecks', () => {
  test('preserves individual membership freshness', preservesIndividualFreshness);
  test('synchronizes overlapping consumers', synchronizesOverlappingConsumers);
  test('prevents cancelled membership writes', preventsCancelledMembershipWrites);
  test('retains linear membership storage while loading pages', retainsLinearMembershipStorage);
  test('bounds large membership refreshes', boundsLargeMembershipRefreshes);
  test('retries only the failed membership batch', retriesOnlyFailedMembershipBatch);
  test(
    'refreshes resolved membership after invalidation',
    refreshesResolvedMembershipAfterInvalidation,
  );
  test('retries unavailable batch membership', retriesUnavailableBatchMembership);
  test('propagates single lookup failure', propagatesFavoriteLookupFailure);
  test('propagates batch lookup failure', propagatesFavoriteBatchFailure);
  test('requests only unique missing article ids', requestsOnlyMissingIds);
  test('isolates favorite caches by user and database', isolatesUserAndDatabaseScopes);
  test('disables anonymous and empty scopes', disablesInvalidScopes);
});
