'use client';

/**
 * Relation-graph workspace: an interactive network of an article and its related articles beside
 * the reading desk.
 */

import { ReactFlowProvider, useReactFlow } from '@xyflow/react';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { ListTree, Loader2, Maximize, Minus, Network, Plus, RotateCcw, Search } from 'lucide-react';
import { parseAsInteger, parseAsString, useQueryStates } from 'nuqs';
import { useCallback, useEffect, useMemo, useState, type FormEvent, type ReactNode } from 'react';

import {
  CheckedArticleDetailDialogContent,
  CheckedArticleDetailPane,
} from '@/components/feature/article-detail-dialog-content';
import { ReaderPlaceholder } from '@/components/feature/reader-placeholder';
import { useFavoriteChecks } from '@/components/feature/use-favorite-checks';
import { WorkspaceShell } from '@/components/feature/workspace-shell';
import { RelationCanvas } from '@/components/relations/relation-canvas';
import {
  RelationGraphFavoriteControls,
  relationGraphFavoriteKey,
} from '@/components/relations/relation-graph-favorite-controls';
import { Button } from '@/components/ui/button';
import { Dialog } from '@/components/ui/dialog';
import { Input } from '@/components/ui/input';
import { useIsMotionReduced } from '@/components/ui/motion';
import {
  getArticle,
  getArticles,
  getRelatedArticles,
  getRelationGraphFavorite,
  type Article,
  type RelationGraphFavorite,
} from '@/lib/api';
import { getArticleDisplayTitle } from '@/lib/article-title';
import { useAuth } from '@/lib/auth-context';
import {
  RELATION_CHILD_LIMIT,
  RELATION_NODE_LIMIT,
  collapseRelationNode,
  createRelationGraph,
  expandRelationNode,
  listRelationNodes,
  normalizeRelationGraph,
  readStoredRelationGraph,
  relationPathContext,
  relationNodeId,
  relayoutRelationGraph,
  storeRelationGraph,
  storeRelationPositions,
  type RelationGraph,
  type RelationPoint,
} from '@/lib/relation-graph';
import { useSelectedDatabase } from '@/lib/selected-database';
import { READER_DOCK_MEDIA_QUERY, useMediaQuery } from '@/lib/use-media-query';
import { cn } from '@/lib/utils';

/** URL state of the workspace: center article, its database, and an opened saved graph. */
const RELATION_QUERY_PARSERS = {
  article: parseAsString,
  db: parseAsString,
  saved: parseAsInteger,
};

/** Related lists are shared with the article reader and change only when the index updates. */
const RELATED_STALE_TIME_MS = 10 * 60 * 1000;

type RelationViewMode = 'graph' | 'list';

/**
 * Render the relation workspace, restoring the session's last graph when no article is chosen
 * and loading a saved graph when the URL names one.
 *
 * @returns Relation-graph workspace.
 */
export function RelationsWorkspaceView() {
  const [{ article: articleId, db: dbName, saved: savedId }, setParams] =
    useQueryStates(RELATION_QUERY_PARSERS);
  const [stored] = useState(readStoredRelationGraph);
  const isRestoring = (!articleId || !dbName) && stored !== null;
  const savedQuery = useQuery({
    queryKey: relationGraphFavoriteKey(savedId),
    queryFn: ({ signal }) => getRelationGraphFavorite(savedId as number, signal),
    enabled: savedId !== null && Boolean(articleId && dbName),
    staleTime: Number.POSITIVE_INFINITY,
    retry: false,
  });

  useEffect(() => {
    if (isRestoring && stored) {
      void setParams(
        { article: stored.graph.rootId, db: stored.dbName, saved: stored.savedId ?? null },
        { history: 'replace' },
      );
    }
  }, [isRestoring, setParams, stored]);

  if (!articleId || !dbName) {
    return isRestoring ? null : (
      <RelationPicker
        onPick={(article, pickedDbName) =>
          void setParams({ article: relationNodeId(article), db: pickedDbName })
        }
      />
    );
  }
  if (savedId !== null && savedQuery.isPending) {
    return null;
  }
  const savedFavorite =
    savedQuery.data &&
    savedQuery.data.db_name === dbName &&
    String(savedQuery.data.root_article_id) === articleId
      ? savedQuery.data
      : null;
  return (
    <ReactFlowProvider>
      <RelationExplorer
        key={`${dbName}\u0000${articleId}\u0000${savedId ?? ''}`}
        articleId={articleId}
        dbName={dbName}
        savedFavorite={savedFavorite}
        isSavedFavoriteMissing={savedId !== null && savedFavorite === null}
      />
    </ReactFlowProvider>
  );
}

