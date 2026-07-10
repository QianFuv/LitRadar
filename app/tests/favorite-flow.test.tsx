/**
 * Favorite cache update coverage using the production button and API client.
 */

import { screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { http, HttpResponse } from 'msw';
import { describe, expect, test } from 'vitest';

import { FavoriteButton } from '@/components/feature/favorite-button';
import { AuthProvider } from '@/lib/auth-context';
import type { FavoriteCheck } from '@/lib/api';
import { server } from '@/tests/mocks/server';
import { renderWithQuery } from '@/tests/render';

let favoriteRequestBody: unknown = null;

/**
 * Return an authenticated fixture user.
 *
 * @returns Current-user response.
 */
function currentUserResponse(): Response {
  return HttpResponse.json({ id: 21, username: 'favorite_user', is_admin: false });
}

/**
 * Return one available favorite folder.
 *
 * @returns Favorite folder list response.
 */
function foldersResponse(): Response {
  return HttpResponse.json([
    { id: 3, name: 'Reading', is_tracking: false, article_count: 0, created_at: 1 },
  ]);
}

/**
 * Return an initially empty favorite check.
 *
 * @returns Empty favorite check response.
 */
function emptyFavoriteResponse(): Response {
  return HttpResponse.json([]);
}

/**
 * Capture the add-favorite request and return the inserted row.
 *
 * @param context - MSW request context.
 * @returns Inserted favorite response.
 */
async function addFavoriteResponse(context: { request: Request }): Promise<Response> {
  favoriteRequestBody = await context.request.json();
  return HttpResponse.json({
    id: 9,
    folder_id: 3,
    article_id: 'article-1',
    db_name: 'fixture.sqlite',
    note: '',
    created_at: 2,
  });
}

/**
 * Verify a successful mutation updates the immediate favorite state and query cache.
 */
async function updatesFavoriteCache(): Promise<void> {
  favoriteRequestBody = null;
  server.use(
    http.get('http://localhost/api/auth/me', currentUserResponse),
    http.get('http://localhost/api/favorites/folders', foldersResponse),
    http.get('http://localhost/api/favorites/check', emptyFavoriteResponse),
    http.post('http://localhost/api/favorites/folders/3/articles', addFavoriteResponse),
  );
  const user = userEvent.setup();
  const { queryClient } = renderWithQuery(
    <AuthProvider>
      <FavoriteButton articleId="article-1" dbName="fixture.sqlite" />
    </AuthProvider>,
  );

  await user.click(await screen.findByRole('button', { name: '收藏' }));
  await user.click(await screen.findByRole('button', { name: 'Reading' }));

  expect(await screen.findByRole('button', { name: '已收藏' })).toBeInTheDocument();
  expect(favoriteRequestBody).toEqual({
    article_id: 'article-1',
    db_name: 'fixture.sqlite',
    note: '',
  });
  await waitFor(() => {
    expect(
      queryClient.getQueryData<FavoriteCheck[]>(['fav-check', 'article-1', 'fixture.sqlite']),
    ).toEqual([{ folder_id: 3, folder_name: 'Reading' }]);
  });
}

describe('favorite mutation flow', () => {
  test('updates visible state and cached folder membership', updatesFavoriteCache);
});
