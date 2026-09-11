'use client';

/**
 * Shared batch favorite-check cache orchestration for article lists.
 */

import {
  useQueries,
  useQuery,
  useQueryClient,
  type QueryClient,
  type QueryKey,
} from '@tanstack/react-query';

import { checkFavorite, checkFavoritesBatch, type ArticleId, type FavoriteCheck } from '@/lib/api';

/**
 * Favorite state returned to article-list consumers.
 */
export type FavoriteChecksResult = Readonly<{
  favoriteChecksByArticle: Record<ArticleId, FavoriteCheck[]>;
  isFavoriteStatePending: boolean;
  favoriteStateError: Error | null;
  retryFavoriteChecks: () => void;
}>;

/** Membership freshness and the server's maximum batch size. */
const FAVORITE_CACHE_STALE_TIME = 5 * 60 * 1000;
const FAVORITE_CHECK_BATCH_SIZE = 500;

const EMPTY_FAVORITE_CHECKS: Record<ArticleId, FavoriteCheck[]> = {};

/**
 * Deduplicate and sort article ids for stable cache and request identity.
 *
 * @param articleIds - Visible article ids from a list consumer.
 * @returns Sorted unique non-empty ids.
 */
function normalizeArticleIds(articleIds: readonly ArticleId[]): ArticleId[] {
  return Array.from(new Set(articleIds.filter((articleId) => articleId.length > 0))).sort();
}

/**
 * Read one successful, fresh, non-invalidated article membership.
 *
 * @param queryClient - Owning browser query cache.
 * @param queryKey - User, database, and article identity.
 * @returns Fresh membership or undefined when another lookup is required.
 */
export function readFreshFavoriteCheck(
  queryClient: QueryClient,
  queryKey: QueryKey,
): FavoriteCheck[] | undefined {
  const state = queryClient.getQueryState<FavoriteCheck[]>(queryKey);
  return state?.status === 'success' &&
    !state.isInvalidated &&
    state.dataUpdatedAt > Date.now() - FAVORITE_CACHE_STALE_TIME
    ? state.data
    : undefined;
}

/**
 * Cancel old reads and refresh single and batch memberships after an owned mutation.
 *
 * @param queryClient - Owning browser query cache.
 * @param userId - User whose favorite rows changed.
 * @returns Completion of active membership refreshes.
 */
export async function invalidateFavoriteMemberships(
  queryClient: QueryClient,
  userId: number,
): Promise<void> {
  const prefixes = [
    ['fav-check', userId],
    ['fav-check-batch', userId],
  ];
  await Promise.all(prefixes.map((queryKey) => queryClient.cancelQueries({ queryKey })));
  for (const queryKey of prefixes) {
    queryClient.removeQueries({ queryKey, predicate: (query) => query.getObserversCount() === 0 });
  }
  await queryClient.invalidateQueries({ queryKey: prefixes[0], refetchType: 'none' });
  await Promise.all([
    queryClient.refetchQueries({ queryKey: prefixes[0], type: 'active' }),
    queryClient.invalidateQueries({ queryKey: prefixes[1] }),
  ]);
}

/**
 * Subscribe to per-article memberships and batch only missing or stale lookups.
 *
 * @param articleIds - Article ids needed by the current list.
 * @param dbName - Database containing the articles.
 * @param userId - Authenticated user id, or an empty value for anonymous state.
 * @returns Favorite checks and whether the missing-id request is pending.
 */
export function useFavoriteChecks(
  articleIds: readonly ArticleId[],
  dbName: string,
  userId?: number | null,
): FavoriteChecksResult {
  const queryClient = useQueryClient();
  const normalizedArticleIds = normalizeArticleIds(articleIds);
  const hasUser = userId !== null && typeof userId !== 'undefined';
  const hasActiveScope = hasUser && dbName.length > 0 && normalizedArticleIds.length > 0;
  const favoriteBaseKey = ['fav-check', userId, dbName] as const;
  const favoriteBatchBaseKey = ['fav-check-batch', userId, dbName] as const;
  const batchQueryKey = [...favoriteBatchBaseKey, 'visible', normalizedArticleIds.join(',')];
  const memberships = useQueries({
    queries: hasActiveScope
      ? normalizedArticleIds.map((articleId) => ({
          queryKey: [...favoriteBaseKey, articleId],
          queryFn: () => checkFavorite(articleId, dbName),
          enabled: false,
          staleTime: FAVORITE_CACHE_STALE_TIME,
        }))
      : [],
  });
  const { isPending, error, refetch } = useQuery<number>({
    queryKey: batchQueryKey,
    queryFn: async ({ signal }) => {
      const batchState = queryClient.getQueryState(batchQueryKey);
      if (batchState?.isInvalidated && batchState.status === 'success') {
        const visibleArticleIds = new Set(normalizedArticleIds);
        await queryClient.invalidateQueries({
          queryKey: favoriteBaseKey,
          predicate: (query) => visibleArticleIds.has(query.queryKey[3] as ArticleId),
          refetchType: 'none',
        });
      }
      const missing = normalizedArticleIds.filter(
        (articleId) =>
          readFreshFavoriteCheck(queryClient, [...favoriteBaseKey, articleId]) === undefined,
      );
      for (let offset = 0; offset < missing.length; offset += FAVORITE_CHECK_BATCH_SIZE) {
        const articleBatch = missing.slice(offset, offset + FAVORITE_CHECK_BATCH_SIZE);
        const checks = await checkFavoritesBatch(articleBatch, dbName, signal);
        signal.throwIfAborted();
        for (const articleId of articleBatch) {
          if (checks[articleId] !== undefined) {
            queryClient.setQueryData([...favoriteBaseKey, articleId], checks[articleId]);
          }
        }
      }
      return normalizedArticleIds.reduce(
        (oldestUpdatedAt, articleId) =>
          Math.min(
            oldestUpdatedAt,
            queryClient.getQueryState([...favoriteBaseKey, articleId])?.dataUpdatedAt ?? 0,
          ),
        Number.POSITIVE_INFINITY,
      );
    },
    enabled: hasActiveScope,
    gcTime: 0,
    staleTime: (query) =>
      Math.max(0, (query.state.data ?? 0) + FAVORITE_CACHE_STALE_TIME - query.state.dataUpdatedAt),
  });

  /** Retry unresolved membership only while the authenticated scope is active. */
  const retryFavoriteChecks = () => {
    if (hasActiveScope) void refetch();
  };

  if (!hasActiveScope) {
    return {
      favoriteChecksByArticle: EMPTY_FAVORITE_CHECKS,
      isFavoriteStatePending: false,
      favoriteStateError: null,
      retryFavoriteChecks,
    };
  }

  const favoriteChecksByArticle: Record<ArticleId, FavoriteCheck[]> = {};
  memberships.forEach((membership, index) => {
    const articleId = normalizedArticleIds[index];
    const checks = error
      ? readFreshFavoriteCheck(queryClient, [...favoriteBaseKey, articleId])
      : membership.data;
    if (checks !== undefined) favoriteChecksByArticle[articleId] = checks;
  });

  return {
    favoriteChecksByArticle,
    isFavoriteStatePending: isPending,
    favoriteStateError: error,
    retryFavoriteChecks,
  };
}
