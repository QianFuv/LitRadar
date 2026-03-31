'use client';

import { useQuery } from '@tanstack/react-query';
import { useParams, useRouter } from 'next/navigation';
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from '@/components/ui/card';
import { Badge } from '@/components/ui/badge';
import { Skeleton } from '@/components/ui/skeleton';
import { Button } from '@/components/ui/button';
import { FavoriteButton } from '@/components/feature/favorite-button';
import { ExternalLink, ArrowLeft } from 'lucide-react';
import { getArticleById, getFullTextUrlForDatabase, getCurrentDatabase } from '@/lib/api';
import { useAuth } from '@/lib/auth-context';

export default function ArticlePage() {
  const { id } = useParams<{ id: string }>();
  const router = useRouter();
  const { token } = useAuth();
  const currentDb = getCurrentDatabase();

  const { data: article, isLoading, isError, error } = useQuery({
    queryKey: ['article', id, currentDb],
    queryFn: () => getArticleById(Number(id), currentDb, token!),
    enabled: !!id && !!token,
  });

  if (isLoading) {
      return (
          <div className="mx-auto w-full max-w-4xl px-4 sm:px-6 py-6 space-y-6">
               <Skeleton className="h-8 w-1/4" />
               <Skeleton className="h-64 w-full" />
          </div>
      )
  }

  if (isError) {
      return (
          <div className="container mx-auto p-6 text-red-500">
              错误：{error instanceof Error ? error.message : '未知错误'}
          </div>
      )
  }

  if (!article) return null;

  return (
    <div className="mx-auto w-full max-w-4xl px-4 sm:px-6 py-6">
      <Button 
        variant="ghost" 
        className="mb-4 pl-0 hover:pl-0 hover:bg-transparent text-slate-500 hover:text-slate-900 dark:hover:text-slate-100"
        onClick={() => router.back()}
      >
          <ArrowLeft className="mr-2 h-4 w-4" /> 返回搜索结果
      </Button>
      
      <Card>
          <CardHeader>
              <div className="space-y-2">
                <div className="flex justify-between items-start gap-4">
                    <CardTitle className="text-2xl font-bold text-slate-900 dark:text-slate-100 leading-tight">
                        {article.title}
                    </CardTitle>
                    <div className="flex gap-2 shrink-0">
                         {article.open_access === 1 && <Badge variant="secondary">开放获取</Badge>}
                         {article.in_press === 1 && <Badge variant="outline">预发表</Badge>}
                    </div>
                </div>
                <CardDescription className="text-base">
                    {article.journal_title} • {article.date} • 第 {article.volume || '暂无'} 卷，第 {article.issue_id || '暂无'} 期
                </CardDescription>
              </div>
          </CardHeader>
          <CardContent className="space-y-6">
              <div>
                  <h3 className="font-semibold mb-2">摘要</h3>
                  <p className="text-slate-700 dark:text-slate-300 leading-relaxed text-justify">
                      {article.abstract || "暂无摘要。"}
                  </p>
              </div>

              {article.authors && (
                  <div>
                      <h3 className="font-semibold mb-2">作者</h3>
                      <p className="text-slate-600 dark:text-slate-400">
                          {article.authors}
                      </p>
                  </div>
              )}

              <div className="flex flex-wrap gap-4 pt-4 border-t">
                  {(article.doi || article.platform_id) && (
                      <a
                          href={
                                  article.doi
                                      ? `https://doi.org/${article.doi}`
                                      : getFullTextUrlForDatabase(article.article_id, currentDb)
                              }
                          target="_blank"
                          rel="noreferrer"
                      >
                          <Button>
                              <ExternalLink className="mr-2 h-4 w-4" />
                              查看全文
                          </Button>
                      </a>
                  )}
                  <FavoriteButton articleId={article.article_id} dbName={currentDb} />
              </div>
          </CardContent>
      </Card>
    </div>
  );
}
