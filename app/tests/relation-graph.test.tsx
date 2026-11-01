/**
 * Relation graph state: growing branches, cross links, collapsing, limits, radial layout, and
 * dragging.
 */

import { describe, expect, test } from 'vitest';

import type { Article } from '@/lib/api';
import {
  RELATION_BRANCH_SPAN,
  RELATION_NODE_HALF_HEIGHT,
  RELATION_NODE_HALF_WIDTH,
  RELATION_NODE_LIMIT,
  RELATION_ROOT_RADIUS,
  buildRelationHref,
  buildSavedRelationHref,
  collapseRelationNode,
  computeRelationSectors,
  createRelationGraph,
  expandRelationNode,
  listRelationNodes,
  normalizeRelationGraph,
  readStoredRelationGraph,
  relationGraphSignature,
  relationPathContext,
  relayoutRelationGraph,
  routeRelationCrossLink,
  storeRelationGraph,
  storeRelationPositions,
  type RelationGraph,
} from '@/lib/relation-graph';

/**
 * Build a minimal article.
 *
 * @param id - Article identifier.
 * @returns Article.
 */
function article(id: string): Article {
  return { article_id: id, journal_id: 'j', title: `Article ${id}`, abstract: '', authors: [] };
}

/** Expansion attaches new articles as children and keeps known ones as cross links. */
function growsBranchesWithoutDuplicates(): void {
  let graph = createRelationGraph(article('root'));
  graph = expandRelationNode(graph, 'root', [article('a'), article('b'), article('c')]);
  graph = expandRelationNode(graph, 'a', [
    article('root'),
    article('b'),
    article('d'),
    article('a'),
  ]);

  expect(graph.childIds.root).toEqual(['a', 'b', 'c']);
  expect(graph.childIds.a).toEqual(['d']);
  expect(graph.nodes.d).toMatchObject({ parentId: 'a', depth: 2 });
  expect(graph.crossLinks).toEqual([['a', 'b']]);
  expect(graph.expandedIds).toEqual(['root', 'a']);
  expect(listRelationNodes(graph).map((node) => node.id)).toEqual(['root', 'a', 'd', 'b', 'c']);
}

/** Collapsing removes the whole branch and the cross links that touched it. */
function collapsesWholeBranch(): void {
  let graph = createRelationGraph(article('root'));
  graph = expandRelationNode(graph, 'root', [article('a'), article('b')]);
  graph = expandRelationNode(graph, 'a', [article('d')]);
  graph = expandRelationNode(graph, 'd', [article('e'), article('b')]);

  const collapsed = collapseRelationNode(graph, 'a');

  expect(Object.keys(collapsed.nodes).sort()).toEqual(['a', 'b', 'root']);
  expect(collapsed.childIds.a).toEqual([]);
  expect(collapsed.expandedIds).toEqual(['root']);
  expect(collapsed.crossLinks).toEqual([]);
  expect(Object.keys(collapsed.positions).sort()).toEqual(['a', 'b', 'root']);
}

/** The graph stops growing at its node limit. */
function capsGraphSize(): void {
  const graph = expandRelationNode(
    createRelationGraph(article('root')),
    'root',
    Array.from({ length: RELATION_NODE_LIMIT + 10 }, (_, index) => article(`n${index}`)),
  );
  expect(Object.keys(graph.nodes)).toHaveLength(RELATION_NODE_LIMIT);
}

/**
 * Return the direction from one node to another.
 *
 * @param graph - Relation graph.
 * @param fromId - Origin node.
 * @param toId - Target node.
 * @returns Angle in radians.
 */
function angleBetween(graph: RelationGraph, fromId: string, toId: string): number {
  const from = graph.positions[fromId];
  const to = graph.positions[toId];
  return Math.atan2(to.y - from.y, to.x - from.x);
}

/**
 * Return the signed difference between two angles, normalized to (-pi, pi].
 *
 * @param first - First angle.
 * @param second - Second angle.
 * @returns Difference in radians.
 */
