/**
 * Pure single-article citation generation and safe external-link helpers.
 */

import type { Article } from '@/lib/api';

/** Citation formats supported by the article detail workflow. */
export type ArticleCitationFormat = 'gb-t-7714' | 'bibtex';

/** Recognizes a DOI value without a resolver URL. */
const DOI_VALUE_PATTERN = /^10\.\d{4,9}\/\S+$/iu;

/** Recognizes an explicit URI scheme before a value is treated as a bare DOI. */
const URI_SCHEME_PATTERN = /^[a-z][a-z\d+.-]*:/iu;

/** Escapes characters with special meaning inside a BibTeX braced value. */
const BIBTEX_SPECIAL_CHARACTER_PATTERN = /[\\{}%&#_$]/gu;

/**
 * Return trimmed content or a fallback when an article field is empty.
 *
 * @param value - Optional article text.
 * @param fallback - Text used when the field is empty.
 * @returns Non-empty text.
 */
function valueOrFallback(value: string | null | undefined, fallback: string): string {
  const normalizedValue = value?.trim();
  return normalizedValue || fallback;
}

/**
 * Extract a four-digit publication year without applying local timezone conversion.
 *
 * @param value - Article date value.
 * @param fallback - Text used when no valid year is available.
 * @returns Publication year or fallback.
 */
function extractPublicationYear(value: string | null | undefined, fallback: string): string {
  const normalizedValue = value?.trim();
  if (!normalizedValue) {
    return fallback;
  }
  const leadingYear = /^(\d{4})/u.exec(normalizedValue)?.[1];
  if (leadingYear) {
    return leadingYear;
  }
  const parsedDate = new Date(normalizedValue);
  return Number.isNaN(parsedDate.getTime()) ? fallback : String(parsedDate.getUTCFullYear());
}

/**
 * Validate and normalize an absolute HTTP(S) URL.
 *
 * @param value - Candidate external URL.
 * @returns Normalized safe URL or null for relative, malformed, or unsafe schemes.
 */
export function getSafeHttpUrl(value: string | null | undefined): string | null {
  const normalizedValue = value?.trim();
  if (!normalizedValue) {
    return null;
  }
  try {
    const url = new URL(normalizedValue);
    return url.protocol === 'http:' || url.protocol === 'https:' ? url.toString() : null;
  } catch {
    return null;
  }
}

/**
 * Normalize a DOI from a raw value, DOI prefix, or doi.org resolver URL.
 *
 * @param value - Candidate DOI value.
 * @returns Bare DOI or null when the value is not a DOI.
 */
function normalizeDoiValue(value: string | null | undefined): string | null {
  let normalizedValue = value?.trim();
  if (!normalizedValue) {
    return null;
  }
  normalizedValue = normalizedValue.replace(/^doi:\s*/iu, '');

  const safeUrl = getSafeHttpUrl(normalizedValue);
  if (safeUrl) {
    const url = new URL(safeUrl);
    if (url.hostname !== 'doi.org' && url.hostname !== 'dx.doi.org') {
      return null;
    }
    try {
      normalizedValue = decodeURIComponent(url.pathname.replace(/^\//u, ''));
    } catch {
      return null;
    }
  }

  return DOI_VALUE_PATTERN.test(normalizedValue) ? normalizedValue : null;
}

/**
 * Resolve a DOI field to a safe clickable destination.
 *
 * @param value - Raw DOI field.
 * @returns Safe HTTP(S) resolver URL or null.
 */
export function getDoiUrl(value: string | null | undefined): string | null {
  const normalizedValue = value?.trim();
  if (!normalizedValue) {
    return null;
  }

  const explicitUrl = getSafeHttpUrl(normalizedValue);
  if (explicitUrl) {
    return explicitUrl;
  }
  if (URI_SCHEME_PATTERN.test(normalizedValue) && !/^doi:/iu.test(normalizedValue)) {
    return null;
  }

  const doi = normalizeDoiValue(normalizedValue);
  return doi ? `https://doi.org/${doi}` : null;
}

/**
 * Format the volume and issue segment used by journal citations.
 *
 * @param article - Article record.
 * @returns Volume/issue text or an empty string.
 */
function formatVolumeIssue(article: Article): string {
  const volume = article.volume?.trim();
  const issue = article.number?.trim();
  if (volume && issue) {
    return `${volume}(${issue})`;
  }
  if (volume) {
    return volume;
  }
  return issue ? `(${issue})` : '';
}

/**
 * Escape one value for inclusion in a BibTeX braced field.
 *
 * @param value - Plain citation field value.
 * @returns BibTeX-safe value.
 */
function escapeBibtexValue(value: string): string {
  return value.replace(BIBTEX_SPECIAL_CHARACTER_PATTERN, (character) => {
    return character === '\\' ? '\\textbackslash{}' : `\\${character}`;
  });
}

/**
 * Build a deterministic BibTeX key from the stable article id.
 *
 * @param articleId - Stable article id.
 * @returns Sanitized BibTeX key.
 */
function buildBibtexKey(articleId: string): string {
  const normalizedId = articleId
    .normalize('NFKD')
    .replace(/[^\p{L}\p{N}]+/gu, '_')
    .replace(/^_+|_+$/gu, '')
    .toLowerCase();
  return `litradar_${normalizedId || 'article'}`;
}

/**
 * Generate a GB/T 7714-style journal article reference.
 *
 * @param article - Article record.
 * @returns Plain-text reference.
 */
function generateGbT7714Citation(article: Article): string {
  const authors = valueOrFallback(article.authors, '佚名');
  const title = valueOrFallback(article.title, '未命名文章');
  const journal = valueOrFallback(article.journal_title, '未知期刊');
  const year = extractPublicationYear(article.date, '日期不详');
  const volumeIssue = formatVolumeIssue(article);
  const publication = [journal, year, volumeIssue].filter(Boolean).join(', ');
  const doi = normalizeDoiValue(article.doi);
  return `${authors}. ${title}[J]. ${publication}.${doi ? ` DOI:${doi}.` : ''}`;
}

/**
 * Generate a BibTeX journal article record.
 *
 * @param article - Article record.
 * @returns BibTeX record.
 */
function generateBibtexCitation(article: Article): string {
  const authors = valueOrFallback(article.authors, 'Unknown author')
    .split(/\s*;\s*/u)
    .join(' and ');
  const fields: Array<[string, string]> = [
    ['author', authors],
    ['title', valueOrFallback(article.title, 'Untitled article')],
    ['journal', valueOrFallback(article.journal_title, 'Unknown journal')],
    ['year', extractPublicationYear(article.date, 'n.d.')],
  ];
  if (article.volume?.trim()) {
    fields.push(['volume', article.volume.trim()]);
  }
  if (article.number?.trim()) {
    fields.push(['number', article.number.trim()]);
  }
  const doi = normalizeDoiValue(article.doi);
  if (doi) {
    fields.push(['doi', doi]);
  }
  const url = getDoiUrl(article.doi) ?? getSafeHttpUrl(article.permalink);
  if (url) {
    fields.push(['url', url]);
  }

  const fieldLines: string[] = [];
  for (let index = 0; index < fields.length; index += 1) {
    const [name, value] = fields[index];
    const suffix = index === fields.length - 1 ? '' : ',';
    fieldLines.push(`  ${name} = {${escapeBibtexValue(value)}}${suffix}`);
  }
  return `@article{${buildBibtexKey(article.article_id)},\n${fieldLines.join('\n')}\n}`;
}

/**
 * Generate one supported single-article citation format.
 *
 * @param article - Article record.
 * @param format - Requested single-article format.
 * @returns Citation text.
 */
export function generateArticleCitation(article: Article, format: ArticleCitationFormat): string {
  return format === 'gb-t-7714'
    ? generateGbT7714Citation(article)
    : generateBibtexCitation(article);
}
