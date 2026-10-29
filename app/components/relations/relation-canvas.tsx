'use client';

/**
 * Interactive relation-graph canvas built on React Flow with a d3-force simulation: articles are
 * dots with a title, linked by springy curved edges, so dragging one node tows its neighbours. A
 * hover card shows the article details. Pan, zoom, drag, select, expand, and collapse.
 */

import '@xyflow/react/dist/base.css';

import {
  Background,
  BackgroundVariant,
  BaseEdge,
  Handle,
  Position,
  ReactFlow,
  type Edge,
  type EdgeProps,
  type Node,
  type NodeProps,
} from '@xyflow/react';
import {
  forceLink,
  forceManyBody,
  forceSimulation,
  type Force,
  type ForceLink,
  type Simulation,
  type SimulationLinkDatum,
  type SimulationNodeDatum,
} from 'd3-force';
import { Loader2, Minus, Plus } from 'lucide-react';
import {
  memo,
  useCallback,
  useEffect,
  useEffectEvent,
  useMemo,
  useRef,
  useState,
  type FocusEvent,
  type PointerEvent,
} from 'react';
import { createPortal } from 'react-dom';

import { useIsMotionReduced } from '@/components/ui/motion';
import type { Article } from '@/lib/api';
import { getArticleDisplayTitle } from '@/lib/article-title';
import {
  computeRelationSectors,
  relationAngleOffset,
  listRelationNodes,
  type RelationGraph,
  type RelationPoint,
} from '@/lib/relation-graph';
import { cn } from '@/lib/utils';

/**
 * Node sizes in flow coordinates, with the dot centre measured from the node top. Fixed sizes keep
 * React Flow from re-measuring nodes while the graph moves, which otherwise triggers
 * ResizeObserver loop errors.
 */
const NODE_SIZE = { width: 160, height: 40, dotCenter: 8 };
const ROOT_NODE_SIZE = { width: 220, height: 60, dotCenter: 11 };

/** Hover card width in pixels. */
const HOVER_CARD_WIDTH = 300;

/** Space reserved below a node before the hover card flips above it. */
const HOVER_CARD_RESERVED_HEIGHT = 240;

/** Delay before the hover card appears, so passing the pointer over nodes does not flash it. */
const HOVER_CARD_DELAY_MS = 220;

/** Pointer travel in pixels before a press on a node becomes a drag. */
const NODE_DRAG_THRESHOLD = 4;

/** How far an edge bows away from the straight line, relative to its length. */
const EDGE_BEND = 0.16;

/** Force simulation tuning: spring lengths and strengths, repulsion, spacing, damping, and cooling. */
const FORCE = {
  rootLinkDistance: 200,
  linkDistance: 160,
  linkStrength: 0.7,
  crossLinkDistance: 260,
  crossLinkStrength: 0.04,
  charge: -420,
  chargeDistanceMax: 560,
  labelHalfWidth: 80,
  labelHalfHeight: 20,
  labelCollideStrength: 0.5,
  sectorStrength: 0.6,
  sectorCentering: 0.12,
  sectorMargin: 0.08,
  radialGap: 90,
  radialStrength: 0.3,
  maxSpeed: 12,
  velocityDecay: 0.38,
  alphaDecay: 0.045,
  alphaMin: 0.004,
  dragAlphaTarget: 0.3,
  growAlpha: 0.5,
  settleAlpha: 0.15,
  reducedMotionTicks: 120,
};

/** Invisible handle centred on the dot, so edges run from dot to dot. */
const HANDLE_CLASS_NAME =
  '!pointer-events-none !size-px !min-h-0 !min-w-0 !border-0 !bg-transparent';

type ArticleNodeData = {
  title: string;
  isRoot: boolean;
  isSelected: boolean;
  isExpanded: boolean;
  isLoading: boolean;
  onSelect: () => void;
  onToggle: () => void;
  onPreviewStart: (anchor: HTMLElement, isImmediate: boolean) => void;
  onPreviewEnd: () => void;
};

type ArticleFlowNode = Node<ArticleNodeData, 'article'>;

type SimulationNode = SimulationNodeDatum & { id: string };

type SimulationLink = SimulationLinkDatum<SimulationNode> & {
  isCrossLink: boolean;
  isRootLink: boolean;
};

