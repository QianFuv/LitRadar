'use client';

import { useState, type ReactNode } from 'react';
import { useQuery } from '@tanstack/react-query';
import Link from 'next/link';
import { usePathname, useSearchParams } from 'next/navigation';
import { Check, Copy, ExternalLink, FileDown, Loader2, Settings } from 'lucide-react';

import {
  getArticleAccess,
  getCnkiSession,
  getFullTextUrlForDatabase,
  type ArticleAccessResponse,
  type Article,
  type CnkiSessionStatus,
} from '@/lib/api';
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
import {
  generateArticleCitation,
  getDoiUrl,
  getSafeHttpUrl,
  type ArticleCitationFormat,
} from '@/lib/citation';
import { buildSettingsCenterHref } from '@/lib/settings-center';

type ArticleDetailDialogArticle = Article;

type ArticleDetailDialogContentProps = {
  article: ArticleDetailDialogArticle;
  dbName: string;
  initialFolderIds?: number[];
  isFavoriteStatePending?: boolean;
  extraActions?: ReactNode;
};

type ArticleAccessQuerySnapshot = {
  state: {
    data?: unknown;
    error?: unknown;
  };
};

type ArticleCopyTarget = 'title' | 'info' | 'gb-t-7714' | 'bibtex' | 'doi' | 'permalink';

const ARTICLE_ACCESS_STALE_TIME_MS = 5 * 60 * 1000;
const CNKI_SESSION_STALE_TIME_MS = 60 * 1000;
const CNKI_SESSION_EXPIRY_REFRESH_WINDOW_SECONDS = 10 * 60;

/**
 * Build the existing plain-text article information summary.
 *
 * @param article - Article record.
 * @returns Multi-line article information.
 */
