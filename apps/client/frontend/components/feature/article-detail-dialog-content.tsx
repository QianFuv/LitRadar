/**
 * Article metadata and responsive, accessible detail actions.
 */

import { useEffect, useRef, useState, type MouseEventHandler, type ReactNode } from 'react';
import { useQuery } from '@tanstack/react-query';
import { Link, useSearch } from '@tanstack/react-router';
import {
  AlertCircle,
  AlertTriangle,
  ArrowLeft,
  BookOpen,
  Download,
  ExternalLink,
  FileText,
  Settings,
  Share2,
  X,
  Loader2,
} from '@/components/ui/animated-icons';

import {
  getArticleActionUrlForDatabase,
  favoriteCheckQuery,
  articleAccessQuery,
  journalQuery,
  type ArticleAccessResponse,
  type Article,
  type JournalRecord,
} from '@/lib/api';
import { FavoriteButton } from '@/components/feature/favorite-button';
import { RelatedArticles } from '@/components/feature/related-articles';
import { useAuth } from '@/lib/auth-context';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { RevealItem } from '@/components/ui/reveal';
import { attachHorizontalWheelScroll } from '@/lib/horizontal-wheel-scroll';
import { CopyButton } from '@/components/ui/copy-button';
import { DialogClose, DialogContent, DialogDescription, DialogTitle } from '@/components/ui/dialog';
import {
  MotionArticle,
  MotionDiv,
  MotionParagraph,
  MotionPresence,
  PANEL_VARIANTS,
  SWAP_VARIANTS,
} from '@/components/ui/motion';
import { copyTextToClipboard } from '@/lib/clipboard';
import { getDoiUrl } from '@/lib/citation';
import { getArticleDisplayTitle, hasArticleTitle } from '@/lib/article-title';
import { useFtsHighlighter } from '@/lib/fts-highlight';
import { useTransientState } from '@/lib/use-transient-state';
import { JOURNAL_RATING_SYSTEMS } from '@/lib/journal-ratings';
import { buildRelationSearch } from '@/lib/relation-graph';
import { WORKSPACE_ROUTE_ID, type WorkspaceSearch } from '@/lib/search-params';
import { withSettingsSection } from '@/lib/settings-center';
import * as stylex from '@stylexjs/stylex';
import { animations } from '@/lib/animations.stylex';
import { colors, lineHeights } from '@/lib/theme.stylex';

type ArticleDetailDialogArticle = Article;

type ArticleDetailDialogContentProps = {
  article: ArticleDetailDialogArticle;
  dbName: string;
  initialFolderIds?: number[];
  isFavoriteStatePending?: boolean;
  isFavoriteStateUnavailable?: boolean;
  /** Folder whose membership the favorite button toggles directly, without a folder picker. */
  favoriteToggleFolderId?: number;
  extraActions?: ReactNode;
};

type ArticleCopyTarget = 'title' | 'info';

type ArticleDetailVariant = 'dialog' | 'pane';

/** Time copy feedback stays visible after the last copy. */
const COPY_FEEDBACK_MS = 3000;

/** Short forms of the default access labels used when the docked toolbar is narrow. */
const ACCESS_SHORT_LABELS = new Map(
  Object.entries({
    查看摘要页: '摘要页',
    获取全文: '全文',
  }),
);

/**
 * Check whether an API flag is explicitly enabled.
 *
 * @param value - API flag value.
 * @returns True when the flag is explicitly enabled.
 */
function isEnabledFlag(value: number | boolean | string | null | undefined): boolean {
  return value === true || value === 1 || value === '1';
}

/**
 * Build the existing plain-text article information summary.
 *
 * @param article - Article record.
 * @returns Multi-line article information.
 */
function buildArticleInfoText(article: ArticleDetailDialogArticle): string {
  const doiUrl = getDoiUrl(article.doi);
  const authors = article.authors?.join('; ') ?? '';
  return [
    `标题：${hasArticleTitle(article) ? article.title : '缺失'}`,
    `作者：${authors || '暂无'}`,
    `期刊：${article.journal_title || '暂无'}`,
    `日期：${article.date || '暂无'}`,
    ...getOptionalArticleInfoFields(article, doiUrl),
  ]
    .filter(Boolean)
    .join('\n');
}

/**
 * Build the concise dialog description from journal metadata.
 *
 * @param article - Article record.
 * @returns Human-readable journal/date description.
 */
function buildArticleDescription(article: ArticleDetailDialogArticle): string {
  const parts = [
    article.journal_title || (article.journal_id ? `期刊 ${article.journal_id}` : ''),
    (article.volume || article.number) &&
      [article.volume && `第 ${article.volume} 卷`, article.number && `第 ${article.number} 期`]
        .filter(Boolean)
        .join(', '),
    article.date || (article.publication_year ? String(article.publication_year) : ''),
  ].filter(Boolean);

  return parts.join(' • ');
}

/** Navigation callbacks for related articles opened inside the same reader. */
type ArticleTrailProps = {
  /** Return to the previously opened article; absent for the article chosen from the list. */
  onBack?: () => void;
  /** Article that `onBack` returns to. */
  previousArticle?: Article;
  /** Open a related article in place. */
  onOpenRelated: (article: Article) => void;
};

/**
 * Track related articles opened on top of the selected article.
 *
 * @returns Current related article, the related article below it, and navigation actions.
 */
function useRelatedArticleTrail() {
  const [trail, setTrail] = useState<Article[]>([]);
  return {
    current: trail.at(-1),
    previous: trail.at(-2),
    open: (article: Article) => setTrail((current) => [...current.slice(-9), article]),
    back: () => setTrail((current) => current.slice(0, -1)),
  };
}

/**
 * Load the real favorite membership of a related article before rendering its details.
 *
 * @param props - Related article, database, and a render callback receiving favorite props.
 * @returns Rendered details with accurate favorite state.
 */
function RelatedArticleFavoriteState({
  article,
  dbName,
  children,
}: {
  article: Article;
  dbName: string;
  children: (favoriteProps: {
    initialFolderIds: number[];
    isFavoriteStatePending: boolean;
  }) => ReactNode;
}) {
  const { user } = useAuth();
  const { data, isPending } = useQuery({
    ...favoriteCheckQuery(article.article_id, dbName),
    enabled: Boolean(user),
    staleTime: 5 * 60 * 1000,
  });
  return children({
    initialFolderIds: data?.map((item) => item.folder_id) ?? [],
    isFavoriteStatePending: Boolean(user) && isPending,
  });
}