type RelationCanvasProps = {
  graph: RelationGraph;
  selectedId: string | null;
  loadingIds: ReadonlySet<string>;
  onSelect: (nodeId: string) => void;
  onToggle: (nodeId: string) => void;
  onPositionsSettle: (positions: Record<string, RelationPoint>, pinnedId?: string) => void;
};

/** Node shown in the hover card, tied to the graph it was measured in. */
type HoverPreview = {
  nodeId: string;
  graph: RelationGraph;
  anchor: DOMRect;
};

/**
 * Format the journal and date line of an article.
 *
 * @param article - Article.
 * @returns Source text, empty when unknown.
 */
function formatArticleSource(article: Article): string {
  const date = article.date ?? article.publication_year;
  return [article.journal_title, date].filter(Boolean).join(' · ');
}

/**
 * Format the author line of an article, keeping the first three names.
 *
 * @param authors - Article authors.
 * @returns Author text, empty when unknown.
 */
function formatArticleAuthors(authors: Article['authors']): string {
  if (!authors || authors.length === 0) {
    return '';
  }
  return authors.length > 3 ? `${authors.slice(0, 3).join('、')} 等` : authors.join('、');
}

/**
 * Render the branch toggle of a node beside its dot.
 *
 * @param props - Node data and dot centre.
 * @returns Expand or collapse button.
 */
function NodeToggle({ data, dotCenter }: { data: ArticleNodeData; dotCenter: number }) {
  return (
    <button
      type="button"
      aria-label={`${data.isExpanded ? '收起' : '展开'}相关文章：${data.title}`}
      title={data.isExpanded ? '收起分支' : '展开相关文章'}
      aria-expanded={data.isExpanded}
      disabled={data.isLoading}
      className={cn(
        'nodrag motion-control absolute flex size-4 -translate-y-1/2 items-center justify-center rounded-full border bg-background text-muted-foreground outline-none transition-colors hover:border-foreground/40 hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring',
        data.isExpanded && 'border-primary/60 text-primary',
      )}
      style={{ top: dotCenter, left: `calc(50% + ${data.isRoot ? 15 : 11}px)` }}
      onClick={data.onToggle}
    >
      {data.isLoading ? (
        <Loader2 className="size-3 animate-spin" aria-hidden="true" />
      ) : data.isExpanded ? (
        <Minus className="size-3" aria-hidden="true" />
      ) : (
        <Plus className="size-3" aria-hidden="true" />
      )}
    </button>
  );
}

/**
 * Render one article as a dot with its title below and a branch toggle beside the dot. The node
 * re-renders only when its data changes; React Flow moves it on every simulation tick through the
 * wrapper transform, so position props are ignored here.
 *
 * @param props - React Flow node props.
 * @returns Article node.
 */
const ArticleNode = memo(
  function ArticleNode({ data }: NodeProps<ArticleFlowNode>) {
    const size = data.isRoot ? ROOT_NODE_SIZE : NODE_SIZE;
    const handleStyle = { top: size.dotCenter, left: '50%' };
    const previewHandlers = {
      onPointerEnter: (event: PointerEvent<HTMLButtonElement>) =>
        data.onPreviewStart(event.currentTarget, false),
      onPointerLeave: data.onPreviewEnd,
      onPointerDown: data.onPreviewEnd,
      onFocus: (event: FocusEvent<HTMLButtonElement>) => {
        if (event.currentTarget.matches(':focus-visible')) {
          data.onPreviewStart(event.currentTarget, true);
        }
      },
      onBlur: data.onPreviewEnd,
    };
    return (
      <div
        className="pointer-events-auto relative flex cursor-grab flex-col items-center active:cursor-grabbing"
        style={{ width: size.width, height: size.height }}
      >
        <Handle
          id="center-source"
          type="source"
          position={Position.Top}
          isConnectable={false}
          className={HANDLE_CLASS_NAME}
          style={handleStyle}
        />
        <Handle
          id="center-target"
          type="target"
          position={Position.Top}
          isConnectable={false}
          className={HANDLE_CLASS_NAME}
          style={handleStyle}
        />
        <button
          type="button"
          aria-pressed={data.isSelected}
          aria-label={`${data.isRoot ? '中心文章' : '相关文章'}：${data.title}`}
          className="group/node motion-control flex max-w-full cursor-pointer flex-col items-center gap-1 rounded-md outline-none focus-visible:ring-2 focus-visible:ring-ring"
          onClick={data.onSelect}
          {...previewHandlers}
        >
          <span
            className="flex shrink-0 items-center justify-center"
            style={{ width: size.dotCenter * 2, height: size.dotCenter * 2 }}
          >
            <span
              aria-hidden="true"
              className={cn(
                'block rounded-full border-[1.5px] transition-[background-color,border-color,box-shadow]',
                data.isRoot ? 'size-[18px]' : 'size-3',
                data.isSelected
                  ? 'border-primary bg-primary shadow-[0_0_0_4px_color-mix(in_srgb,var(--primary)_22%,transparent)]'
                  : data.isRoot || data.isExpanded
                    ? 'border-foreground bg-foreground'
                    : 'border-foreground/55 bg-background group-hover/node:border-foreground',
              )}
            />
          </span>
          <span
            className={cn(
              'max-w-full rounded bg-background/85 px-1 text-center transition-colors group-hover/node:text-foreground',
              data.isRoot
                ? 'line-clamp-2 text-[12.5px] leading-[1.4] font-bold'
                : 'truncate text-[12px] leading-[18px]',
              data.isSelected ? 'font-bold text-primary' : !data.isRoot && 'text-foreground/80',
            )}
          >
            {data.title}
          </span>
        </button>
        <NodeToggle data={data} dotCenter={size.dotCenter} />
      </div>
    );
  },
  (previous, next) => previous.data === next.data,
);

