/**
 * Display missing article titles without changing bibliographic metadata.
 */

import type { Article } from '@/lib/api';

/**
 * Determine whether a record contains a usable source title.
 *
 * @param article - Article metadata.
 * @returns Whether title text can be displayed or copied as source metadata.
 */
export function hasArticleTitle(article: Article): article is Article & { title: string } {
  return Boolean(article.title?.trim());
}

/**
 * Return source text or a clearly identified missing-title label for presentation only.
 *
 * @param article - Article metadata with its stable identifier.
 * @returns A visible and distinguishable article heading.
 */
export function getArticleDisplayTitle(article: Article): string {
  return hasArticleTitle(article)
    ? article.title.trim()
    : article.doi
      ? `标题缺失 · DOI: ${article.doi}`
      : `标题缺失 · 文章 #${article.article_id}`;
}