/**
 * Render article details as a right-side sheet on narrow screens.
 *
 * @param props - Article detail configuration.
 * @returns Article detail dialog content.
 */
export function ArticleDetailDialogContent(props: ArticleDetailDialogContentProps) {
  const trail = useRelatedArticleTrail();
  return (
    <DialogContent placement="right" xstyle={styles.articleDetailDialogContentDialogContent}>
      <div {...stylex.props(styles.articleDetailDialogContentDiv)}>
        <DialogClose
          render={
            <Button
              type="button"
              variant="ghost"
              size="icon"
              static
              aria-label="关闭"
              xstyle={styles.articleDetailDialogContentButton}
            />
          }
        >
          <X xstyle={styles.articleDetailDialogContentX} aria-hidden="true" />
        </DialogClose>
        {trail.current ? (
          <Button
            type="button"
            variant="ghost"
            size="sm"
            xstyle={styles.articleDetailDialogContentButton2}
            onClick={trail.back}
          >
            <ArrowLeft xstyle={styles.articleDetailDialogContentArrowLeft} aria-hidden="true" />
            返回上一篇
          </Button>
        ) : (
          <span {...stylex.props(styles.articleDetailDialogContentSpan)}>文献详情</span>
        )}
      </div>
      <div {...stylex.props(styles.articleDetailDialogContentDiv2)}>
        {trail.current ? (
          <RelatedArticleFavoriteState article={trail.current} dbName={props.dbName}>
            {(favoriteProps) => (
              <ArticleDialogBodyView
                key={String(trail.current?.article_id)}
                article={trail.current!}
                dbName={props.dbName}
                {...favoriteProps}
                onBack={trail.back}
                onOpenRelated={trail.open}
              />
            )}
          </RelatedArticleFavoriteState>
        ) : (
          <ArticleDialogBodyView {...props} onOpenRelated={trail.open} />
        )}
      </div>
    </DialogContent>
  );
}

/**
 * Render one article inside the narrow-screen sheet.
 *
 * @param props - Article detail configuration and trail callbacks.
 * @returns Sheet body for one article.
 */
function ArticleDialogBodyView(props: ArticleDetailDialogContentProps & ArticleTrailProps) {
  const state = useArticleDetailViewState(props);
  const rootRef = useRef<HTMLDivElement>(null);
  const isRelated = Boolean(props.onBack);
  useEffect(() => {
    if (isRelated) rootRef.current?.scrollIntoView?.({ block: 'start' });
  }, [isRelated]);
  return <div ref={rootRef}>{renderArticleDetailBody(state, 'dialog', props.onOpenRelated)}</div>;
}

/**
 * Render a docked article whose favorite membership is loaded first, for readers that show
 * articles outside favorite-aware lists such as the relation graph.
 *
 * @param props - Article and database.
 * @returns Reading-desk article with accurate favorite state.
 */
export function CheckedArticleDetailPane({
  article,
  dbName,
}: {
  article: Article;
  dbName: string;
}) {
  return (
    <RelatedArticleFavoriteState article={article} dbName={dbName}>
      {(favoriteProps) => (
        <ArticleDetailPane article={article} dbName={dbName} {...favoriteProps} />
      )}
    </RelatedArticleFavoriteState>
  );
}

/**
 * Render the narrow-screen article sheet with favorite membership loaded first, for views that
 * show articles outside favorite-aware lists such as the relation graph.
 *
 * @param props - Article and database.
 * @returns Article detail sheet content with accurate favorite state.
 */
export function CheckedArticleDetailDialogContent({
  article,
  dbName,
}: {
  article: Article;
  dbName: string;
}) {
  return (
    <RelatedArticleFavoriteState article={article} dbName={dbName}>
      {(favoriteProps) => (
        <ArticleDetailDialogContent article={article} dbName={dbName} {...favoriteProps} />
      )}
    </RelatedArticleFavoriteState>
  );
}

/**
 * Render article details inside the docked side panel.
 *
 * @param props - Article detail configuration.
 * @returns Reading-desk article content.
 */
export function ArticleDetailPane(props: ArticleDetailDialogContentProps) {
  const trail = useRelatedArticleTrail();
  if (trail.current) {
    const current = trail.current;
    return (
      <RelatedArticleFavoriteState article={current} dbName={props.dbName}>
        {(favoriteProps) => (
          <ArticlePaneView
            key={String(current.article_id)}
            article={current}
            dbName={props.dbName}
            {...favoriteProps}
            onBack={trail.back}
            previousArticle={trail.previous ?? props.article}
            onOpenRelated={trail.open}
          />
        )}
      </RelatedArticleFavoriteState>
    );
  }
  return <ArticlePaneView {...props} onOpenRelated={trail.open} />;
}

/**
 * Render one article in the docked reader with its sticky toolbar, clearing in like a tab panel
 * each time the reader switches to it.
 *
 * The pane owns its presence boundary because it is portaled out of list rows whose presence
 * context keeps `initial: false` from the list's first render, which would block this entrance.
 *
 * @param props - Article detail configuration and trail callbacks.
 * @returns Reading-desk article.
 */