/**
 * Render an edge as a gentle arc between two dots, bowing to the same side for every edge.
 *
 * @param props - React Flow edge props.
 * @returns Curved edge path.
 */
function CurvedEdge({ id, sourceX, sourceY, targetX, targetY, style }: EdgeProps) {
  const controlX = (sourceX + targetX) / 2 - (targetY - sourceY) * EDGE_BEND;
  const controlY = (sourceY + targetY) / 2 + (targetX - sourceX) * EDGE_BEND;
  return (
    <BaseEdge
      id={id}
      path={`M ${sourceX},${sourceY} Q ${controlX},${controlY} ${targetX},${targetY}`}
      style={style}
    />
  );
}

const NODE_TYPES = { article: ArticleNode };
const EDGE_TYPES = { curved: CurvedEdge };

/**
 * Render the hover card of an article next to its node, below it when there is room.
 *
 * @param props - Article and the node's screen rectangle.
 * @returns Portaled hover card.
 */
function RelationHoverCard({ article, anchor }: { article: Article; anchor: DOMRect }) {
  const isAbove = anchor.bottom + HOVER_CARD_RESERVED_HEIGHT > window.innerHeight;
  const left = Math.min(
    Math.max(8, anchor.left + anchor.width / 2 - HOVER_CARD_WIDTH / 2),
    Math.max(8, window.innerWidth - HOVER_CARD_WIDTH - 8),
  );
  const authors = formatArticleAuthors(article.authors);
  const source = formatArticleSource(article);
  return createPortal(
    <div
      aria-hidden="true"
      data-slot="relation-hover-card"
      data-state="open"
      data-side={isAbove ? 'top' : 'bottom'}
      className="motion-popover pointer-events-none fixed z-50 rounded-xl bg-popover p-3.5 text-popover-foreground shadow-overlay"
      style={{
        width: HOVER_CARD_WIDTH,
        left,
        ...(isAbove ? { bottom: window.innerHeight - anchor.top + 8 } : { top: anchor.bottom + 8 }),
      }}
    >
      <p className="text-[13px] leading-[1.45] font-bold">{getArticleDisplayTitle(article)}</p>
      {authors && <p className="mt-1.5 truncate text-xs text-muted-foreground">{authors}</p>}
      {source && <p className="mt-0.5 truncate text-xs text-muted-foreground">{source}</p>}
      {article.abstract && (
        <p className="mt-2 line-clamp-4 text-xs leading-[1.6] text-foreground/80">
          {article.abstract}
        </p>
      )}
    </div>,
    document.body,
  );
}

/**
 * Build a collision force that keeps the label rectangles of nodes apart, pushing overlapping
 * pairs along the axis with the smaller overlap. Pinned nodes stay put and the other node takes
 * the whole push.
 *
 * @returns Rectangular collision force.
 */
