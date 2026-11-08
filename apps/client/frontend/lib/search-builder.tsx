/**
 * Structured advanced-search builder: conditions, FTS5 expression generation, and session state.
 *
 * Users only type plain words. Every word is quoted, embedded quotes are doubled, and every
 * condition is parenthesized, so the generated expression is always valid FTS5 syntax.
 */

/** Field a condition searches. `all` searches every indexed text column. */
export type SearchBuilderField =
  | 'all'
  | 'title'
  | 'abstract_text'
  | 'authors'
  | 'journal_title'
  | 'doi';

/** How the words of one condition must match. */
export type SearchBuilderMatch = 'all' | 'phrase' | 'any' | 'prefix';

/** How a condition joins the conditions before it. */
export type SearchBuilderConnector = 'AND' | 'OR' | 'NOT';

/** One builder row. */
export type SearchBuilderCondition = {
  id: string;
  connector: SearchBuilderConnector;
  field: SearchBuilderField;
  match: SearchBuilderMatch;
  value: string;
};

/** Selectable field labels in display order. */
export const SEARCH_BUILDER_FIELDS: readonly { value: SearchBuilderField; label: string }[] = [
  { value: 'all', label: '全部字段' },
  { value: 'title', label: '标题' },
  { value: 'abstract_text', label: '摘要' },
  { value: 'authors', label: '作者' },
  { value: 'journal_title', label: '期刊' },
  { value: 'doi', label: 'DOI' },
];

/** Selectable match labels in display order. */
export const SEARCH_BUILDER_MATCHES: readonly { value: SearchBuilderMatch; label: string }[] = [
  { value: 'all', label: '包含全部词' },
  { value: 'phrase', label: '精确短语' },
  { value: 'any', label: '包含任一词' },
  { value: 'prefix', label: '前缀匹配' },
];

/** Selectable connector labels in display order. */
export const SEARCH_BUILDER_CONNECTORS: readonly {
  value: SearchBuilderConnector;
  label: string;
}[] = [
  { value: 'AND', label: '且' },
  { value: 'OR', label: '或' },
  { value: 'NOT', label: '不含' },
];

/** Maximum number of builder rows. */
export const SEARCH_BUILDER_MAX_CONDITIONS = 8;

const SEARCH_BUILDER_STORAGE_KEY = 'litradar:v1:search_builder';

let nextConditionId = 0;

/**
 * Create a builder row.
 *
 * @param overrides - Optional initial values.
 * @returns New condition with a unique id.
 */
export function createSearchCondition(
  overrides: Partial<Omit<SearchBuilderCondition, 'id'>> = {},
): SearchBuilderCondition {
  nextConditionId += 1;
  return {
    id: `condition-${Date.now().toString(36)}-${nextConditionId}`,
    connector: 'AND',
    field: 'all',
    match: 'all',
    value: '',
    ...overrides,
  };
}

/**
 * Split condition text into words.
 *
 * @param value - Raw condition text.
 * @returns Non-empty words.
 */
function splitWords(value: string): string[] {
  return value.trim().split(/\s+/).filter(Boolean);
}

/**
 * Quote text as one FTS5 string, doubling embedded quotes.
 *
 * @param text - Literal text.
 * @returns Quoted FTS5 string.
 */
function quote(text: string): string {
  return `"${text.replaceAll('"', '""')}"`;
}

/**
 * Build the FTS5 expression for one condition without its connector.
 *
 * @param condition - Builder row.
 * @returns Expression, or null when the row has no words.
 */
export function buildConditionExpression(condition: SearchBuilderCondition): string | null {
  const words = splitWords(condition.value);
  if (words.length === 0) {
    return null;
  }
  let expression: string;
  if (condition.match === 'phrase') {
    expression = quote(words.join(' '));
  } else if (condition.match === 'prefix') {
    expression = words.map((word) => `${quote(word)} *`).join(' AND ');
  } else {
    expression = words.map(quote).join(condition.match === 'any' ? ' OR ' : ' AND ');
  }
  if (condition.field !== 'all') {
    return `${condition.field} : (${expression})`;
  }
  return words.length > 1 && condition.match !== 'phrase' ? `(${expression})` : expression;
}

/** Result of compiling the builder rows. */
export type SearchBuilderResult = {
  query: string;
  activeCount: number;
  error: string | null;
};

/**
 * Compile builder rows into one valid FTS5 expression.
 *
 * Rows without words are ignored. Because FTS5 has no unary NOT, the first row with words must
 * not be an exclusion; it is reported instead of silently turned into an inclusion.
 *
 * @param conditions - Builder rows in display order.
 * @returns Expression, number of contributing rows, and an optional validation error.
 */
export function compileSearchConditions(
  conditions: readonly SearchBuilderCondition[],
): SearchBuilderResult {
  let query = '';
  let activeCount = 0;
  for (const condition of conditions) {
    const expression = buildConditionExpression(condition);
    if (!expression) continue;
    if (activeCount === 0) {
      if (condition.connector === 'NOT') {
        return { query: '', activeCount: 0, error: '“不含”条件前至少需要一个包含条件。' };
      }
      query = expression;
    } else {
      const left = activeCount > 1 ? `(${query})` : query;
      query = `${left} ${condition.connector} ${expression}`;
    }
    activeCount += 1;
  }
  return { query, activeCount, error: activeCount === 0 ? '请至少填写一个检索词。' : null };
}

