'use client';

import { useEffect, useRef, useState, type ReactNode } from 'react';
import { useQuery } from '@tanstack/react-query';
import Link from 'next/link';
import { usePathname, useSearchParams } from 'next/navigation';
import { Check, Copy, ExternalLink, FileDown, Loader2, Settings } from 'lucide-react';

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
  const dataSourceSettingsHref = buildSettingsCenterHref(pathname, searchParams, 'data-sources');

  return (
    <DialogContent className="max-h-[90dvh] w-[calc(100%-2rem)] max-w-[calc(100%-2rem)] overflow-y-auto md:max-w-4xl">
      <DialogHeader>
        <DialogTitle className="pr-8 text-xl leading-snug">
          {article.title || '未命名文章'}
          <Button
            variant="ghost"
            size="sm"
            className="h-6 w-6 p-0 ml-2 inline-flex align-middle"
            aria-label="复制文章标题"
            onClick={handleCopyTitle}
          >
            {copyStatus === 'title' ? (
              <Check className="h-3 w-3 text-green-600" />
            ) : (
              <Copy className="h-3 w-3" />
            )}
          </Button>
        </DialogTitle>
        <DialogDescription>{buildArticleDescription(article)}</DialogDescription>
        {copyError && (
          <p role="alert" className="text-sm text-destructive">
            {copyError}
          </p>
        )}
      </DialogHeader>
      <div className="space-y-6 py-4">
        {article.authors && article.authors.length > 0 && (
          <div>
            <h3 className="font-semibold mb-2 text-sm text-foreground/80">作者</h3>
            <p className="text-sm text-muted-foreground">{article.authors.join('; ')}</p>
          </div>
        )}

        <div>
          <h3 className="font-semibold mb-2 text-sm text-foreground/80">摘要</h3>
          <p className="text-sm text-muted-foreground leading-relaxed text-justify">
            {article.abstract || '暂无摘要。'}
          </p>
        </div>

        <div className="pt-4 border-t">
          <div className="flex flex-wrap gap-4">
            <Button variant="outline" size="sm" onClick={handleCopyArticleInfo}>
              {copyStatus === 'info' ? (
                <>
                  <Check className="mr-2 h-4 w-4 text-green-600" />
                  已复制
                </>
              ) : (
                <>
                  <Copy className="mr-2 h-4 w-4" />
                  复制信息
                </>
              )}
            </Button>
            {isAccessLoading && (
              <Button variant="outline" size="sm" disabled>
                <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                {isAccessPending ? '加载访问' : '刷新访问'}
              </Button>
            )}
            {isAccessQueryEnabled && !isAccessFetching && isAccessError && (
              <Button
                variant="outline"
                size="sm"
                disabled
                title={accessError instanceof Error ? accessError.message : '访问状态不可用'}
              >
                <ExternalLink className="mr-2 h-4 w-4" />
                访问状态失败
              </Button>
            )}
            {canShowAccessActions && abstractUrl && (
              <Button asChild variant="outline" size="sm">
                <a href={abstractUrl} target="_blank" rel="noreferrer">
                  <ExternalLink className="mr-2 h-4 w-4" />
                  {abstractAction?.label ?? '查看摘要页'}
                </a>
              </Button>
            )}
            {canShowAccessActions && fullTextUrl && (
              <Button asChild variant="outline" size="sm">
                <a href={fullTextUrl} target="_blank" rel="noreferrer">
                  <FileDown className="mr-2 h-4 w-4" />
                  {fulltextAction?.label ?? '获取全文'}
                </a>
              </Button>
            )}
            {canShowAccessActions && fulltextAction?.requires_login && (
              <DialogClose asChild>
                <Button asChild variant="outline" size="sm">
                  <Link href={dataSourceSettingsHref}>
                    <Settings className="mr-2 h-4 w-4" />
                    去设置登录
                  </Link>
                </Button>
              </DialogClose>
            )}
            {isFavoriteStatePending ? (
              <Button variant="outline" size="sm" disabled>
                加载收藏…
              </Button>
            ) : (
              <FavoriteButton
                articleId={article.article_id}
                dbName={dbName}
                initialFolderIds={initialFolderIds}
              />
            )}
            {extraActions}
          </div>
        </div>
      </div>
    </DialogContent>
  );
}