function ArticlePaneView(props: ArticleDetailDialogContentProps & ArticleTrailProps) {
  const state = useArticleDetailViewState(props);
  const articleRef = useRef<HTMLElement>(null);
  const isRelated = Boolean(props.onBack);
  useEffect(() => {
    if (isRelated) articleRef.current?.scrollIntoView?.({ block: 'start' });
  }, [isRelated]);
  return (
    <MotionPresence initial>
      <MotionArticle
        key={String(state.article.article_id)}
        ref={articleRef}
        aria-label={`文献详情：${getArticleDisplayTitle(state.article)}`}
        {...stylex.props(styles.articlePaneViewMotionArticle)}
        variants={PANEL_VARIANTS}
        initial="hidden"
        animate="visible"
      >
        <div {...stylex.props(styles.articlePaneViewDiv)}>
          {props.onBack && (
            <nav aria-label="阅读路径" {...stylex.props(styles.articlePaneViewNav)}>
              <button
                type="button"
                aria-label="返回上一篇"
                title={
                  props.previousArticle ? getArticleDisplayTitle(props.previousArticle) : undefined
                }
                {...stylex.props(styles.articlePaneViewButton)}
                onClick={props.onBack}
              >
                <ArrowLeft xstyle={styles.articlePaneViewArrowLeft} aria-hidden="true" />
                <span {...stylex.props(styles.articlePaneViewSpan)}>返回上一篇</span>
                {props.previousArticle && (
                  <span {...stylex.props(styles.articlePaneViewSpan2)}>
                    · {getArticleDisplayTitle(props.previousArticle)}
                  </span>
                )}
              </button>
            </nav>
          )}
          <div
            data-slot="article-detail-toolbar"
            ref={attachHorizontalWheelScroll}
            {...stylex.props(styles.articleDetailToolbar)}
          >
            {renderArticleActions(state, 'pane')}
          </div>
        </div>
        <div {...stylex.props(styles.articlePaneViewDiv2)}>
          {renderArticleDetailBody(state, 'pane', props.onOpenRelated)}
        </div>
      </MotionArticle>
    </MotionPresence>
  );
}

/** Render the shared article header, actions, abstract, and identifiers. */
function renderArticleDetailBody(
  state: ArticleDetailViewState,
  variant: ArticleDetailVariant,
  onOpenRelated: (article: Article) => void,
) {
  const { article } = state;
  const isOpenAccess = isEnabledFlag(article.open_access);
  const isInPress = isEnabledFlag(article.in_press);

  return (
    <div data-article-detail-variant={variant} {...stylex.props(styles.renderArticleDetailBodyDiv)}>
      {renderArticleDetailHeader(state, variant)}
      {article.authors && article.authors.length > 0 && (
        <section {...stylex.props(styles.renderArticleDetailBodySection)}>
          <h3 {...stylex.props(styles.renderArticleDetailBodyH3)}>作者</h3>
          <p {...stylex.props(styles.renderArticleDetailBodyP)}>{article.authors.join('; ')}</p>
        </section>
      )}
      {(isOpenAccess || isInPress) && (
        <div {...stylex.props(styles.renderArticleDetailBodyDiv2)}>
          {isOpenAccess && <Badge variant="success">开放获取</Badge>}
          {isInPress && <Badge variant="outline">预发表</Badge>}
        </div>
      )}

      {renderRetractionNotice(article)}

      {variant === 'dialog' && (
        <div {...stylex.props(styles.renderArticleDetailBodyDiv3)}>
          {renderArticleActions(state, variant)}
        </div>
      )}

      <section {...stylex.props(styles.renderArticleDetailBodySection2)}>
        <h3 {...stylex.props(styles.renderArticleDetailBodyH32)}>
          <span {...stylex.props(styles.renderArticleDetailBodySpan)}>
            <FileText
              animateOnView
              xstyle={styles.renderArticleDetailBodyFileText}
              aria-hidden="true"
            />
            摘要
          </span>
        </h3>
        <p {...stylex.props(styles.renderArticleDetailBodyP2)}>
          {article.abstract ? state.highlightText(article.abstract) : '暂无摘要。'}
        </p>
      </section>

      {renderArticleMetadata(article, state.journal)}
      <RelatedArticles article={article} dbName={state.dbName} onOpen={onOpenRelated} />
    </div>
  );
}

/**
 * Format the article page range.
 *
 * @param article - Article record.
 * @returns Page range, a single page, or null.
 */
function formatPageRange(article: ArticleDetailDialogArticle): string | null {
  const start = article.start_page?.trim();
  const end = article.end_page?.trim();
  if (start && end && start !== end) return `${start}–${end}`;
  return start || end || null;
}

/**
 * Render a persistent identifier as selectable text without creating an external link.
 *
 * @param value - Identifier text.
 * @returns Selectable identifier.
 */
function renderIdentifier(value: string) {
  return <span {...stylex.props(styles.renderIdentifierSpan)}>{value}</span>;
}

/** Render a prominent notice when the article has published retractions. */
function renderRetractionNotice(article: ArticleDetailDialogArticle) {
  const retractions = article.retraction_dois ?? [];
  if (retractions.length === 0) return null;
  return (
    <div role="note" aria-label="撤稿提醒" {...stylex.props(styles.renderRetractionNoticeDiv)}>
      <AlertTriangle xstyle={styles.renderRetractionNoticeAlertTriangle} aria-hidden="true" />
      <div {...stylex.props(styles.renderArticleDetailBodyDiv)}>
        <p {...stylex.props(styles.renderRetractionNoticeP)}>此文章已被撤稿</p>
        <p {...stylex.props(styles.renderRetractionNoticeP2)}>
          撤稿声明：
          {retractions.map((doi, index) => (
            <span key={doi}>
              {index > 0 && '、'}
              {renderIdentifier(doi)}
            </span>
          ))}
        </p>
      </div>
    </div>
  );
}

/**
 * Render metadata not already shown in the source line: pages, ISSNs, ratings, and identifiers.
 *
 * @param article - Article record.
 * @param journal - Journal record when it could be loaded.
 * @returns Metadata section, or null when nothing is known.
 */
