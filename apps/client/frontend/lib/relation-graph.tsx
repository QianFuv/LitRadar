/**
 * Pure state and layout helpers for the article relation graph.
 *
 * The graph is a tree grown from one root article: expanding a node attaches its related articles
 * as children. Articles that are already in the tree are not duplicated; the relation is kept as a
 * cross link instead.
 *
 * New nodes are seeded radially: the root's children are spread evenly around it, and an expanded
 * node's children are spread evenly across a fan that points away from its parent. Siblings are
 * ordered so cross-linked branches sit side by side. The canvas then refines the seeds with a
 * force simulation and reports settled positions back, so the stored positions always describe
 * the last settled picture.
 */

import { z } from 'zod';

import type { Article } from '@/lib/api';
import { parseStoredJson } from '@/lib/browser-storage';
import type { WorkspaceSearch } from '@/lib/search-params';
import { isFiniteNumber, isRecord, isString } from '@/lib/type-guards';

/** Number of related articles requested when a node is expanded. */
export const RELATION_CHILD_LIMIT = 6;

/** Maximum number of articles kept in one graph. */
export const RELATION_NODE_LIMIT = 80;

/** Minimum distance from the root to its children in flow coordinates. */
export const RELATION_ROOT_RADIUS = 200;

/** Minimum distance from an expanded node to its children in flow coordinates. */
const RELATION_BRANCH_RADIUS = 180;

/** Angle of the fan that holds a non-root node's children, centred away from its parent. */
export const RELATION_BRANCH_SPAN = (4 * Math.PI) / 3;

/**
 * Half extents of the area a node occupies around its dot, label below included, used to keep
 * nodes apart.
 */
export const RELATION_NODE_HALF_WIDTH = 80;
export const RELATION_NODE_HALF_HEIGHT = 26;

/** Minimum distance between neighbouring children of one node. */
const RELATION_SIBLING_SPACING = 150;

/** Distance a child moves outward when its slot overlaps another node. */
const RELATION_RADIUS_STEP = 40;

/** Number of outward moves tried before a child keeps an overlapping slot. */
const RELATION_RADIUS_ATTEMPTS = 8;

/**
 * Largest offset from a sector middle, in radians, at which a node outside the branch still
 * counts as lying on one side of it; beyond this it sits behind the branch and pulls neither way.
 */
const RELATION_FACING_LIMIT = 0.9 * Math.PI;

/** Smallest angle around the root a cross link must span before it is routed around the graph. */
const RELATION_CROSS_LINK_MIN_SWEEP = 0.2;

/**
 * Distance within which segment ends count as meeting at the same node; edge handles sit a
 * fraction of a pixel away from the dot positions the tree edges are measured from.
 */
const RELATION_SHARED_END_TOLERANCE = 2;

/** Clearance between a routed cross link and the farthest node it passes. */
const RELATION_CROSS_LINK_MARGIN = 60;

/** One article placed in the relation tree. */
export type RelationNode = {
  id: string;
  article: Article;
  parentId: string | null;
  depth: number;
};

/** Flow coordinates of a node's dot. */
export type RelationPoint = {
  x: number;
  y: number;
};

/** Node positions keyed by node id. */
export type RelationPositions = Record<string, RelationPoint>;

/** Angular range around the root, in radians, that a branch may occupy; `end` is above `start`. */
type RelationSector = {
  start: number;
  end: number;
};

/** Branch sectors keyed by node id. */
type RelationSectors = Record<string, RelationSector>;

/** Serializable relation tree with expansion state, cross links, and node positions. */
export type RelationGraph = {
  rootId: string;
  nodes: Record<string, RelationNode>;
  childIds: Record<string, string[]>;
  expandedIds: string[];
  crossLinks: Array<[string, string]>;
  positions: RelationPositions;
  /** Nodes the reader dropped in place; the simulation keeps them fixed. */
  pinnedIds: string[];
  /** Incremented when every position is recomputed, so the canvas adopts the new layout. */
  layoutRevision: number;
};

/**
 * Build the relation-graph workspace search state centred on one article.
 *
 * @param article - Center article.
 * @param dbName - Database that contains the article.
 * @returns Root workspace search state.
 */
export function buildRelationSearch(article: Pick<Article, 'article_id'>, dbName: string) {
  return {
    view: 'relations',
    db: dbName,
    article: relationNodeId(article),
  } satisfies WorkspaceSearch;
}

/**
 * Build the relation-graph workspace search state that reopens a saved graph.
 *
 * @param favorite - Saved graph database, root article, and id.
 * @returns Workspace search state carrying the saved graph id.
 */
export function buildSavedRelationSearch(favorite: {
  db_name: string;
  id: number;
  root_article_id: string;
}) {
  return {
    view: 'relations',
    db: favorite.db_name,
    article: favorite.root_article_id,
    saved: favorite.id,
  } satisfies WorkspaceSearch;
}

