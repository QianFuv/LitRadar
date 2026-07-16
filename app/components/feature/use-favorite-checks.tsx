'use client';

/**
 * Shared batch favorite-check cache orchestration for article lists.
 */

import { useQuery, useQueryClient, type QueryKey } from '@tanstack/react-query';

import { checkFavoritesBatch, type ArticleId, type FavoriteCheck } from '@/lib/api';

/**
 * Favorite state returned to article-list consumers.
 */
export type FavoriteChecksResult = Readonly<{
  favoriteChecksByArticle: Record<ArticleId, FavoriteCheck[]>;
  isFavoriteStatePending: boolean;
}>;

/** Shared immutable value for disabled favorite-check scopes. */
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
 * Merge every favorite-check record returned by a query-key prefix lookup.
 *
 * @param queryData - Matching query keys and cached record values.
 * @returns Combined favorite checks keyed by article id.
 */
function mergeCachedFavoriteChecks(
  queryData: Array<[QueryKey, Record<ArticleId, FavoriteCheck[]> | undefined]>,
): Record<ArticleId, FavoriteCheck[]> {
  const mergedChecks: Record<ArticleId, FavoriteCheck[]> = {};
  for (const [, checks] of queryData) {
    if (!checks) {
      continue;
    }
    for (const [articleId, folders] of Object.entries(checks)) {
      mergedChecks[articleId] = folders;
    }
  }
  return mergedChecks;
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
  const cachedQueryData = hasActiveScope
    ? queryClient.getQueriesData<Record<ArticleId, FavoriteCheck[]>>({
        queryKey: favoriteBatchBaseKey,
      })
    : [];
  const cachedFavoriteChecksByArticle = mergeCachedFavoriteChecks(cachedQueryData);
  const missingFavoriteArticleIds = hasActiveScope
    ? normalizedArticleIds.filter((articleId) => !(articleId in cachedFavoriteChecksByArticle))
    : [];
  const missingFavoriteArticleIdsKey = missingFavoriteArticleIds.join(',');
  const isMissingQueryEnabled = hasActiveScope && missingFavoriteArticleIds.length > 0;

  const { data: fetchedFavoriteChecksByArticle = {}, isPending } = useQuery({
    queryKey: [...favoriteBatchBaseKey, 'missing', missingFavoriteArticleIdsKey],
    queryFn: () => checkFavoritesBatch(missingFavoriteArticleIds, dbName),
    enabled: isMissingQueryEnabled,
    staleTime: 5 * 60 * 1000,
  });

  if (!hasActiveScope) {
    return {
      favoriteChecksByArticle: EMPTY_FAVORITE_CHECKS,
      isFavoriteStatePending: false,
    };
  }

  return {
    favoriteChecksByArticle: selectRequestedFavoriteChecks(normalizedArticleIds, {
      ...cachedFavoriteChecksByArticle,
      ...fetchedFavoriteChecksByArticle,
    }),
    isFavoriteStatePending: isMissingQueryEnabled && isPending,
  };
}
