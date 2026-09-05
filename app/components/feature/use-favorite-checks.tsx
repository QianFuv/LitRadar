'use client';

/**
 * Shared batch favorite-check cache orchestration for article lists.
 */

import { useQuery, useQueryClient, type QueryClient, type QueryKey } from '@tanstack/react-query';

import { checkFavoritesBatch, type ArticleId, type FavoriteCheck } from '@/lib/api';

/**
 * Favorite state returned to article-list consumers.
 */
export type FavoriteChecksResult = Readonly<{
  favoriteChecksByArticle: Record<ArticleId, FavoriteCheck[]>;
  isFavoriteStatePending: boolean;
  favoriteStateError: Error | null;
  retryFavoriteChecks: () => void;
}>;

/** Shared immutable value for disabled favorite-check scopes. */
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
 * Read only successful, fresh, non-invalidated membership snapshots in one scope.
 *
 * @param queryClient - Owning browser query cache.
 * @param queryKey - User and database prefix.
 * @returns Latest available membership for each article.
 */
function readFreshFavoriteChecks(
  queryClient: QueryClient,
  queryKey: QueryKey,
): Record<ArticleId, FavoriteCheck[]> {
  const cutoff = Date.now() - FAVORITE_CACHE_STALE_TIME;
  const queries = queryClient
    .getQueryCache()
    .findAll({ queryKey })
    .filter(
      (query) =>
        query.state.status === 'success' &&
        !query.state.isInvalidated &&
        query.state.dataUpdatedAt > cutoff,
    )
    .sort((left, right) => left.state.dataUpdatedAt - right.state.dataUpdatedAt);
  const checks: Record<ArticleId, FavoriteCheck[]> = {};
  for (const query of queries) {
    Object.assign(checks, query.state.data);
  }
  return checks;
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
    queryClient.removeQueries({ queryKey, type: 'inactive' });
  }
  await Promise.all(prefixes.map((queryKey) => queryClient.invalidateQueries({ queryKey })));
}

/**
 * Limit a merged cache record to ids requested by the current consumer.
 *
 * @param articleIds - Normalized current article ids.
 * @param checksByArticle - Merged cached and fetched checks.
 * @returns Favorite checks for current ids only.
 */
function selectRequestedFavoriteChecks(
  articleIds: readonly ArticleId[],
  checksByArticle: Record<ArticleId, FavoriteCheck[]>,
): Record<ArticleId, FavoriteCheck[]> {
  const selectedChecks: Record<ArticleId, FavoriteCheck[]> = {};
  for (const articleId of articleIds) {
    if (articleId in checksByArticle) {
      selectedChecks[articleId] = checksByArticle[articleId];
    }
  }
  return selectedChecks;
}

/**
 * Merge cached batch checks and request only missing article ids.
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
  const favoriteBatchBaseKey = ['fav-check-batch', userId, dbName] as const;
  const cachedFavoriteChecksByArticle = hasActiveScope
    ? readFreshFavoriteChecks(queryClient, favoriteBatchBaseKey)
    : EMPTY_FAVORITE_CHECKS;
  const { data, isPending, error, refetch } = useQuery({
    queryKey: [...favoriteBatchBaseKey, 'visible', normalizedArticleIds.join(',')],
    queryFn: async () => {
      const cached = readFreshFavoriteChecks(queryClient, favoriteBatchBaseKey);
      const missing = normalizedArticleIds.filter((articleId) => !(articleId in cached));
      const resolved = { ...cached };
      for (let offset = 0; offset < missing.length; offset += FAVORITE_CHECK_BATCH_SIZE) {
        Object.assign(
          resolved,
          await checkFavoritesBatch(
            missing.slice(offset, offset + FAVORITE_CHECK_BATCH_SIZE),
            dbName,
          ),
        );
      }
      return selectRequestedFavoriteChecks(normalizedArticleIds, resolved);
    },
    enabled: hasActiveScope,
    staleTime: FAVORITE_CACHE_STALE_TIME,
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

  return {
    favoriteChecksByArticle: selectRequestedFavoriteChecks(
      normalizedArticleIds,
      error ? cachedFavoriteChecksByArticle : (data ?? cachedFavoriteChecksByArticle),
    ),
    isFavoriteStatePending: isPending,
    favoriteStateError: error,
    retryFavoriteChecks,
  };
}