/**
 * Return the graph key of an article.
 *
 * @param article - Article.
 * @returns Stable string identifier.
 */
export function relationNodeId(article: Pick<Article, 'article_id'>): string {
  return String(article.article_id);
}

/**
 * Start a graph containing only the root article.
 *
 * @param root - Root article.
 * @returns Graph with one node.
 */
export function createRelationGraph(root: Article): RelationGraph {
  const rootId = relationNodeId(root);
  return {
    rootId,
    nodes: { [rootId]: { id: rootId, article: root, parentId: null, depth: 0 } },
    childIds: { [rootId]: [] },
    expandedIds: [],
    crossLinks: [],
    positions: { [rootId]: { x: 0, y: 0 } },
    pinnedIds: [],
    layoutRevision: 0,
  };
}

/**
 * Check whether two nodes are already linked by the tree or a cross link.
 *
 * @param graph - Relation graph.
 * @param firstId - First node.
 * @param secondId - Second node.
 * @returns True when an edge already connects them.
 */
function isLinked(graph: RelationGraph, firstId: string, secondId: string): boolean {
  const first = graph.nodes[firstId];
  const second = graph.nodes[secondId];
  return (
    first?.parentId === secondId ||
    second?.parentId === firstId ||
    graph.crossLinks.some(
      ([left, right]) =>
        (left === firstId && right === secondId) || (left === secondId && right === firstId),
    )
  );
}

/**
 * Check whether a node at one point would overlap a node at another point.
 *
 * @param first - First dot position.
 * @param second - Second dot position.
 * @returns True when their occupied areas intersect.
 */
function isOverlapping(first: RelationPoint, second: RelationPoint): boolean {
  return (
    Math.abs(first.x - second.x) < RELATION_NODE_HALF_WIDTH * 2 &&
    Math.abs(first.y - second.y) < RELATION_NODE_HALF_HEIGHT * 2
  );
}

/**
 * Return the radius at which slots one angle step apart keep the sibling spacing.
 *
 * @param minimum - Smallest radius to use.
 * @param step - Angle between neighbouring slots.
 * @param count - Number of slots.
 * @returns Radius in flow coordinates.
 */
function slotRadius(minimum: number, step: number, count: number): number {
  return count < 2
    ? minimum
    : Math.max(minimum, RELATION_SIBLING_SPACING / (2 * Math.sin(step / 2)));
}

/**
 * Place new children around their parent at evenly spaced angles: all around the root, or across
 * a fan pointing away from the grandparent for deeper nodes. A child whose slot overlaps another
 * node moves outward along its own angle, so the angles stay even and no existing node moves.
 *
 * @param graph - Graph that already contains the children.
 * @param positions - Positions of the nodes placed so far.
 * @param parentId - Expanded node.
 * @param childIds - Children to place, in slot order.
 * @returns Positions including the new children.
 */
function placeRelationChildren(
  graph: RelationGraph,
  positions: RelationPositions,
  parentId: string,
  childIds: string[],
) {
  const placed = { ...positions };
  const center = placed[parentId];
  if (!center || childIds.length === 0) {
    return placed;
  }
  const grandparentId = graph.nodes[parentId]?.parentId ?? null;
  const grandparent = grandparentId === null ? undefined : placed[grandparentId];
  const count = childIds.length;
  let firstAngle: number;
  let step: number;
  let radius: number;
  if (!grandparent) {
    step = (2 * Math.PI) / count;
    firstAngle = -Math.PI / 2;
    radius = slotRadius(RELATION_ROOT_RADIUS, step, count);
  } else {
    const outward =
      center.x === grandparent.x && center.y === grandparent.y
        ? -Math.PI / 2
        : Math.atan2(center.y - grandparent.y, center.x - grandparent.x);
    step = RELATION_BRANCH_SPAN / count;
    firstAngle = outward - RELATION_BRANCH_SPAN / 2 + step / 2;
    radius = slotRadius(RELATION_BRANCH_RADIUS, step, count);
  }
  const others = Object.values(placed);
  childIds.forEach((childId, index) => {
    const angle = firstAngle + index * step;
    let point = center;
    for (let attempt = 0; attempt <= RELATION_RADIUS_ATTEMPTS; attempt += 1) {
      const distance = radius + attempt * RELATION_RADIUS_STEP;
      point = {
        x: center.x + distance * Math.cos(angle),
        y: center.y + distance * Math.sin(angle),
      };
      if (!others.some((other) => isOverlapping(point, other))) {
        break;
      }
    }
    placed[childId] = point;
    others.push(point);
  });
  return placed;
}

/**
 * Attach the related articles of an expanded node and place them around it. When the expansion
 * adds cross links that call for a different branch order, the reordered branches turn into
 * their new sectors and the layout revision advances so the canvas adopts the moved positions.
 *
 * @param graph - Relation graph.
 * @param parentId - Expanded node.
 * @param related - Related articles, most similar first.
 * @returns Updated graph; unchanged when the parent is no longer in the graph.
 */