function angleDifference(first: number, second: number): number {
  const difference = (first - second) % (2 * Math.PI);
  if (difference > Math.PI) {
    return difference - 2 * Math.PI;
  }
  return difference <= -Math.PI ? difference + 2 * Math.PI : difference;
}

/**
 * Assert that a node's children sit at evenly spaced angles around it.
 *
 * @param graph - Relation graph.
 * @param parentId - Expanded node.
 * @param step - Expected angle between neighbouring children.
 */
function expectEvenAngles(graph: RelationGraph, parentId: string, step: number): void {
  const angles = graph.childIds[parentId].map((childId) => angleBetween(graph, parentId, childId));
  for (let index = 1; index < angles.length; index += 1) {
    expect(angleDifference(angles[index], angles[index - 1])).toBeCloseTo(step);
  }
}

/** The root's related articles are spread evenly all around it. */
function spreadsRootChildrenEvenly(): void {
  const graph = expandRelationNode(
    createRelationGraph(article('root')),
    'root',
    ['a', 'b', 'c', 'd', 'e'].map(article),
  );

  expectEvenAngles(graph, 'root', (2 * Math.PI) / 5);
  for (const childId of graph.childIds.root) {
    const { x, y } = graph.positions[childId];
    expect(Math.hypot(x, y)).toBeGreaterThan(RELATION_ROOT_RADIUS - 1e-6);
  }
}

/** An outer node's children fan out evenly away from its parent, and existing nodes stay put. */
function fansOuterBranchEvenly(): void {
  let graph = createRelationGraph(article('root'));
  graph = expandRelationNode(graph, 'root', ['a', 'b', 'c', 'd', 'e'].map(article));
  const firstLevel = structuredClone(graph.positions);
  graph = expandRelationNode(graph, 'b', ['b1', 'b2', 'b3', 'b4'].map(article));
  graph = expandRelationNode(graph, 'b4', ['x1', 'x2', 'x3', 'x4', 'x5'].map(article));

  for (const [parentId, grandparentId] of [
    ['b', 'root'],
    ['b4', 'b'],
  ]) {
    const count = graph.childIds[parentId].length;
    expectEvenAngles(graph, parentId, RELATION_BRANCH_SPAN / count);
    const outward = angleBetween(graph, grandparentId, parentId);
    for (const childId of graph.childIds[parentId]) {
      expect(
        Math.abs(angleDifference(angleBetween(graph, parentId, childId), outward)),
      ).toBeLessThan(RELATION_BRANCH_SPAN / 2);
    }
  }
  for (const [id, position] of Object.entries(firstLevel)) {
    expect(graph.positions[id]).toEqual(position);
  }
  const points = Object.values(graph.positions);
  for (let first = 0; first < points.length; first += 1) {
    for (let second = first + 1; second < points.length; second += 1) {
      const isOverlapping =
        Math.abs(points[first].x - points[second].x) < RELATION_NODE_HALF_WIDTH * 2 &&
        Math.abs(points[first].y - points[second].y) < RELATION_NODE_HALF_HEIGHT * 2;
      expect(isOverlapping).toBe(false);
    }
  }
}

/** Settled positions and pins are stored, and relayout restores the automatic seeds. */
function storesSettledPositionsAndPins(): void {
  let graph = createRelationGraph(article('root'));
  graph = expandRelationNode(graph, 'root', ['a', 'b', 'c'].map(article));
  graph = expandRelationNode(graph, 'a', ['a1', 'a2'].map(article));
  const automatic = structuredClone(graph.positions);

  graph = storeRelationPositions(graph, { a: { x: 400, y: 10 }, gone: { x: 1, y: 1 } }, 'a');
  graph = storeRelationPositions(graph, { a1: { x: 500, y: 40 } }, 'a');

  expect(graph.positions.a).toEqual({ x: 400, y: 10 });
  expect(graph.positions.a1).toEqual({ x: 500, y: 40 });
  expect(graph.positions.b).toEqual(automatic.b);
  expect(graph.positions).not.toHaveProperty('gone');
  expect(graph.pinnedIds).toEqual(['a']);

  graph = expandRelationNode(graph, 'a1', ['z'].map(article));
  expect(
    angleDifference(angleBetween(graph, 'a1', 'z'), angleBetween(graph, 'a', 'a1')),
  ).toBeCloseTo(0);
  expect(collapseRelationNode(graph, 'root').pinnedIds).toEqual([]);

  const relaid = relayoutRelationGraph(graph);
  expect(relaid.pinnedIds).toEqual([]);
  expect(relaid.layoutRevision).toBe(graph.layoutRevision + 1);
  for (const id of Object.keys(automatic)) {
    expect(relaid.positions[id].x).toBeCloseTo(automatic[id].x);
    expect(relaid.positions[id].y).toBeCloseTo(automatic[id].y);
  }
}