function forceLabelCollide(): Force<SimulationNode, SimulationLink> {
  let nodes: SimulationNode[] = [];
  const force = () => {
    const width = FORCE.labelHalfWidth * 2;
    const height = FORCE.labelHalfHeight * 2;
    for (let first = 0; first < nodes.length; first += 1) {
      for (let second = first + 1; second < nodes.length; second += 1) {
        const left = nodes[first];
        const right = nodes[second];
        const deltaX = (right.x ?? 0) + (right.vx ?? 0) - (left.x ?? 0) - (left.vx ?? 0);
        const deltaY = (right.y ?? 0) + (right.vy ?? 0) - (left.y ?? 0) - (left.vy ?? 0);
        const overlapX = width - Math.abs(deltaX);
        const overlapY = height - Math.abs(deltaY);
        if (overlapX <= 0 || overlapY <= 0) {
          continue;
        }
        const isLeftPinned = left.fx != null;
        const isRightPinned = right.fx != null;
        if (isLeftPinned && isRightPinned) {
          continue;
        }
        const leftShare = isLeftPinned ? 0 : isRightPinned ? 1 : 0.5;
        if (overlapX < overlapY) {
          const shift = overlapX * FORCE.labelCollideStrength * (deltaX < 0 ? -1 : 1);
          left.vx = (left.vx ?? 0) - shift * leftShare;
          right.vx = (right.vx ?? 0) + shift * (1 - leftShare);
        } else {
          const shift = overlapY * FORCE.labelCollideStrength * (deltaY < 0 ? -1 : 1);
          left.vy = (left.vy ?? 0) - shift * leftShare;
          right.vy = (right.vy ?? 0) + shift * (1 - leftShare);
        }
      }
    }
  };
  force.initialize = (simulationNodes: SimulationNode[]) => {
    nodes = simulationNodes;
  };
  return force;
}

/**
 * Build a force that keeps every branch inside its own angular sector around the root and farther
 * out than its parent, so tree edges do not cross. Nodes outside their sector are pushed back
 * along the circle toward the nearer edge, nodes inside drift gently toward the sector middle, and
 * pinned nodes are left where the reader put them.
 *
 * @param readGraph - Returns the graph the simulation currently models.
 * @returns Sector force.
 */
function forceSectors(
  readGraph: () => RelationGraph | null,
): Force<SimulationNode, SimulationLink> {
  let nodes: SimulationNode[] = [];
  const force = (alpha: number) => {
    const graph = readGraph();
    if (!graph) {
      return;
    }
    const positions: Record<string, RelationPoint> = {};
    for (const node of nodes) {
      positions[node.id] = { x: node.x ?? 0, y: node.y ?? 0 };
    }
    const center = positions[graph.rootId];
    if (!center) {
      return;
    }
    const sectors = computeRelationSectors(graph, positions);
    for (const node of nodes) {
      const sector = sectors[node.id];
      if (!sector || node.fx != null) {
        continue;
      }
      const deltaX = (node.x ?? 0) - center.x;
      const deltaY = (node.y ?? 0) - center.y;
      const radius = Math.hypot(deltaX, deltaY) || 1;
      const angle = Math.atan2(deltaY, deltaX);
      const halfWidth = (sector.end - sector.start) / 2;
      const allowance = halfWidth - Math.min(FORCE.sectorMargin, halfWidth * 0.5);
      const offset = relationAngleOffset(angle, sector.start + halfWidth);
      const correction =
        Math.abs(offset) > allowance
          ? -Math.sign(offset) * (Math.abs(offset) - allowance)
          : -offset * FORCE.sectorCentering;
      const tangentialPush = correction * radius * FORCE.sectorStrength * alpha;
      node.vx = (node.vx ?? 0) - Math.sin(angle) * tangentialPush;
      node.vy = (node.vy ?? 0) + Math.cos(angle) * tangentialPush;
      const parentId = graph.nodes[node.id]?.parentId;
      const parent = parentId && parentId !== graph.rootId ? positions[parentId] : undefined;
      if (parent) {
        const parentRadius = Math.hypot(parent.x - center.x, parent.y - center.y);
        const deficit = parentRadius + FORCE.radialGap - radius;
        if (deficit > 0) {
          const radialPush = deficit * FORCE.radialStrength * alpha;
          node.vx += (deltaX / radius) * radialPush;
          node.vy += (deltaY / radius) * radialPush;
        }
      }
    }
  };
  force.initialize = (simulationNodes: SimulationNode[]) => {
    nodes = simulationNodes;
  };
  return force;
}

