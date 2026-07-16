/**
 * Single-article citation generation and safe-link coverage.
 */

import { describe, expect, test } from 'vitest';

import type { Article } from '@/lib/api';
import {
  generateArticleCitation,
  getDoiUrl,
  getSafeHttpUrl,
  type ArticleCitationFormat,
} from '@/lib/citation';

const FULL_ARTICLE: Article = {
  article_id: 'article:1',
  title: 'Computing & Society',
  authors: 'Ada Lovelace; Alan Turing',
  journal_title: 'Journal of Tests',
  date: '2024-05-17',
  volume: '12',
  number: '3',
  doi: 'https://doi.org/10.1000/example',
  permalink: 'https://example.com/articles/1',
};

/**
 * Verify a complete article produces deterministic GB/T 7714 and BibTeX text.
 */
function generatesCompleteCitations(): void {
  expect(generateArticleCitation(FULL_ARTICLE, 'gb-t-7714')).toBe(
    'Ada Lovelace; Alan Turing. Computing & Society[J]. Journal of Tests, 2024, 12(3). DOI:10.1000/example.',
  );
  expect(generateArticleCitation(FULL_ARTICLE, 'bibtex')).toBe(`@article{litradar_article_1,
  author = {Ada Lovelace and Alan Turing},
  title = {Computing \\& Society},
  journal = {Journal of Tests},
  year = {2024},
  volume = {12},
  number = {3},
  doi = {10.1000/example},
  url = {https://doi.org/10.1000/example}
}`);
}

/**
 * Verify sparse records keep explicit, readable fallback values.
 */
function generatesSparseCitations(): void {
  const article: Article = { article_id: 'sparse' };

  expect(generateArticleCitation(article, 'gb-t-7714')).toBe(
    '佚名. 未命名文章[J]. 未知期刊, 日期不详.',
  );
  expect(generateArticleCitation(article, 'bibtex')).toBe(`@article{litradar_sparse,
  author = {Unknown author},
  title = {Untitled article},
  journal = {Unknown journal},
  year = {n.d.}
}`);
}

/**
 * Verify the single-article format type remains intentionally narrow.
 */
function supportsDeclaredFormats(): void {
  const formats: ArticleCitationFormat[] = ['gb-t-7714', 'bibtex'];
  expect(formats.map((format) => generateArticleCitation(FULL_ARTICLE, format))).toHaveLength(2);
}

/**
 * Verify only absolute HTTP(S) destinations become clickable links.
 */
function validatesExternalDestinations(): void {
  expect(getSafeHttpUrl('https://example.com/article')).toBe('https://example.com/article');
  expect(getSafeHttpUrl('http://example.com/article')).toBe('http://example.com/article');
  expect(getSafeHttpUrl('javascript:alert(1)')).toBeNull();
  expect(getSafeHttpUrl('data:text/html,unsafe')).toBeNull();
  expect(getSafeHttpUrl('/relative/article')).toBeNull();

  expect(getDoiUrl('10.1000/example')).toBe('https://doi.org/10.1000/example');
  expect(getDoiUrl('doi:10.1000/example')).toBe('https://doi.org/10.1000/example');
  expect(getDoiUrl('https://doi.org/10.1000/example')).toBe('https://doi.org/10.1000/example');
  expect(getDoiUrl('javascript:alert(1)')).toBeNull();
}

describe('single-article citations', () => {
  test('generates complete GB/T 7714 and BibTeX citations', generatesCompleteCitations);
  test('generates explicit sparse-record fallbacks', generatesSparseCitations);
  test('supports only the declared single-article formats', supportsDeclaredFormats);
  test('validates DOI and permalink destinations', validatesExternalDestinations);
});