function renderArticleMetadata(
  article: ArticleDetailDialogArticle,
  journal: JournalRecord | undefined,
) {
  const pages = formatPageRange(article);
  const issns = journal
    ? Array.from(new Set([journal.issn, journal.eissn, ...journal.issns].filter(Boolean)))
    : [];
  const ratings = journal
    ? JOURNAL_RATING_SYSTEMS.flatMap((system) => {
        const value = journal[system.key];
        return value ? [{ label: system.label, value }] : [];
      })
    : [];
  const rows: { label: string; value: ReactNode }[] = [];
  if (pages)
    rows.push({
      label: '页码',
      value: <span {...stylex.props(styles.renderArticleMetadataSpan)}>{pages}</span>,
    });
  if (issns.length > 0) {
    rows.push({
      label: 'ISSN',
      value: <span {...stylex.props(styles.renderArticleMetadataSpan)}>{issns.join(' / ')}</span>,
    });
  }
  if (ratings.length > 0) {
    rows.push({
      label: '期刊评级',
      value: (
        <span {...stylex.props(styles.renderArticleMetadataSpan2)}>
          {ratings.map((rating) => (
            <span key={rating.label} {...stylex.props(styles.renderArticleMetadataSpan3)}>
              <span {...stylex.props(styles.renderRetractionNoticeP2)}>{rating.label}</span>
              <span {...stylex.props(styles.renderArticleMetadataSpan4)}>{rating.value}</span>
            </span>
          ))}
        </span>
      ),
    });
  }
  if (journal?.area) rows.push({ label: '学科领域', value: journal.area });
  if (article.doi) rows.push({ label: 'DOI', value: renderIdentifier(article.doi) });
  if (article.pmid) rows.push({ label: 'PMID', value: renderIdentifier(article.pmid) });
  if (rows.length === 0) return null;
  return (
    <section {...stylex.props(styles.renderArticleDetailBodySection2)}>
      <h3 {...stylex.props(styles.renderArticleDetailBodyH32)}>
        <span {...stylex.props(styles.renderArticleDetailBodySpan)}>
          <BookOpen
            animateOnView
            xstyle={styles.renderArticleDetailBodyFileText}
            aria-hidden="true"
          />
          文献信息
        </span>
      </h3>
      <dl {...stylex.props(styles.renderArticleMetadataDl)}>
        {rows.map((row) => (
          <div
            key={row.label}
            {...stylex.props(styles.renderArticleMetadataDlItem, styles.renderArticleMetadataDiv)}
          >
            <dt {...stylex.props(styles.renderRetractionNoticeP2)}>{row.label}</dt>
            <dd {...stylex.props(styles.renderArticleMetadataDd)}>{row.value}</dd>
          </div>
        ))}
      </dl>
    </section>
  );
}

/**
 * Render an action label with a shorter form the docked toolbar switches to when it is narrow.
 *
 * The docked toolbar is the only size container, so the switch follows the toolbar's own width
 * instead of measuring overflow, and the labels do not flip when access actions load after an
 * article opens. The breakpoint fits the full labels of favorite, copy, abstract, full-text,
 * login, and relation actions.
 *
 * @param label - Full label.
 * @param shortLabel - Short label shown in a compact toolbar.
 * @returns Both label forms; only the full one is visible by default.
 */
function renderActionLabel(label: string, shortLabel: string) {
  return (
    <>
      <span data-action-label {...stylex.props(styles.renderActionLabelSpan)}>
        {label}
      </span>
      <span data-action-short {...stylex.props(styles.renderActionLabelSpan2)}>
        {shortLabel}
      </span>
    </>
  );
}

/**
 * Shorten known access labels for the compact toolbar.
 *
 * @param label - Access label from the server or its default.
 * @returns Short label, or the original label when no shorter form is known.
 */
function shortenAccessLabel(label: string): string {
  return ACCESS_SHORT_LABELS.get(label) ?? label;
}

/** Render the favorite, copy, access, and caller-provided article actions as one toolbar group. */
function renderArticleActions(state: ArticleDetailViewState, variant: ArticleDetailVariant) {
  return (
    <fieldset
      aria-label="文章操作"
      {...stylex.props(
        styles.groupFieldset,
        styles.renderArticleActionsDiv,
        variant === 'pane' ? styles.paneActions : styles.renderArticleActionsDivNotPane,
      )}
    >
      {renderArticleFavoriteAction(state)}
      {renderArticleInfoCopy(state)}
      {renderArticleAccessActions(state, variant)}
      {state.extraActions}
      {renderArticleRelationAction(state, variant)}
    </fieldset>
  );
}

/**
 * Link to the relation graph centred on this article; the docked toolbar pins it to the right.
 *
 * @param state - Article detail state.
 * @param variant - Detail layout.
 * @returns Relation-graph link.
 */
function renderArticleRelationAction(state: ArticleDetailViewState, variant: ArticleDetailVariant) {
  return (
    <Button
      asChild
      variant="outline"
      size="sm"
      xstyle={[
        styles.articleActionButton,
        variant === 'pane' && styles.renderArticleRelationActionButtonPane,
      ]}
    >
      <Link
        to="/"
        search={buildRelationSearch(state.article, state.dbName)}
        aria-label="查看关系图"
        title="查看关系图"
      >
        <Share2 xstyle={styles.renderArticleRelationActionShare2} aria-hidden="true" />
        {renderActionLabel('关系图', '关系')}
      </Link>
    </Button>
  );
}

/** Own unchanged detail hooks, query settings, and debounced copy feedback. */
function useArticleDetailViewState({
  article,
  dbName,
  initialFolderIds = [],
  isFavoriteStatePending = false,
  isFavoriteStateUnavailable = false,
  favoriteToggleFolderId,
  extraActions,
}: ArticleDetailDialogContentProps) {
  const search = useSearch({ from: WORKSPACE_ROUTE_ID });
  const highlightText = useFtsHighlighter(search.q ?? null);
  const [copyFeedback, setCopyFeedback] = useTransientState<{
    status: ArticleCopyTarget | null;
    error: string | null;
  } | null>(null, COPY_FEEDBACK_MS);
  const copyStatus = copyFeedback?.status ?? null;
  const copyError = copyFeedback?.error ?? null;
  const isAccessQueryEnabled = !!dbName && !!article.article_id;
  const canCopyTitle = hasArticleTitle(article);
  const {
    data: access,
    isPending: isAccessPending,
    isFetching: isAccessFetching,
    isError: isAccessError,
    error: accessError,
  } = useQuery({
    ...articleAccessQuery(article.article_id, dbName),
    enabled: isAccessQueryEnabled,
    staleTime: 0,
    refetchOnMount: 'always',
  });

  const journalId = article.journal_id ? String(article.journal_id) : '';
  const { data: journal } = useQuery({
    ...journalQuery(journalId, dbName),
    enabled: Boolean(dbName && journalId),
    staleTime: 30 * 60 * 1000,
    retry: false,
  });

  /**
   * Copy one article value and update its inline state.
   *
   * @param text - Text to copy.
   * @param status - Copy action identifier.
   */
  const handleCopy = async (text: string, status: ArticleCopyTarget) => {
    try {
      await copyTextToClipboard(text);
      setCopyFeedback({ status, error: null });
    } catch {
      setCopyFeedback({ status: null, error: '复制失败，请手动选择文本复制。' });
    }
  };

  /** Copy the article title. */
  const handleCopyTitle = async () => {
    if (hasArticleTitle(article)) {
      await handleCopy(article.title, 'title');
    }
  };

  /** Copy the plain-text article information summary. */
  const handleCopyArticleInfo = async () => {
    await handleCopy(buildArticleInfoText(article), 'info');
  };

  const {
    abstractAction,
    fulltextAction,
    abstractUrl,
    fullTextUrl,
    isAccessLoading,
    canShowAccessActions,
    accessState,
  } = getArticleAccessPresentation(
    article.article_id,
    dbName,
    access,
    isAccessQueryEnabled,
    isAccessPending,
    isAccessFetching,
    isAccessError,
  );
  const dataSourceSettingsSearch = withSettingsSection(search, 'data-sources');

  return {
    article,
    dbName,
    initialFolderIds,
    isFavoriteStatePending,
    isFavoriteStateUnavailable,
    favoriteToggleFolderId,
    extraActions,
    journal,
    copyStatus,
    copyError,
    isAccessQueryEnabled,
    canCopyTitle,
    isAccessPending,
    isAccessFetching,
    isAccessError,
    accessError,
    handleCopyTitle,
    handleCopyArticleInfo,
    abstractAction,
    fulltextAction,
    abstractUrl,
    fullTextUrl,
    isAccessLoading,
    canShowAccessActions,
    accessState,
    dataSourceSettingsSearch,
    highlightText,
  };
}