/**
 * Choose the graph a new explorer starts from: this session's graph when it matches the center
 * article and saved favorite (it may hold unsaved changes), otherwise the saved snapshot.
 *
 * @param dbName - Database of the center article.
 * @param articleId - Center article id.
 * @param savedFavorite - Saved graph named by the URL, if any.
 * @returns Starting graph (null to build a fresh one) and the favorite it belongs to.
 */
function resolveInitialRelationGraph(
  dbName: string,
  articleId: string,
  savedFavorite: RelationGraphFavorite | null,
): { graph: RelationGraph | null; savedId: number | null } {
  const stored = readStoredRelationGraph();
  const storedMatches = stored && stored.dbName === dbName && stored.graph.rootId === articleId;
  if (savedFavorite) {
    if (storedMatches && stored.savedId === savedFavorite.id) {
      return { graph: stored.graph, savedId: savedFavorite.id };
    }
    const snapshot = normalizeRelationGraph(savedFavorite.graph);
    return snapshot
      ? { graph: snapshot, savedId: savedFavorite.id }
      : { graph: null, savedId: null };
  }
  return storedMatches
    ? { graph: stored.graph, savedId: stored.savedId ?? null }
    : { graph: null, savedId: null };
}

/**
 * Let the reader choose a center article by searching the selected database.
 *
 * @param props - Pick callback.
 * @returns Empty-state workspace with search.
 */
function RelationPicker({ onPick }: { onPick: (article: Article, dbName: string) => void }) {
  const dbName = useSelectedDatabase();
  const [draft, setDraft] = useState('');
  const [query, setQuery] = useState('');
  const resultsQuery = useQuery({
    queryKey: ['relation-picker', dbName, query],
    queryFn: ({ signal }) =>
      getArticles(new URLSearchParams({ q: query, limit: '8' }), null, false, dbName, signal),
    enabled: query.length > 0,
    staleTime: 60 * 1000,
  });

  /**
   * Search for candidate center articles.
   *
   * @param event - Form submission.
   */
  function handleSubmit(event: FormEvent<HTMLFormElement>): void {
    event.preventDefault();
    setQuery(draft.trim());
  }

  return (
    <WorkspaceShell
      listLabel="选择中心文章"
      emphasis="list"
      header={<RelationHeading subtitle="选择一篇文章作为中心，查看与它相关的文献网络" />}
      readerPlaceholder={null}
      reader={{
        content: (
          <ReaderPlaceholder
            title="从一篇文章开始"
            description="在左侧检索并选择文章，或在任意文章详情中点击「关系图」。"
          />
        ),
        isSheetOpen: false,
        onSheetOpenChange: () => undefined,
        title: '文献关系图',
        description: '选择中心文章',
      }}
    >
      <div className="mx-auto w-full max-w-xl px-4 py-8 sm:px-5">
        <form role="search" className="flex gap-2" onSubmit={handleSubmit}>
          <Input
            value={draft}
            onChange={(event) => setDraft(event.target.value)}
            placeholder="输入标题、作者或关键词"
            aria-label="检索中心文章"
          />
          <Button type="submit" className="shrink-0 gap-1.5">
            <Search className="size-4" aria-hidden="true" />
            检索
          </Button>
        </form>
        {resultsQuery.isFetching && (
          <p role="status" className="mt-4 text-sm text-muted-foreground">
            正在检索…
          </p>
        )}
        {resultsQuery.isError && (
          <p role="alert" className="mt-4 text-sm text-destructive">
            检索失败，请稍后重试。
          </p>
        )}
        {resultsQuery.data && resultsQuery.data.items.length === 0 && (
          <p className="mt-4 text-sm text-muted-foreground">没有找到匹配的文章。</p>
        )}
        {resultsQuery.data && resultsQuery.data.items.length > 0 && (
          <ul aria-label="候选中心文章" className="mt-4 divide-y border-y">
            {resultsQuery.data.items.map((article) => (
              <li key={relationNodeId(article)}>
                <button
                  type="button"
                  className="motion-control block w-full px-1 py-3 text-left outline-none transition-colors hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset"
                  onClick={() => onPick(article, dbName)}
                >
                  <span className="line-clamp-2 text-[13.5px] leading-[1.5] font-bold">
                    {getArticleDisplayTitle(article)}
                  </span>
                  <span className="mt-1 block truncate text-xs text-muted-foreground">
                    {[article.journal_title, article.date].filter(Boolean).join(' · ')}
                  </span>
                </button>
              </li>
            ))}
          </ul>
        )}
      </div>
    </WorkspaceShell>
  );
}