/**
 * Build a force, applied after all others, that caps how far a node may move in one tick. Large
 * corrections, such as a branch moving around a dragged neighbour to keep edges from crossing,
 * then play out as a glide instead of a jump.
 *
 * @returns Speed-limit force.
 */
function forceSpeedLimit(): Force<SimulationNode, SimulationLink> {
  let nodes: SimulationNode[] = [];
  const force = () => {
    for (const node of nodes) {
      const speed = Math.hypot(node.vx ?? 0, node.vy ?? 0);
      if (speed > FORCE.maxSpeed) {
        node.vx = ((node.vx ?? 0) * FORCE.maxSpeed) / speed;
        node.vy = ((node.vy ?? 0) * FORCE.maxSpeed) / speed;
      }
    }
  };
  force.initialize = (simulationNodes: SimulationNode[]) => {
    nodes = simulationNodes;
  };
  return force;
}

/**
 * Create the force simulation shared by every graph update of one canvas.
 *
 * @param readGraph - Returns the graph the simulation currently models.
 * @returns Stopped simulation with springs, repulsion, label collision, branch sectors, and a
 *   speed limit.
 */
function createRelationSimulation(
  readGraph: () => RelationGraph | null,
): Simulation<SimulationNode, SimulationLink> {
  return forceSimulation<SimulationNode, SimulationLink>()
    .velocityDecay(FORCE.velocityDecay)
    .alphaDecay(FORCE.alphaDecay)
    .alphaMin(FORCE.alphaMin)
    .force(
      'link',
      forceLink<SimulationNode, SimulationLink>()
        .id((node) => node.id)
        .distance((link) =>
          link.isCrossLink
            ? FORCE.crossLinkDistance
            : link.isRootLink
              ? FORCE.rootLinkDistance
              : FORCE.linkDistance,
        )
        .strength((link) => (link.isCrossLink ? FORCE.crossLinkStrength : FORCE.linkStrength)),
    )
    .force(
      'charge',
      forceManyBody<SimulationNode>().strength(FORCE.charge).distanceMax(FORCE.chargeDistanceMax),
    )
    .force('collide', forceLabelCollide())
    .force('sectors', forceSectors(readGraph))
    .force('speedLimit', forceSpeedLimit())
    .stop();
}

/**
 * Render the relation graph as a live force layout with curved edges, dashed cross links,
 * draggable dot nodes, and a hover card that stays hidden while a node is pressed or dragged, or
 * while the canvas pans. Dropped nodes and the root stay pinned; settled positions are reported so
 * the graph can be restored later.
 *
 * @param props - Graph, selection, loading state, and callbacks.
 * @returns React Flow canvas.
 */
