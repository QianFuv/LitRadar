'use client';

/**
 * Article metadata and responsive, accessible detail actions.
 */

import { useEffect, useRef, useState, type ReactNode } from 'react';
import { useQuery } from '@tanstack/react-query';
import Link from 'next/link';
import { usePathname, useSearchParams } from 'next/navigation';
import { Check, CircleAlert, Copy, ExternalLink, FileDown, Loader2, Settings } from 'lucide-react';

import { getArticleActionUrlForDatabase, getArticleAccess, type Article } from '@/lib/api';
import { FavoriteButton } from '@/components/feature/favorite-button';
import { Button } from '@/components/ui/button';
import {
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
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
import { buildSettingsCenterHref } from '@/lib/settings-center';

type ArticleDetailDialogArticle = Article;

type ArticleDetailDialogContentProps = {
  article: ArticleDetailDialogArticle;
  dbName: string;
  initialFolderIds?: number[];
  isFavoriteStatePending?: boolean;
  isFavoriteStateUnavailable?: boolean;
  extraActions?: ReactNode;
};

type ArticleCopyTarget = 'title' | 'info';

const ARTICLE_ACTION_BUTTON_CLASS_NAME = 'size-11 p-0 md:h-10 md:w-auto md:px-3';

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
    article.date,
  ].filter(Boolean);

  return parts.join(' • ');
}

/**
 * Render article metadata, access actions, and favorite controls.
 *
 * @param props - Article detail dialog configuration.
 * @returns Article detail dialog content.
 */
export function ArticleDetailDialogContent(props: ArticleDetailDialogContentProps) {
  const state = useArticleDetailViewState(props);
  const { article, extraActions } = state;
  return (
    <DialogContent className="max-h-[90dvh] w-[calc(100%-2rem)] max-w-[calc(100%-2rem)] overflow-y-auto md:max-w-4xl">
      {renderArticleDetailHeader(state)}
      <div className="space-y-5 py-3">
        {article.authors && article.authors.length > 0 && (
          <div>
            <h3 className="mb-2 text-sm font-semibold text-foreground/80">作者</h3>
            <p className="text-sm text-muted-foreground">{article.authors.join('; ')}</p>
          </div>
        )}

        <div>
          <h3 className="mb-2 text-sm font-semibold text-foreground/80">摘要</h3>
          <p className="text-justify text-sm leading-relaxed text-muted-foreground">
            {article.abstract || '暂无摘要。'}
          </p>
        </div>

        <div className="border-t pt-4">
          <div
            role="group"
            aria-label="文章操作"
            className="flex flex-wrap items-center gap-1 md:gap-2"
          >
            {renderArticleInfoCopy(state)}
            {renderArticleAccessActions(state)}
            {renderArticleFavoriteAction(state)}
            {extraActions}
          </div>
        </div>
      </div>
    </DialogContent>
  );
}
/** Own unchanged detail hooks, query settings, clipboard ref and completion/timer order. */
function useArticleDetailViewState({
  article,
  dbName,
  initialFolderIds = [],
  isFavoriteStatePending = false,
  isFavoriteStateUnavailable = false,
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
    queryFn: () => getArticleAccess(article.article_id, dbName),
    enabled: isAccessQueryEnabled,
    staleTime: 0,
    refetchOnMount: 'always',
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
    extraActions,
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
          <span className="hidden md:inline">{abstractAction?.label ?? '查看摘要页'}</span>
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
          <span className="hidden md:inline">{fulltextAction?.label ?? '获取全文'}</span>
        </a>
      </Button>
    )
  );
}

/** Retain direct keyed access loading/error/action children and wait ownership. */
function renderArticleAccessActions(state: ArticleDetailViewState) {
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
        className="flex flex-wrap gap-1 md:gap-2"
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
            <span className="hidden md:inline">访问状态失败</span>
          </Button>
        )}
        {renderArticleAbstractAction(state)}
        {renderArticleFulltextAction(state)}
        {canShowAccessActions && fulltextAction?.requires_login && (
          <DialogClose asChild>
            <Button
              asChild
              variant="outline"
              size="sm"
              className={ARTICLE_ACTION_BUTTON_CLASS_NAME}
            >
              <Link href={dataSourceSettingsHref} aria-label="去设置登录" title="去设置登录">
                <Settings className="h-4 w-4" aria-hidden="true" />
                <span className="hidden md:inline">去设置登录</span>
              </Link>
            </Button>
          </DialogClose>
        )}
      </MotionDiv>
    </MotionPresence>
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
            <span className="hidden md:inline">加载收藏…</span>
          </Button>
        ) : (
          <FavoriteButton
            articleId={article.article_id}
            dbName={dbName}
            initialFolderIds={initialFolderIds}
            isFavoriteStateUnavailable={isFavoriteStateUnavailable}
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
            <span className="hidden md:inline">
              {copyStatus === 'info' ? '已复制' : '复制信息'}
            </span>
          </MotionSpan>
        </MotionPresence>
      </span>
    </Button>
  );
}

/** Retain source-title copying, accessible copy error and visual feedback ownership. */
function renderArticleDetailHeader(state: ArticleDetailViewState) {
  const { article, copyStatus, copyError, stateTransition, canCopyTitle, handleCopyTitle } = state;

  return (
    <DialogHeader>
      <DialogTitle className="text-wrap break-words text-xl leading-snug">
        {getArticleDisplayTitle(article)}
        <Button
          variant="ghost"
          size="sm"
          className="ml-2 inline-flex h-6 w-6 p-0 align-middle"
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
                  <Check className="h-3 w-3 text-success-foreground" aria-hidden="true" />
                ) : (
                  <Copy className="h-3 w-3" aria-hidden="true" />
                )}
              </MotionSpan>
            </MotionPresence>
          </span>
        </Button>
      </DialogTitle>
      <DialogDescription>{buildArticleDescription(article)}</DialogDescription>
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
            className="text-sm text-destructive"
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
    </DialogHeader>
  );
}

type ArticleDetailViewState = ReturnType<typeof useArticleDetailViewState>;
/** Retain optional issue/DOI fields in their original order before truthy filtering. */
function getOptionalArticleInfoFields(article: ArticleDetailDialogArticle, doiUrl: string | null) {
  return [
    article.volume && `卷号：${article.volume}`,
    article.number && `期号：${article.number}`,
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
        <span className="hidden md:inline">{isAccessPending ? '加载访问' : '刷新访问'}</span>
      </Button>
    )
  );
}