/**
 * Render the workspace title with a context line.
 *
 * @param props - Context line and optional controls.
 * @returns Header block.
 */
function RelationHeading({ subtitle, children }: { subtitle: string; children?: ReactNode }) {
  return (
    <div className="flex flex-wrap items-start justify-between gap-x-3 gap-y-2">
      <div className="min-w-0 basis-full sm:flex-1 sm:basis-auto">
        <h1 className="text-lg leading-7 font-bold">文献关系图</h1>
        <p className="truncate text-xs text-muted-foreground">{subtitle}</p>
      </div>
      {children && <div className="flex shrink-0 items-center gap-1">{children}</div>}
    </div>
  );
}

/**
 * Grow, show, and navigate the relation graph of one center article, batch-loading the favorite
 * state of every node so selecting one shows its favorite control at once, as in article lists.
 *
 * @param props - Center article and database.
 * @returns Graph workspace with reader.
 */
function RelationExplorer({
  articleId,
  dbName,
  savedFavorite,
  isSavedFavoriteMissing,
}: {
  articleId: string;
  dbName: string;
  savedFavorite: RelationGraphFavorite | null;
  isSavedFavoriteMissing: boolean;
}) {
  const queryClient = useQueryClient();
  const { user } = useAuth();
  const { fitView } = useReactFlow();
  const isDocked = useMediaQuery(READER_DOCK_MEDIA_QUERY);
  const isMotionReduced = useIsMotionReduced();
  const [initial] = useState(() => resolveInitialRelationGraph(dbName, articleId, savedFavorite));
  const [graph, setGraph] = useState<RelationGraph | null>(initial.graph);
  const [savedGraphId, setSavedGraphId] = useState<number | null>(initial.savedId);
  const [loadingIds, setLoadingIds] = useState<ReadonlySet<string>>(() => new Set());
  const [failure, setFailure] = useState<string | null>(
    isSavedFavoriteMissing ? '收藏的关系图不存在或已被删除，已显示新的关系图。' : null,
  );
  const [selectedId, setSelectedId] = useState<string | null>(articleId);
  const [isSheetOpen, setIsSheetOpen] = useState(false);
  const [viewMode, setViewMode] = useState<RelationViewMode>('graph');

  const fetchRelated = useCallback(
    (nodeId: string) =>
      queryClient.fetchQuery({
        queryKey: ['related-articles', dbName, nodeId],
        queryFn: ({ signal }) => getRelatedArticles(nodeId, dbName, RELATION_CHILD_LIMIT, signal),
        staleTime: RELATED_STALE_TIME_MS,
      }),
    [dbName, queryClient],
  );

  const initialQuery = useQuery({
    queryKey: ['relation-root', dbName, articleId],
    queryFn: async ({ signal }) => {
      const root = await getArticle(articleId, dbName, signal);
      const related = await fetchRelated(relationNodeId(root));
      return expandRelationNode(createRelationGraph(root), relationNodeId(root), related);
    },
    enabled: graph === null,
    staleTime: RELATED_STALE_TIME_MS,
    retry: false,
  });
  const currentGraph = graph ?? initialQuery.data ?? null;
  const graphArticleIds = useMemo(
    () =>
      currentGraph ? Object.values(currentGraph.nodes).map((node) => node.article.article_id) : [],
    [currentGraph],
  );
  useFavoriteChecks(graphArticleIds, dbName, user?.id);

  const focusBranch = useCallback(
    (nodeIds: string[]) => {
      window.requestAnimationFrame(() =>
        window.requestAnimationFrame(() => {
          void fitView({
            nodes: nodeIds.map((id) => ({ id })),
            duration: isMotionReduced ? 0 : 300,
            padding: 0.15,
            maxZoom: 1,
          });
        }),
      );
    },
    [fitView, isMotionReduced],
  );

  const expand = useCallback(
    async (base: RelationGraph, nodeId: string) => {
      setFailure(null);
      setLoadingIds((current) => new Set(current).add(nodeId));
      try {
        const related = await fetchRelated(nodeId);
        const next = expandRelationNode(base, nodeId, related);
        setGraph((current) => expandRelationNode(current ?? base, nodeId, related));
        focusBranch(relationPathContext(next, nodeId));
      } catch {
        setFailure('暂时无法加载相关文章，请稍后重试。');
      } finally {
        setLoadingIds((current) => {
          const next = new Set(current);
          next.delete(nodeId);
          return next;
        });
      }
    },
    [fetchRelated, focusBranch],
  );

  useEffect(() => {
    if (currentGraph) {
      storeRelationGraph({ dbName, graph: currentGraph, savedId: savedGraphId });
    }
  }, [dbName, currentGraph, savedGraphId]);

  const select = useCallback(
    (nodeId: string) => {
      setSelectedId(nodeId);
      if (!isDocked) {
        setIsSheetOpen(true);
      }
    },
    [isDocked],
  );

  const toggle = useCallback(
    (nodeId: string) => {
      if (!currentGraph) {
        return;
      }
      if (currentGraph.expandedIds.includes(nodeId)) {
        const collapsed = collapseRelationNode(currentGraph, nodeId);
        setGraph(collapsed);
        setSelectedId((current) => (current && !collapsed.nodes[current] ? nodeId : current));
      } else {
        void expand(currentGraph, nodeId);
      }
    },
    [currentGraph, expand],
  );

  const settlePositions = useCallback(
    (positions: Record<string, RelationPoint>, pinnedId?: string) => {
      setGraph((current) => {
        const base = current ?? currentGraph;
        return base ? storeRelationPositions(base, positions, pinnedId) : current;
      });
    },
    [currentGraph],
  );

  const resetToSaved = useCallback(
    (saved: RelationGraph) => {
      setFailure(null);
      setGraph({
        ...saved,
        layoutRevision: Math.max(saved.layoutRevision, currentGraph?.layoutRevision ?? 0) + 1,
      });
      setSelectedId((current) => (current && saved.nodes[current] ? current : saved.rootId));
      focusBranch(Object.keys(saved.nodes));
    },
    [currentGraph, focusBranch],
  );

  const relayout = useCallback(() => {
    if (currentGraph) {
      setGraph(relayoutRelationGraph(currentGraph));
      focusBranch(Object.keys(currentGraph.nodes));
    }
  }, [currentGraph, focusBranch]);

  const root = currentGraph?.nodes[currentGraph.rootId]?.article;
  const selectedArticle = (selectedId && currentGraph?.nodes[selectedId]?.article) || root;
  const nodeCount = currentGraph ? Object.keys(currentGraph.nodes).length : 0;

  const header = (
    <div className="space-y-2">
      <RelationHeading
        subtitle={
          root
            ? `以「${getArticleDisplayTitle(root)}」为中心 · ${nodeCount} 篇`
            : '正在读取中心文章…'
        }
      >
        <div role="radiogroup" aria-label="显示方式" className="mr-1 flex rounded-lg border p-0.5">
          {(
            [
              ['graph', '网络', Network],
              ['list', '列表', ListTree],
            ] as const
          ).map(([mode, label, Icon]) => (
            <button
              key={mode}
              type="button"
              role="radio"
              aria-checked={viewMode === mode}
              className={cn(
                'motion-control flex h-10 items-center gap-1 rounded-md px-3 text-xs outline-none transition-colors focus-visible:ring-2 focus-visible:ring-ring md:h-7 md:px-2',
                viewMode === mode
                  ? 'bg-foreground text-background'
                  : 'text-muted-foreground hover:text-foreground',
              )}
              onClick={() => setViewMode(mode)}
            >
              <Icon className="size-3.5" aria-hidden="true" />
              {label}
            </button>
          ))}
        </div>
        {viewMode === 'graph' && (
          <Button
            type="button"
            variant="ghost"
            size="icon"
            className="size-11 md:size-8"
            aria-label="重新排列"
            title="重新排列"
            onClick={relayout}
          >
            <RotateCcw className="size-4" aria-hidden="true" />
          </Button>
        )}
        {viewMode === 'graph' && (
          <Button
            type="button"
            variant="ghost"
            size="icon"
            className="size-11 md:size-8"
            aria-label="适应窗口"
            title="适应窗口"
            onClick={() => void fitView({ duration: isMotionReduced ? 0 : 300, padding: 0.2 })}
          >
            <Maximize className="size-4" aria-hidden="true" />
          </Button>
        )}
        {user && currentGraph && root && (
          <RelationGraphFavoriteControls
            dbName={dbName}
            graph={currentGraph}
            rootTitle={getArticleDisplayTitle(root)}
            savedId={savedGraphId}
            onError={setFailure}
            onReset={resetToSaved}
            onSavedIdChange={setSavedGraphId}
          />
        )}
      </RelationHeading>
      {failure && (
        <p role="alert" className="text-xs text-destructive">
          {failure}
        </p>
      )}
      {nodeCount >= RELATION_NODE_LIMIT && (
        <p className="text-xs text-muted-foreground">
          图中已有 {RELATION_NODE_LIMIT} 篇文章，收起部分分支后可继续展开。
        </p>
      )}
    </div>
  );

  let body: ReactNode;
  if (initialQuery.isError && currentGraph === null) {
    body = (
      <p role="alert" className="px-5 py-8 text-sm text-destructive">
        无法读取中心文章，它可能已不在当前数据库中。
      </p>
    );
  } else if (currentGraph === null) {
    body = (
      <p role="status" className="flex items-center gap-2 px-5 py-8 text-sm text-muted-foreground">
        <Loader2 className="size-4 animate-spin" aria-hidden="true" />
        正在构建关系图…
      </p>
    );
  } else if (viewMode === 'graph') {
    body = (
      <div data-slot="relation-canvas" className="min-h-0 flex-1">
        <RelationCanvas
          graph={currentGraph}
          selectedId={selectedId}
          loadingIds={loadingIds}
          onSelect={select}
          onToggle={toggle}
          onPositionsSettle={settlePositions}
        />
      </div>
    );
  } else {
    body = (
      <RelationList
        graph={currentGraph}
        selectedId={selectedId}
        loadingIds={loadingIds}
        onSelect={select}
        onToggle={toggle}
      />
    );
  }

  return (
    <>
      <WorkspaceShell
        listLabel="关系网络"
        emphasis="list"
        isListSelfScrolling
        header={header}
        readerPlaceholder={null}
        reader={{
          content: selectedArticle ? (
            <CheckedArticleDetailPane
              key={relationNodeId(selectedArticle)}
              article={selectedArticle}
              dbName={dbName}
            />
          ) : (
            <ReaderPlaceholder title="选择一个节点" description="点击图中的文章查看详情。" />
          ),
          isSheetOpen: false,
          onSheetOpenChange: () => undefined,
          title: '文献详情',
          description: '关系图中选中文章的详情',
        }}
      >
        {body}
      </WorkspaceShell>
      {!isDocked && selectedArticle && (
        <Dialog open={isSheetOpen} onOpenChange={setIsSheetOpen}>
          <CheckedArticleDetailDialogContent
            key={relationNodeId(selectedArticle)}
            article={selectedArticle}
            dbName={dbName}
          />
        </Dialog>
      )}
    </>
  );
}