/** One piece of a natural-language condition description. */
type SearchConditionSegment = {
  text: string;
  /** True for a user-entered search term, rendered emphasized and wrapped in corner quotes. */
  isTerm: boolean;
};

const FIELD_PHRASES: Record<SearchBuilderField, string> = {
  all: '任意字段',
  title: '标题',
  abstract_text: '摘要',
  authors: '作者',
  journal_title: '期刊',
  doi: 'DOI',
};

const CONNECTOR_PHRASES: Record<SearchBuilderConnector, string> = {
  AND: '并且',
  OR: '或者',
  NOT: '排除',
};

/**
 * Join terms with a word between each pair.
 *
 * @param words - Search terms.
 * @param separator - Word between terms.
 * @returns Alternating term and separator segments.
 */
function joinTerms(words: readonly string[], separator: string): SearchConditionSegment[] {
  return words.flatMap((word, index) => [
    ...(index > 0 ? [{ text: separator, isTerm: false }] : []),
    { text: word, isTerm: true },
  ]);
}

/**
 * Describe one condition as a short Chinese sentence split into segments.
 *
 * @param condition - Builder row.
 * @returns Segments such as 标题 / 含短语 / 「graph neural」.
 */
function describeSearchConditionParts(condition: SearchBuilderCondition): SearchConditionSegment[] {
  const words = splitWords(condition.value);
  const field = { text: FIELD_PHRASES[condition.field], isTerm: false };
  if (condition.match === 'phrase') {
    return [field, { text: '含短语', isTerm: false }, { text: words.join(' '), isTerm: true }];
  }
  if (condition.match === 'prefix') {
    return [
      field,
      { text: '含以', isTerm: false },
      ...joinTerms(words, '、'),
      { text: '开头的词', isTerm: false },
    ];
  }
  if (condition.match === 'any') {
    return [field, { text: '含', isTerm: false }, ...joinTerms(words, '或')];
  }
  return [
    field,
    { text: words.length > 1 ? '同时含' : '含', isTerm: false },
    ...joinTerms(words, '和'),
  ];
}

/**
 * Describe one condition as plain text for chips and accessible names.
 *
 * @param condition - Builder row.
 * @returns Sentence such as 标题含短语「graph neural」.
 */
export function describeSearchCondition(condition: SearchBuilderCondition): string {
  return describeSearchConditionParts(condition)
    .map((segment) => (segment.isTerm ? `「${segment.text}」` : segment.text))
    .join('');
}

/**
 * Describe how a condition joins the ones before it.
 *
 * @param connector - Connector value.
 * @returns 并且, 或者, or 排除.
 */
export function describeSearchConnector(connector: SearchBuilderConnector): string {
  return CONNECTOR_PHRASES[connector];
}

/** Builder rows remembered together with the query they produced. */
export type StoredSearchBuilder = {
  query: string;
  conditions: SearchBuilderCondition[];
};

/**
 * Check whether a parsed value is a valid stored builder snapshot.
 *
 * @param value - Parsed JSON.
 * @returns True when the snapshot is usable.
 */
function isStoredSearchBuilder(value: unknown): value is StoredSearchBuilder {
  if (!value || typeof value !== 'object') return false;
  const candidate = value as Partial<StoredSearchBuilder>;
  return (
    typeof candidate.query === 'string' &&
    Array.isArray(candidate.conditions) &&
    candidate.conditions.every(
      (condition) =>
        condition &&
        typeof condition.id === 'string' &&
        typeof condition.value === 'string' &&
        SEARCH_BUILDER_FIELDS.some((field) => field.value === condition.field) &&
        SEARCH_BUILDER_MATCHES.some((match) => match.value === condition.match) &&
        SEARCH_BUILDER_CONNECTORS.some((connector) => connector.value === condition.connector),
    )
  );
}

/**
 * Read the builder rows that produced the current query in this tab.
 *
 * @returns Stored snapshot, or null when absent or invalid.
 */
export function readStoredSearchBuilder(): StoredSearchBuilder | null {
  if (typeof window === 'undefined') return null;
  try {
    const raw = window.sessionStorage.getItem(SEARCH_BUILDER_STORAGE_KEY);
    if (!raw) return null;
    const parsed: unknown = JSON.parse(raw);
    return isStoredSearchBuilder(parsed) ? parsed : null;
  } catch {
    return null;
  }
}

/**
 * Remember the builder rows for the query they produced.
 *
 * @param snapshot - Query and rows.
 */
export function writeStoredSearchBuilder(snapshot: StoredSearchBuilder): void {
  if (typeof window === 'undefined') return;
  try {
    window.sessionStorage.setItem(SEARCH_BUILDER_STORAGE_KEY, JSON.stringify(snapshot));
  } catch {}
}
