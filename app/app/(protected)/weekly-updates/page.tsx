'use client';

import Link from 'next/link';
import { useEffect, useMemo, useState } from 'react';
import { useSearchParams } from 'next/navigation';
import { useQuery } from '@tanstack/react-query';
import {
  ArrowLeft,
  CalendarDays,
  Database,
  ExternalLink,
  FileText,
  Menu,
} from 'lucide-react';

import {
  getArticleById,
  getArticles,
  getDatabases,
  getFullTextUrlForDatabase,
  getWeeklyUpdates,
  setDatabase,
  type WeeklyArticle,
  type WeeklyDatabaseUpdate,
  type WeeklyJournalUpdate,
} from '@/lib/api';
import { SearchBar } from '@/components/feature/search-bar';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from '@/components/ui/card';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from '@/components/ui/dialog';
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select';
import { Skeleton } from '@/components/ui/skeleton';
import { cn } from '@/lib/utils';

const DATE_TIME_FORMATTER = new Intl.DateTimeFormat('zh-CN', {
  year: 'numeric',
  month: '2-digit',
  day: '2-digit',
  hour: '2-digit',
  minute: '2-digit',
  hour12: false,
});

function formatDate(value?: string): string {
  if (!value) {
    return '未知日期';
  }
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) {
    return value;
  }
  return DATE_TIME_FORMATTER.format(date).split(',')[0];
}

function selectDefaultDatabase(
  databases: string[],
  currentDb: string,
  preferredDb: string,
): string {
  if (databases.length === 0) {
    return '';
  }
  if (currentDb && databases.includes(currentDb)) {
    return currentDb;
  }
  if (preferredDb && databases.includes(preferredDb)) {
    return preferredDb;
  }
  return databases[0];
}

function selectDefaultJournal(
  journals: WeeklyJournalUpdate[],
  currentJournalId: number | null,
): number | null {
  if (journals.length === 0) {
    return null;
  }
  if (currentJournalId === null) {
    return journals[0].journal_id;
  }
  if (journals.some((item) => item.journal_id === currentJournalId)) {
    return currentJournalId;
  }
  return journals[0].journal_id;
}

function getJournalLabel(journal: WeeklyJournalUpdate): string {
  if (journal.journal_title && journal.journal_title.trim()) {
    return journal.journal_title;
  }
  return `期刊 ${journal.journal_id}`;
}

function buildArticleInfoText(article: WeeklyArticle): string {
  return `${article.title || `文章 ${article.article_id}`} · ${formatDate(article.date)}`;
}

type JournalPanelProps = {
  className?: string;
  contentClassName?: string;
  availableDatabases: string[];
  effectiveSelectedDb: string;
  journals: WeeklyJournalUpdate[];
  effectiveSelectedJournalId: number | null;
  onDatabaseChange: (value: string) => void;
  onSelectJournal: (journalId: number) => void;
};