/**
 * Render the graph as an indented list for keyboard, screen-reader, and narrow-screen use.
 *
 * @param props - Graph, selection, loading state, and callbacks.
 * @returns Indented node list.
 */
function RelationList({
  graph,
  selectedId,
  loadingIds,
  onSelect,
  onToggle,
}: {
  graph: RelationGraph;
  selectedId: string | null;
  loadingIds: ReadonlySet<string>;
  onSelect: (nodeId: string) => void;
  onToggle: (nodeId: string) => void;
}) {
  const nodes = useMemo(() => listRelationNodes(graph), [graph]);
  return (
    <ul aria-label="关系节点列表" className="min-h-0 flex-1 overflow-y-auto py-2">
      {nodes.map((node) => {
        const title = getArticleDisplayTitle(node.article);
        const isExpanded = graph.expandedIds.includes(node.id);
        const isLoading = loadingIds.has(node.id);
        return (
          <li
            key={node.id}
            className="flex items-start gap-1 pr-4"
            style={{ paddingLeft: `${16 + node.depth * 22}px` }}
          >
            <button
              type="button"
              aria-label={`${isExpanded ? '收起' : '展开'}相关文章：${title}`}
              aria-expanded={isExpanded}
              disabled={isLoading}
              className="motion-control mt-2 flex size-6 shrink-0 items-center justify-center rounded-full border text-muted-foreground outline-none transition-colors hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
              onClick={() => onToggle(node.id)}
            >
              {isLoading ? (
                <Loader2 className="size-3.5 animate-spin" aria-hidden="true" />
              ) : isExpanded ? (
                <Minus className="size-3.5" aria-hidden="true" />
              ) : (
                <Plus className="size-3.5" aria-hidden="true" />
              )}
            </button>
            <button
              type="button"
              aria-pressed={node.id === selectedId}
              className={cn(
                'motion-control min-w-0 flex-1 rounded-md px-2 py-1.5 text-left outline-none transition-colors hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring',
                node.id === selectedId && 'bg-accent',
              )}
              onClick={() => onSelect(node.id)}
            >
              <span
                className={cn(
                  'line-clamp-2 text-[13px] leading-[1.5]',
                  node.depth === 0 ? 'font-bold' : 'font-medium',
                )}
              >
                {title}
              </span>
              <span className="mt-0.5 block truncate text-[11px] text-muted-foreground">
                {[node.article.journal_title, node.article.date].filter(Boolean).join(' · ')}
              </span>
            </button>
          </li>
        );
      })}
    </ul>
  );
}
