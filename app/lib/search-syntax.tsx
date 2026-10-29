/**
 * Advanced article-search syntax helpers: detection, rewrite suggestions, and insert templates.
 *
 * The article index is an SQLite FTS5 table with the columns `title`, `abstract_text`,
 * `authors`, `journal_title`, `doi`, and `pmid`. Advanced syntax is only honored when the
 * article request is sent with `search_mode=advanced`.
 */

/** Article search mode understood by the article list endpoint. */
export type SearchSyntaxMode = 'simple' | 'advanced';

/** Searchable FTS5 column exposed to users. */
export type SearchFieldOption = {
  column: string;
  label: string;
};

/** Searchable columns in the order they are offered. */
const SEARCH_FIELD_OPTIONS: readonly SearchFieldOption[] = [
  { column: 'title', label: '标题' },
  { column: 'authors', label: '作者' },
  { column: 'journal_title', label: '期刊' },
  { column: 'abstract_text', label: '摘要' },
  { column: 'doi', label: 'DOI' },
];

const FIELD_FILTER_PATTERN = new RegExp(
  `(^|[\\s(])(?:${['title', 'abstract_text', 'authors', 'journal_title', 'doi', 'pmid'].join('|')})\\s*:`,
);
const COLUMN_SET_PATTERN = /\{[^}]*\}\s*:/;
const BOOLEAN_OPERATOR_PATTERN = /(^|[\s)])(AND|OR|NOT)(?=[\s(])/;
const NEAR_PATTERN = /(^|[\s(])NEAR\s*\(/;
const PREFIX_PATTERN = /[^\s*"]\*(?=$|[\s)])/;
const INITIAL_TOKEN_PATTERN = /(^|[\s(:])\^[^\s]/;

/**
 * Check whether a query uses FTS5 syntax that needs the advanced search mode.
 *
 * Operators must be upper-case and standalone, matching FTS5 itself, so ordinary words such as
 * "and" or "Not" keep the forgiving simple mode.
 *
 * @param query - Raw query text.
 * @returns True when the query contains advanced syntax.
 */
export function isAdvancedSearchQuery(query: string): boolean {
  const trimmed = query.trim();
  if (!trimmed) {
    return false;
  }
  return (
    trimmed.includes('"') ||
    BOOLEAN_OPERATOR_PATTERN.test(trimmed) ||
    NEAR_PATTERN.test(trimmed) ||
    FIELD_FILTER_PATTERN.test(trimmed) ||
    COLUMN_SET_PATTERN.test(trimmed) ||
    PREFIX_PATTERN.test(trimmed) ||
    INITIAL_TOKEN_PATTERN.test(trimmed)
  );
}

/**
 * Choose the search mode for a query.
 *
 * @param query - Raw query text.
 * @returns Advanced when the query contains FTS5 syntax, otherwise simple.
 */
export function getSearchSyntaxMode(query: string): SearchSyntaxMode {
  return isAdvancedSearchQuery(query) ? 'advanced' : 'simple';
}

/**
 * Translate a backend search error into user-facing guidance.
 *
 * @param message - Error message returned by the API.
 * @returns Localized guidance, or the original message for unrelated errors.
 */
export function describeSearchError(message: string): string {
  if (message.trim() === 'Invalid search expression') {
    return '检索语法有误：请检查引号与括号是否成对、运算符 AND / OR / NOT / NEAR 是否大写，以及字段名是否正确。';
  }
  return message;
}

/** One rewrite of the current draft offered while typing. */
export type SearchRewriteSuggestion = {
  id: string;
  label: string;
  value: string;
};

/**
 * Quote a phrase for FTS5, doubling embedded quotes.
 *
 * @param text - Phrase text.
 * @returns Quoted phrase.
 */
function quotePhrase(text: string): string {
  return `"${text.replaceAll('"', '""')}"`;
}

/**
 * Wrap a draft as a field-scoped operand, quoting it when it contains spaces.
 *
 * @param column - FTS5 column name.
 * @param text - Draft text.
 * @returns Field-scoped expression.
 */
function scopeToField(column: string, text: string): string {
  const isQuoted = text.startsWith('"') && text.endsWith('"') && text.length > 1;
  const isSingleToken = !/\s/.test(text) && !text.includes('"');
  return `${column}:${isQuoted || isSingleToken ? text : quotePhrase(text)}`;
}

/**
 * Build rewrite suggestions for a non-empty draft.
 *
 * @param draft - Current input text.
 * @returns Distinct rewrites that change the draft.
 */
export function buildRewriteSuggestions(draft: string): SearchRewriteSuggestion[] {
  const text = draft.trim();
  if (!text) {
    return [];
  }
  const words = text.split(/\s+/);
  const isPlain = !isAdvancedSearchQuery(text);
  const suggestions: SearchRewriteSuggestion[] = [];
  if (isPlain && words.length > 1) {
    suggestions.push({ id: 'phrase', label: '作为精确短语', value: quotePhrase(text) });
    suggestions.push({ id: 'all', label: '全部词都要出现', value: words.join(' AND ') });
    suggestions.push({ id: 'any', label: '任一词出现即可', value: words.join(' OR ') });
  }
  if (isPlain && words.length === 1 && !text.endsWith('*')) {
    suggestions.push({ id: 'prefix', label: '前缀匹配', value: `${text}*` });
  }
  if (
    !FIELD_FILTER_PATTERN.test(text) &&
    !COLUMN_SET_PATTERN.test(text) &&
    !/\b(AND|OR|NOT|NEAR)\b/.test(text)
  ) {
    for (const field of SEARCH_FIELD_OPTIONS.slice(0, 3)) {
      suggestions.push({
        id: `field-${field.column}`,
        label: `只在${field.label}中查找`,
        value: scopeToField(field.column, text),
      });
    }
  }
  if (!/\sNOT\s*$/.test(text)) {
    suggestions.push({ id: 'exclude', label: '排除某个词', value: `${text} NOT ` });
  }
  return suggestions.filter((suggestion) => suggestion.value !== text);
}