/**
 * Render the abstract-page link from the latest access data, kept through access refreshes.
 *
 * Abstract availability depends only on the article's identifiers and the configured providers,
 * never on the CNKI session, so refetching access only refreshes the full-text and login slot.
 *
 * @param state - Article detail view state.
 * @returns Abstract-page link, or nothing while access is unknown or unavailable.
 */
function renderArticleAbstractAction(state: ArticleDetailViewState) {
  const { abstractAction, abstractUrl } = state;

  return (
    abstractUrl && (
      <Button asChild variant="outline" size="sm" xstyle={styles.articleActionButton}>
        <a
          href={abstractUrl}
          target="_blank"
          rel="noreferrer"
          aria-label={abstractAction?.label ?? '查看摘要页'}
          title={abstractAction?.label ?? '查看摘要页'}
        >
          <ExternalLink xstyle={styles.renderArticleRelationActionShare2} aria-hidden="true" />
          {renderActionLabel(
            abstractAction?.label ?? '查看摘要页',
            shortenAccessLabel(abstractAction?.label ?? '查看摘要页'),
          )}
        </a>
      </Button>
    )
  );
}

/** Retain availability, exact nullish labels and fulltext action routing. */
function renderArticleFulltextAction(state: ArticleDetailViewState) {
  const { fulltextAction, fullTextUrl, canShowAccessActions } = state;

  return (
    canShowAccessActions &&
    fullTextUrl && (
      <Button asChild variant="outline" size="sm" xstyle={styles.articleActionButton}>
        <a
          href={fullTextUrl}
          target="_blank"
          rel="noreferrer"
          aria-label={fulltextAction?.label ?? '获取全文'}
          title={fulltextAction?.label ?? '获取全文'}
        >
          <Download xstyle={styles.renderArticleRelationActionShare2} aria-hidden="true" />
          {renderActionLabel(
            fulltextAction?.label ?? '获取全文',
            shortenAccessLabel(fulltextAction?.label ?? '获取全文'),
          )}
        </a>
      </Button>
    )
  );
}

/** Retain direct keyed access loading/error/action children and wait ownership. */
function renderArticleAccessActions(state: ArticleDetailViewState, variant: ArticleDetailVariant) {
  const {
    isAccessQueryEnabled,
    isAccessPending,
    isAccessFetching,
    isAccessError,
    accessError,
    fulltextAction,
    canShowAccessActions,
    accessState,
    dataSourceSettingsSearch,
  } = state;

  const rowStyles = [
    styles.accessRow,
    variant === 'pane' ? styles.accessRowPane : styles.accessRowWrapped,
  ];

  return (
    <RevealItem
      isReady={!(isAccessQueryEnabled && isAccessPending)}
      placeholder={
        <div data-article-access-state="loading" {...stylex.props(rowStyles)}>
          {renderArticleAccessLoading(state)}
        </div>
      }
    >
      <div {...stylex.props(rowStyles)}>
        {renderArticleAbstractAction(state)}
        <MotionPresence mode="wait">
          <MotionDiv
            key={accessState}
            data-article-access-state={accessState}
            {...stylex.props(rowStyles)}
            variants={SWAP_VARIANTS}
            initial="hidden"
            animate="visible"
            exit="exit"
          >
            {renderArticleAccessLoading(state)}
            {isAccessQueryEnabled && !isAccessFetching && isAccessError && (
              <Button
                variant="outline"
                size="sm"
                xstyle={styles.articleActionButton}
                aria-label="访问状态失败"
                disabled
                title={accessError instanceof Error ? accessError.message : '访问状态不可用'}
              >
                <AlertCircle
                  xstyle={styles.renderArticleAccessActionsAlertCircle}
                  aria-hidden="true"
                />
                {renderActionLabel('访问状态失败', '状态失败')}
              </Button>
            )}
            {renderArticleFulltextAction(state)}
            {canShowAccessActions &&
              fulltextAction?.requires_login &&
              (variant === 'dialog' ? (
                <DialogClose
                  nativeButton={false}
                  render={({ onClick }) =>
                    renderDataSourceSettingsLink(dataSourceSettingsSearch, onClick)
                  }
                />
              ) : (
                renderDataSourceSettingsLink(dataSourceSettingsSearch)
              ))}
          </MotionDiv>
        </MotionPresence>
      </div>
    </RevealItem>
  );
}

/**
 * Render the data-source settings shortcut shown when full text requires login.
 *
 * @param search - Workspace search that opens the data-source settings.
 * @param onClick - Click handler that also closes the surrounding detail dialog; the link keeps its
 * own link semantics instead of taking the dialog close button's.
 * @returns Settings link styled as a button.
 */
