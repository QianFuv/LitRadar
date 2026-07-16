/**
 * Parse FTS query operands for safe visual highlighting without rewriting the query.
 */

/**
 * Describes how one extracted FTS operand should match article text.
 */
export type FtsHighlightTerm = Readonly<{
  value: string;
  matchMode: 'exact' | 'prefix';
}>;

/**
 * Represents a lightweight token from the FTS query syntax.
 */
type FtsQueryToken = Readonly<{
  kind: 'word' | 'phrase' | 'symbol';
  value: string;
}>;

/**
 * Tracks whether a parenthesized group belongs to a NEAR expression.
 */
type ParenthesisFrame = {
  isNear: boolean;
  isAfterComma: boolean;
};

/** Recognized boolean and proximity operators. */
const FTS_OPERATORS = new Set(['AND', 'OR', 'NOT', 'NEAR']);

/** Syntax characters that affect operand extraction. */
const FTS_SYMBOLS = new Set(['(', ')', '{', '}', ':', ',', '*']);

/** Finds the next Unicode word token at a fixed query offset. */
const WORD_TOKEN_PATTERN = /[\p{L}\p{N}_]+/uy;

/** Identifies CJK values that retain substring highlight behavior. */
const CJK_PATTERN = /[\u3400-\u9fff]/u;

/** Identifies one Unicode word character for boundary construction. */
const WORD_CHARACTER_PATTERN = /^[\p{L}\p{N}_]$/u;

/** Unicode word class used inside generated regular expression lookarounds. */
const WORD_CHARACTER_CLASS = '[\\p{L}\\p{N}_]';

/** Escapes regular expression syntax in an extracted operand. */
const REGEX_SPECIAL_CHARACTER_PATTERN = /[.*+?^${}()|[\]\\]/g;

/**
 * Read a quoted phrase, including FTS doubled-quote escapes.
 *
 * @param query - Complete FTS query.
 * @param startIndex - Offset of the opening quote.
 * @returns Phrase token and the offset immediately after its consumed text.
 */
function readQuotedPhrase(
  query: string,
  startIndex: number,
): { token: FtsQueryToken; nextIndex: number } {
  let cursor = startIndex + 1;
  let value = '';

  while (cursor < query.length) {
    if (query[cursor] !== '"') {
      value += query[cursor];
      cursor += 1;
      continue;
    }

    if (query[cursor + 1] === '"') {
      value += '"';
      cursor += 2;
      continue;
    }

    return {
      token: { kind: 'phrase', value },
      nextIndex: cursor + 1,
    };
  }

  return {
    token: { kind: 'phrase', value },
    nextIndex: cursor,
  };
}

/**
 * Tokenize only the FTS syntax needed to identify visual highlight operands.
 *
 * @param query - FTS query submitted to article search.
 * @returns Ordered word, phrase, and syntax tokens.
 */
function tokenizeFtsQuery(query: string): FtsQueryToken[] {
  const tokens: FtsQueryToken[] = [];
  let cursor = 0;

  while (cursor < query.length) {
    const character = query[cursor];
    if (/\s/u.test(character)) {
      cursor += 1;
      continue;
    }

    if (character === '"') {
      const phrase = readQuotedPhrase(query, cursor);
      tokens.push(phrase.token);
      cursor = phrase.nextIndex;
      continue;
    }

    WORD_TOKEN_PATTERN.lastIndex = cursor;
    const wordMatch = WORD_TOKEN_PATTERN.exec(query);
    if (wordMatch) {
      tokens.push({ kind: 'word', value: wordMatch[0] });
      cursor = WORD_TOKEN_PATTERN.lastIndex;
      continue;
    }

    if (FTS_SYMBOLS.has(character)) {
      tokens.push({ kind: 'symbol', value: character });
    }
    cursor += 1;
  }

  return tokens;
}

/**
 * Normalize whitespace inside a quoted or bare operand.
 *
 * @param value - Raw token value.
 * @returns Trimmed value with internal whitespace collapsed.
 */
function normalizeHighlightValue(value: string): string {
  return value.trim().replace(/\s+/gu, ' ');
}

/**
 * Check whether an operand is long enough to highlight.
 *
 * @param value - Normalized operand value.
 * @returns True when the value meets the CJK-aware minimum length.
 */
function meetsHighlightLength(value: string): boolean {
  return CJK_PATTERN.test(value) ? value.length >= 2 : value.length > 2;
}

/**
 * Check whether a numeric token is the distance argument of an active NEAR group.
 *
 * @param token - Candidate query token.
 * @param frames - Active parenthesis frames.
 * @returns True when the token is syntax rather than a searchable operand.
 */
function isNearDistance(token: FtsQueryToken, frames: readonly ParenthesisFrame[]): boolean {
  if (token.kind !== 'word' || !/^\d+$/u.test(token.value)) {
    return false;
  }

  for (let index = frames.length - 1; index >= 0; index -= 1) {
    if (frames[index].isNear) {
      return frames[index].isAfterComma;
    }
  }

  return false;
}

/**
 * Mark the closest active NEAR group as having reached its distance separator.
 *
 * @param frames - Active mutable parenthesis frames.
 */
function markNearDistanceSeparator(frames: ParenthesisFrame[]): void {
  for (let index = frames.length - 1; index >= 0; index -= 1) {
    if (frames[index].isNear) {
      frames[index].isAfterComma = true;
      return;
    }
  }
}