export function expandRelationNode(
  graph: RelationGraph,
  parentId: string,
  related: Article[],
): RelationGraph {
  const parent = graph.nodes[parentId];
  if (!parent) {
    return graph;
  }
  const nodes = { ...graph.nodes };
  const childIds = { ...graph.childIds, [parentId]: [...(graph.childIds[parentId] ?? [])] };
  const next: RelationGraph = {
    ...graph,
    nodes,
    childIds,
    expandedIds: graph.expandedIds.includes(parentId)
      ? graph.expandedIds
      : [...graph.expandedIds, parentId],
    crossLinks: [...graph.crossLinks],
  };
  const addedIds: string[] = [];
  for (const article of related) {
    const id = relationNodeId(article);
    if (id === parentId) {
      continue;
    }
    if (nodes[id]) {
      if (!isLinked(next, parentId, id)) {
        next.crossLinks.push([parentId, id]);
      }
      continue;
    }
    if (Object.keys(nodes).length >= RELATION_NODE_LIMIT) {
      break;
    }
    nodes[id] = { id, article, parentId, depth: parent.depth + 1 };
    childIds[id] = [];
    childIds[parentId].push(id);
    addedIds.push(id);
  }
  next.positions = placeRelationChildren(next, graph.positions, parentId, addedIds);
  if (next.crossLinks.length > graph.crossLinks.length) {
    const arranged = arrangeRelationBranches(next, next.positions);
    if (arranged) {
      next.positions = arranged;
      next.layoutRevision = (graph.layoutRevision ?? 0) + 1;
    }
  }
  return next;
}

/**
 * Remove a node's descendants and the cross links that touched them.
 *
 * @param graph - Relation graph.
 * @param nodeId - Node to collapse.
 * @returns Updated graph.
 */
export function collapseRelationNode(graph: RelationGraph, nodeId: string): RelationGraph {
  const removed = new Set(listRelationBranch(graph, nodeId).filter((id) => id !== nodeId));
  const nodes: Record<string, RelationNode> = {};
  const childIds: Record<string, string[]> = {};
  const positions: RelationPositions = {};
  for (const [id, node] of Object.entries(graph.nodes)) {
    if (!removed.has(id)) {
      nodes[id] = node;
      childIds[id] = id === nodeId ? [] : (graph.childIds[id] ?? []);
      if (graph.positions[id]) {
        positions[id] = graph.positions[id];
      }
    }
  }
  return {
    ...graph,
    nodes,
    childIds,
    expandedIds: graph.expandedIds.filter((id) => id !== nodeId && !removed.has(id)),
    crossLinks: graph.crossLinks.filter(
      ([left, right]) => !removed.has(left) && !removed.has(right),
    ),
    positions,
    pinnedIds: graph.pinnedIds.filter((id) => !removed.has(id)),
  };
}

/**
 * Recompute every position from scratch, replaying expansions in the order they happened and
 * ordering branches for their cross links, and release every pin. Used to undo manual dragging
 * and to lay out graphs stored without positions.
 *
 * @param graph - Relation graph.
 * @returns Graph with fresh automatic positions and a new layout revision.
 */
export function relayoutRelationGraph(graph: RelationGraph): RelationGraph {
  let positions = { [graph.rootId]: { x: 0, y: 0 } };
  const order = [graph.rootId, ...graph.expandedIds.filter((id) => id !== graph.rootId)];
  for (const id of order) {
    positions = placeRelationChildren(graph, positions, id, graph.childIds[id] ?? []);
  }
  const unpinned = { ...graph, pinnedIds: [] };
  return {
    ...unpinned,
    positions: arrangeRelationBranches(unpinned, positions) ?? positions,
    pinnedIds: [],
    layoutRevision: (graph.layoutRevision ?? 0) + 1,
  };
}

/**
 * Turn every branch whose place in its sibling order changed into its new sector in one step, so
 * branches that cross-link pulls far around the graph do not have to travel there through the
 * force layout. Each moved branch rotates around the root as a whole, keeping its shape; branches
 * holding a pinned node keep their order and never move.
 *
 * @param graph - Relation graph.
 * @param positions - Current positions.
 * @returns New positions, or null when every sibling order is already in place.
 */
