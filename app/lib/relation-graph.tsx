/**
 * Pure state and layout helpers for the article relation graph.
 *
 * The graph is a tree grown from one root article: expanding a node attaches its related articles
 * as children. Articles that are already in the tree are not duplicated; the relation is kept as a
 * cross link instead.
 *
 * New nodes are seeded radially: the root's children are spread evenly around it, and an expanded
 * node's children are spread evenly across a fan that points away from its parent. The canvas then
 * refines the seeds with a force simulation and reports settled positions back, so the stored
 * positions always describe the last settled picture.
 */

import type { Article } from '@/lib/api';

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

/** Angular range around the root, in radians, that a branch may occupy; `end` is above `start`. */
export type RelationSector = {
  start: number;
  end: number;
};

/** Serializable relation tree with expansion state, cross links, and node positions. */
export type RelationGraph = {
  rootId: string;
  nodes: Record<string, RelationNode>;
  childIds: Record<string, string[]>;
  expandedIds: string[];
  crossLinks: Array<[string, string]>;
  positions: Record<string, RelationPoint>;
  /** Nodes the reader dropped in place; the simulation keeps them fixed. */
  pinnedIds: string[];
  /** Incremented when every position is recomputed, so the canvas adopts the new layout. */
  layoutRevision: number;
};

/**
 * Build the relation-graph workspace URL centred on one article.
 *
 * @param article - Center article.
 * @param dbName - Database that contains the article.
 * @returns Root workspace URL.
 */
export function buildRelationHref(article: Pick<Article, 'article_id'>, dbName: string): string {
  const params = new URLSearchParams({
    view: 'relations',
    db: dbName,
    article: relationNodeId(article),
  });
  return `/?${params.toString()}`;
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
  positions: Record<string, RelationPoint>,
  parentId: string,
  childIds: string[],
): Record<string, RelationPoint> {
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
 * Attach the related articles of an expanded node and place them around it.
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
  const positions: Record<string, RelationPoint> = {};
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
 * Recompute every position from scratch, replaying expansions in the order they happened, and
 * release every pin. Used to undo manual dragging and to lay out graphs stored without positions.
 *
 * @param graph - Relation graph.
 * @returns Graph with fresh automatic positions and a new layout revision.
 */
export function relayoutRelationGraph(graph: RelationGraph): RelationGraph {
  let positions: Record<string, RelationPoint> = { [graph.rootId]: { x: 0, y: 0 } };
  const order = [graph.rootId, ...graph.expandedIds.filter((id) => id !== graph.rootId)];
  for (const id of order) {
    positions = placeRelationChildren(graph, positions, id, graph.childIds[id] ?? []);
  }
  return {
    ...graph,
    positions,
    pinnedIds: [],
    layoutRevision: (graph.layoutRevision ?? 0) + 1,
  };
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
  positions: Record<string, RelationPoint>,
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
 * Give every branch a disjoint angular sector around the root, so tree edges cannot cross.
 *
 * The root's children share the full circle and each node's children share their parent's
 * sector. Siblings keep the order of their current angles, so a dragged branch moves its sector
 * along, and sector sizes follow the square root of each branch's leaf count, which keeps the
 * first level close to even while an expanded branch gets room for its fan. The first level is
 * rotated to match the current angles as closely as possible.
 *
 * @param graph - Relation graph.
 * @param positions - Current dot positions by node id.
 * @returns Sector of every non-root node that has a position.
 */
export function computeRelationSectors(
  graph: RelationGraph,
  positions: Record<string, RelationPoint>,
): Record<string, RelationSector> {
  const center = positions[graph.rootId];
  const sectors: Record<string, RelationSector> = {};
  if (!center) {
    return sectors;
  }
  const leaves = new Map<string, number>();
  const angleOf = (id: string) => {
    const position = positions[id];
    return position ? Math.atan2(position.y - center.y, position.x - center.x) : 0;
  };
  const weightOf = (id: string) => Math.sqrt(countLeaves(graph, id, leaves));

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
    const ordered = children
      .map((id) => ({ id, offset: relationAngleOffset(angleOf(id), middle) }))
      .sort((first, second) => first.offset - second.offset);
    const totalWeight = ordered.reduce((total, child) => total + weightOf(child.id), 0);
    let start = sector.start;
    for (const child of ordered) {
      const end = start + ((sector.end - sector.start) * weightOf(child.id)) / totalWeight;
      sectors[child.id] = { start, end };
      divide(child.id, sectors[child.id]);
      start = end;
    }
  }

  const firstLevel = (graph.childIds[graph.rootId] ?? []).filter((id) => positions[id]);
  if (firstLevel.length === 0) {
    return sectors;
  }
  const ordered = firstLevel
    .map((id) => ({ id, angle: normalizeAngle(angleOf(id)) }))
    .sort((first, second) => first.angle - second.angle);
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
    sectors[child.id] = { start, end: start + spans[index] };
    divide(child.id, sectors[child.id]);
    start += spans[index];
  });
  return sectors;
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

const RELATION_GRAPH_STORAGE_KEY = 'litradar:v1:relation_graph';

/** Last relation graph of the browser session. */
export type StoredRelationGraph = {
  dbName: string;
  graph: RelationGraph;
};

/**
 * Read the last relation graph of this browser session, laying it out again when it was stored
 * without positions.
 *
 * @returns Stored graph, or null when absent or unreadable.
 */
export function readStoredRelationGraph(): StoredRelationGraph | null {
  if (typeof window === 'undefined') {
    return null;
  }
  try {
    const raw = window.sessionStorage.getItem(RELATION_GRAPH_STORAGE_KEY);
    const stored = raw ? (JSON.parse(raw) as StoredRelationGraph) : null;
    const graph = stored?.graph;
    if (typeof stored?.dbName !== 'string' || !graph?.nodes?.[graph.rootId]) {
      return null;
    }
    const hasPositions = Object.keys(graph.nodes).every((id) => graph.positions?.[id]);
    const normalized: RelationGraph = {
      ...graph,
      pinnedIds: Array.isArray(graph.pinnedIds) ? graph.pinnedIds : [],
      layoutRevision: typeof graph.layoutRevision === 'number' ? graph.layoutRevision : 0,
    };
    return { ...stored, graph: hasPositions ? normalized : relayoutRelationGraph(normalized) };
  } catch {
    return null;
  }
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