/** Branches get disjoint nested sectors that follow the current angular order of the nodes. */
function allocatesDisjointSectors(): void {
  let graph = createRelationGraph(article('root'));
  graph = expandRelationNode(graph, 'root', ['a', 'b', 'c', 'd'].map(article));
  const even = computeRelationSectors(graph, graph.positions);
  for (const id of ['a', 'b', 'c', 'd']) {
    expect(even[id].end - even[id].start).toBeCloseTo(Math.PI / 2);
    const angle = angleBetween(graph, 'root', id);
    const middle = (even[id].start + even[id].end) / 2;
    expect(Math.abs(angleDifference(angle, middle))).toBeLessThan(1e-6);
  }

  graph = expandRelationNode(graph, 'b', ['b1', 'b2', 'b3', 'b4'].map(article));
  const sectors = computeRelationSectors(graph, graph.positions);
  const firstLevel = ['a', 'b', 'c', 'd']
    .map((id) => sectors[id])
    .sort((first, second) => first.start - second.start);
  expect(firstLevel.at(-1)!.end - firstLevel[0].start).toBeCloseTo(2 * Math.PI);
  for (let index = 1; index < firstLevel.length; index += 1) {
    expect(firstLevel[index].start).toBeCloseTo(firstLevel[index - 1].end);
  }
  expect(sectors.b.end - sectors.b.start).toBeGreaterThan(sectors.a.end - sectors.a.start);
  const children = ['b1', 'b2', 'b3', 'b4']
    .map((id) => sectors[id])
    .sort((first, second) => first.start - second.start);
  expect(children[0].start).toBeCloseTo(sectors.b.start);
  expect(children.at(-1)!.end).toBeCloseTo(sectors.b.end);

  const swapped = { ...graph.positions, a: graph.positions.c, c: graph.positions.a };
  const reordered = computeRelationSectors(graph, swapped);
  expect(reordered.a.start).toBeCloseTo(sectors.c.start);
  expect(reordered.c.start).toBeCloseTo(sectors.a.start);
}

/** A child that drifts just past its parent's sector edge keeps the nearest slot. */
function keepsSlotsAcrossSectorEdges(): void {
  let graph = createRelationGraph(article('root'));
  graph = expandRelationNode(graph, 'root', ['a', 'b', 'c', 'd'].map(article));
  graph = expandRelationNode(graph, 'b', ['b1', 'b2', 'b3'].map(article));
  const sectors = computeRelationSectors(graph, graph.positions);
  const firstChild = ['b1', 'b2', 'b3'].find((id) => sectors[id].start === sectors.b.start)!;
  const { x, y } = graph.positions[firstChild];
  const radius = Math.hypot(x, y);
  const outside = sectors.b.start - 0.05;
  const drifted = {
    ...graph.positions,
    [firstChild]: { x: radius * Math.cos(outside), y: radius * Math.sin(outside) },
  };

  const next = computeRelationSectors(graph, drifted);

  expect(next[firstChild].start).toBeCloseTo(next.b.start);
}

/**
 * Check whether two sectors around the root touch, allowing for the wrap at a full turn.
 *
 * @param first - First sector.
 * @param second - Second sector.
 * @returns True when one sector ends where the other starts.
 */
