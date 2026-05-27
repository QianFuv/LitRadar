/**
 * Formatting helpers for the desktop frontend.
 */

import type { Article } from '@/lib/client-api';

const DATE_FORMATTER = new Intl.DateTimeFormat('zh-CN', {
  day: '2-digit',
  month: '2-digit',
  timeZone: 'UTC',
  year: 'numeric',
});

const DATE_TIME_FORMATTER = new Intl.DateTimeFormat('zh-CN', {
  day: '2-digit',
  hour: '2-digit',
  minute: '2-digit',
  month: '2-digit',
  year: 'numeric',
});

/**
 * Format a date string in Chinese locale.
 *
 * @param value - Date string.
 * @returns Formatted date or fallback text.
 */
export function formatDate(value?: string | null): string {
  if (!value) {
    return '未知日期';
  }
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) {
    return value;
  }
  return DATE_FORMATTER.format(date);
}

/**
 * Format a Unix timestamp in seconds.
 *
 * @param value - Timestamp in seconds.
 * @returns Formatted date-time.
 */
export function formatTimestamp(value?: number | null): string {
  if (!value) {
    return '从未';
  }
  return DATE_TIME_FORMATTER.format(new Date(value * 1000));
}

/**
 * Format an integer with locale grouping.
 *
 * @param value - Numeric value.
 * @returns Formatted number.
 */
export function formatCount(value?: number | null): string {
  return typeof value === 'number' ? value.toLocaleString('zh-CN') : '0';
}

/**
 * Resolve the title displayed for an article.
 *
 * @param article - Article record.
 * @returns Display title.
 */
export function getArticleTitle(article: Pick<Article, 'article_id' | 'title'>): string {
  return article.title?.trim() || `文章 #${article.article_id}`;
}

/**
 * Build a compact article venue line.
 *
 * @param article - Article record.
 * @returns Venue metadata line.
 */
export function getArticleVenue(article: Article): string {
  return [
    article.journal_title || (article.journal_id ? `期刊 ${article.journal_id}` : ''),
    article.volume ? `Vol. ${article.volume}` : '',
    article.number ? `No. ${article.number}` : '',
    article.date ? formatDate(article.date) : '',
  ]
    .filter(Boolean)
    .join(' · ');
}

/**
 * Build copyable article metadata text.
 *
 * @param article - Article record.
 * @returns Multi-line article metadata.
 */
export function buildArticleClipboardText(article: Article): string {
  return [
    `标题：${article.title || '暂无'}`,
    `作者：${article.authors || '暂无'}`,
    `期刊：${article.journal_title || '暂无'}`,
    `日期：${article.date || '暂无'}`,
    article.volume ? `卷号：${article.volume}` : '',
    article.number ? `期号：${article.number}` : '',
    article.doi ? `DOI：${article.doi}` : '',
    article.doi ? `链接：https://doi.org/${article.doi}` : '',
  ]
    .filter(Boolean)
    .join('\n');
}

/**
 * Format a weekly update date range.
 *
 * @param start - Start date.
 * @param end - End date.
 * @returns Date range label.
 */
export function formatDateRange(start?: string | null, end?: string | null): string {
  if (!start && !end) {
    return '时间窗口未知';
  }
  return `${formatDate(start)} - ${formatDate(end)}`;
}
