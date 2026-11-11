/**
 * Relation-graph workspace: an interactive network of an article and its related articles beside
 * the reading desk.
 */

import { ReactFlowProvider, useReactFlow } from '@xyflow/react';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import {
  List,
  Maximize,
  Minus,
  Plus,
  RotateCcw,
  Search,
  Share2,
  Loader2,
} from '@/components/ui/animated-icons';
import { useSearch } from '@tanstack/react-router';
import { useCallback, useEffect, useMemo, useState, type FormEvent, type ReactNode } from 'react';

import {
  CheckedArticleDetailDialogContent,
  CheckedArticleDetailPane,
} from '@/components/feature/article-detail-dialog-content';
import { ReaderPlaceholder } from '@/components/feature/reader-placeholder';
import { useFavoriteChecks } from '@/components/feature/use-favorite-checks';
import { WorkspaceShell } from '@/components/feature/workspace-shell';
import { RelationCanvas } from '@/components/relations/relation-canvas';
import { RelationGraphFavoriteControls } from '@/components/relations/relation-graph-favorite-controls';
import { Button } from '@/components/ui/button';
import { IconTooltip } from '@/components/ui/tooltip';
import { Dialog } from '@/components/ui/dialog';
import { Input } from '@/components/ui/input';
import { RevealItem } from '@/components/ui/reveal';
import { ToggleGroup, ToggleGroupItem } from '@/components/ui/toggle-group';
import {
  articlePagesQueryKey,
  articleQuery,
  getArticles,
  relatedArticlesQuery,
  relationGraphFavoriteQuery,
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
import { useUpdateWorkspaceSearch, WORKSPACE_ROUTE_ID } from '@/lib/search-params';
import { useSelectedDatabase } from '@/lib/selected-database';
import { READER_DOCK_MEDIA_QUERY, useMediaQuery } from '@/lib/use-media-query';
import { LoadingText } from '@/components/ui/loading-text';
import * as stylex from '@stylexjs/stylex';
import { animations } from '@/lib/animations.stylex';
import { colors, lineHeights } from '@/lib/theme.stylex';

/** Related lists are shared with the article reader and change only when the index updates. */
const RELATED_STALE_TIME_MS = 10 * 60 * 1000;
/** Candidate center articles listed for one picker search. */
const RELATION_PICKER_LIMIT = 8;

type RelationViewMode = 'graph' | 'list';

/**
 * Render the relation workspace, restoring the session's last graph when no article is chosen
 * and loading a saved graph when the URL names one.
 *
 * @returns Relation-graph workspace.
 */
export function RelationsWorkspaceView() {
  const search = useSearch({ from: WORKSPACE_ROUTE_ID });
  const updateSearch = useUpdateWorkspaceSearch();
  const articleId = search.article ?? null;
  const dbName = search.db ?? null;
  const savedId = search.saved ?? null;
  const [stored] = useState(readStoredRelationGraph);
  const isRestoring = (!articleId || !dbName) && stored !== null;
  const savedQuery = useQuery({
    ...relationGraphFavoriteQuery(savedId ?? 0),
    enabled: savedId !== null && Boolean(articleId && dbName),
    staleTime: Number.POSITIVE_INFINITY,
    retry: false,
  });

  useEffect(() => {
    if (isRestoring && stored) {
      void updateSearch((current) => ({
        ...current,
        article: stored.graph.rootId,
        db: stored.dbName,
        saved: stored.savedId ?? undefined,
      }));
    }
  }, [isRestoring, stored, updateSearch]);

  if (!articleId || !dbName) {
    return isRestoring ? null : (
      <RelationPicker
        onPick={(article, pickedDbName) =>
          void updateSearch((current) => ({
            ...current,
            article: relationNodeId(article),
            db: pickedDbName,
          }))
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
    savedQuery.data.root_article_id === articleId
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
) {
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
    queryKey: articlePagesQueryKey({ q: query, limit: RELATION_PICKER_LIMIT }, dbName),
    queryFn: ({ signal }) =>
      getArticles({ q: query, limit: RELATION_PICKER_LIMIT }, null, dbName, signal),
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
      <div {...stylex.props(styles.relationPickerDiv)}>
        <search>
          <form {...stylex.props(styles.relationPickerForm)} onSubmit={handleSubmit}>
            <Input
              value={draft}
              onChange={(event) => setDraft(event.target.value)}
              placeholder="输入标题、作者或关键词"
              aria-label="检索中心文章"
            />
            <Button type="submit" xstyle={styles.relationPickerButton}>
              <Search xstyle={styles.relationPickerSearch} aria-hidden="true" />
              检索
            </Button>
          </form>
        </search>
        {resultsQuery.isFetching && (
          <output {...stylex.props(styles.statusOutput, styles.relationPickerP)}>
            <LoadingText text="正在检索…" />
          </output>
        )}
        {resultsQuery.isError && (
          <p role="alert" {...stylex.props(styles.relationPickerP2)}>
            检索失败，请稍后重试。
          </p>
        )}
        {resultsQuery.data && resultsQuery.data.items.length === 0 && (
          <p {...stylex.props(styles.relationPickerP)}>没有找到匹配的文章。</p>
        )}
        {resultsQuery.data && resultsQuery.data.items.length > 0 && (
          <ul aria-label="候选中心文章" {...stylex.props(styles.relationPickerUl)}>
            {resultsQuery.data.items.map((article) => (
              <li {...stylex.props(styles.relationPickerUlItem)} key={relationNodeId(article)}>
                <button
                  type="button"
                  {...stylex.props(styles.relationPickerButton2)}
                  onClick={() => onPick(article, dbName)}
                >
                  <span {...stylex.props(styles.relationPickerSpan)}>
                    {getArticleDisplayTitle(article)}
                  </span>
                  <span {...stylex.props(styles.relationPickerSpan2)}>
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
function RelationHeading({
  subtitle,
  isSubtitleReady = true,
  children,
}: {
  subtitle: string;
  isSubtitleReady?: boolean;
  children?: ReactNode;
}) {
  const subtitleElement = <p {...stylex.props(styles.subtitleElementP)}>{subtitle}</p>;
  return (
    <div {...stylex.props(styles.relationHeadingDiv)}>
      <div {...stylex.props(styles.relationHeadingDiv2)}>
        <h1 {...stylex.props(styles.relationHeadingH1)}>文献关系图</h1>
        <RevealItem isReady={isSubtitleReady} placeholder={subtitleElement}>
          {subtitleElement}
        </RevealItem>
      </div>
      {children && <div {...stylex.props(styles.relationHeadingDiv3)}>{children}</div>}
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
        ...relatedArticlesQuery(nodeId, dbName, RELATION_CHILD_LIMIT),
        staleTime: RELATED_STALE_TIME_MS,
      }),
    [dbName, queryClient],
  );

  const initialQuery = useQuery({
    queryKey: [...articleQuery(articleId, dbName).queryKey, 'relation-graph'],
    queryFn: async () => {
      const root = await queryClient.fetchQuery(articleQuery(articleId, dbName));
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
            duration: 300,
            padding: 0.15,
            maxZoom: 1,
          });
        }),
      );
    },
    [fitView],
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
    <div {...stylex.props(styles.headerDiv)}>
      <RelationHeading
        isSubtitleReady={Boolean(root) || initialQuery.isError}
        subtitle={
          root
            ? `以「${getArticleDisplayTitle(root)}」为中心 · ${nodeCount} 篇`
            : '正在读取中心文章…'
        }
      >
        <ToggleGroup
          aria-label="显示方式"
          xstyle={styles.headerToggleGroup}
          value={viewMode}
          onValueChange={setViewMode}
        >
          {(
            [
              ['graph', '网络', Share2],
              ['list', '列表', List],
            ] as const
          ).map(([mode, label, Icon]) => (
            <ToggleGroupItem key={mode} value={mode} xstyle={styles.headerToggleGroupItem}>
              <Icon xstyle={styles.headerIcon} aria-hidden="true" />
              {label}
            </ToggleGroupItem>
          ))}
        </ToggleGroup>
        {viewMode === 'graph' && (
          <IconTooltip label="重新排列">
            <Button
              type="button"
              variant="ghost"
              size="icon"
              xstyle={styles.headerButton}
              aria-label="重新排列"
              onClick={relayout}
            >
              <RotateCcw xstyle={styles.relationPickerSearch} aria-hidden="true" />
            </Button>
          </IconTooltip>
        )}
        {viewMode === 'graph' && (
          <IconTooltip label="适应窗口">
            <Button
              type="button"
              variant="ghost"
              size="icon"
              xstyle={styles.headerButton}
              aria-label="适应窗口"
              onClick={() => void fitView({ duration: 300, padding: 0.2 })}
            >
              <Maximize xstyle={styles.relationPickerSearch} aria-hidden="true" />
            </Button>
          </IconTooltip>
        )}
        <RevealItem isReady={Boolean(currentGraph && root)} placeholder={null}>
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
        </RevealItem>
      </RelationHeading>
      {failure && (
        <p role="alert" {...stylex.props(styles.headerP)}>
          {failure}
        </p>
      )}
      {nodeCount >= RELATION_NODE_LIMIT && (
        <p {...stylex.props(styles.headerP2)}>
          图中已有 {RELATION_NODE_LIMIT} 篇文章，收起部分分支后可继续展开。
        </p>
      )}
    </div>
  );

  let body: ReactNode;
  if (initialQuery.isError && currentGraph === null) {
    body = (
      <p role="alert" {...stylex.props(styles.relationExplorerP)}>
        无法读取中心文章，它可能已不在当前数据库中。
      </p>
    );
  } else if (currentGraph === null) {
    body = null;
  } else if (viewMode === 'graph') {
    body = (
      <div data-slot="relation-canvas" {...stylex.props(styles.relationCanvas)}>
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
        <RevealItem
          isReady={currentGraph !== null || initialQuery.isError}
          xstyle={styles.relationExplorerRevealItem}
          placeholder={
            <output {...stylex.props(styles.statusOutput, styles.relationExplorerP2)}>
              <Loader2 xstyle={styles.relationExplorerLoader2} aria-hidden="true" />
              <LoadingText text="正在构建关系图…" />
            </output>
          }
        >
          {body}
        </RevealItem>
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
    <ul aria-label="关系节点列表" {...stylex.props(styles.relationListUl)}>
      {nodes.map((node) => {
        const title = getArticleDisplayTitle(node.article);
        const isExpanded = graph.expandedIds.includes(node.id);
        const isLoading = loadingIds.has(node.id);
        return (
          <li
            key={node.id}
            {...stylex.props(styles.relationListLi, styles.indent(`${16 + node.depth * 22}px`))}
          >
            <button
              type="button"
              aria-label={`${isExpanded ? '收起' : '展开'}相关文章：${title}`}
              aria-expanded={isExpanded}
              disabled={isLoading}
              {...stylex.props(styles.relationListButton)}
              onClick={() => onToggle(node.id)}
            >
              {isLoading ? (
                <Loader2 xstyle={styles.relationListLoader2} aria-hidden="true" />
              ) : isExpanded ? (
                <Minus xstyle={styles.headerIcon} aria-hidden="true" />
              ) : (
                <Plus xstyle={styles.headerIcon} aria-hidden="true" />
              )}
            </button>
            <button
              type="button"
              aria-pressed={node.id === selectedId}
              {...stylex.props(
                styles.relationListButton2,
                node.id === selectedId && styles.relationListButtonSelectedId,
              )}
              onClick={() => onSelect(node.id)}
            >
              <span
                {...stylex.props(
                  styles.relationListSpan,
                  node.depth === 0 ? styles.relationListSpanRoot : styles.relationListSpanChild,
                )}
              >
                {title}
              </span>
              <span {...stylex.props(styles.relationListSpan2)}>
                {[node.article.journal_title, node.article.date].filter(Boolean).join(' · ')}
              </span>
            </button>
          </li>
        );
      })}
    </ul>
  );
}

const styles = stylex.create({
  statusOutput: {
    display: 'block',
  },
  indent: (paddingLeft: string) => ({
    paddingLeft,
  }),
  relationPickerDiv: {
    marginInline: 'auto',
    width: '100%',
    maxWidth: '36rem',
    paddingInline: {
      default: '1rem',
      '@media (min-width: 40rem)': '1.25rem',
    },
    paddingBlock: '2rem',
  },
  relationPickerForm: {
    display: 'flex',
    gap: '0.5rem',
  },
  relationPickerButton: {
    flexShrink: 0,
    gap: '0.375rem',
  },
  relationPickerSearch: {
    width: '1rem',
    height: '1rem',
  },
  relationPickerP: {
    marginTop: '1rem',
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
    color: colors.mutedForeground,
  },
  relationPickerP2: {
    marginTop: '1rem',
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
    color: colors.destructive,
  },
  relationPickerUlItem: {
    borderTopStyle: {
      default: null,
      ':not(:last-child)': 'solid',
    },
    borderTopWidth: {
      default: null,
      ':not(:last-child)': '0px',
    },
    borderBottomStyle: {
      default: null,
      ':not(:last-child)': 'solid',
    },
    borderBottomWidth: {
      default: null,
      ':not(:last-child)': '1px',
    },
  },
  relationPickerUl: {
    marginTop: '1rem',
    borderBlockStyle: 'solid',
    borderBlockWidth: '1px',
  },
  relationPickerButton2: {
    display: 'block',
    width: '100%',
    paddingInline: '0.25rem',
    paddingBlock: '0.75rem',
    textAlign: 'left',
    transitionProperty:
      'color, background-color, border-color, outline-color, text-decoration-color, fill, stroke',
    transitionTimingFunction: 'cubic-bezier(0.25, 0.1, 0.25, 1)',
    transitionDuration: '200ms',
    outlineStyle: 'none',
    backgroundColor: {
      default: null,
      ':hover': {
        default: null,
        '@media (hover: hover)': colors.accent,
      },
    },
    boxShadow: {
      default: null,
      ':focus-visible': `inset 0 0 0 2px ${colors.ring}`,
    },
  },
  relationPickerSpan: {
    overflow: 'hidden',
    display: '-webkit-box',
    WebkitBoxOrient: 'vertical',
    WebkitLineClamp: 2,
    fontSize: '13.5px',
    lineHeight: 1.5,
    fontWeight: 700,
  },
  relationPickerSpan2: {
    marginTop: '0.25rem',
    display: 'block',
    overflow: 'hidden',
    textOverflow: 'ellipsis',
    whiteSpace: 'nowrap',
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    color: colors.mutedForeground,
  },
  subtitleElementP: {
    overflow: 'hidden',
    textOverflow: 'ellipsis',
    whiteSpace: 'nowrap',
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    color: colors.mutedForeground,
  },
  relationHeadingDiv: {
    display: 'flex',
    flexWrap: 'wrap',
    alignItems: 'flex-start',
    justifyContent: 'space-between',
    columnGap: '0.75rem',
    rowGap: '0.5rem',
  },
  relationHeadingDiv2: {
    minWidth: '0px',
    flexGrow: {
      default: null,
      '@media (min-width: 40rem)': 1,
    },
    flexShrink: {
      default: null,
      '@media (min-width: 40rem)': 1,
    },
    flexBasis: {
      default: '100%',
      '@media (min-width: 40rem)': '0%',
    },
  },
  relationHeadingH1: {
    fontSize: '1.125rem',
    lineHeight: '1.75rem',
    fontWeight: 700,
  },
  relationHeadingDiv3: {
    display: 'flex',
    flexShrink: 0,
    alignItems: 'center',
    gap: '0.25rem',
  },
  headerDiv: {
    display: 'flex',
    flexDirection: 'column',
    rowGap: '0.5rem',
  },
  headerToggleGroup: {
    marginRight: '0.25rem',
  },
  headerToggleGroupItem: {
    paddingInline: {
      default: '0.75rem',
      '@media (min-width: 48rem)': '0.5rem',
    },
  },
  headerIcon: {
    width: '0.875rem',
    height: '0.875rem',
  },
  headerButton: {
    width: {
      default: '2.75rem',
      '@media (min-width: 48rem)': '2rem',
    },
    height: {
      default: '2.75rem',
      '@media (min-width: 48rem)': '2rem',
    },
  },
  headerP: {
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    color: colors.destructive,
  },
  headerP2: {
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    color: colors.mutedForeground,
  },
  relationExplorerP: {
    paddingInline: '1.25rem',
    paddingBlock: '2rem',
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
    color: colors.destructive,
  },
  relationCanvas: {
    minHeight: '0px',
    flex: '1',
  },
  relationExplorerRevealItem: {
    display: 'flex',
    minHeight: '0px',
    flex: '1',
    flexDirection: 'column',
  },
  relationExplorerP2: {
    display: 'flex',
    alignItems: 'center',
    gap: '0.5rem',
    paddingInline: '1.25rem',
    paddingBlock: '2rem',
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
    color: colors.mutedForeground,
  },
  relationExplorerLoader2: {
    width: '1rem',
    height: '1rem',
    animationName: animations.spin,
    animationDuration: '1s',
    animationTimingFunction: 'linear',
    animationIterationCount: 'infinite',
  },
  relationListUl: {
    minHeight: '0px',
    flex: '1',
    overflowY: 'auto',
    paddingBlock: '0.5rem',
  },
  relationListLi: {
    display: 'flex',
    alignItems: 'flex-start',
    gap: '0.25rem',
    paddingRight: '1rem',
  },
  relationListButton: {
    marginTop: '0.5rem',
    display: 'flex',
    width: '1.5rem',
    height: '1.5rem',
    flexShrink: 0,
    alignItems: 'center',
    justifyContent: 'center',
    borderRadius: 'calc(infinity * 1px)',
    borderStyle: 'solid',
    borderWidth: '1px',
    color: {
      default: colors.mutedForeground,
      ':hover': {
        default: null,
        '@media (hover: hover)': colors.foreground,
      },
    },
    transitionProperty:
      'color, background-color, border-color, outline-color, text-decoration-color, fill, stroke',
    transitionTimingFunction: 'cubic-bezier(0.25, 0.1, 0.25, 1)',
    transitionDuration: '200ms',
    outlineStyle: 'none',
    boxShadow: {
      default: null,
      ':focus-visible': `0 0 0 2px ${colors.ring}`,
    },
  },
  relationListLoader2: {
    width: '0.875rem',
    height: '0.875rem',
    animationName: animations.spin,
    animationDuration: '1s',
    animationTimingFunction: 'linear',
    animationIterationCount: 'infinite',
  },
  relationListButton2: {
    minWidth: '0px',
    flex: '1',
    borderRadius: '6px',
    paddingInline: '0.5rem',
    paddingBlock: '0.375rem',
    textAlign: 'left',
    transitionProperty:
      'color, background-color, border-color, outline-color, text-decoration-color, fill, stroke',
    transitionTimingFunction: 'cubic-bezier(0.25, 0.1, 0.25, 1)',
    transitionDuration: '200ms',
    outlineStyle: 'none',
    backgroundColor: {
      default: null,
      ':hover': {
        default: null,
        '@media (hover: hover)': colors.accent,
      },
    },
    boxShadow: {
      default: null,
      ':focus-visible': `0 0 0 2px ${colors.ring}`,
    },
  },
  relationListButtonSelectedId: {
    backgroundColor: {
      default: colors.accent,
      ':hover': {
        default: null,
        '@media (hover: hover)': colors.accent,
      },
    },
  },
  relationListSpan: {
    overflow: 'hidden',
    display: '-webkit-box',
    WebkitBoxOrient: 'vertical',
    WebkitLineClamp: 2,
    fontSize: '13px',
    lineHeight: 1.5,
  },
  relationListSpanRoot: {
    fontWeight: 700,
  },
  relationListSpanChild: {
    fontWeight: 500,
  },
  relationListSpan2: {
    marginTop: '0.125rem',
    display: 'block',
    overflow: 'hidden',
    textOverflow: 'ellipsis',
    whiteSpace: 'nowrap',
    fontSize: '11px',
    color: colors.mutedForeground,
  },
});