function renderDataSourceSettingsLink(
  search: WorkspaceSearch,
  onClick?: MouseEventHandler<HTMLAnchorElement>,
) {
  return (
    <Button asChild variant="outline" size="sm" xstyle={styles.articleActionButton}>
      <Link to="/" search={search} aria-label="去设置登录" title="去设置登录" onClick={onClick}>
        <Settings xstyle={styles.renderArticleRelationActionShare2} aria-hidden="true" />
        {renderActionLabel('去设置登录', '登录')}
      </Link>
    </Button>
  );
}

/** Retain direct keyed favorite pending/ready control identity. */
function renderArticleFavoriteAction(state: ArticleDetailViewState) {
  const {
    article,
    dbName,
    initialFolderIds,
    isFavoriteStatePending,
    isFavoriteStateUnavailable,
    favoriteToggleFolderId,
  } = state;

  return (
    <RevealItem
      isReady={!isFavoriteStatePending}
      placeholder={
        <div data-article-favorite-state="loading">
          <Button
            variant="outline"
            size="sm"
            xstyle={styles.articleActionButton}
            aria-label="加载收藏…"
            disabled
          >
            <Loader2 xstyle={styles.renderArticleFavoriteActionLoader2} aria-hidden="true" />
            {renderActionLabel('加载收藏…', '加载…')}
          </Button>
        </div>
      }
    >
      <div data-article-favorite-state="ready">
        <FavoriteButton
          articleId={article.article_id}
          dbName={dbName}
          initialFolderIds={initialFolderIds}
          isFavoriteStateUnavailable={isFavoriteStateUnavailable}
          toggleFolderId={favoriteToggleFolderId}
        />
      </div>
    </RevealItem>
  );
}

/** Retain copy-info callback, labels and complete keyed feedback presence. */
function renderArticleInfoCopy(state: ArticleDetailViewState) {
  const { copyStatus, handleCopyArticleInfo } = state;

  return (
    <CopyButton
      variant="outline"
      size="sm"
      xstyle={styles.articleActionButton}
      aria-label={copyStatus === 'info' ? '已复制' : '复制信息'}
      title={copyStatus === 'info' ? '已复制' : '复制信息'}
      isCopied={copyStatus === 'info'}
      onCopy={() => void handleCopyArticleInfo()}
    >
      {renderActionLabel(
        copyStatus === 'info' ? '已复制' : '复制信息',
        copyStatus === 'info' ? '已复制' : '复制',
      )}
    </CopyButton>
  );
}

/** Retain source-title copying, accessible copy error and visual feedback ownership. */
function renderArticleDetailHeader(state: ArticleDetailViewState, variant: ArticleDetailVariant) {
  const { article, copyStatus, copyError, canCopyTitle, handleCopyTitle, highlightText } = state;
  const TitleElement = variant === 'dialog' ? DialogTitle : 'h2';
  const DescriptionElement = variant === 'dialog' ? DialogDescription : 'p';

  return (
    <header>
      <DescriptionElement {...stylex.props(styles.renderArticleDetailHeaderDescriptionElement)}>
        {buildArticleDescription(article)}
      </DescriptionElement>
      <TitleElement
        data-slot="article-detail-title"
        {...stylex.props(
          styles.articleDetailTitle,
          variant === 'pane' ? styles.articleDetailTitlePane : styles.articleDetailTitleNotPane,
        )}
      >
        {highlightText(getArticleDisplayTitle(article))}
        <CopyButton
          variant="ghost"
          size="icon-sm"
          xstyle={styles.renderArticleDetailHeaderCopyButton}
          iconXstyle={styles.copyIcon}
          aria-label="复制文章标题"
          disabled={!canCopyTitle}
          isCopied={copyStatus === 'title'}
          onCopy={() => void handleCopyTitle()}
        />
      </TitleElement>
      {copyError && (
        <p {...stylex.props(styles.renderArticleDetailBodyH3)} role="alert">
          {copyError}
        </p>
      )}
      <MotionPresence>
        {copyError && (
          <MotionParagraph
            key="copy-error"
            aria-hidden="true"
            {...stylex.props(styles.renderArticleDetailHeaderMotionParagraph)}
            variants={SWAP_VARIANTS}
            initial="hidden"
            animate="visible"
            exit="exit"
          >
            {copyError}
          </MotionParagraph>
        )}
      </MotionPresence>
    </header>
  );
}

type ArticleDetailViewState = ReturnType<typeof useArticleDetailViewState>;
/** Retain optional issue/DOI fields in their original order before truthy filtering. */
function getOptionalArticleInfoFields(article: ArticleDetailDialogArticle, doiUrl: string | null) {
  return [
    article.volume && `卷号：${article.volume}`,
    article.number && `期号：${article.number}`,
    formatPageRange(article) && `页码：${formatPageRange(article)}`,
    article.pmid && `PMID：${article.pmid}`,
    article.doi && `DOI: ${article.doi}`,
    doiUrl && `DOI 链接：${doiUrl}`,
  ];
}

/** Derive access presentation with loading-before-error and refresh suppression unchanged. */
function getArticleAccessPresentation(
  articleId: Article['article_id'],
  dbName: string,
  access: ArticleAccessResponse | undefined,
  isAccessQueryEnabled: boolean,
  isAccessPending: boolean,
  isAccessFetching: boolean,
  isAccessError: boolean,
) {
  const abstractAction = access?.abstract_page;
  const fulltextAction = access?.fulltext;
  const abstractUrl = getAvailableArticleActionUrl(abstractAction, articleId, dbName, 'abstract');
  const fullTextUrl = getAvailableArticleActionUrl(fulltextAction, articleId, dbName, 'fulltext');
  const isAccessLoading = isAccessQueryEnabled && (isAccessPending || isAccessFetching);
  const canShowAccessActions = !isAccessFetching && !isAccessError;
  const accessState = isAccessLoading ? 'loading' : isAccessError ? 'error' : 'ready';

  return {
    abstractAction,
    fulltextAction,
    abstractUrl,
    fullTextUrl,
    isAccessLoading,
    canShowAccessActions,
    accessState,
  };
}

/** Construct the server action route only for an available access action. */
function getAvailableArticleActionUrl(
  action: { available: boolean } | undefined,
  articleId: Article['article_id'],
  dbName: string,
  kind: 'abstract' | 'fulltext',
) {
  return action?.available ? getArticleActionUrlForDatabase(articleId, dbName, kind) : null;
}