function areSectorsAdjacent(
  first: { start: number; end: number },
  second: { start: number; end: number },
): boolean {
  const touches = (end: number, start: number) => Math.abs(angleDifference(end, start)) < 1e-6;
  return touches(first.end, second.start) || touches(second.end, first.start);
}

/**
 * Build a graph whose opposite first-level branches a and c are cross-linked through a's first
 * child, keeping the positions they had before the link appeared.
 *
 * @returns Graph with a cross link from c to a1.
 */
function buildCrossLinkedGraph(): RelationGraph {
  let graph = createRelationGraph(article('root'));
  graph = expandRelationNode(graph, 'root', ['a', 'b', 'c', 'd'].map(article));
  graph = expandRelationNode(graph, 'a', ['a1', 'a2', 'a3'].map(article));
  graph = expandRelationNode(graph, 'c', [article('c1')]);
  return { ...graph, crossLinks: [['c', 'a1']] };
}

/**
 * List first-level nodes in their angular order around the root.
 *
 * @param graph - Relation graph.
 * @returns First-level node ids sorted by angle.
 */
function listFirstLevelByAngle(graph: RelationGraph): string[] {
  return [...graph.childIds[graph.rootId]].sort(
    (first, second) => angleBetween(graph, 'root', first) - angleBetween(graph, 'root', second),
  );
}

/** Cross-linked branches move next to each other and the linked child turns toward its partner. */
function ordersCrossLinkedBranchesTogether(): void {
  const graph = buildCrossLinkedGraph();
  const plain = computeRelationSectors({ ...graph, crossLinks: [] }, graph.positions);
  expect(areSectorsAdjacent(plain.a, plain.c)).toBe(false);

  const sectors = computeRelationSectors(graph, graph.positions);

  expect(areSectorsAdjacent(sectors.a, sectors.c)).toBe(true);
  const middleOfA = (sectors.a.start + sectors.a.end) / 2;
  const middleOfC = (sectors.c.start + sectors.c.end) / 2;
  const facingEdge = angleDifference(middleOfC, middleOfA) < 0 ? sectors.a.start : sectors.a.end;
  const a1Edge = angleDifference(middleOfC, middleOfA) < 0 ? sectors.a1.start : sectors.a1.end;
  expect(Math.abs(angleDifference(a1Edge, facingEdge))).toBeLessThan(1e-6);

  const settled: RelationGraph['positions'] = { ...graph.positions };
  for (const [id, sector] of Object.entries(sectors)) {
    const radius = Math.hypot(graph.positions[id].x, graph.positions[id].y);
    const middle = (sector.start + sector.end) / 2;
    settled[id] = { x: radius * Math.cos(middle), y: radius * Math.sin(middle) };
  }
  const again = computeRelationSectors(graph, settled);
  for (const id of Object.keys(sectors)) {
    expect(Math.abs(angleDifference(again[id].start, sectors[id].start))).toBeLessThan(1e-6);
  }
}

/** A branch holding a fixed node keeps the reader's order even when cross links pull on it. */
function keepsFixedBranchesInPlace(): void {
  const graph = buildCrossLinkedGraph();

  const pinned = computeRelationSectors(graph, graph.positions, ['b']);
  const dragged = computeRelationSectors({ ...graph, pinnedIds: ['a2'] }, graph.positions);

  expect(areSectorsAdjacent(pinned.a, pinned.c)).toBe(false);
  expect(areSectorsAdjacent(dragged.a, dragged.c)).toBe(false);
}

