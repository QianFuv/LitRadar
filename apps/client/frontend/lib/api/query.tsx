/**
 * TanStack Query options built from openapi-react-query keys and requests.
 *
 * `$api.queryOptions` types query errors as the server's problem body, but the client middleware
 * rejects every failure with an `ApiError` (or a transport `TypeError`), so these wrappers declare
 * `Error`. Validated variants parse inside the query function: the cache only ever holds checked
 * data, and a contract mismatch fails the query with `ApiContractError`.
 */

import {
  infiniteQueryOptions,
  queryOptions,
  type QueryFunctionContext,
  type QueryKey,
} from '@tanstack/react-query';

import type { ContractParser } from '@/lib/api-contract';
import type { paths } from '@/lib/generated/api-schema';

/** Key and request produced by `$api.queryOptions`. */
type GeneratedQuery<TKey extends QueryKey, TData> = {
  queryKey: TKey;
  queryFn: (context: QueryFunctionContext<TKey>) => TData | Promise<TData>;
};

/**
 * Use an openapi-react-query request as query options with `Error` failures.
 *
 * @param query - Result of `$api.queryOptions`.
 * @returns Query options with the generated key and request.
 */
export function apiQuery<TKey extends QueryKey, TData>(query: GeneratedQuery<TKey, TData>) {
  return queryOptions<TData, Error, TData, TKey>({
    queryKey: query.queryKey,
    queryFn: async (context) => query.queryFn(context),
  });
}

/**
 * Use an openapi-react-query request as query options whose response is contract-validated.
 *
 * @param query - Result of `$api.queryOptions`.
 * @param parse - Runtime contract parser that throws `ApiContractError` on a mismatch.
 * @returns Query options that cache only validated data.
 */
export function validatedApiQuery<TKey extends QueryKey, TResponse, TData>(
  query: GeneratedQuery<TKey, TResponse>,
  parse: ContractParser<TData>,
) {
  return queryOptions<TData, Error, TData, TKey>({
    queryKey: query.queryKey,
    queryFn: async (context) => parse(await query.queryFn(context)),
  });
}

/**
 * Build cursor-paged infinite query options under an openapi-react-query key.
 *
 * @param queryKey - Key of the first-page request from `$api.queryOptions`, without a cursor.
 * @param fetchPage - Request for one page at an opaque cursor, or null for the first page.
 * @param readNextCursor - Cursor of the page after the given one, or null on the last page.
 * @returns Infinite query options paged by opaque cursors.
 */
export function apiCursorQuery<TKey extends QueryKey, TPage>(
  queryKey: TKey,
  fetchPage: (cursor: string | null, signal: AbortSignal) => Promise<TPage>,
  readNextCursor: (page: TPage) => string | null | undefined,
) {
  return infiniteQueryOptions<
    TPage,
    Error,
    { pages: TPage[]; pageParams: (string | null)[] },
    TKey,
    string | null
  >({
    queryKey,
    queryFn: ({ pageParam, signal }) => fetchPage(pageParam, signal),
    initialPageParam: null,
    getNextPageParam: (page) => readNextCursor(page) ?? undefined,
  });
}

/** Path templates that accept one HTTP method. */
type PathWithMethod<TMethod extends 'get' | 'post' | 'put' | 'delete'> = {
  [Path in keyof paths]: undefined extends paths[Path][TMethod] ? never : Path;
}[keyof paths];

/**
 * Key prefix of every openapi-react-query query for one endpoint, whatever its parameters.
 *
 * @param method - HTTP method.
 * @param path - OpenAPI path template.
 * @returns Key prefix `[method, path]`.
 */
export function apiPathKey<TMethod extends 'get' | 'post', TPath extends PathWithMethod<TMethod>>(
  method: TMethod,
  path: TPath,
) {
  return [method, path] as const;
}
