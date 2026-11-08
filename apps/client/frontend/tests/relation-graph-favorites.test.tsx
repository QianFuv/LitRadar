/**
 * Saved relation graph coverage: saving, change detection, updating, resetting, leaving with
 * unsaved changes, and managing saved graphs.
 */

import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { http, HttpResponse } from 'msw';
import { Link } from 'react-router';
import { NuqsTestingAdapter } from 'nuqs/adapters/testing';
import { beforeEach, describe, expect, test, vi } from 'vitest';

import { RelationGraphFavoritesContent } from '@/components/favorites/relation-graph-favorites-content';
import { RelationGraphFavoriteControls } from '@/components/relations/relation-graph-favorite-controls';
import type { Article, RelationGraphFavorite, RelationGraphFavoriteSummary } from '@/lib/api';
import { createRelationGraph, expandRelationNode, type RelationGraph } from '@/lib/relation-graph';
import { server } from '@/tests/mocks/server';
import { renderWithQuery } from '@/tests/render';

const GRAPHS_URL = 'http://localhost/api/favorites/graphs';

const navigation = vi.hoisted(() => ({ navigate: vi.fn() }));

vi.mock('react-router', async (importOriginal) => ({
  ...(await importOriginal<typeof import('react-router')>()),
  useNavigate: () => navigation.navigate,
}));

beforeEach(() => {
  navigation.navigate.mockReset();
});

/**
 * Build a minimal article.
 *
 * @param id - Numeric article identifier.
 * @returns Article.
 */
function article(id: string): Article {
  return { article_id: id, journal_id: 'j', title: `Article ${id}`, abstract: '', authors: [] };
}

/**
 * Build a center article with two related articles.
 *
 * @returns Relation graph of three nodes.
 */
function threeNodeGraph(): RelationGraph {
  return expandRelationNode(createRelationGraph(article('1')), '1', [article('2'), article('3')]);
}

/**
 * Build a saved graph response.
 *
 * @param id - Saved graph id.
 * @param graph - Snapshot.
 * @returns Saved graph.
 */
function savedGraph(id: number, graph: RelationGraph): RelationGraphFavorite {
  return {
    id,
    title: '以「Article 1」为中心的关系图',
    db_name: 'fixture.sqlite',
    root_article_id: 1,
    node_count: Object.keys(graph.nodes).length,
    created_at: 1_700_000_000,
    updated_at: 1_700_000_000,
    graph: graph as unknown as Record<string, unknown>,
  };
}

/**
 * Render the favorite control for a graph.
 *
 * @param graph - Current graph.
 * @param savedId - Linked saved graph id.
 * @returns Callback spies.
 */
function renderControls(graph: RelationGraph, savedId: number | null) {
  const callbacks = { onError: vi.fn(), onReset: vi.fn(), onSavedIdChange: vi.fn() };
  renderWithQuery(
    <RelationGraphFavoriteControls
      dbName="fixture.sqlite"
      graph={graph}
      rootTitle="Article 1"
      savedId={savedId}
      {...callbacks}
    />,
  );
  return callbacks;
}

/** Saving an unsaved graph posts its full snapshot and links the new favorite. */
async function savesNewGraph(): Promise<void> {
  const graph = threeNodeGraph();
  let body: Record<string, unknown> | null = null;
  server.use(
    http.post(GRAPHS_URL, async ({ request }) => {
      body = (await request.json()) as Record<string, unknown>;
      return HttpResponse.json(savedGraph(11, graph));
    }),
  );
  const callbacks = renderControls(graph, null);

  await userEvent.click(screen.getByRole('button', { name: '收藏关系图' }));

  await waitFor(() => expect(callbacks.onSavedIdChange).toHaveBeenCalledWith(11));
  expect(body).toMatchObject({
    title: '以「Article 1」为中心的关系图',
    db_name: 'fixture.sqlite',
    root_article_id: 1,
    node_count: 3,
    graph: { rootId: '1', expandedIds: ['1'] },
  });
  expect(screen.getByTestId('relation-graph-favorite-status')).toHaveTextContent('已收藏关系图。');
}

