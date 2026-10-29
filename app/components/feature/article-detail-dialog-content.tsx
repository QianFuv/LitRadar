'use client';

/**
 * Article metadata and responsive, accessible detail actions.
 */

import {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type ReactNode,
  type RefObject,
} from 'react';
import { useQuery } from '@tanstack/react-query';
import Link from 'next/link';
import { usePathname, useSearchParams } from 'next/navigation';
import {
  Check,
  CircleAlert,
  Copy,
  ExternalLink,
  FileDown,
  Loader2,
  ArrowLeft,
  Settings,
  TriangleAlert,
  X,
} from 'lucide-react';

import {
  getArticleActionUrlForDatabase,
  checkFavorite,
  getArticleAccess,
  getJournal,
  type Article,
  type JournalRecord,
} from '@/lib/api';
import { FavoriteButton } from '@/components/feature/favorite-button';
import { RelatedArticles } from '@/components/feature/related-articles';
import { useAuth } from '@/lib/auth-context';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { DialogClose, DialogContent, DialogDescription, DialogTitle } from '@/components/ui/dialog';
import {
  FADE_VARIANTS,
  MOTION_DURATION_SECONDS,
  MotionDiv,
  MotionParagraph,
  MotionPresence,
  MotionSpan,
  useMotionTransition,
} from '@/components/ui/motion';
import { copyTextToClipboard } from '@/lib/clipboard';
import { getDoiUrl } from '@/lib/citation';
import { getArticleDisplayTitle, hasArticleTitle } from '@/lib/article-title';
import { JOURNAL_RATING_SYSTEMS } from '@/lib/journal-ratings';
import { buildSettingsCenterHref } from '@/lib/settings-center';
import { cn } from '@/lib/utils';

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

const ARTICLE_ACTION_BUTTON_CLASS_NAME = 'size-11 p-0 md:h-9 md:w-auto md:px-2.5';

/** Short forms of the default access labels used when the docked toolbar is narrow. */
const ACCESS_SHORT_LABELS: Record<string, string> = {
  查看摘要页: '摘要页',
  获取全文: '全文',
};

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
    queryKey: ['fav-check', user?.id, dbName, article.article_id],
    queryFn: () => checkFavorite(article.article_id, dbName),
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
    <DialogContent placement="right" className="flex flex-col gap-0 overflow-hidden bg-background">
      <div className="flex h-14 shrink-0 items-center gap-2 border-b px-1.5">
        <DialogClose asChild>
          <Button
            type="button"
            variant="ghost"
            size="icon"
            static
            aria-label="关闭"
            className="size-11"
          >
            <X className="size-5" aria-hidden="true" />
          </Button>
        </DialogClose>
        {trail.current ? (
          <Button
            type="button"
            variant="ghost"
            size="sm"
            className="h-11 gap-1.5"
            onClick={trail.back}
          >
            <ArrowLeft className="size-4" aria-hidden="true" />
            返回上一篇
          </Button>
        ) : (
          <span className="text-[13px] text-muted-foreground">文献详情</span>
        )}
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto px-5 py-6 sm:px-8">
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
 * Render one article in the docked reader with its sticky toolbar.
 *
 * @param props - Article detail configuration and trail callbacks.
 * @returns Reading-desk article.
 */