function arrangeRelationBranches(
  graph: RelationGraph,
  positions: RelationPositions,
): RelationPositions | null {
  const center = positions[graph.rootId];
  if (!center) {
    return null;
  }
  const sectors = computeRelationSectors(graph, positions);
  const angleOf = (id: string) =>
    Math.atan2(positions[id].y - center.y, positions[id].x - center.x);
  const middleOf = (id: string) => (sectors[id].start + sectors[id].end) / 2;
  const rotations = new Map<string, number>([[graph.rootId, 0]]);
  let isChanged = false;
  const pending = [graph.rootId];
  while (pending.length > 0) {
    const parentId = pending.shift()!;
    const inherited = rotations.get(parentId) ?? 0;
    const children = (graph.childIds[parentId] ?? []).filter((id) => positions[id] && sectors[id]);
    const isCircular = parentId === graph.rootId;
    const reference = isCircular ? 0 : middleOf(parentId);
    const sortBy = (angleFor: (id: string) => number) =>
      [...children].sort(
        (first, second) =>
          relationAngleOffset(angleFor(first), reference) -
          relationAngleOffset(angleFor(second), reference),
      );
    const current = sortBy((id) => angleOf(id) + inherited);
    const target = sortBy(middleOf);
    const isReordered = countDisplaced(current, target, isCircular) > 0;
    for (const id of children) {
      const rotation = isReordered
        ? inherited + relationAngleOffset(middleOf(id), angleOf(id) + inherited)
        : inherited;
      rotations.set(id, rotation);
      pending.push(id);
    }
    isChanged ||= isReordered;
  }
  if (!isChanged) {
    return null;
  }
  const arranged = { ...positions };
  for (const [id, rotation] of rotations) {
    const point = positions[id];
    if (point && rotation !== 0) {
      const deltaX = point.x - center.x;
      const deltaY = point.y - center.y;
      arranged[id] = {
        x: center.x + deltaX * Math.cos(rotation) - deltaY * Math.sin(rotation),
        y: center.y + deltaX * Math.sin(rotation) + deltaY * Math.cos(rotation),
      };
    }
  }
  return arranged;
}

/**
 * Collect a node and all of its descendants.
 *
 * @param graph - Relation graph.
 * @param nodeId - Branch root.
 * @returns Node ids in the branch, the branch root first.
 */
function listRelationBranch(graph: RelationGraph, nodeId: string): string[] {
  const ids: string[] = [];
  const pending = [nodeId];
  while (pending.length > 0) {
    const id = pending.pop()!;
    if (graph.nodes[id]) {
      ids.push(id);
      pending.push(...(graph.childIds[id] ?? []));
    }
  }
  return ids;
}

/**
 * Store positions reported by the canvas for nodes that are still in the graph, optionally
 * pinning the node the reader just dropped.
 *
 * @param graph - Relation graph.
 * @param positions - Settled positions by node id.
 * @param pinnedId - Node to keep fixed from now on.
 * @returns Graph with the new positions.
 */
export function storeRelationPositions(
  graph: RelationGraph,
  positions: RelationPositions,
  pinnedId?: string,
): RelationGraph {
  const next = { ...graph.positions };
  for (const [id, position] of Object.entries(positions)) {
    if (graph.nodes[id]) {
      next[id] = position;
    }
  }
  const pinnedIds =
    pinnedId && graph.nodes[pinnedId] && !graph.pinnedIds.includes(pinnedId)
      ? [...graph.pinnedIds, pinnedId]
      : graph.pinnedIds;
  return { ...graph, positions: next, pinnedIds };
}

/**
 * Normalize an angle to [0, 2pi).
 *
 * @param angle - Angle in radians.
 * @returns Equivalent angle in [0, 2pi).
 */
function normalizeAngle(angle: number): number {
  const turn = 2 * Math.PI;
  return ((angle % turn) + turn) % turn;
}

/**
 * Return the signed angle from a reference direction to an angle, in (-pi, pi], so directions on
 * either side of the reference stay close to zero instead of wrapping to a full turn.
 *
 * @param angle - Angle in radians.
 * @param reference - Reference angle in radians.
 * @returns Signed difference in radians.
 */
export function relationAngleOffset(angle: number, reference: number): number {
  const offset = normalizeAngle(angle - reference);
  return offset > Math.PI ? offset - 2 * Math.PI : offset;
}

/**
 * Count the leaves below a node, treating a childless node as one leaf.
 *
 * @param graph - Relation graph.
 * @param nodeId - Subtree root.
 * @param cache - Memoized counts.
 * @returns Leaf count.
 */
function countLeaves(graph: RelationGraph, nodeId: string, cache: Map<string, number>): number {
  const cached = cache.get(nodeId);
  if (cached !== undefined) {
    return cached;
  }
  const children = graph.childIds[nodeId] ?? [];
  const count =
    children.length === 0
      ? 1
      : children.reduce((total, childId) => total + countLeaves(graph, childId, cache), 0);
  cache.set(nodeId, count);
  return count;
}

/**
 * Cross links that shape sibling order: pairs of siblings whose branches are linked, keyed by
 * their parent, and children that hold a link to a node outside their parent's branch, keyed by
 * that parent.
 */
type RelationLinkPulls = {
  pairs: Map<string, Array<[string, string]>>;
  facings: Map<string, Array<{ childId: string; targetId: string }>>;
};

/**
 * List the tree path from the root to a node.
 *
 * @param graph - Relation graph.
 * @param nodeId - Node.
 * @returns Node ids from the root to the node; empty when the node is not in the graph.
 */