export function RelationCanvas({
  graph,
  selectedId,
  loadingIds,
  onSelect,
  onToggle,
  onPositionsSettle,
}: RelationCanvasProps) {
  const isMotionReduced = useIsMotionReduced();
  const [preview, setPreview] = useState<HoverPreview | null>(null);
  const [positions, setPositions] = useState<Record<string, RelationPoint>>(graph.positions);
  const previewTimerRef = useRef<number | null>(null);
  const isClickSuppressedRef = useRef(false);
  const isDraggingRef = useRef(false);
  const isPanningRef = useRef(false);
  const simulationRef = useRef<Simulation<SimulationNode, SimulationLink> | null>(null);
  const simulationNodesRef = useRef(new Map<string, SimulationNode>());
  const layoutRevisionRef = useRef<number | null>(null);
  const simulatedGraphRef = useRef<RelationGraph | null>(null);

  const readSimulationPositions = useCallback((): Record<string, RelationPoint> => {
    const snapshot: Record<string, RelationPoint> = {};
    for (const [id, node] of simulationNodesRef.current) {
      snapshot[id] = { x: node.x ?? 0, y: node.y ?? 0 };
    }
    return snapshot;
  }, []);

  const reportSettled = useEffectEvent(() => onPositionsSettle(readSimulationPositions()));

  const syncSimulation = useEffectEvent(() => {
    let simulation = simulationRef.current;
    if (!simulation) {
      simulation = createRelationSimulation(() => simulatedGraphRef.current);
      simulation.on('tick', () => setPositions(readSimulationPositions()));
      simulation.on('end', () => reportSettled());
      simulationRef.current = simulation;
    }
    simulatedGraphRef.current = graph;
    const isFirstSync = layoutRevisionRef.current === null;
    const isRelayout = !isFirstSync && layoutRevisionRef.current !== graph.layoutRevision;
    layoutRevisionRef.current = graph.layoutRevision;
    const simulationNodes = simulationNodesRef.current;
    const pinnedIds = new Set([graph.rootId, ...graph.pinnedIds]);
    let hasNewNodes = false;
    for (const node of Object.values(graph.nodes)) {
      const seed = graph.positions[node.id] ??
        (node.parentId ? graph.positions[node.parentId] : undefined) ?? { x: 0, y: 0 };
      let simulationNode = simulationNodes.get(node.id);
      if (!simulationNode) {
        simulationNode = { id: node.id, x: seed.x, y: seed.y };
        simulationNodes.set(node.id, simulationNode);
        hasNewNodes = true;
      } else if (isRelayout) {
        simulationNode.x = seed.x;
        simulationNode.y = seed.y;
        simulationNode.vx = 0;
        simulationNode.vy = 0;
      }
      const isPinned = pinnedIds.has(node.id);
      simulationNode.fx = isPinned ? simulationNode.x : null;
      simulationNode.fy = isPinned ? simulationNode.y : null;
    }
    for (const id of [...simulationNodes.keys()]) {
      if (!graph.nodes[id]) {
        simulationNodes.delete(id);
      }
    }
    const links: SimulationLink[] = [];
    for (const node of Object.values(graph.nodes)) {
      if (node.parentId !== null) {
        links.push({
          source: node.parentId,
          target: node.id,
          isCrossLink: false,
          isRootLink: node.parentId === graph.rootId,
        });
      }
    }
    for (const [source, target] of graph.crossLinks) {
      links.push({ source, target, isCrossLink: true, isRootLink: false });
    }
    simulation.nodes([...simulationNodes.values()]);
    simulation.force<ForceLink<SimulationNode, SimulationLink>>('link')?.links(links);
    const alpha = isFirstSync || isRelayout || hasNewNodes ? FORCE.growAlpha : FORCE.settleAlpha;
    if (isMotionReduced) {
      simulation.stop();
      simulation.alpha(alpha);
      simulation.tick(FORCE.reducedMotionTicks);
      window.requestAnimationFrame(() => {
        setPositions(readSimulationPositions());
        reportSettled();
      });
    } else {
      simulation.alpha(alpha).restart();
    }
  });

  useEffect(() => {
    syncSimulation();
  }, [graph.nodes, graph.crossLinks, graph.pinnedIds, graph.layoutRevision, isMotionReduced]);

  useEffect(() => () => void simulationRef.current?.stop(), []);

  const clearPreviewTimer = useCallback(() => {
    if (previewTimerRef.current !== null) {
      window.clearTimeout(previewTimerRef.current);
      previewTimerRef.current = null;
    }
  }, []);

  const hidePreview = useCallback(() => {
    clearPreviewTimer();
    setPreview(null);
  }, [clearPreviewTimer]);

  const showPreview = useCallback(
    (nodeGraph: RelationGraph, nodeId: string, anchor: HTMLElement, isImmediate: boolean) => {
      clearPreviewTimer();
      const show = () => {
        if (!isDraggingRef.current && !isPanningRef.current) {
          setPreview({ nodeId, graph: nodeGraph, anchor: anchor.getBoundingClientRect() });
        }
      };
      if (isImmediate) {
        show();
      } else {
        previewTimerRef.current = window.setTimeout(show, HOVER_CARD_DELAY_MS);
      }
    },
    [clearPreviewTimer],
  );

  useEffect(() => clearPreviewTimer, [clearPreviewTimer]);

  /**
   * Hold a dragged node at the pointer while the simulation tows its neighbours.
   *
   * @param node - Dragged React Flow node.
   */
  const holdDraggedNode = useCallback((node: ArticleFlowNode) => {
    const simulationNode = simulationNodesRef.current.get(node.id);
    if (simulationNode) {
      simulationNode.fx = node.position.x;
      simulationNode.fy = node.position.y;
      simulationNode.x = node.position.x;
      simulationNode.y = node.position.y;
    }
  }, []);

  const nodeEntries = useMemo(
    () =>
      listRelationNodes(graph).map((node) => {
        const isRoot = node.id === graph.rootId;
        const data: ArticleNodeData = {
          title: getArticleDisplayTitle(node.article),
          isRoot,
          isSelected: node.id === selectedId,
          isExpanded: graph.expandedIds.includes(node.id),
          isLoading: loadingIds.has(node.id),
          onSelect: () => {
            if (!isClickSuppressedRef.current) {
              onSelect(node.id);
            }
          },
          onToggle: () => {
            hidePreview();
            onToggle(node.id);
          },
          onPreviewStart: (anchor, isImmediate) => showPreview(graph, node.id, anchor, isImmediate),
          onPreviewEnd: hidePreview,
        };
        return { id: node.id, size: isRoot ? ROOT_NODE_SIZE : NODE_SIZE, data };
      }),
    [graph, selectedId, loadingIds, onSelect, onToggle, hidePreview, showPreview],
  );

  const nodes = useMemo<ArticleFlowNode[]>(
    () =>
      nodeEntries.map(({ id, size, data }) => ({
        id,
        type: 'article',
        position: positions[id] ?? graph.positions[id] ?? { x: 0, y: 0 },
        origin: [0.5, size.dotCenter / size.height],
        width: size.width,
        height: size.height,
        measured: { width: size.width, height: size.height },
        selectable: false,
        focusable: false,
        data,
      })),
    [nodeEntries, positions, graph.positions],
  );

  const edges = useMemo(() => {
    const highlight = (first: string, second: string) =>
      selectedId !== null && (first === selectedId || second === selectedId);
    const flowEdges: Edge[] = [];
    for (const node of Object.values(graph.nodes)) {
      if (node.parentId === null) {
        continue;
      }
      const isHighlighted = highlight(node.parentId, node.id);
      flowEdges.push({
        id: `tree:${node.parentId}:${node.id}`,
        source: node.parentId,
        target: node.id,
        type: 'curved',
        focusable: false,
        style: {
          stroke: isHighlighted ? 'var(--primary)' : 'var(--input-hover)',
          strokeWidth: isHighlighted ? 1.75 : 1.25,
        },
      });
    }
    for (const [source, target] of graph.crossLinks) {
      const isHighlighted = highlight(source, target);
      flowEdges.push({
        id: `cross:${source}:${target}`,
        source,
        target,
        type: 'curved',
        focusable: false,
        style: {
          stroke: isHighlighted ? 'var(--primary)' : 'var(--input-hover)',
          strokeWidth: 1,
          strokeDasharray: '4 4',
        },
      });
    }
    return flowEdges;
  }, [graph.nodes, graph.crossLinks, selectedId]);

  const previewArticle =
    preview && preview.graph === graph ? graph.nodes[preview.nodeId]?.article : undefined;

  return (
    <>
      <ReactFlow
        nodes={nodes}
        edges={edges}
        nodeTypes={NODE_TYPES}
        edgeTypes={EDGE_TYPES}
        fitView
        fitViewOptions={{ padding: 0.12, maxZoom: 1 }}
        minZoom={0.2}
        maxZoom={1.75}
        nodesDraggable
        nodeDragThreshold={NODE_DRAG_THRESHOLD}
        nodesConnectable={false}
        elementsSelectable={false}
        nodesFocusable={false}
        edgesFocusable={false}
        zoomOnDoubleClick={false}
        onMoveStart={() => {
          isPanningRef.current = true;
          hidePreview();
        }}
        onMoveEnd={() => {
          isPanningRef.current = false;
        }}
        onNodeDragStart={(_, node) => {
          isDraggingRef.current = true;
          hidePreview();
          holdDraggedNode(node);
          simulationRef.current?.alphaTarget(FORCE.dragAlphaTarget).restart();
        }}
        onNodeDrag={(_, node) => holdDraggedNode(node)}
        onNodeDragStop={(_, node) => {
          isDraggingRef.current = false;
          holdDraggedNode(node);
          simulationRef.current?.alphaTarget(0).alpha(FORCE.growAlpha).restart();
          isClickSuppressedRef.current = true;
          window.setTimeout(() => {
            isClickSuppressedRef.current = false;
          }, 0);
          onPositionsSettle(readSimulationPositions(), node.id);
        }}
        aria-label="文献关系图"
        className="relation-canvas"
      >
        <Background variant={BackgroundVariant.Dots} gap={22} size={1} color="var(--border)" />
      </ReactFlow>
      {preview && previewArticle && (
        <RelationHoverCard article={previewArticle} anchor={preview.anchor} />
      )}
    </>
  );
}