/** A changed linked graph is flagged, resets to the saved state, and saves from the chip. */
async function flagsUpdatesAndResetsChangedGraph(): Promise<void> {
  const saved = threeNodeGraph();
  const current = expandRelationNode(saved, '2', [article('4')]);
  let updateBody: Record<string, unknown> | null = null;
  server.use(
    http.get(`${GRAPHS_URL}/5`, () => HttpResponse.json(savedGraph(5, saved))),
    http.put(`${GRAPHS_URL}/5`, async ({ request }) => {
      updateBody = (await request.json()) as Record<string, unknown>;
      return HttpResponse.json(savedGraph(5, current));
    }),
  );
  const callbacks = renderControls(current, 5);
  const user = userEvent.setup();

  const trigger = await screen.findByRole('button', {
    name: '已收藏的关系图操作（有未保存的改动）',
  });
  await user.hover(trigger);
  expect(await screen.findByRole('tooltip')).toHaveTextContent(
    /^已收藏：以「Article 1」为中心的关系图（有未保存的改动）$/,
  );
  await user.unhover(trigger);
  expect(screen.getByTestId('relation-graph-unsaved-dot')).toBeInTheDocument();

  await user.click(trigger);
  await user.click(await screen.findByRole('menuitem', { name: '重置为收藏时的状态' }));
  expect(callbacks.onReset).toHaveBeenCalledOnce();
  expect(Object.keys(callbacks.onReset.mock.calls[0][0].nodes).sort()).toEqual(['1', '2', '3']);

  await user.click(trigger);
  await user.click(await screen.findByRole('menuitem', { name: '保存改动到收藏' }));
  await waitFor(() => expect(updateBody).toMatchObject({ node_count: 4 }));
  expect(updateBody).not.toHaveProperty('title');
  await waitFor(() =>
    expect(screen.queryByTestId('relation-graph-unsaved-dot')).not.toBeInTheDocument(),
  );
  await user.click(screen.getByRole('button', { name: '已收藏的关系图操作' }));
  expect(await screen.findByRole('menuitem', { name: '重置为收藏时的状态' })).toHaveAttribute(
    'aria-disabled',
    'true',
  );
}

/** Free layout drift after reopening a saved graph is not reported as a change. */
async function ignoresLayoutDrift(): Promise<void> {
  const saved = threeNodeGraph();
  const drifted: RelationGraph = {
    ...saved,
    positions: Object.fromEntries(
      Object.entries(saved.positions).map(([id, point]) => [
        id,
        { x: point.x + 25, y: point.y - 25 },
      ]),
    ),
  };
  server.use(http.get(`${GRAPHS_URL}/5`, () => HttpResponse.json(savedGraph(5, saved))));
  renderControls(drifted, 5);

  await userEvent.setup().hover(await screen.findByRole('button', { name: '已收藏的关系图操作' }));
  await waitFor(() =>
    expect(screen.getByRole('tooltip')).toHaveTextContent(
      /^已收藏：以「Article 1」为中心的关系图$/,
    ),
  );
  expect(screen.queryByTestId('relation-graph-unsaved-dot')).not.toBeInTheDocument();
}

/**
 * Render a changed saved graph next to an in-app link.
 *
 * @returns Captured update requests.
 */
async function renderChangedGraphWithLink() {
  const saved = threeNodeGraph();
  const current = expandRelationNode(saved, '2', [article('4')]);
  const updates: unknown[] = [];
  server.use(
    http.get(`${GRAPHS_URL}/5`, () => HttpResponse.json(savedGraph(5, saved))),
    http.put(`${GRAPHS_URL}/5`, async ({ request }) => {
      updates.push(await request.json());
      return HttpResponse.json(savedGraph(5, current));
    }),
  );
  renderWithQuery(
    <>
      <RelationGraphFavoriteControls
        dbName="fixture.sqlite"
        graph={current}
        rootTitle="Article 1"
        savedId={5}
        onError={vi.fn()}
        onReset={vi.fn()}
        onSavedIdChange={vi.fn()}
      />
      <Link to="/?view=favorites">我的收藏</Link>
    </>,
  );
  await screen.findByTestId('relation-graph-unsaved-dot');
  return { updates };
}

/** Leaving a changed saved graph asks first and can save before navigating. */
async function asksBeforeLeavingAndSaves(): Promise<void> {
  const { updates } = await renderChangedGraphWithLink();
  const user = userEvent.setup();

  await user.click(screen.getByRole('link', { name: '我的收藏' }));
  const dialog = await screen.findByRole('dialog', { name: '关系图有未保存的改动' });
  expect(navigation.navigate).not.toHaveBeenCalled();

  await user.click(within(dialog).getByRole('button', { name: '保存并离开' }));
  await waitFor(() => expect(navigation.navigate).toHaveBeenCalledWith('/?view=favorites'));
  expect(updates).toHaveLength(1);
}

/** Leaving without saving navigates and keeps the favorite untouched. */
async function leavesWithoutSaving(): Promise<void> {
  const { updates } = await renderChangedGraphWithLink();
  const user = userEvent.setup();

  await user.click(screen.getByRole('link', { name: '我的收藏' }));
  const dialog = await screen.findByRole('dialog', { name: '关系图有未保存的改动' });
  await user.click(within(dialog).getByRole('button', { name: '不保存' }));

  expect(navigation.navigate).toHaveBeenCalledWith('/?view=favorites');
  expect(updates).toHaveLength(0);
}