function relationAncestry(graph: RelationGraph, nodeId: string): string[] {
  const path: string[] = [];
  for (let id: string | null = nodeId; id !== null; id = graph.nodes[id]?.parentId ?? null) {
    if (!graph.nodes[id]) {
      return [];
    }
    path.unshift(id);
  }
  return path;
}

/**
 * Translate every cross link into ordering pulls. Below the lowest common ancestor of its two
 * ends, a link asks that ancestor to keep the two branches next to each other, and asks every
 * node on the way down to either end to turn the child leading to that end toward the other end.
 * Links from a node to its own ancestor or descendant cannot be shortened by ordering and are
 * ignored.
 *
 * @param graph - Relation graph.
 * @returns Sibling pairs and facing requests.
 */
function collectRelationLinkPulls(graph: RelationGraph): RelationLinkPulls {
  const pulls: RelationLinkPulls = { pairs: new Map(), facings: new Map() };
  for (const [firstId, secondId] of graph.crossLinks) {
    const firstPath = relationAncestry(graph, firstId);
    const secondPath = relationAncestry(graph, secondId);
    let shared = 0;
    while (
      shared < firstPath.length &&
      shared < secondPath.length &&
      firstPath[shared] === secondPath[shared]
    ) {
      shared += 1;
    }
    if (shared === 0 || shared === firstPath.length || shared === secondPath.length) {
      continue;
    }
    const ancestorId = firstPath[shared - 1];
    const pairs = pulls.pairs.get(ancestorId) ?? [];
    pairs.push([firstPath[shared], secondPath[shared]]);
    pulls.pairs.set(ancestorId, pairs);
    for (const [ancestry, targetId] of [
      [firstPath, secondId],
      [secondPath, firstId],
    ] as const) {
      for (let index = shared; index < ancestry.length - 1; index += 1) {
        const facings = pulls.facings.get(ancestry[index]) ?? [];
        facings.push({ childId: ancestry[index + 1], targetId });
        pulls.facings.set(ancestry[index], facings);
      }
    }
  }
  return pulls;
}

/**
 * List every ordering of a few items.
 *
 * @param items - Items to arrange.
 * @returns All permutations.
 */
function listPermutations(items: string[]): string[][] {
  if (items.length <= 1) {
    return [items];
  }
  return items.flatMap((item, index) =>
    listPermutations([...items.slice(0, index), ...items.slice(index + 1)]).map((rest) => [
      item,
      ...rest,
    ]),
  );
}

/**
 * Count the siblings an order moves away from the current order, ignoring rotation on a circle.
 *
 * @param current - Current order.
 * @param order - Candidate order.
 * @param isCircular - Whether the first and last slots are neighbours.
 * @returns Number of displaced siblings.
 */
function countDisplaced(current: string[], order: string[], isCircular: boolean): number {
  const count = current.length;
  let fewest = count;
  for (let shift = 0; shift < (isCircular ? count : 1); shift += 1) {
    let displaced = 0;
    for (let index = 0; index < count; index += 1) {
      if (order[(index + shift) % count] !== current[index]) {
        displaced += 1;
      }
    }
    fewest = Math.min(fewest, displaced);
  }
  return fewest;
}

/**
 * Pick the sibling order that keeps cross-linked branches closest together and turns linked
 * children toward the side they link to, preferring the current order among equally good ones so
 * the layout settles instead of flipping.
 *
 * @param current - Siblings in their current angular order.
 * @param pairs - Sibling pairs whose branches are cross-linked.
 * @param preferredSlots - Siblings that should sit at a given slot.
 * @param isCircular - Whether the siblings ring the root, making the first and last neighbours.
 * @returns Chosen order.
 */
function chooseSiblingOrder(
  current: string[],
  pairs: Array<[string, string]>,
  preferredSlots: Array<{ childId: string; slot: number }>,
  isCircular: boolean,
): string[] {
  const count = current.length;
  const members = new Set(current);
  const relevantPairs = pairs.filter(
    ([first, second]) => members.has(first) && members.has(second),
  );
  if (
    count < 2 ||
    count > RELATION_CHILD_LIMIT ||
    (relevantPairs.length === 0 && preferredSlots.length === 0)
  ) {
    return current;
  }
  let best = current;
  let bestCost = Infinity;
  let bestDisplaced = Infinity;
  for (const order of listPermutations(current)) {
    const slotOf = new Map(order.map((id, index) => [id, index]));
    let cost = 0;
    for (const [first, second] of relevantPairs) {
      const distance = Math.abs(slotOf.get(first)! - slotOf.get(second)!);
      cost += isCircular ? Math.min(distance, count - distance) : distance;
    }
    for (const { childId, slot } of preferredSlots) {
      cost += Math.abs(slotOf.get(childId)! - slot);
    }
    const displaced = countDisplaced(current, order, isCircular);
    if (cost < bestCost || (cost === bestCost && displaced < bestDisplaced)) {
      best = order;
      bestCost = cost;
      bestDisplaced = displaced;
    }
  }
  return best;
}