/** Expanding into a cross link turns the linked branches next to each other in one step. */
function arrangesBranchesWhenCrossLinksAppear(): void {
  let graph = createRelationGraph(article('root'));
  graph = expandRelationNode(graph, 'root', ['a', 'b', 'c', 'd'].map(article));
  graph = expandRelationNode(graph, 'a', ['a1', 'a2', 'a3'].map(article));
  const before = listFirstLevelByAngle(graph);
  expect(Math.abs(before.indexOf('a') - before.indexOf('c'))).toBe(2);
  const branchShape = Math.hypot(
    graph.positions.a1.x - graph.positions.a.x,
    graph.positions.a1.y - graph.positions.a.y,
  );

  const linked = expandRelationNode(graph, 'c', [article('c1'), article('a1')]);

  const after = listFirstLevelByAngle(linked);
  expect([1, 3]).toContain(Math.abs(after.indexOf('a') - after.indexOf('c')));
  expect(linked.layoutRevision).toBe(graph.layoutRevision + 1);
  expect(
    Math.hypot(
      linked.positions.a1.x - linked.positions.a.x,
      linked.positions.a1.y - linked.positions.a.y,
    ),
  ).toBeCloseTo(branchShape);

  const pinned = expandRelationNode({ ...graph, pinnedIds: ['b'] }, 'c', [article('a1')]);
  expect(pinned.positions.a).toEqual(graph.positions.a);
  expect(pinned.layoutRevision).toBe(graph.layoutRevision);
}

/**
 * Cross links that would cut across tree edges run on a circle outside the nodes they pass;
 * links that cross nothing, touch the root, or join nearly aligned nodes stay direct.
 */
function routesCrossLinksAroundTheGraph(): void {
  const center = { x: 0, y: 0 };
  const source = { x: 200, y: 0 };
  const target = { x: 0, y: 200 };
  const passed = { x: 300 * Math.SQRT1_2, y: 300 * Math.SQRT1_2 };
  const behind = { x: -500, y: 0 };
  const points = [center, source, target, passed, behind];
  const treeEdges = [
    [center, source],
    [center, target],
    [center, passed],
    [center, behind],
  ] as const;

  const ring = routeRelationCrossLink([source, target], center, points, treeEdges);

  expect(ring).not.toBeNull();
  expect(ring!.radius).toBeGreaterThan(300);
  expect(ring!.radius).toBeLessThan(500);
  expect(ring!.startAngle).toBeCloseTo(0);
  expect(ring!.sweep).toBeCloseTo(Math.PI / 2);
  expect(
    routeRelationCrossLink([source, target], center, points, treeEdges.slice(0, 2)),
  ).toBeNull();
  expect(routeRelationCrossLink([source, center], center, points, treeEdges)).toBeNull();
  const childOfSource = { x: 100, y: -50 };
  expect(
    routeRelationCrossLink([{ x: 200, y: -0.5 }, target], center, points, [
      [source, childOfSource],
    ]),
  ).toBeNull();
  expect(routeRelationCrossLink([source, { x: 220, y: 10 }], center, points, treeEdges)).toBeNull();
}

/** Expanding a deeper node keeps the whole first level and every ancestor's children in view. */
function keepsUpperLevelsInView(): void {
  let graph = createRelationGraph(article('root'));
  graph = expandRelationNode(graph, 'root', ['a', 'b', 'c'].map(article));
  graph = expandRelationNode(graph, 'a', ['a1'].map(article));
  graph = expandRelationNode(graph, 'b', ['b1', 'b2'].map(article));
  graph = expandRelationNode(graph, 'b1', ['x', 'y'].map(article));

  expect(relationPathContext(graph, 'b1')).toEqual(['root', 'a', 'b', 'c', 'b1', 'b2', 'x', 'y']);
  expect(relationPathContext(graph, 'root')).toEqual(['root', 'a', 'b', 'c']);
  expect(relationPathContext(graph, 'missing')).toEqual([]);
}

/** The workspace URL carries the center article and its database. */
function buildsWorkspaceHref(): void {
  expect(buildRelationHref({ article_id: 'x 1' }, 'chinese_journals.sqlite')).toBe(
    '/?view=relations&db=chinese_journals.sqlite&article=x+1',
  );
}

/** A saved graph reopens through a URL that carries its favorite id. */
function buildsSavedGraphHref(): void {
  expect(
    buildSavedRelationHref({ db_name: 'chinese_journals.sqlite', id: 7, root_article_id: 42 }),
  ).toBe('/?view=relations&db=chinese_journals.sqlite&article=42&saved=7');
}

