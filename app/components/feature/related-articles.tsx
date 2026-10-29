'use client';

/**
 * Related-article list shown at the end of article details.
 */

import { useQuery } from '@tanstack/react-query';
import { useInView } from 'react-intersection-observer';

import { Skeleton } from '@/components/ui/skeleton';
import { getRelatedArticles, type Article } from '@/lib/api';
import { getArticleDisplayTitle } from '@/lib/article-title';

/** Number of related articles requested from the backend. */
const RELATED_ARTICLE_LIMIT = 6;

type RelatedArticlesProps = {
  article: Article;
  dbName: string;
  /** Open a related article in the current reader. */
  onOpen: (article: Article) => void;
};

/**
 * Render related articles, ranked by the backend, once the section scrolls near the viewport.
 *
 * @param props - Source article, database, and open callback.
 * @returns Related-article section.
 */
export function RelatedArticles({ article, dbName, onOpen }: RelatedArticlesProps) {
  const { ref, inView } = useInView({
    triggerOnce: true,
    rootMargin: '300px',
    fallbackInView: false,
  });
  const relatedQuery = useQuery({
    queryKey: ['related-articles', dbName, String(article.article_id)],
    queryFn: ({ signal }) =>
      getRelatedArticles(article.article_id, dbName, RELATED_ARTICLE_LIMIT, signal),
    enabled: inView && Boolean(dbName),
    staleTime: 10 * 60 * 1000,
    retry: false,
  });
  const related = relatedQuery.data ?? [];

  return (
    <section ref={ref} aria-label="相关文章" className="mt-8">
      <h3 className="flex items-center gap-3 text-[11px] font-bold tracking-[0.16em] text-muted-foreground after:h-px after:flex-1 after:bg-border">
        相关文章
      </h3>
      {relatedQuery.isError ? (
        <p className="mt-3 text-[13px] text-muted-foreground">暂时无法加载相关文章。</p>
      ) : !relatedQuery.isSuccess && relatedQuery.fetchStatus === 'idle' ? (
        <div aria-hidden="true" className="h-16" />
      ) : !relatedQuery.isSuccess ? (
        <div role="status" className="mt-3 space-y-3">
          <span className="sr-only">正在查找相关文章</span>
          {[0, 1, 2].map((index) => (
            <div key={index} className="space-y-1.5">
              <Skeleton className="h-4 w-11/12" />
              <Skeleton className="h-3 w-1/2" />
            </div>
          ))}
        </div>
      ) : related.length === 0 ? (
        <p className="mt-3 text-[13px] text-muted-foreground">暂未找到相关文章。</p>
      ) : (
        <ul className="mt-1 divide-y divide-border/70">
          {related.map((candidate) => (
            <li key={String(candidate.article_id)}>
              <button
                type="button"
                className="motion-control -mx-2 block w-[calc(100%+1rem)] rounded-md px-2 py-3 text-left outline-none transition-colors hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring"
                onClick={() => onOpen(candidate)}
              >
                <span className="line-clamp-2 text-[13.5px] leading-[1.5] font-bold">
                  {getArticleDisplayTitle(candidate)}
                </span>
                <span className="mt-1 block truncate text-xs text-muted-foreground">
                  {[candidate.journal_title, candidate.date].filter(Boolean).join(' · ')}
                </span>
              </button>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