/**
 * Give every branch a disjoint angular sector around the root, so tree edges cannot cross.
 *
 * The root's children share the full circle and each node's children share their parent's
 * sector. Sector sizes follow the square root of each branch's leaf count, which keeps the first
 * level close to even while an expanded branch gets room for its fan. The first level is rotated
 * to match the current angles as closely as possible.
 *
 * Siblings are reordered so cross-linked branches sit side by side and linked children face the
 * branch they link to, which keeps dashed cross links from cutting across other branches. Among
 * equally good orders the current angular order wins, so a dragged branch moves its sector along.
 * Siblings whose branch holds a fixed node keep their current order, so a branch the reader placed
 * stays where it was put.
 *
 * @param graph - Relation graph.
 * @param positions - Current dot positions by node id.
 * @param fixedIds - Nodes held in place; defaults to the pinned nodes.
 * @returns Sector of every non-root node that has a position.
 */
export function computeRelationSectors(
  graph: RelationGraph,
  positions: RelationPositions,
  fixedIds: Iterable<string> = graph.pinnedIds,
): RelationSectors {
  const center = positions[graph.rootId];
  const sectors = new Map<string, RelationSector>();
  if (!center) {
    return Object.fromEntries(sectors);
  }
  const leaves = new Map<string, number>();
  const angleOf = (id: string) => {
    const position = positions[id];
    return position ? Math.atan2(position.y - center.y, position.x - center.x) : 0;
  };
  const weightOf = (id: string) => Math.sqrt(countLeaves(graph, id, leaves));
  const pulls = collectRelationLinkPulls(graph);
  const anchoredIds = new Set<string>();
  for (const fixedId of fixedIds) {
    if (fixedId !== graph.rootId) {
      for (const id of relationAncestry(graph, fixedId)) {
        anchoredIds.add(id);
      }
    }
  }

  /**
   * Order siblings for their cross links unless one of their branches holds a fixed node.
   *
   * @param parentId - Parent of the siblings.
   * @param current - Siblings in their current angular order.
   * @param middle - Middle angle of the parent's sector, or null around the root.
   * @returns Sibling order to lay out.
   */
  function orderSiblings(parentId: string, current: string[], middle: number | null): string[] {
    if (current.some((id) => anchoredIds.has(id))) {
      return current;
    }
    const preferredSlots: Array<{ childId: string; slot: number }> = [];
    if (middle !== null) {
      for (const { childId, targetId } of pulls.facings.get(parentId) ?? []) {
        if (!positions[targetId]) {
          continue;
        }
        const offset = relationAngleOffset(angleOf(targetId), middle);
        if (Math.abs(offset) <= RELATION_FACING_LIMIT) {
          preferredSlots.push({ childId, slot: offset < 0 ? 0 : current.length - 1 });
        }
      }
    }
    return chooseSiblingOrder(
      current,
      pulls.pairs.get(parentId) ?? [],
      preferredSlots,
      middle === null,
    );
  }

  /**
   * Split a sector between children in the order of their signed angles from the sector middle,
   * which stays continuous when a child drifts just outside either edge.
   *
   * @param parentId - Node whose children share the sector.
   * @param sector - Sector to split.
   */
  function divide(parentId: string, sector: RelationSector): void {
    const children = (graph.childIds[parentId] ?? []).filter((id) => positions[id]);
    if (children.length === 0) {
      return;
    }
    const middle = (sector.start + sector.end) / 2;
    const ordered = orderSiblings(
      parentId,
      children
        .map((id) => ({ id, offset: relationAngleOffset(angleOf(id), middle) }))
        .sort((first, second) => first.offset - second.offset)
        .map((child) => child.id),
      middle,
    );
    const totalWeight = ordered.reduce((total, id) => total + weightOf(id), 0);
    let start = sector.start;
    for (const id of ordered) {
      const end = start + ((sector.end - sector.start) * weightOf(id)) / totalWeight;
      const childSector = { start, end };
      sectors.set(id, childSector);
      divide(id, childSector);
      start = end;
    }
  }

  const firstLevel = (graph.childIds[graph.rootId] ?? []).filter((id) => positions[id]);
  if (firstLevel.length === 0) {
    return Object.fromEntries(sectors);
  }
  const ordered = orderSiblings(
    graph.rootId,
    firstLevel
      .map((id) => ({ id, angle: normalizeAngle(angleOf(id)) }))
      .sort((first, second) => first.angle - second.angle)
      .map((child) => child.id),
    null,
  ).map((id) => ({ id, angle: normalizeAngle(angleOf(id)) }));
  const totalWeight = ordered.reduce((total, child) => total + weightOf(child.id), 0);
  let cumulative = 0;
  let sine = 0;
  let cosine = 0;
  const spans = ordered.map((child) => {
    const span = (2 * Math.PI * weightOf(child.id)) / totalWeight;
    const offset = child.angle - (cumulative + span / 2);
    sine += Math.sin(offset);
    cosine += Math.cos(offset);
    cumulative += span;
    return span;
  });
  let start = Math.atan2(sine, cosine);
  ordered.forEach((child, index) => {
    const sector = { start, end: start + spans[index] };
    sectors.set(child.id, sector);
    divide(child.id, sector);
    start += spans[index];
  });
  return Object.fromEntries(sectors);
}