function JournalPanel({
  className,
  contentClassName,
  availableDatabases,
  effectiveSelectedDb,
  journals,
  effectiveSelectedJournalId,
  onDatabaseChange,
  onSelectJournal,
}: JournalPanelProps) {
  return (
    <Card className={cn('min-h-0 overflow-hidden', className)}>
      <CardHeader className="space-y-3 pb-3">
        <CardTitle className="text-base">期刊</CardTitle>
        <div className="space-y-1.5">
          <span className="text-xs font-medium text-muted-foreground">数据库</span>
          <Select value={effectiveSelectedDb} onValueChange={onDatabaseChange}>
            <SelectTrigger className="w-full">
              <SelectValue placeholder="选择数据库" />
            </SelectTrigger>
            <SelectContent>
              {availableDatabases.map((dbName) => (
                <SelectItem key={dbName} value={dbName}>
                  {dbName}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
      </CardHeader>
      <CardContent className={cn('space-y-2 overflow-y-auto', contentClassName)}>
        {journals.length === 0 && (
          <div className="rounded-md border border-dashed p-4 text-sm text-muted-foreground">
            当前时间窗口内没有新增期刊。
          </div>
        )}

        {journals.map((journal) => {
          const active = effectiveSelectedJournalId === journal.journal_id;
          return (
            <button
              key={journal.journal_id}
              type="button"
              onClick={() => onSelectJournal(journal.journal_id)}
              className={`w-full rounded-md border p-3 text-left transition-colors ${
                active
                  ? 'border-primary bg-primary/5'
                  : 'border-border hover:bg-muted/40'
              }`}
            >
              <div className="flex items-center justify-between gap-2">
                <p className="line-clamp-2 text-sm font-medium">
                  {getJournalLabel(journal)}
                </p>
                <Badge variant={active ? 'default' : 'outline'}>
                  {journal.new_article_count}
                </Badge>
              </div>
            </button>
          );
        })}
      </CardContent>
    </Card>
  );
}

export default function WeeklyUpdatesPage() {
  const searchParams = useSearchParams();
  const requestedDb = (searchParams.get('db') || '').trim();
  const searchQuery = (searchParams.get('q') || '').trim();
  const [selectedDb, setSelectedDb] = useState<string>('');
  const [selectedJournalId, setSelectedJournalId] = useState<number | null>(null);

  const {
    data: weeklyData,
    isLoading: loadingWeekly,
    isError: weeklyError,
    error: weeklyErrorData,
  } = useQuery({
    queryKey: ['weekly-updates', 7],
    queryFn: () => getWeeklyUpdates(7),
    staleTime: 5 * 60 * 1000,
  });

  const { data: databaseOptions } = useQuery({
    queryKey: ['meta', 'databases'],
    queryFn: getDatabases,
    staleTime: 10 * 60 * 1000,
  });

  const dbMap = useMemo(() => {
    const map = new Map<string, WeeklyDatabaseUpdate>();
    for (const item of weeklyData?.databases ?? []) {
      map.set(item.db_name, item);
    }
    return map;
  }, [weeklyData]);

  const availableDatabases = useMemo(() => {
    if (!databaseOptions || databaseOptions.length === 0) {
      return Array.from(dbMap.keys());
    }
    const merged = new Set<string>();
    for (const item of databaseOptions) {
      merged.add(item);
    }
    for (const item of dbMap.keys()) {
      merged.add(item);
    }
    return Array.from(merged);
  }, [databaseOptions, dbMap]);

  const effectiveSelectedDb = useMemo(
    () => selectDefaultDatabase(availableDatabases, selectedDb, requestedDb),
    [availableDatabases, requestedDb, selectedDb],
  );

  const selectedDbData = useMemo(() => {
    if (!effectiveSelectedDb) {
      return null;
    }
    return dbMap.get(effectiveSelectedDb) ?? null;
  }, [dbMap, effectiveSelectedDb]);

  const journals = useMemo(() => selectedDbData?.journals ?? [], [selectedDbData]);

  const effectiveSelectedJournalId = useMemo(
    () => selectDefaultJournal(journals, selectedJournalId),
    [journals, selectedJournalId],
  );

  const selectedJournal = useMemo(() => {
    if (effectiveSelectedJournalId === null) {
      return null;
    }
    return (
      journals.find((item) => item.journal_id === effectiveSelectedJournalId) ?? null
    );
  }, [journals, effectiveSelectedJournalId]);

  const {
    data: searchedArticles,
    isLoading: loadingSearch,
    isError: searchError,
    error: searchErrorData,
  } = useQuery({
    queryKey: [
      'weekly-search',
      effectiveSelectedDb,
      effectiveSelectedJournalId,
      searchQuery,
    ],
    queryFn: async () => {
      if (!searchQuery || !effectiveSelectedDb || effectiveSelectedJournalId === null) {
        return [];
      }
      const params = new URLSearchParams();
      params.set('db', effectiveSelectedDb);
      params.append('journal_id', String(effectiveSelectedJournalId));
      params.set('q', searchQuery);
      params.set('limit', '200');
      const page = await getArticles(params, null, false);
      return page.items;
    },
    enabled: Boolean(
      searchQuery && effectiveSelectedDb && effectiveSelectedJournalId !== null,
    ),
    staleTime: 60 * 1000,
  });

  const visibleArticles = useMemo(() => {
    const weeklyArticles = selectedJournal?.articles ?? [];
    if (!searchQuery) {
      return weeklyArticles;
    }
    if (!searchedArticles) {
      return [];
    }
    const weeklyById = new Map<number, WeeklyArticle>();
    for (const article of weeklyArticles) {
      weeklyById.set(article.article_id, article);
    }
    const matched: WeeklyArticle[] = [];
    for (const article of searchedArticles) {
      const weeklyArticle = weeklyById.get(article.article_id);
      if (weeklyArticle) {
        matched.push(weeklyArticle);
      }
    }
    return matched;
  }, [searchedArticles, searchQuery, selectedJournal]);

  useEffect(() => {
    const listContainer = document.getElementById('weekly-articles-scroll-container');
    if (!listContainer) {
      return;
    }
    listContainer.scrollTop = 0;
  }, [effectiveSelectedDb, effectiveSelectedJournalId]);

  const totalDatabases = weeklyData?.databases.length ?? 0;
  const totalArticles = useMemo(() => {
    if (!weeklyData) {
      return 0;
    }
    return weeklyData.databases.reduce((sum, db) => sum + db.new_article_count, 0);
  }, [weeklyData]);

  const handleDatabaseChange = (value: string) => {
    setSelectedDb(value);
    setDatabase(value);
    setSelectedJournalId(null);
  };

  return (
    <div className="h-screen bg-background text-foreground">
      <div className="mx-auto flex h-full w-full max-w-[1400px] flex-col px-4 py-4 sm:px-6">
        <div className="mb-4 flex items-center justify-between gap-3">
          <div className="space-y-1">
            <div className="flex items-center gap-2 text-sm text-muted-foreground">
              <Dialog>
                <DialogTrigger asChild>
                  <Button
                    variant="outline"
                    size="icon"
                    className="shrink-0 lg:hidden"
                    aria-label="打开期刊筛选"
                  >
                    <Menu className="h-5 w-5" />
                  </Button>
                </DialogTrigger>
                <DialogContent
                  className="h-full w-[85vw] max-w-xs rounded-none left-0 top-0 translate-x-0 translate-y-0 p-0 gap-0 lg:hidden"
                  showCloseButton
                >
                  <DialogHeader className="sr-only">
                    <DialogTitle>期刊筛选</DialogTitle>
                    <DialogDescription>
                      选择数据库和期刊以查看每周更新。
                    </DialogDescription>
                  </DialogHeader>
                  <JournalPanel
                    className="h-full rounded-none border-0"
                    contentClassName="h-[calc(100%-140px)]"
                    availableDatabases={availableDatabases}
                    effectiveSelectedDb={effectiveSelectedDb}
                    journals={journals}
                    effectiveSelectedJournalId={effectiveSelectedJournalId}
                    onDatabaseChange={handleDatabaseChange}
                    onSelectJournal={setSelectedJournalId}
                  />
                </DialogContent>
              </Dialog>
              <CalendarDays className="h-4 w-4" />
              <span>每周新文章</span>
            </div>
            <h1 className="text-xl font-semibold tracking-tight">
              期刊每周更新
              {weeklyData
                ? ` (${formatDate(weeklyData.window_start)} - ${formatDate(weeklyData.window_end)})`
                : ''}
            </h1>
          </div>
          <Button asChild variant="outline" size="sm">
            <Link href="/">
              <ArrowLeft className="mr-2 h-4 w-4" />
              返回
            </Link>
          </Button>
        </div>

        {loadingWeekly && (
          <div className="space-y-4">
            <Skeleton className="h-20 w-full" />
            <Skeleton className="h-[70vh] w-full" />
          </div>
        )}

        {weeklyError && (
          <Card>
            <CardHeader>
              <CardTitle>加载每周更新失败</CardTitle>
              <CardDescription>
                {weeklyErrorData instanceof Error
                  ? weeklyErrorData.message
                  : '未知错误'}
              </CardDescription>
            </CardHeader>
          </Card>
        )}

        {!loadingWeekly && !weeklyError && weeklyData && (
          <>
            <div className="mb-4 grid grid-cols-1 gap-3 lg:grid-cols-[340px_1fr]">
              <div className="flex flex-wrap gap-2">
                <Badge variant="secondary" className="gap-1">
                  <Database className="h-3.5 w-3.5" />
                  {totalDatabases} 个数据库
                </Badge>
                <Badge variant="secondary" className="gap-1">
                  <FileText className="h-3.5 w-3.5" />
                  {totalArticles} 篇新文章
                </Badge>
              </div>
              <SearchBar className="w-full max-w-none" />
            </div>

            <div className="grid min-h-0 flex-1 grid-cols-1 gap-4 lg:grid-cols-[340px_1fr]">
              <JournalPanel
                className="hidden lg:flex lg:flex-col"
                contentClassName="h-[calc(100%-140px)]"
                availableDatabases={availableDatabases}
                effectiveSelectedDb={effectiveSelectedDb}
                journals={journals}
                effectiveSelectedJournalId={effectiveSelectedJournalId}
                onDatabaseChange={handleDatabaseChange}
                onSelectJournal={setSelectedJournalId}
              />

              <Card className="min-h-0 overflow-hidden">
                <CardHeader className="pb-3">
                  <CardTitle className="text-base">
                    {selectedJournal ? getJournalLabel(selectedJournal) : '文章'}
                  </CardTitle>
                  <CardDescription>
                    {selectedJournal
                      ? searchQuery
                        ? `匹配到 ${visibleArticles.length} 篇本周文章`
                        : `本周新增 ${selectedJournal.new_article_count} 篇文章`
                      : '请选择左侧期刊'}
                  </CardDescription>
                </CardHeader>
                <CardContent
                  id="weekly-articles-scroll-container"
                  className="h-[calc(100%-88px)] space-y-3 overflow-y-auto"
                >
                  {!selectedJournal && (
                    <div className="rounded-md border border-dashed p-4 text-sm text-muted-foreground">
                      请选择一个期刊以查看新收录文章。
                    </div>
                  )}

                  {searchQuery && loadingSearch && (
                    <div className="space-y-2">
                      <Skeleton className="h-16 w-full" />
                      <Skeleton className="h-16 w-full" />
                    </div>
                  )}

                  {searchQuery && searchError && (
                    <div className="rounded-md border border-dashed p-4 text-sm text-destructive">
                      {searchErrorData instanceof Error
                        ? searchErrorData.message
                        : '全文检索失败'}
                    </div>
                  )}

                  {selectedJournal && !loadingSearch && visibleArticles.length === 0 && (
                    <div className="rounded-md border border-dashed p-4 text-sm text-muted-foreground">
                      {searchQuery
                        ? '该期刊中没有匹配全文检索条件的本周文章。'
                        : '该期刊暂无文章。'}
                    </div>
                  )}

                  {visibleArticles.map((article) => (
                    <Dialog key={article.article_id}>
                      <DialogTrigger asChild>
                        <button
                          type="button"
                          className="w-full rounded-md border p-3 text-left transition-colors hover:bg-muted/40"
                        >
                          <div className="flex items-start justify-between gap-2">
                            <p className="line-clamp-2 text-sm font-medium">
                              {buildArticleInfoText(article)}
                            </p>
                            <div className="flex gap-1 shrink-0">
                              {article.open_access === 1 && (
                                <Badge variant="secondary" className="text-xs">开放获取</Badge>
                              )}
                              {article.in_press === 1 && (
                                <Badge variant="outline" className="text-xs">预发表</Badge>
                              )}
                            </div>
                          </div>
                          <p className="mt-1 text-xs text-muted-foreground">
                            DOI: {article.doi || '暂无'}
                          </p>
                        </button>
                      </DialogTrigger>
                      <ArticleDetailDialog
                        articleId={article.article_id}
                        dbName={effectiveSelectedDb}
                      />
                    </Dialog>
                  ))}
                </CardContent>
              </Card>
            </div>
          </>
        )}
      </div>
    </div>
  );
}

function ArticleDetailDialog({
  articleId,
  dbName,
}: {
  articleId: number;
  dbName: string;
}) {
  const { data, isLoading, isError, error } = useQuery({
    queryKey: ['weekly-article-detail', dbName, articleId],
    queryFn: () => getArticleById(articleId, dbName),
    staleTime: 10 * 60 * 1000,
  });

  return (
    <DialogContent className="max-h-[90vh] w-[calc(100%-2rem)] max-w-[calc(100%-2rem)] overflow-y-auto md:max-w-4xl">
      {isLoading && (
        <div className="space-y-3 py-4">
          <Skeleton className="h-6 w-3/4" />
          <Skeleton className="h-4 w-1/2" />
          <Skeleton className="h-32 w-full" />
        </div>
      )}

      {isError && (
        <div className="py-4 text-sm text-destructive">
          {error instanceof Error
            ? error.message
            : '加载文章详情失败'}
        </div>
      )}

      {data && (
        <>
          <DialogHeader>
            <DialogTitle className="text-xl leading-snug">
              {data.title || '未命名文章'}
            </DialogTitle>
            <DialogDescription>
              {data.journal_title || `期刊 ${data.journal_id}`}
              {data.date ? ` · ${data.date}` : ''}
              {data.volume ? ` · 第 ${data.volume} 卷` : ''}
              {data.number ? ` · 第 ${data.number} 期` : ''}
            </DialogDescription>
          </DialogHeader>

          <div className="space-y-6 py-4">
            {data.authors && (
              <div>
                <h3 className="mb-2 text-sm font-semibold text-foreground/80">
                  作者
                </h3>
                <p className="text-sm text-muted-foreground">{data.authors}</p>
              </div>
            )}

            <div>
              <h3 className="mb-2 text-sm font-semibold text-foreground/80">
                摘要
              </h3>
              <p className="text-justify text-sm leading-relaxed text-muted-foreground">
                {data.abstract || '暂无摘要。'}
              </p>
            </div>

            <div>
              <h3 className="mb-2 text-sm font-semibold text-foreground/80">DOI</h3>
              <p className="text-sm text-muted-foreground">{data.doi || '暂无'}</p>
            </div>

            {(data.doi || data.platform_id) && (
              <div className="pt-2">
                <a
                  href={
                    data.doi
                      ? `https://doi.org/${data.doi}`
                      : getFullTextUrlForDatabase(data.article_id, dbName)
                  }
                  target="_blank"
                  rel="noreferrer"
                >
                  <Button variant="outline">
                    <ExternalLink className="mr-2 h-4 w-4" />
                    查看全文
                  </Button>
                </a>
              </div>
            )}
          </div>
        </>
      )}
    </DialogContent>
  );
}