/** Saved snapshots are untrusted: unusable ones are rejected and missing fields are filled. */
function normalizesSavedSnapshots(): void {
  const graph = expandRelationNode(
    createRelationGraph(article('root')),
    'root',
    ['a', 'b'].map(article),
  );

  expect(normalizeRelationGraph(null)).toBeNull();
  expect(normalizeRelationGraph([graph])).toBeNull();
  expect(normalizeRelationGraph({ ...graph, rootId: 'missing' })).toBeNull();
  expect(normalizeRelationGraph({ ...graph, expandedIds: undefined })).toBeNull();

  const legacy: Partial<RelationGraph> = JSON.parse(JSON.stringify(graph));
  delete legacy.pinnedIds;
  delete legacy.layoutRevision;
  const restored = normalizeRelationGraph(legacy);
  expect(restored?.pinnedIds).toEqual([]);
  expect(restored?.layoutRevision).toBe(0);
  expect(restored?.positions).toEqual(graph.positions);

  const relaidOut = normalizeRelationGraph({ ...graph, positions: {} });
  expect(Object.keys(relaidOut?.positions ?? {}).sort()).toEqual(['a', 'b', 'root']);
}

/** The session copy remembers which saved favorite the graph belongs to. */
function remembersSavedFavoriteInSession(): void {
  const graph = createRelationGraph(article('root'));
  storeRelationGraph({ dbName: 'fixture.sqlite', graph, savedId: 9 });
  expect(readStoredRelationGraph()).toMatchObject({ dbName: 'fixture.sqlite', savedId: 9 });

  storeRelationGraph({ dbName: 'fixture.sqlite', graph });
  expect(readStoredRelationGraph()?.savedId).toBeNull();
  window.sessionStorage.clear();
}

/** Only deliberate changes alter the signature; free layout drift does not. */
function signsDeliberateChangesOnly(): void {
  const graph = expandRelationNode(
    createRelationGraph(article('root')),
    'root',
    ['a', 'b'].map(article),
  );
  const signature = relationGraphSignature(graph);
  const drifted = { ...graph, positions: { ...graph.positions, a: { x: 999, y: 999 } } };

  expect(relationGraphSignature(drifted)).toBe(signature);
  expect(relationGraphSignature(expandRelationNode(graph, 'a', [article('c')]))).not.toBe(
    signature,
  );
  expect(relationGraphSignature(collapseRelationNode(graph, 'root'))).not.toBe(signature);
  const dragged = storeRelationPositions(graph, { a: { x: 10, y: 20 } }, 'a');
  expect(relationGraphSignature(dragged)).not.toBe(signature);
  const draggedAgain = storeRelationPositions(dragged, { a: { x: 40, y: 20 } }, 'a');
  expect(relationGraphSignature(draggedAgain)).not.toBe(relationGraphSignature(dragged));
}

describe('relation graph', () => {
  test('grows branches without duplicating articles', growsBranchesWithoutDuplicates);
  test('collapses a whole branch', collapsesWholeBranch);
  test('caps the graph size', capsGraphSize);
  test('spreads the root children evenly', spreadsRootChildrenEvenly);
  test('fans outer branches evenly without moving or overlapping nodes', fansOuterBranchEvenly);
  test('stores settled positions and pins', storesSettledPositionsAndPins);
  test('allocates disjoint sectors in angular order', allocatesDisjointSectors);
  test('keeps child slots across sector edges', keepsSlotsAcrossSectorEdges);
  test('orders cross-linked branches together', ordersCrossLinkedBranchesTogether);
  test('keeps fixed branches in place', keepsFixedBranchesInPlace);
  test('arranges branches when cross links appear', arrangesBranchesWhenCrossLinksAppear);
  test('routes cross links around the graph', routesCrossLinksAroundTheGraph);
  test('keeps upper levels in view after expanding', keepsUpperLevelsInView);
  test('builds the workspace href', buildsWorkspaceHref);
  test('builds the saved graph href', buildsSavedGraphHref);
  test('normalizes saved snapshots', normalizesSavedSnapshots);
  test('remembers the saved favorite in the session', remembersSavedFavoriteInSession);
  test('signs deliberate changes only', signsDeliberateChangesOnly);
});