/** Circular route of a cross link around the graph, centred on the root. */
export type RelationCrossLinkRing = {
  /** Distance of the route from the root. */
  radius: number;
  /** Angle of the source end around the root. */
  startAngle: number;
  /** Signed angle from the source end to the target end, the shorter way around. */
  sweep: number;
};

/**
 * Return on which side of the line through two points a third point lies.
 *
 * @param start - Line start.
 * @param end - Line end.
 * @param point - Point to classify.
 * @returns 1 or -1 for either side, 0 on the line.
 */
function sideOfLine(start: RelationPoint, end: RelationPoint, point: RelationPoint): number {
  return Math.sign(
    (end.x - start.x) * (point.y - start.y) - (end.y - start.y) * (point.x - start.x),
  );
}

/**
 * Check whether two segments cross at a point inside both; segments that only touch, such as
 * edges meeting at a shared node, do not count.
 *
 * @param first - First segment.
 * @param second - Second segment.
 * @returns True when the segments properly cross.
 */
function doSegmentsCross(
  first: readonly [RelationPoint, RelationPoint],
  second: readonly [RelationPoint, RelationPoint],
): boolean {
  const isSharingEnd = first.some((end) =>
    second.some(
      (other) => Math.hypot(end.x - other.x, end.y - other.y) <= RELATION_SHARED_END_TOLERANCE,
    ),
  );
  return (
    !isSharingEnd &&
    sideOfLine(first[0], first[1], second[0]) * sideOfLine(first[0], first[1], second[1]) < 0 &&
    sideOfLine(second[0], second[1], first[0]) * sideOfLine(second[0], second[1], first[1]) < 0
  );
}

/**
 * Route a cross link whose direct curve would cut across tree edges along a circle around the
 * root that clears every node it passes, so the dashed link runs outside the branches instead.
 * Links that cross nothing, touch the root, or join nearly aligned nodes keep their direct curve.
 *
 * @param directPath - Polyline following the direct curve, from the source dot to the target dot.
 * @param center - Root dot position.
 * @param points - Dot positions of every node.
 * @param treeEdges - Parent and child dot positions of every tree edge.
 * @returns Circular route, or null when the link should stay direct.
 */
export function routeRelationCrossLink(
  directPath: RelationPoint[],
  center: RelationPoint,
  points: RelationPoint[],
  treeEdges: ReadonlyArray<readonly [RelationPoint, RelationPoint]>,
): RelationCrossLinkRing | null {
  const source = directPath[0];
  const target = directPath[directPath.length - 1];
  const sourceRadius = Math.hypot(source.x - center.x, source.y - center.y);
  const targetRadius = Math.hypot(target.x - center.x, target.y - center.y);
  if (sourceRadius < 1 || targetRadius < 1) {
    return null;
  }
  const startAngle = Math.atan2(source.y - center.y, source.x - center.x);
  const sweep = relationAngleOffset(
    Math.atan2(target.y - center.y, target.x - center.x),
    startAngle,
  );
  if (Math.abs(sweep) < RELATION_CROSS_LINK_MIN_SWEEP) {
    return null;
  }
  const isCrossingTree = directPath
    .slice(1)
    .some((point, index) =>
      treeEdges.some((edge) => doSegmentsCross([directPath[index], point], edge)),
    );
  if (!isCrossingTree) {
    return null;
  }
  let farthest = Math.max(sourceRadius, targetRadius);
  for (const point of points) {
    const radius = Math.hypot(point.x - center.x, point.y - center.y);
    const offset = relationAngleOffset(
      Math.atan2(point.y - center.y, point.x - center.x),
      startAngle,
    );
    if (radius > farthest && offset * sweep >= 0 && Math.abs(offset) <= Math.abs(sweep)) {
      farthest = radius;
    }
  }
  return { radius: farthest + RELATION_CROSS_LINK_MARGIN, startAngle, sweep };
}

/**
 * Collect the nodes to keep in view after a node is expanded: the root and, for every node on the
 * path from the root to the expanded node, that node and all of its children. The first level and
 * each ancestor's siblings therefore stay visible while a deeper branch grows; unrelated expanded
 * branches may leave the view.
 *
 * @param graph - Relation graph.
 * @param nodeId - Expanded node.
 * @returns Node ids along the path with their children; empty when the node is not in the graph.
 */