/** Closing or reloading the tab asks the browser to confirm while changes are unsaved. */
async function guardsTabClose(): Promise<void> {
  await renderChangedGraphWithLink();
  const event = new Event('beforeunload', { cancelable: true });
  window.dispatchEvent(event);
  expect(event.defaultPrevented).toBe(true);
}

/** Updating a favorite deleted elsewhere unlinks the graph and explains why. */
async function unlinksDeletedFavorite(): Promise<void> {
  const graph = threeNodeGraph();
  server.use(
    http.get(`${GRAPHS_URL}/5`, () => HttpResponse.json(savedGraph(5, graph))),
    http.put(`${GRAPHS_URL}/5`, () =>
      HttpResponse.json({ detail: 'Not found', code: 'not_found' }, { status: 404 }),
    ),
  );
  const callbacks = renderControls(graph, 5);
  const user = userEvent.setup();

  await user.click(screen.getByRole('button', { name: '已收藏的关系图操作' }));
  await user.click(await screen.findByRole('menuitem', { name: '更新收藏' }));

  await waitFor(() => expect(callbacks.onSavedIdChange).toHaveBeenCalledWith(null));
  expect(callbacks.onError).toHaveBeenCalledWith('这张关系图的收藏已被删除，可以重新收藏。');
}

/**
 * Build a saved graph summary.
 *
 * @param id - Saved graph id.
 * @param title - Saved title.
 * @returns Summary.
 */
function summary(id: number, title: string): RelationGraphFavoriteSummary {
  return {
    id,
    title,
    db_name: 'fixture.sqlite',
    root_article_id: 40 + id,
    node_count: 3,
    created_at: 1_700_000_000,
    updated_at: 1_700_000_000,
  };
}

/** The favorites list opens, renames, and deletes saved graphs. */
async function managesSavedGraphs(): Promise<void> {
  let items = [summary(1, 'First graph'), summary(2, 'Second graph')];
  const renames: unknown[] = [];
  server.use(
    http.get(GRAPHS_URL, () => HttpResponse.json({ items, next_cursor: null })),
    http.put(`${GRAPHS_URL}/1`, async ({ request }) => {
      const body = (await request.json()) as { title: string };
      renames.push(body);
      items = items.map((item) => (item.id === 1 ? { ...item, title: body.title } : item));
      return HttpResponse.json({ ...items[0], graph: {} });
    }),
    http.delete(`${GRAPHS_URL}/2`, () => {
      items = items.filter((item) => item.id !== 2);
      return HttpResponse.json({ ok: true });
    }),
  );
  renderWithQuery(
    <NuqsTestingAdapter searchParams="?view=favorites&kind=graphs" hasMemory>
      <RelationGraphFavoritesContent />
    </NuqsTestingAdapter>,
  );
  const user = userEvent.setup();

  const list = await screen.findByRole('list', { name: '收藏的关系图' });
  expect(within(list).getByRole('link', { name: 'First graph' })).toHaveAttribute(
    'href',
    '/?view=relations&db=fixture.sqlite&article=41&saved=1',
  );
  expect(screen.getByRole('radio', { name: '关系图' })).toHaveAttribute('aria-checked', 'true');

  await user.click(screen.getByRole('button', { name: '管理关系图 First graph' }));
  await user.click(await screen.findByRole('menuitem', { name: '重命名' }));
  const input = await screen.findByRole('textbox', { name: '关系图名称' });
  await user.clear(input);
  await user.type(input, 'Renamed graph');
  await user.click(screen.getByRole('button', { name: '保存' }));
  expect(await within(list).findByRole('link', { name: 'Renamed graph' })).toBeInTheDocument();
  expect(renames).toEqual([{ title: 'Renamed graph' }]);

  await user.click(screen.getByRole('button', { name: '管理关系图 Second graph' }));
  await user.click(await screen.findByRole('menuitem', { name: '删除' }));
  await user.click(await screen.findByRole('button', { name: '确认删除' }));
  await waitFor(() =>
    expect(within(list).queryByRole('link', { name: 'Second graph' })).not.toBeInTheDocument(),
  );
}

describe('relation graph favorites', () => {
  test('saves a new graph snapshot', savesNewGraph);
  test('flags, updates, and resets a changed graph', flagsUpdatesAndResetsChangedGraph);
  test('ignores layout drift of a reopened graph', ignoresLayoutDrift);
  test('asks before leaving and saves', asksBeforeLeavingAndSaves);
  test('leaves without saving', leavesWithoutSaving);
  test('guards closing the tab', guardsTabClose);
  test('unlinks a favorite deleted elsewhere', unlinksDeletedFavorite);
  test('manages saved graphs on the favorites page', managesSavedGraphs);
});
