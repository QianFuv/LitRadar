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
import { buildSettingsCenterHref } from '@/lib/settings-center';

type ArticleDetailDialogArticle = Article;

type ArticleDetailDialogContentProps = {
  article: ArticleDetailDialogArticle;
  dbName: string;
  initialFolderIds?: number[];
  isFavoriteStatePending?: boolean;
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
    `标题：${article.title || '暂无'}`,
    `作者：${authors || '暂无'}`,
    `期刊：${article.journal_title || '暂无'}`,
    `日期：${article.date || '暂无'}`,
    article.volume && `卷号：${article.volume}`,
    article.number && `期号：${article.number}`,
    article.doi && `DOI: ${article.doi}`,
    doiUrl && `DOI 链接：${doiUrl}`,
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
export function ArticleDetailDialogContent({
  article,
  dbName,
  initialFolderIds = [],
  isFavoriteStatePending = false,
  extraActions,
}: ArticleDetailDialogContentProps) {
  const pathname = usePathname();
  const searchParams = useSearchParams();
  const [copyStatus, setCopyStatus] = useState<ArticleCopyTarget | null>(null);
  const [copyError, setCopyError] = useState<string | null>(null);
  const copyResetTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const stateTransition = useMotionTransition(MOTION_DURATION_SECONDS.fast);
  const isAccessQueryEnabled = !!dbName && !!article.article_id;
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
    await handleCopy(article.title || '', 'title');
  };

  /** Copy the plain-text article information summary. */
  const handleCopyArticleInfo = async () => {
    await handleCopy(buildArticleInfoText(article), 'info');
  };

  const abstractAction = access?.abstract_page;
  const fulltextAction = access?.fulltext;
  const abstractUrl = abstractAction?.available
    ? getArticleActionUrlForDatabase(article.article_id, dbName, 'abstract')
    : null;
  const fullTextUrl = fulltextAction?.available
    ? getArticleActionUrlForDatabase(article.article_id, dbName, 'fulltext')
    : null;
  const isAccessLoading = isAccessQueryEnabled && (isAccessPending || isAccessFetching);
  const canShowAccessActions = !isAccessFetching && !isAccessError;
  const accessState = isAccessLoading ? 'loading' : isAccessError ? 'error' : 'ready';
  const dataSourceSettingsHref = buildSettingsCenterHref(pathname, searchParams, 'data-sources');

  return (
    <DialogContent className="max-h-[90dvh] w-[calc(100%-2rem)] max-w-[calc(100%-2rem)] overflow-y-auto md:max-w-4xl">
      <DialogHeader>
        <DialogTitle className="text-xl leading-snug">
          {article.title || '未命名文章'}
          <Button
            variant="ghost"
            size="sm"
            className="ml-2 inline-flex h-6 w-6 p-0 align-middle"
            aria-label="复制文章标题"
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
                    <Check className="h-3 w-3 text-green-600" aria-hidden="true" />
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
                      <Check className="h-4 w-4 text-green-600" aria-hidden="true" />
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
                {isAccessLoading && (
                  <Button
                    variant="outline"
                    size="sm"
                    className={ARTICLE_ACTION_BUTTON_CLASS_NAME}
                    aria-label={isAccessPending ? '加载访问' : '刷新访问'}
                    disabled
                  >
                    <Loader2 className="h-4 w-4 animate-spin" aria-hidden="true" />
                    <span className="hidden md:inline">
                      {isAccessPending ? '加载访问' : '刷新访问'}
                    </span>
                  </Button>
                )}
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
                {canShowAccessActions && abstractUrl && (
                  <Button
                    asChild
                    variant="outline"
                    size="sm"
                    className={ARTICLE_ACTION_BUTTON_CLASS_NAME}
                  >
                    <a
                      href={abstractUrl}
                      target="_blank"
                      rel="noreferrer"
                      aria-label={abstractAction?.label ?? '查看摘要页'}
                      title={abstractAction?.label ?? '查看摘要页'}
                    >
                      <ExternalLink className="h-4 w-4" aria-hidden="true" />
                      <span className="hidden md:inline">
                        {abstractAction?.label ?? '查看摘要页'}
                      </span>
                    </a>
                  </Button>
                )}
                {canShowAccessActions && fullTextUrl && (
                  <Button
                    asChild
                    variant="outline"
                    size="sm"
                    className={ARTICLE_ACTION_BUTTON_CLASS_NAME}
                  >
                    <a
                      href={fullTextUrl}
                      target="_blank"
                      rel="noreferrer"
                      aria-label={fulltextAction?.label ?? '获取全文'}
                      title={fulltextAction?.label ?? '获取全文'}
                    >
                      <FileDown className="h-4 w-4" aria-hidden="true" />
                      <span className="hidden md:inline">
                        {fulltextAction?.label ?? '获取全文'}
                      </span>
                    </a>
                  </Button>
                )}
                {canShowAccessActions && fulltextAction?.requires_login && (
                  <DialogClose asChild>
                    <Button
                      asChild
                      variant="outline"
                      size="sm"
                      className={ARTICLE_ACTION_BUTTON_CLASS_NAME}
                    >
                      <Link
                        href={dataSourceSettingsHref}
                        aria-label="去设置登录"
                        title="去设置登录"
                      >
                        <Settings className="h-4 w-4" aria-hidden="true" />
                        <span className="hidden md:inline">去设置登录</span>
                      </Link>
                    </Button>
                  </DialogClose>
                )}
              </MotionDiv>
            </MotionPresence>
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
                  />
                )}
              </MotionDiv>
            </MotionPresence>
            {extraActions}
          </div>
        </div>
      </div>
    </DialogContent>
  );
}