export function relationPathContext(graph: RelationGraph, nodeId: string): string[] {
  const path: string[] = [];
  for (let id: string | null = nodeId; id !== null; id = graph.nodes[id]?.parentId ?? null) {
    if (!graph.nodes[id]) {
      return [];
    }
    path.unshift(id);
  }
  const ids = new Set<string>();
  for (const id of path) {
    ids.add(id);
    for (const childId of graph.childIds[id] ?? []) {
      ids.add(childId);
    }
  }
  return [...ids];
}

/**
 * List nodes depth-first in tree order, for keyboard and screen-reader navigation.
 *
 * @param graph - Relation graph.
 * @returns Nodes with the root first and each child after its parent.
 */
export function listRelationNodes(graph: RelationGraph): RelationNode[] {
  const ordered: RelationNode[] = [];
  const pending = [graph.rootId];
  while (pending.length > 0) {
    const id = pending.shift()!;
    const node = graph.nodes[id];
    if (!node) {
      continue;
    }
    ordered.push(node);
    pending.unshift(...(graph.childIds[id] ?? []));
  }
  return ordered;
}

/**
 * Summarize what the reader deliberately shaped in a graph: its articles, expanded branches,
 * cross links, and the nodes they dragged into place. Free node positions are left out because
 * the force layout settles them slightly differently every time a graph is shown.
 *
 * @param graph - Relation graph.
 * @returns Stable text that differs exactly when the reader changed the graph.
 */
export function relationGraphSignature(graph: RelationGraph): string {
  const pinned = [...graph.pinnedIds].sort().map((id) => {
    const position = graph.positions[id];
    return `${id}@${Math.round(position?.x ?? 0)},${Math.round(position?.y ?? 0)}`;
  });
  return JSON.stringify({
    nodes: Object.keys(graph.nodes).sort(),
    expanded: [...graph.expandedIds].sort(),
    crossLinks: graph.crossLinks.map(([source, target]) => `${source}>${target}`).sort(),
    pinned,
  });
}

const RELATION_GRAPH_STORAGE_KEY = 'litradar:v1:relation_graph';

/** Last relation graph of the browser session. */
/** Stored session snapshot; the graph itself is checked by {@link normalizeRelationGraph}. */
const STORED_RELATION_GRAPH_SCHEMA = z.object({
  dbName: z.string(),
  graph: z.unknown(),
  savedId: z.number().nullable().catch(null),
});

export type StoredRelationGraph = {
  dbName: string;
  graph: RelationGraph;
  /** Saved-graph favorite the graph belongs to, so a reload keeps offering update and reset. */
  savedId?: number | null;
};

/**
 * Accept an untrusted graph snapshot (session storage or a saved favorite) when its root node
 * exists, filling optional fields and laying it out again when positions are missing.
 *
 * @param value - Parsed snapshot of unknown shape.
 * @returns Usable graph, or null when the snapshot cannot be shown.
 */
export function normalizeRelationGraph(value: unknown): RelationGraph | null {
  const graph = value as Partial<RelationGraph> | null;
  if (
    !isRecord(graph) ||
    !isString(graph.rootId) ||
    !graph.nodes?.[graph.rootId] ||
    !graph.childIds ||
    !Array.isArray(graph.expandedIds) ||
    !Array.isArray(graph.crossLinks)
  ) {
    return null;
  }
  const normalized: RelationGraph = {
    ...(graph as RelationGraph),
    positions: graph.positions ?? {},
    pinnedIds: Array.isArray(graph.pinnedIds) ? graph.pinnedIds : [],
    layoutRevision: isFiniteNumber(graph.layoutRevision) ? graph.layoutRevision : 0,
  };
  const hasPositions = Object.keys(normalized.nodes).every((id) => normalized.positions[id]);
  return hasPositions ? normalized : relayoutRelationGraph(normalized);
}

/**
 * Read the last relation graph of this browser session, laying it out again when it was stored
 * without positions.
 *
 * @returns Stored graph, or null when absent or unreadable.
 */
export function readStoredRelationGraph(): StoredRelationGraph | null {
  let raw: string | null = null;
  try {
    raw = window.sessionStorage.getItem(RELATION_GRAPH_STORAGE_KEY);
  } catch {}
  const stored = raw ? parseStoredJson(raw, STORED_RELATION_GRAPH_SCHEMA) : null;
  const graph = normalizeRelationGraph(stored?.graph);
  if (!stored || !graph) {
    return null;
  }
  return { dbName: stored.dbName, graph, savedId: stored.savedId };
}

/**
 * Remember the current relation graph for this browser session; storage failures are ignored
 * because the graph keeps working for the current page.
 *
 * @param stored - Database and graph.
 */
export function storeRelationGraph(stored: StoredRelationGraph): void {
  try {
    window.sessionStorage.setItem(RELATION_GRAPH_STORAGE_KEY, JSON.stringify(stored));
  } catch {}
}