/**
 * Extract operands that may be highlighted from an FTS query.
 *
 * The original query is never rewritten or interpreted as an executable search expression.
 *
 * @param query - FTS query submitted to article search.
 * @returns Ordered, case-insensitively deduplicated highlight terms.
 */
export function parseFtsHighlightTerms(query: string | null | undefined): FtsHighlightTerm[] {
  if (!query?.trim()) {
    return [];
  }

  const tokens = tokenizeFtsQuery(query);
  const terms: FtsHighlightTerm[] = [];
  const seenTerms = new Set<string>();
  const parenthesisFrames: ParenthesisFrame[] = [];
  let braceDepth = 0;

  for (let index = 0; index < tokens.length; index += 1) {
    const token = tokens[index];

    if (token.kind === 'symbol') {
      if (token.value === '{') {
        braceDepth += 1;
      } else if (token.value === '}') {
        braceDepth = Math.max(0, braceDepth - 1);
      } else if (token.value === '(') {
        const previousToken = tokens[index - 1];
        parenthesisFrames.push({
          isNear: previousToken?.kind === 'word' && previousToken.value.toUpperCase() === 'NEAR',
          isAfterComma: false,
        });
      } else if (token.value === ')') {
        parenthesisFrames.pop();
      } else if (token.value === ',') {
        markNearDistanceSeparator(parenthesisFrames);
      }
      continue;
    }

    if (braceDepth > 0) {
      continue;
    }

    if (token.kind === 'word' && FTS_OPERATORS.has(token.value.toUpperCase())) {
      continue;
    }

    if (tokens[index + 1]?.kind === 'symbol' && tokens[index + 1]?.value === ':') {
      continue;
    }

    if (isNearDistance(token, parenthesisFrames)) {
      continue;
    }

    const value = normalizeHighlightValue(token.value);
    if (!meetsHighlightLength(value)) {
      continue;
    }

    const matchMode =
      tokens[index + 1]?.kind === 'symbol' && tokens[index + 1]?.value === '*' ? 'prefix' : 'exact';
    const deduplicationKey = `${matchMode}:${value.toLowerCase()}`;
    if (seenTerms.has(deduplicationKey)) {
      continue;
    }

    seenTerms.add(deduplicationKey);
    terms.push({ value, matchMode });
  }

  return terms;
}

/**
 * Escape an operand while allowing flexible whitespace inside phrases.
 *
 * @param value - Normalized operand value.
 * @returns Safe regular expression source.
 */
function escapeHighlightValue(value: string): string {
  const parts = value.split(/\s+/u);
  const escapedParts: string[] = [];
  for (const part of parts) {
    escapedParts.push(part.replace(REGEX_SPECIAL_CHARACTER_PATTERN, '\\$&'));
  }
  return escapedParts.join('\\s+');
}

/**
 * Build one exact or prefix alternative with Unicode-aware token boundaries.
 *
 * @param term - Extracted highlight term.
 * @returns Regular expression source for the term.
 */
function buildHighlightAlternative(term: FtsHighlightTerm): string {
  const escapedValue = escapeHighlightValue(term.value);
  if (CJK_PATTERN.test(term.value)) {
    return escapedValue;
  }

  const characters = Array.from(term.value);
  const hasLeadingBoundary = WORD_CHARACTER_PATTERN.test(characters[0] ?? '');
  const hasTrailingBoundary = WORD_CHARACTER_PATTERN.test(characters.at(-1) ?? '');
  const prefix = hasLeadingBoundary ? `(?<!${WORD_CHARACTER_CLASS})` : '';
  const suffix =
    term.matchMode === 'exact' && hasTrailingBoundary ? `(?!${WORD_CHARACTER_CLASS})` : '';
  return `${prefix}${escapedValue}${suffix}`;
}

/**
 * Order longer operands before overlapping shorter operands.
 *
 * @param left - First extracted term.
 * @param right - Second extracted term.
 * @returns Standard array sort comparison value.
 */
function compareHighlightTerms(left: FtsHighlightTerm, right: FtsHighlightTerm): number {
  const lengthDifference = right.value.length - left.value.length;
  if (lengthDifference !== 0) {
    return lengthDifference;
  }
  if (left.matchMode !== right.matchMode) {
    return left.matchMode === 'prefix' ? -1 : 1;
  }
  const leftValue = left.value.toLowerCase();
  const rightValue = right.value.toLowerCase();
  if (leftValue === rightValue) {
    return 0;
  }
  return leftValue < rightValue ? -1 : 1;
}

/**
 * Build the matcher used to split article text into highlighted and plain segments.
 *
 * @param terms - Terms returned by the highlight parser.
 * @returns A case-insensitive Unicode matcher, or null when construction is not possible.
 */
export function createFtsHighlightPattern(terms: readonly FtsHighlightTerm[]): RegExp | null {
  const usableTerms = terms.filter((term) => term.value.length > 0).sort(compareHighlightTerms);
  if (usableTerms.length === 0) {
    return null;
  }

  try {
    const alternatives = usableTerms.map(buildHighlightAlternative);
    return new RegExp(`(${alternatives.join('|')})`, 'giu');
  } catch {
    return null;
  }
}