/** Retain the complete loading guard and pending-versus-refresh labels. */
function renderArticleAccessLoading(state: ArticleDetailViewState) {
  const { isAccessPending, isAccessLoading } = state;

  return (
    isAccessLoading && (
      <Button
        variant="outline"
        size="sm"
        xstyle={styles.articleActionButton}
        aria-label={isAccessPending ? '加载访问' : '刷新访问'}
        disabled
      >
        <Loader2 xstyle={styles.renderArticleFavoriteActionLoader2} aria-hidden="true" />
        {renderActionLabel(
          isAccessPending ? '加载访问' : '刷新访问',
          isAccessPending ? '加载' : '刷新',
        )}
      </Button>
    )
  );
}

const styles = stylex.create({
  groupFieldset: {
    minInlineSize: 0,
  },
  accessRow: {
    display: 'flex',
    gap: {
      default: '0.25rem',
      '@media (min-width: 48rem)': '0.5rem',
    },
  },
  accessRowPane: {
    flexWrap: 'nowrap',
  },
  accessRowWrapped: {
    flexWrap: 'wrap',
  },
  copyIcon: {
    width: '0.875rem',
    height: '0.875rem',
  },
  articleDetailDialogContentDialogContent: {
    display: 'flex',
    flexDirection: 'column',
    gap: '0px',
    overflow: 'hidden',
    backgroundColor: colors.background,
  },
  articleDetailDialogContentDiv: {
    display: 'flex',
    height: '3.5rem',
    flexShrink: 0,
    alignItems: 'center',
    gap: '0.5rem',
    borderBottomStyle: 'solid',
    borderBottomWidth: '1px',
    paddingInline: '0.375rem',
  },
  articleDetailDialogContentButton: {
    width: '2.75rem',
    height: '2.75rem',
  },
  articleDetailDialogContentX: {
    width: '1.25rem',
    height: '1.25rem',
  },
  articleDetailDialogContentButton2: {
    height: '2.75rem',
    gap: '0.375rem',
  },
  articleDetailDialogContentArrowLeft: {
    width: '1rem',
    height: '1rem',
  },
  articleDetailDialogContentSpan: {
    fontSize: '13px',
    color: colors.mutedForeground,
  },
  articleDetailDialogContentDiv2: {
    minHeight: '0px',
    flex: '1',
    overflowY: 'auto',
    paddingInline: {
      default: '1.25rem',
      '@media (min-width: 40rem)': '2rem',
    },
    paddingBlock: '1.5rem',
  },
  articlePaneViewMotionArticle: {
    minHeight: '100%',
  },
  articlePaneViewDiv: {
    position: 'sticky',
    top: '0px',
    zIndex: 10,
    backgroundColor: colors.background,
  },
  articlePaneViewNav: {
    display: 'flex',
    height: '2.5rem',
    alignItems: 'center',
    borderBottomStyle: 'solid',
    borderBottomWidth: '1px',
    backgroundColor: `color-mix(in oklab, ${colors.canvas} 60%, transparent)`,
    paddingInline: {
      default: '1rem',
      '@media (min-width: 96rem)': '1.5rem',
    },
  },
  articlePaneViewButton: {
    marginInline: '-0.5rem',
    display: 'flex',
    minWidth: '0px',
    alignItems: 'center',
    gap: '0.375rem',
    borderRadius: '6px',
    paddingInline: '0.5rem',
    paddingBlock: '0.25rem',
    fontSize: '13px',
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
  articlePaneViewArrowLeft: {
    width: '1rem',
    height: '1rem',
    flexShrink: 0,
  },
  articlePaneViewSpan: {
    flexShrink: 0,
    fontWeight: 500,
    color: colors.foreground,
  },
  articlePaneViewSpan2: {
    overflow: 'hidden',
    textOverflow: 'ellipsis',
    whiteSpace: 'nowrap',
  },
  articleDetailToolbar: {
    containerType: 'inline-size',
    display: 'flex',
    height: '3.5rem',
    alignItems: 'center',
    overflowX: 'auto',
    borderBottomStyle: 'solid',
    borderBottomWidth: '1px',
    paddingInline: {
      default: '1rem',
      '@media (min-width: 96rem)': '1.5rem',
    },
  },
  articlePaneViewDiv2: {
    marginInline: 'auto',
    width: '100%',
    maxWidth: '42rem',
    paddingInline: {
      default: '1.75rem',
      '@media (min-width: 96rem)': '2.5rem',
    },
    paddingTop: {
      default: '2.25rem',
      '@media (min-width: 96rem)': '3rem',
    },
    paddingBottom: '3.5rem',
  },
  renderArticleDetailBodyDiv: {
    minWidth: '0px',
  },
  renderArticleDetailBodySection: {
    marginTop: '1rem',
  },
  renderArticleDetailBodyH3: {
    position: 'absolute',
    width: '1px',
    height: '1px',
    padding: 0,
    margin: '-1px',
    overflow: 'hidden',
    clipPath: 'inset(50%)',
    whiteSpace: 'nowrap',
    borderWidth: 0,
  },
  renderArticleDetailBodyP: {
    fontSize: '13px',
    lineHeight: '1.5rem',
    overflowWrap: 'anywhere',
    color: `color-mix(in oklab, ${colors.foreground} 70%, transparent)`,
  },
  renderArticleDetailBodyDiv2: {
    marginTop: '0.75rem',
    display: 'flex',
    flexWrap: 'wrap',
    gap: '0.375rem',
  },
  renderArticleDetailBodyDiv3: {
    marginInline: '-0.625rem',
    marginTop: '1.5rem',
    borderBlockStyle: 'solid',
    borderBlockWidth: '1px',
    borderColor: `color-mix(in oklab, ${colors.border} 70%, transparent)`,
    paddingBlock: '0.5rem',
  },
  renderArticleDetailBodySection2: {
    marginTop: '2rem',
  },
  renderArticleDetailBodyH32: {
    display: 'flex',
    alignItems: 'center',
    gap: '0.75rem',
    fontSize: '11px',
    fontWeight: 700,
    letterSpacing: '0.16em',
    color: colors.mutedForeground,
    '::after': {
      content: '""',
      height: '1px',
      flex: '1',
      backgroundColor: colors.border,
    },
  },
  renderArticleDetailBodySpan: {
    display: 'flex',
    alignItems: 'center',
    gap: '0.375rem',
  },
  renderArticleDetailBodyFileText: {
    width: '0.875rem',
    height: '0.875rem',
    flexShrink: 0,
  },
  renderArticleDetailBodyP2: {
    marginTop: '1rem',
    fontSize: '15px',
    lineHeight: 1.95,
    textWrap: 'pretty',
    overflowWrap: 'anywhere',
    color: `color-mix(in oklab, ${colors.foreground} 85%, transparent)`,
  },
  renderIdentifierSpan: {
    wordBreak: 'break-all',
    color: `color-mix(in oklab, ${colors.foreground} 85%, transparent)`,
    userSelect: 'all',
  },
  renderRetractionNoticeDiv: {
    marginTop: '1rem',
    display: 'flex',
    gap: '0.75rem',
    borderRadius: '8px',
    borderStyle: 'solid',
    borderWidth: '1px',
    borderColor: `color-mix(in oklab, ${colors.destructive} 30%, transparent)`,
    backgroundColor: `color-mix(in oklab, ${colors.destructive} 5%, transparent)`,
    paddingInline: '0.875rem',
    paddingBlock: '0.75rem',
    fontSize: '13px',
    lineHeight: '1.5rem',
  },
  renderRetractionNoticeAlertTriangle: {
    marginTop: '0.25rem',
    width: '1rem',
    height: '1rem',
    flexShrink: 0,
    color: colors.destructive,
  },
  renderRetractionNoticeP: {
    fontWeight: 700,
    color: colors.destructive,
  },
  renderRetractionNoticeP2: {
    color: colors.mutedForeground,
  },
  renderArticleMetadataSpan: {
    fontVariantNumeric: 'tabular-nums',
  },
  renderArticleMetadataSpan2: {
    display: 'flex',
    flexWrap: 'wrap',
    gap: '0.375rem',
  },
  renderArticleMetadataSpan3: {
    display: 'inline-flex',
    height: '1.5rem',
    alignItems: 'center',
    gap: '0.25rem',
    borderRadius: '6px',
    backgroundColor: colors.muted,
    paddingInline: '0.5rem',
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
  },
  renderArticleMetadataSpan4: {
    fontWeight: 700,
  },
  renderArticleMetadataDlItem: {
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
    borderColor: {
      default: null,
      ':not(:last-child)': `color-mix(in oklab, ${colors.border} 70%, transparent)`,
    },
  },
  renderArticleMetadataDl: {
    marginTop: '0.5rem',
    fontSize: '13px',
    lineHeight: '1.5rem',
  },
  renderArticleMetadataDiv: {
    display: 'grid',
    gridTemplateColumns: '5rem minmax(0,1fr)',
    gap: '0.75rem',
    paddingBlock: '0.625rem',
  },
  renderArticleMetadataDd: {
    minWidth: '0px',
    overflowWrap: 'anywhere',
  },
  renderActionLabelSpan: {
    display: {
      default: 'none',
      '@media (min-width: 48rem)': 'inline',
      '@container (width < 43rem)': 'none',
    },
  },
  renderActionLabelSpan2: {
    display: {
      default: 'none',
      '@container (width < 43rem)': 'inline',
    },
  },
  renderArticleActionsDiv: {
    display: 'flex',
    alignItems: 'center',
    columnGap: '0.125rem',
    rowGap: '0.25rem',
  },
  paneActions: {
    flex: '1',
    flexWrap: 'nowrap',
  },
  renderArticleActionsDivNotPane: {
    minWidth: '0px',
    flexWrap: 'wrap',
  },
  articleActionButton: {
    backgroundColor: {
      default: 'transparent',
      ':hover': colors.accent,
    },
    boxShadow: {
      default: 'none',
      ':focus-visible': {
        default: `0 0 0 3px color-mix(in oklab, ${colors.ring} 50%, transparent)`,
        ':is([aria-invalid="true"])': `0 0 0 3px ${colors.destructiveRing}`,
      },
    },
    width: {
      default: '2.75rem',
      '@media (min-width: 48rem)': 'auto',
    },
    height: {
      default: '2.75rem',
      '@media (min-width: 48rem)': '2.25rem',
    },
    padding: '0px',
    paddingInline: {
      default: null,
      ':has( > svg)': '0.625rem',
      '@media (min-width: 48rem)': '0.625rem',
    },
  },
  renderArticleRelationActionButtonPane: {
    marginLeft: 'auto',
  },
  renderArticleRelationActionShare2: {
    height: '1rem',
    width: '1rem',
  },
  renderArticleAccessActionsAlertCircle: {
    height: '1rem',
    width: '1rem',
    color: colors.destructive,
  },
  renderArticleFavoriteActionLoader2: {
    height: '1rem',
    width: '1rem',
    animationName: animations.spin,
    animationDuration: '1s',
    animationTimingFunction: 'linear',
    animationIterationCount: 'infinite',
  },
  renderArticleDetailHeaderDescriptionElement: {
    fontSize: '0.75rem',
    lineHeight: '1.25rem',
    overflowWrap: 'anywhere',
    color: colors.mutedForeground,
  },
  articleDetailTitle: {
    marginTop: '0.75rem',
    fontWeight: 700,
    textWrap: 'wrap',
    overflowWrap: 'break-word',
    color: colors.foreground,
  },
  articleDetailTitlePane: {
    fontSize: {
      default: '22px',
      '@media (min-width: 96rem)': '26px',
    },
    lineHeight: 1.35,
    letterSpacing: '-0.01em',
  },
  articleDetailTitleNotPane: {
    fontSize: {
      default: '1.25rem',
      '@media (min-width: 48rem)': '22px',
    },
    lineHeight: 1.4,
    letterSpacing: '-0.01em',
  },
  renderArticleDetailHeaderCopyButton: {
    marginLeft: '0.375rem',
    display: 'inline-flex',
    width: '1.75rem',
    height: '1.75rem',
    padding: '0px',
    verticalAlign: 'middle',
    color: {
      default: colors.mutedForeground,
      ':hover': {
        default: null,
        '@media (hover: hover)': colors.accentForeground,
      },
    },
  },
  renderArticleDetailHeaderMotionParagraph: {
    marginTop: '0.5rem',
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
    color: colors.destructive,
  },
});