function ArticlePaneView(props: ArticleDetailDialogContentProps & ArticleTrailProps) {
  const state = useArticleDetailViewState(props);
  const articleRef = useRef<HTMLElement>(null);
  const toolbarRef = useRef<HTMLDivElement>(null);
  const isRelated = Boolean(props.onBack);
  useCompactToolbar(toolbarRef);
  useEffect(() => {
    if (isRelated) articleRef.current?.scrollIntoView?.({ block: 'start' });
  }, [isRelated]);
  return (
    <article
      ref={articleRef}
      aria-label={`文献详情：${getArticleDisplayTitle(state.article)}`}
      className="min-h-full"
    >
      <div className="sticky top-0 z-10 bg-background">
        {props.onBack && (
          <nav
            aria-label="阅读路径"
            className="flex h-10 items-center border-b bg-canvas/60 px-4 2xl:px-6"
          >
            <button
              type="button"
              aria-label="返回上一篇"
              title={
                props.previousArticle ? getArticleDisplayTitle(props.previousArticle) : undefined
              }
              className="motion-control -mx-2 flex min-w-0 items-center gap-1.5 rounded-md px-2 py-1 text-[13px] text-muted-foreground outline-none transition-colors hover:bg-accent hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
              onClick={props.onBack}
            >
              <ArrowLeft className="size-4 shrink-0" aria-hidden="true" />
              <span className="shrink-0 font-medium text-foreground">返回上一篇</span>
              {props.previousArticle && (
                <span className="truncate">· {getArticleDisplayTitle(props.previousArticle)}</span>
              )}
            </button>
          </nav>
        )}
        <div
          ref={toolbarRef}
          data-slot="article-detail-toolbar"
          className="flex h-14 items-center overflow-x-auto border-b px-4 2xl:px-6 [&[data-compact]_[data-action-label]]:hidden [&[data-compact]_[data-action-short]]:inline"
        >
          {renderArticleActions(state, 'pane')}
        </div>
      </div>
      <div className="mx-auto w-full max-w-[42rem] px-7 pt-9 pb-14 2xl:px-10 2xl:pt-12">
        {renderArticleDetailBody(state, 'pane', props.onOpenRelated)}
      </div>
    </article>
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
    <div data-article-detail-variant={variant} className="min-w-0">
      {renderArticleDetailHeader(state, variant)}
      {article.authors && article.authors.length > 0 && (
        <section className="mt-4">
          <h3 className="sr-only">作者</h3>
          <p className="text-[13px] leading-6 text-foreground/70 [overflow-wrap:anywhere]">
            {article.authors.join('; ')}
          </p>
        </section>
      )}
      {(isOpenAccess || isInPress) && (
        <div className="mt-3 flex flex-wrap gap-1.5">
          {isOpenAccess && <Badge variant="success">开放获取</Badge>}
          {isInPress && <Badge variant="outline">预发表</Badge>}
        </div>
      )}

      {renderRetractionNotice(article)}

      {variant === 'dialog' && (
        <div className="mt-6 -mx-2.5 border-y border-border/70 py-2">
          {renderArticleActions(state, variant)}
        </div>
      )}

      <section className="mt-8">
        <h3 className="flex items-center gap-3 text-[11px] font-bold tracking-[0.16em] text-muted-foreground after:h-px after:flex-1 after:bg-border">
          摘要
        </h3>
        <p className="mt-4 text-pretty text-[15px] leading-[1.95] text-foreground/85 [overflow-wrap:anywhere]">
          {article.abstract || '暂无摘要。'}
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
  return <span className="break-all text-foreground/85 select-all">{value}</span>;
}

/** Render a prominent notice when the article has published retractions. */
function renderRetractionNotice(article: ArticleDetailDialogArticle) {
  const retractions = article.retraction_dois ?? [];
  if (retractions.length === 0) return null;
  return (
    <div
      role="note"
      aria-label="撤稿提醒"
      className="mt-4 flex gap-3 rounded-lg border border-destructive/30 bg-destructive/5 px-3.5 py-3 text-[13px] leading-6"
    >
      <TriangleAlert className="mt-1 size-4 shrink-0 text-destructive" aria-hidden="true" />
      <div className="min-w-0">
        <p className="font-bold text-destructive">此文章已被撤稿</p>
        <p className="text-muted-foreground">
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
  if (pages) rows.push({ label: '页码', value: <span className="tabular-nums">{pages}</span> });
  if (issns.length > 0) {
    rows.push({ label: 'ISSN', value: <span className="tabular-nums">{issns.join(' / ')}</span> });
  }
  if (ratings.length > 0) {
    rows.push({
      label: '期刊评级',
      value: (
        <span className="flex flex-wrap gap-1.5">
          {ratings.map((rating) => (
            <span
              key={rating.label}
              className="inline-flex h-6 items-center gap-1 rounded-md bg-muted px-2 text-xs"
            >
              <span className="text-muted-foreground">{rating.label}</span>
              <span className="font-bold">{rating.value}</span>
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
    <section className="mt-8">
      <h3 className="flex items-center gap-3 text-[11px] font-bold tracking-[0.16em] text-muted-foreground after:h-px after:flex-1 after:bg-border">
        文献信息
      </h3>
      <dl className="mt-2 divide-y divide-border/70 text-[13px] leading-6">
        {rows.map((row) => (
          <div key={row.label} className="grid grid-cols-[5rem_minmax(0,1fr)] gap-3 py-2.5">
            <dt className="text-muted-foreground">{row.label}</dt>
            <dd className="min-w-0 [overflow-wrap:anywhere]">{row.value}</dd>
          </div>
        ))}
      </dl>
    </section>
  );
}

/**
 * Render an action label with a shorter form the docked toolbar switches to when space is tight.
 *
 * @param label - Full label.
 * @param shortLabel - Short label shown in a compact toolbar.
 * @returns Both label forms; only the full one is visible by default.
 */
function renderActionLabel(label: string, shortLabel: string) {
  return (
    <>
      <span data-action-label className="hidden md:inline">
        {label}
      </span>
      <span data-action-short className="hidden">
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
  return ACCESS_SHORT_LABELS[label] ?? label;
}

/**
 * Switch action labels to their short forms while full labels would overflow the toolbar.
 *
 * Labels are restored before each measurement, which runs on resize and on content changes such
 * as access actions loading or the favorite label switching.
 *
 * @param toolbarRef - Toolbar whose full labels carry `data-action-label`.
 */
function useCompactToolbar(toolbarRef: RefObject<HTMLElement | null>) {
  useLayoutEffect(() => {
    const toolbar = toolbarRef.current;
    if (!toolbar || typeof ResizeObserver === 'undefined') return;
    const update = () => {
      toolbar.removeAttribute('data-compact');
      if (toolbar.scrollWidth > toolbar.clientWidth) toolbar.setAttribute('data-compact', '');
    };
    update();
    const resizeObserver = new ResizeObserver(update);
    resizeObserver.observe(toolbar);
    const mutationObserver = new MutationObserver(update);
    mutationObserver.observe(toolbar, { childList: true, subtree: true, characterData: true });
    return () => {
      resizeObserver.disconnect();
      mutationObserver.disconnect();
    };
  }, [toolbarRef]);
}

/** Render the favorite, copy, access, and caller-provided article actions as one toolbar group. */
function renderArticleActions(state: ArticleDetailViewState, variant: ArticleDetailVariant) {
  return (
    <div
      role="group"
      aria-label="文章操作"
      className={cn(
        'flex items-center gap-x-0.5 gap-y-1 [&_[data-variant]]:bg-transparent [&_[data-variant]]:!shadow-none [&_[data-variant]:hover]:bg-accent',
        variant === 'pane' ? 'flex-1 flex-nowrap' : 'min-w-0 flex-wrap',
      )}
    >
      {renderArticleFavoriteAction(state)}
      {renderArticleInfoCopy(state)}
      {renderArticleAccessActions(state, variant)}
      {state.extraActions}
    </div>
  );
}

/** Own unchanged detail hooks, query settings, clipboard ref and completion/timer order. */
function useArticleDetailViewState({
  article,
  dbName,
  initialFolderIds = [],
  isFavoriteStatePending = false,
  isFavoriteStateUnavailable = false,
  favoriteToggleFolderId,
  extraActions,
}: ArticleDetailDialogContentProps) {
  const pathname = usePathname();
  const searchParams = useSearchParams();
  const [copyStatus, setCopyStatus] = useState<ArticleCopyTarget | null>(null);
  const [copyError, setCopyError] = useState<string | null>(null);
  const copyResetTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const stateTransition = useMotionTransition(MOTION_DURATION_SECONDS.fast);
  const isAccessQueryEnabled = !!dbName && !!article.article_id;
  const canCopyTitle = hasArticleTitle(article);
  const {
    data: access,
    isPending: isAccessPending,
    isFetching: isAccessFetching,
    isError: isAccessError,
    error: accessError,
  } = useQuery({
    queryKey: ['article-access', dbName, article.article_id],
    queryFn: ({ signal }) => getArticleAccess(article.article_id, dbName, signal),
    enabled: isAccessQueryEnabled,
    staleTime: 0,
    refetchOnMount: 'always',
  });

  const journalId = article.journal_id ? String(article.journal_id) : '';
  const { data: journal } = useQuery({
    queryKey: ['journal', dbName, journalId],
    queryFn: ({ signal }) => getJournal(journalId, dbName, signal),
    enabled: Boolean(dbName && journalId),
    staleTime: 30 * 60 * 1000,
    retry: false,
  });

  useEffect(
    () => () => {
      if (copyResetTimeoutRef.current !== null) {
        clearTimeout(copyResetTimeoutRef.current);
      }
    },
    [],
  );

  /**
   * Copy one article value and update its inline state.
   *
   * @param text - Text to copy.
   * @param status - Copy action identifier.
   */
  const handleCopy = async (text: string, status: ArticleCopyTarget) => {
    if (copyResetTimeoutRef.current !== null) {
      clearTimeout(copyResetTimeoutRef.current);
    }
    try {
      await copyTextToClipboard(text);
      setCopyStatus(status);
      setCopyError(null);
    } catch {
      setCopyStatus(null);
      setCopyError('复制失败，请手动选择文本复制。');
    }
    copyResetTimeoutRef.current = setTimeout(() => {
      setCopyStatus(null);
      setCopyError(null);
      copyResetTimeoutRef.current = null;
    }, 3000);
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
  const dataSourceSettingsHref = buildSettingsCenterHref(pathname, searchParams, 'data-sources');

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
    stateTransition,
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
    dataSourceSettingsHref,
  };
}

/** Retain availability, exact nullish labels and abstract action routing. */
function renderArticleAbstractAction(state: ArticleDetailViewState) {
  const { abstractAction, abstractUrl, canShowAccessActions } = state;

  return (
    canShowAccessActions &&
    abstractUrl && (
      <Button asChild variant="outline" size="sm" className={ARTICLE_ACTION_BUTTON_CLASS_NAME}>
        <a
          href={abstractUrl}
          target="_blank"
          rel="noreferrer"
          aria-label={abstractAction?.label ?? '查看摘要页'}
          title={abstractAction?.label ?? '查看摘要页'}
        >
          <ExternalLink className="h-4 w-4" aria-hidden="true" />
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
      <Button asChild variant="outline" size="sm" className={ARTICLE_ACTION_BUTTON_CLASS_NAME}>
        <a
          href={fullTextUrl}
          target="_blank"
          rel="noreferrer"
          aria-label={fulltextAction?.label ?? '获取全文'}
          title={fulltextAction?.label ?? '获取全文'}
        >
          <FileDown className="h-4 w-4" aria-hidden="true" />
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
    stateTransition,
    isAccessQueryEnabled,
    isAccessFetching,
    isAccessError,
    accessError,
    fulltextAction,
    canShowAccessActions,
    accessState,
    dataSourceSettingsHref,
  } = state;

  return (
    <MotionPresence mode="wait">
      <MotionDiv
        key={accessState}
        data-article-access-state={accessState}
        className={cn('flex gap-1 md:gap-2', variant === 'pane' ? 'flex-nowrap' : 'flex-wrap')}
        variants={FADE_VARIANTS}
        initial="hidden"
        animate="visible"
        exit={{ opacity: 0, pointerEvents: 'none' }}
        transition={stateTransition}
      >
        {renderArticleAccessLoading(state)}
        {isAccessQueryEnabled && !isAccessFetching && isAccessError && (
          <Button
            variant="outline"
            size="sm"
            className={ARTICLE_ACTION_BUTTON_CLASS_NAME}
            aria-label="访问状态失败"
            disabled
            title={accessError instanceof Error ? accessError.message : '访问状态不可用'}
          >
            <CircleAlert className="h-4 w-4 text-destructive" aria-hidden="true" />
            {renderActionLabel('访问状态失败', '访问失败')}
          </Button>
        )}
        {renderArticleAbstractAction(state)}
        {renderArticleFulltextAction(state)}
        {canShowAccessActions &&
          fulltextAction?.requires_login &&
          (variant === 'dialog' ? (
            <DialogClose asChild>
              {renderDataSourceSettingsLink(dataSourceSettingsHref)}
            </DialogClose>
          ) : (
            renderDataSourceSettingsLink(dataSourceSettingsHref)
          ))}
      </MotionDiv>
    </MotionPresence>
  );
}

/** Render the data-source settings shortcut shown when full text requires login. */
function renderDataSourceSettingsLink(href: string) {
  return (
    <Button asChild variant="outline" size="sm" className={ARTICLE_ACTION_BUTTON_CLASS_NAME}>
      <Link href={href} prefetch={false} aria-label="去设置登录" title="去设置登录">
        <Settings className="h-4 w-4" aria-hidden="true" />
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
    stateTransition,
  } = state;

  return (
    <MotionPresence mode="wait">
      <MotionDiv
        key={isFavoriteStatePending ? 'favorite-loading' : 'favorite-ready'}
        data-article-favorite-state={isFavoriteStatePending ? 'loading' : 'ready'}
        variants={FADE_VARIANTS}
        initial="hidden"
        animate="visible"
        exit={{ opacity: 0, pointerEvents: 'none' }}
        transition={stateTransition}
      >
        {isFavoriteStatePending ? (
          <Button
            variant="outline"
            size="sm"
            className={ARTICLE_ACTION_BUTTON_CLASS_NAME}
            aria-label="加载收藏…"
            disabled
          >
            <Loader2 className="h-4 w-4 animate-spin" aria-hidden="true" />
            {renderActionLabel('加载收藏…', '加载…')}
          </Button>
        ) : (
          <FavoriteButton
            articleId={article.article_id}
            dbName={dbName}
            initialFolderIds={initialFolderIds}
            isFavoriteStateUnavailable={isFavoriteStateUnavailable}
            toggleFolderId={favoriteToggleFolderId}
          />
        )}
      </MotionDiv>
    </MotionPresence>
  );
}

/** Retain copy-info callback, labels and complete keyed feedback presence. */
function renderArticleInfoCopy(state: ArticleDetailViewState) {
  const { copyStatus, stateTransition, handleCopyArticleInfo } = state;

  return (
    <Button
      variant="outline"
      size="sm"
      className={ARTICLE_ACTION_BUTTON_CLASS_NAME}
      aria-label={copyStatus === 'info' ? '已复制' : '复制信息'}
      title={copyStatus === 'info' ? '已复制' : '复制信息'}
      onClick={handleCopyArticleInfo}
    >
      <span className="grid" aria-hidden="true">
        <MotionPresence>
          <MotionSpan
            key={copyStatus === 'info' ? 'info-copied' : 'info-copy'}
            data-copy-state={copyStatus === 'info' ? 'copied' : 'idle'}
            className="col-start-1 row-start-1 flex items-center gap-2"
            variants={FADE_VARIANTS}
            initial="hidden"
            animate="visible"
            exit={{ opacity: 0, pointerEvents: 'none' }}
            transition={stateTransition}
          >
            {copyStatus === 'info' ? (
              <Check className="h-4 w-4 text-success-foreground" aria-hidden="true" />
            ) : (
              <Copy className="h-4 w-4" aria-hidden="true" />
            )}
            {renderActionLabel(
              copyStatus === 'info' ? '已复制' : '复制信息',
              copyStatus === 'info' ? '已复制' : '复制',
            )}
          </MotionSpan>
        </MotionPresence>
      </span>
    </Button>
  );
}

/** Retain source-title copying, accessible copy error and visual feedback ownership. */
function renderArticleDetailHeader(state: ArticleDetailViewState, variant: ArticleDetailVariant) {
  const { article, copyStatus, copyError, stateTransition, canCopyTitle, handleCopyTitle } = state;
  const TitleElement = variant === 'dialog' ? DialogTitle : 'h2';
  const DescriptionElement = variant === 'dialog' ? DialogDescription : 'p';

  return (
    <header>
      <DescriptionElement className="text-xs leading-5 text-muted-foreground [overflow-wrap:anywhere]">
        {buildArticleDescription(article)}
      </DescriptionElement>
      <TitleElement
        data-slot="article-detail-title"
        className={cn(
          'mt-3 text-wrap break-words font-bold text-foreground',
          variant === 'pane'
            ? 'text-[22px] leading-[1.35] tracking-[-0.01em] 2xl:text-[26px]'
            : 'text-xl leading-[1.4] tracking-[-0.01em] md:text-[22px]',
        )}
      >
        {getArticleDisplayTitle(article)}
        <Button
          variant="ghost"
          size="sm"
          className="ml-1.5 inline-flex size-7 p-0 align-middle text-muted-foreground"
          aria-label="复制文章标题"
          disabled={!canCopyTitle}
          onClick={handleCopyTitle}
        >
          <span className="grid place-items-center" aria-hidden="true">
            <MotionPresence>
              <MotionSpan
                key={copyStatus === 'title' ? 'title-copied' : 'title-copy'}
                data-copy-state={copyStatus === 'title' ? 'copied' : 'idle'}
                className="col-start-1 row-start-1 inline-flex"
                variants={FADE_VARIANTS}
                initial="hidden"
                animate="visible"
                exit={{ opacity: 0, pointerEvents: 'none' }}
                transition={stateTransition}
              >
                {copyStatus === 'title' ? (
                  <Check className="size-3.5 text-success-foreground" aria-hidden="true" />
                ) : (
                  <Copy className="size-3.5" aria-hidden="true" />
                )}
              </MotionSpan>
            </MotionPresence>
          </span>
        </Button>
      </TitleElement>
      {copyError && (
        <p className="sr-only" role="alert">
          {copyError}
        </p>
      )}
      <MotionPresence>
        {copyError && (
          <MotionParagraph
            key="copy-error"
            aria-hidden="true"
            className="mt-2 text-sm text-destructive"
            variants={FADE_VARIANTS}
            initial="hidden"
            animate="visible"
            exit={{ opacity: 0, pointerEvents: 'none' }}
            transition={stateTransition}
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
  access: Awaited<ReturnType<typeof getArticleAccess>> | undefined,
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
        className={ARTICLE_ACTION_BUTTON_CLASS_NAME}
        aria-label={isAccessPending ? '加载访问' : '刷新访问'}
        disabled
      >
        <Loader2 className="h-4 w-4 animate-spin" aria-hidden="true" />
        {renderActionLabel(
          isAccessPending ? '加载访问' : '刷新访问',
          isAccessPending ? '加载' : '刷新',
        )}
      </Button>
    )
  );
}
