'use client';

import { useState, type ReactNode } from 'react';
import { useQuery } from '@tanstack/react-query';
import Link from 'next/link';
import { Check, Copy, ExternalLink, FileDown, Loader2, Settings } from 'lucide-react';

import {
  getArticleAccess,
  getFullTextUrlForDatabase,
  type ArticleId,
  type JournalId,
} from '@/lib/api';
import { FavoriteButton } from '@/components/feature/favorite-button';
import { Button } from '@/components/ui/button';
import {
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';

type ArticleDetailDialogArticle = {
  article_id: ArticleId;
  journal_id?: JournalId | null;
  title?: string | null;
  date?: string | null;
  authors?: string | null;
  abstract?: string | null;
  doi?: string | null;
  platform_id?: string | null;
  permalink?: string | null;
  journal_title?: string | null;
  volume?: string | null;
  number?: string | null;
  full_text_file?: string | null;
};

type ArticleDetailDialogContentProps = {
  article: ArticleDetailDialogArticle;
  dbName: string;
  token?: string;
  initialFolderIds?: number[];
  isFavoriteStatePending?: boolean;
  extraActions?: ReactNode;
};

function buildArticleInfoText(article: ArticleDetailDialogArticle): string {
  return [
    `标题：${article.title || '暂无'}`,
    `作者：${article.authors || '暂无'}`,
    `期刊：${article.journal_title || '暂无'}`,
    `日期：${article.date || '暂无'}`,
    article.volume && `卷号：${article.volume}`,
    article.number && `期号：${article.number}`,
    article.doi && `DOI: ${article.doi}`,
    article.doi && `链接：https://doi.org/${article.doi}`,
  ]
    .filter(Boolean)
    .join('\n');
}

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

export function ArticleDetailDialogContent({
  article,
  dbName,
  token,
  initialFolderIds = [],
  isFavoriteStatePending = false,
  extraActions,
}: ArticleDetailDialogContentProps) {
  const [copyStatus, setCopyStatus] = useState<'title' | 'info' | null>(null);
  const isAccessQueryEnabled = !!token && !!dbName && !!article.article_id;
  const {
    data: access,
    isPending: isAccessPending,
    isError: isAccessError,
    error: accessError,
  } = useQuery({
    queryKey: ['article-access', dbName, article.article_id],
    queryFn: () => getArticleAccess(article.article_id, dbName, token!),
    enabled: isAccessQueryEnabled,
    staleTime: 5 * 60 * 1000,
  });

  const handleCopyTitle = async () => {
    await navigator.clipboard.writeText(article.title || '');
    setCopyStatus('title');
    setTimeout(() => setCopyStatus(null), 3000);
  };

  const handleCopyArticleInfo = async () => {
    await navigator.clipboard.writeText(buildArticleInfoText(article));
    setCopyStatus('info');
    setTimeout(() => setCopyStatus(null), 3000);
  };

  const detailAction = access?.detail;
  const fulltextAction = access?.fulltext;
  const fullTextUrl = fulltextAction?.available
    ? getFullTextUrlForDatabase(article.article_id, dbName, token)
    : null;

  return (
    <DialogContent className="w-[calc(100%-2rem)] max-w-[calc(100%-2rem)] md:max-w-4xl max-h-[90vh] overflow-y-auto [&>button]:hidden">
      <DialogHeader>
        <DialogTitle className="text-xl leading-snug">
          {article.title || '未命名文章'}
          <Button
            variant="ghost"
            size="sm"
            className="h-6 w-6 p-0 ml-2 inline-flex align-middle"
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
            {isAccessQueryEnabled && isAccessPending && (
              <Button variant="outline" size="sm" disabled>
                <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                加载访问
              </Button>
            )}
            {isAccessQueryEnabled && isAccessError && (
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
            {detailAction?.available && detailAction.url && (
              <a href={detailAction.url} target="_blank" rel="noreferrer">
                <Button variant="outline" size="sm">
                  <ExternalLink className="mr-2 h-4 w-4" />
                  {detailAction.label}
                </Button>
              </a>
            )}
            {fullTextUrl && (
              <a href={fullTextUrl} target="_blank" rel="noreferrer">
                <Button variant="outline" size="sm">
                  <FileDown className="mr-2 h-4 w-4" />
                  {fulltextAction?.label ?? '获取全文'}
                </Button>
              </a>
            )}
            {fulltextAction?.requires_login && (
              <Button asChild variant="outline" size="sm">
                <Link href="/settings">
                  <Settings className="mr-2 h-4 w-4" />
                  去设置登录
                </Link>
              </Button>
            )}
            {isFavoriteStatePending ? (
              <Button variant="outline" size="sm" disabled>
                加载收藏...
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
