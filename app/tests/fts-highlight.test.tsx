/**
 * Regression tests for extracting and matching visual FTS highlights.
 */

import { describe, expect, test } from 'vitest';

import { createFtsHighlightPattern, parseFtsHighlightTerms } from '@/lib/fts-highlight';

/**
 * Collect matched text without exposing regular expression capture groups.
 *
 * @param text - Source text to search.
 * @param query - FTS query that supplies highlight terms.
 * @returns Full text matched by the generated highlight pattern.
 */
function collectHighlights(text: string, query: string): string[] {
  const pattern = createFtsHighlightPattern(parseFtsHighlightTerms(query));
  return pattern ? (text.match(pattern) ?? []) : [];
}

/**
 * Verify NEAR syntax contributes operands without its operator or distance.
 */
function extractsNearOperands(): void {
  const query = 'NEAR("gene expression" therapy, 5)';

  expect(parseFtsHighlightTerms(query)).toEqual([
    { value: 'gene expression', matchMode: 'exact' },
    { value: 'therapy', matchMode: 'exact' },
  ]);
  expect(collectHighlights('NEAR gene expression supports therapy at distance 5.', query)).toEqual([
    'gene expression',
    'therapy',
  ]);
  expect(query).toBe('NEAR("gene expression" therapy, 5)');
}

/**
 * Verify field selectors, braces, punctuation, and boolean operators stay out of highlights.
 */
function ignoresFtsSyntax(): void {
  const query = 'title:"gene expression" AND {title abstract}:therapy, OR NOT NEAR';

  expect(parseFtsHighlightTerms(query)).toEqual([
    { value: 'gene expression', matchMode: 'exact' },
    { value: 'therapy', matchMode: 'exact' },
  ]);
}

/**
 * Verify prefix terms match only at the beginning of a non-CJK token.
 */
function respectsPrefixBoundaries(): void {
  expect(collectHighlights('biology microbiology Bioinformatics symbio', 'bio*')).toEqual([
    'bio',
    'Bio',
  ]);
  expect(collectHighlights('biology bio microbiology', 'bio')).toEqual(['bio']);
}

/**
 * Verify longer phrases win before overlapping shorter terms.
 */
function matchesLongestTermsFirst(): void {
  expect(collectHighlights('gene expression', 'gene "gene expression"')).toEqual([
    'gene expression',
  ]);
}

/**
 * Verify CJK length rules and case-insensitive duplicate removal remain deterministic.
 */
function filtersShortAndDuplicateTerms(): void {
  expect(parseFtsHighlightTerms('AI 人 人工 人工 "基因 表达" "基因 表达"')).toEqual([
    { value: '人工', matchMode: 'exact' },
    { value: '基因 表达', matchMode: 'exact' },
  ]);
  expect(collectHighlights('这是人工智能与基因 表达研究。', 'AI 人 人工 "基因 表达"')).toEqual([
    '人工',
    '基因 表达',
  ]);
}

/**
 * Verify malformed delimiters fail predictably without throwing.
 */
function handlesMalformedInput(): void {
  expect(parseFtsHighlightTerms('{title abstract therapy')).toEqual([]);
  expect(parseFtsHighlightTerms('"gene expression')).toEqual([
    { value: 'gene expression', matchMode: 'exact' },
  ]);
  expect(collectHighlights('C++ primer', '"C++"')).toEqual(['C++']);
  expect(parseFtsHighlightTerms('')).toEqual([]);
  expect(createFtsHighlightPattern([])).toBeNull();
}

describe('FTS highlight parsing', () => {
  test('extracts NEAR operands without syntax tokens', extractsNearOperands);
  test('ignores field selectors, punctuation, and boolean operators', ignoresFtsSyntax);
  test('matches prefix terms only at token starts', respectsPrefixBoundaries);
  test('matches longer phrases before overlapping terms', matchesLongestTermsFirst);
  test('applies CJK length rules and removes duplicates', filtersShortAndDuplicateTerms);
  test('handles malformed input without throwing', handlesMalformedInput);
});