function buildArticleInfoText(article: ArticleDetailDialogArticle): string {
  const doiUrl = getDoiUrl(article.doi);
  return [
    `标题：${article.title || '暂无'}`,
    `作者：${article.authors || '暂无'}`,
    `期刊：${article.journal_title || '暂无'}`,
    `日期：${article.date || '暂无'}`,
    article.volume && `卷号：${article.volume}`,
    article.number && `期号：${article.number}`,
    article.doi && `DOI: ${article.doi}`,
    doiUrl && `DOI 链接：${doiUrl}`,
    article.permalink && `永久链接：${article.permalink}`,
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
 * Check whether a CNKI session is close enough to expiry to avoid cached article access.
 *
 * @param session - Current safe CNKI session status.
 * @returns True when access checks should be refreshed aggressively.
 */
function isCnkiSessionNearExpiry(session: CnkiSessionStatus): boolean {
  if (typeof session.seconds_remaining === 'number') {
    return session.seconds_remaining <= CNKI_SESSION_EXPIRY_REFRESH_WINDOW_SECONDS;
  }
  if (typeof session.expires_at === 'number') {
    return session.expires_at - Date.now() / 1000 <= CNKI_SESSION_EXPIRY_REFRESH_WINDOW_SECONDS;
  }
  return false;
}

/**
 * Decide whether the CNKI session state requires live article access checks.
 *
 * @param session - Current safe CNKI session status.
 * @returns True when article access should be refreshed on each mount.
 */
function shouldRefreshArticleAccessForCnkiSession(session?: CnkiSessionStatus): boolean {
  if (!session) {
    return true;
  }
  if (!session.configured || session.status !== 'active' || !session.has_bff_user_token) {
    return true;
  }
  return isCnkiSessionNearExpiry(session);
}

/**
 * Build a cache key segment that separates article access by CNKI session generation.
 *
 * @param session - Current safe CNKI session status.
 * @returns Stable non-secret cache key segment.
 */
function buildArticleAccessSessionKey(session?: CnkiSessionStatus): string {
  if (!session) {
    return 'session:unknown';
  }
  return [
    'session',
    session.status,
    session.configured ? 'configured' : 'empty',
    session.has_bff_user_token ? 'token' : 'no-token',
    session.updated_at ?? 'updated-unknown',
    session.expires_at ?? 'expiry-unknown',
  ].join(':');
}

/**
 * Check whether an article access result indicates missing or unusable full-text access.
 *
 * @param access - Article access response.
 * @returns True when future mounts should use live refresh behavior.
 */
function isUnavailableArticleAccess(access?: ArticleAccessResponse): boolean {
  return access?.fulltext.requires_login === true;
}

/**
 * Decide whether cached article access data should be treated as live-only.
 *
 * @param query - Current article access query snapshot.
 * @param shouldRefreshForSession - Whether the CNKI session requires live refresh.
 * @returns True when this access query should refresh on mount.
 */
function shouldUseLiveArticleAccessRefresh(
  query: ArticleAccessQuerySnapshot,
  shouldRefreshForSession: boolean,
): boolean {
  if (shouldRefreshForSession || query.state.error) {
    return true;
  }
  return isUnavailableArticleAccess(query.state.data as ArticleAccessResponse | undefined);
}

/**
 * Render article metadata, citations, links, access actions, and favorite controls.
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
  const [copyFeedback, setCopyFeedback] = useState<{
    message: string;
    tone: 'error' | 'success';
  } | null>(null);
  const isAccessQueryEnabled = !!dbName && !!article.article_id;
  const { data: cnkiSession } = useQuery({
    queryKey: ['cnki-session', 'current'],
    queryFn: () => getCnkiSession(),
    enabled: isAccessQueryEnabled,
    staleTime: CNKI_SESSION_STALE_TIME_MS,
  });
  const shouldRefreshAccessForSession = shouldRefreshArticleAccessForCnkiSession(cnkiSession);
  const {
    data: access,
    isPending: isAccessPending,
    isFetching: isAccessFetching,
    isError: isAccessError,
    error: accessError,
  } = useQuery({
    queryKey: [
      'article-access',
      dbName,
      article.article_id,
      buildArticleAccessSessionKey(cnkiSession),
      shouldRefreshAccessForSession ? 'live' : 'cached',
    ],
    queryFn: () => getArticleAccess(article.article_id, dbName),
    enabled: isAccessQueryEnabled,
    staleTime: (query) =>
      shouldUseLiveArticleAccessRefresh(query, shouldRefreshAccessForSession)
        ? 0
        : ARTICLE_ACCESS_STALE_TIME_MS,
    refetchOnMount: (query) =>
      shouldUseLiveArticleAccessRefresh(query, shouldRefreshAccessForSession) ? 'always' : true,
  });

  /**
   * Copy one article value and publish accessible feedback.
   *
   * @param text - Text to copy.
   * @param status - Copy action identifier.
   * @param successMessage - Success feedback.
   */
  const handleCopy = async (text: string, status: ArticleCopyTarget, successMessage: string) => {
    try {
      await copyTextToClipboard(text);
      setCopyStatus(status);
      setCopyFeedback({ message: successMessage, tone: 'success' });
    } catch {
      setCopyStatus(null);
      setCopyFeedback({ message: '复制失败，请手动选择文本复制。', tone: 'error' });
    }
    setTimeout(() => {
      setCopyStatus(null);
      setCopyFeedback(null);
    }, 3000);
  };

  /** Copy the article title. */
  const handleCopyTitle = async () => {
    await handleCopy(article.title || '', 'title', '文章标题已复制。');
  };

  /** Copy the plain-text article information summary. */
  const handleCopyArticleInfo = async () => {
    await handleCopy(buildArticleInfoText(article), 'info', '文章信息已复制。');
  };

  /**
   * Copy one generated citation format.
   *
   * @param format - Single-article citation format.
   */
  const handleCopyCitation = async (format: ArticleCitationFormat) => {
    const label = format === 'gb-t-7714' ? 'GB/T 7714' : 'BibTeX';
    await handleCopy(generateArticleCitation(article, format), format, `${label} 引用已复制。`);
  };

  /** Copy the raw DOI field. */
  const handleCopyDoi = async () => {
    await handleCopy(article.doi || '', 'doi', 'DOI 已复制。');
  };

  /** Copy the raw permalink field. */
  const handleCopyPermalink = async () => {
    await handleCopy(article.permalink || '', 'permalink', '永久链接已复制。');
  };

  const detailAction = access?.detail;
  const fulltextAction = access?.fulltext;
  const fullTextUrl = fulltextAction?.available
    ? getFullTextUrlForDatabase(article.article_id, dbName)
    : null;
  const isAccessLoading = isAccessQueryEnabled && (isAccessPending || isAccessFetching);
  const canShowAccessActions = !isAccessFetching && !isAccessError;
  const doiUrl = getDoiUrl(article.doi);
  const permalinkUrl = getSafeHttpUrl(article.permalink);
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
        {copyFeedback && (
          <p
            role={copyFeedback.tone === 'error' ? 'alert' : 'status'}
            className={
              copyFeedback.tone === 'error'
                ? 'text-sm text-destructive'
                : 'text-sm text-muted-foreground'
            }
          >
            {copyFeedback.message}
          </p>
        )}
      </DialogHeader>
      <div className="space-y-6 py-4">
        {article.authors && (
          <div>
            <h3 className="font-semibold mb-2 text-sm text-foreground/80">作者</h3>
            <p className="text-sm text-muted-foreground">{article.authors}</p>
          </div>
        )}

        <div>
          <h3 className="font-semibold mb-2 text-sm text-foreground/80">摘要</h3>
          <p className="text-sm text-muted-foreground leading-relaxed text-justify">
            {article.abstract || '暂无摘要。'}
          </p>
        </div>

        <div>
          <h3 className="mb-2 text-sm font-semibold text-foreground/80">引用与链接</h3>
          <div className="flex flex-wrap gap-2">
            <Button
              variant="outline"
              size="sm"
              onClick={() => void handleCopyCitation('gb-t-7714')}
            >
              {copyStatus === 'gb-t-7714' ? (
                <Check className="mr-2 h-4 w-4 text-green-600" />
              ) : (
                <Copy className="mr-2 h-4 w-4" />
              )}
              复制 GB/T 7714
            </Button>
            <Button variant="outline" size="sm" onClick={() => void handleCopyCitation('bibtex')}>
              {copyStatus === 'bibtex' ? (
                <Check className="mr-2 h-4 w-4 text-green-600" />
              ) : (
                <Copy className="mr-2 h-4 w-4" />
              )}
              复制 BibTeX
            </Button>
            {article.doi && (
              <Button variant="outline" size="sm" onClick={() => void handleCopyDoi()}>
                {copyStatus === 'doi' ? (
                  <Check className="mr-2 h-4 w-4 text-green-600" />
                ) : (
                  <Copy className="mr-2 h-4 w-4" />
                )}
                复制 DOI
              </Button>
            )}
            {doiUrl && (
              <Button asChild variant="outline" size="sm">
                <a href={doiUrl} target="_blank" rel="noreferrer">
                  <ExternalLink className="mr-2 h-4 w-4" />
                  打开 DOI
                </a>
              </Button>
            )}
            {article.permalink && (
              <Button variant="outline" size="sm" onClick={() => void handleCopyPermalink()}>
                {copyStatus === 'permalink' ? (
                  <Check className="mr-2 h-4 w-4 text-green-600" />
                ) : (
                  <Copy className="mr-2 h-4 w-4" />
                )}
                复制永久链接
              </Button>
            )}
            {permalinkUrl && (
              <Button asChild variant="outline" size="sm">
                <a href={permalinkUrl} target="_blank" rel="noreferrer">
                  <ExternalLink className="mr-2 h-4 w-4" />
                  打开永久链接
                </a>
              </Button>
            )}
          </div>
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
            {canShowAccessActions && detailAction?.available && detailAction.url && (
              <Button asChild variant="outline" size="sm">
                <a href={detailAction.url} target="_blank" rel="noreferrer">
                  <ExternalLink className="mr-2 h-4 w-4" />
                  {detailAction.label}
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
