/**
 * Shared article month-range parsing, display, and API-bound conversion helpers.
 */

const MONTH_KEY_PATTERN = /^\d{4}-(0[1-9]|1[0-2])$/;
const MONTH_RANGE_SEPARATOR = '..';

export const MONTH_OPTIONS = [
  '01',
  '02',
  '03',
  '04',
  '05',
  '06',
  '07',
  '08',
  '09',
  '10',
  '11',
  '12',
] as const;

export type MonthRange = [string, string];

export type YearBounds = {
  max: number;
  min: number;
};

/**
 * Build a stable YYYY-MM key.
 *
 * @param year - Four-digit year.
 * @param month - One-based month number.
 * @returns Month key.
 */
export function buildMonthKey(year: number, month: number): string {
  return `${year}-${String(month).padStart(2, '0')}`;
}

/**
 * Check whether a value is a supported YYYY-MM key.
 *
 * @param value - Candidate query value.
 * @returns Whether the value is a valid month key.
 */
export function isMonthKey(value: string | null | undefined): value is string {
  return typeof value === 'string' && MONTH_KEY_PATTERN.test(value);
}

/**
 * Parse and order a compact month range.
 *
 * @param value - Query value in YYYY-MM..YYYY-MM format.
 * @returns Ordered month keys, or null when either endpoint is invalid.
 */
export function parseMonthRange(value: string | null | undefined): MonthRange | null {
  const [startMonth = '', endMonth = '', ...extraParts] = (value ?? '').split(
    MONTH_RANGE_SEPARATOR,
  );
  if (extraParts.length > 0 || !isMonthKey(startMonth) || !isMonthKey(endMonth)) {
    return null;
  }
  return startMonth <= endMonth ? [startMonth, endMonth] : [endMonth, startMonth];
}

/**
 * Build an ordered compact month-range query value.
 *
 * @param startMonth - First month endpoint.
 * @param endMonth - Second month endpoint.
 * @returns Ordered YYYY-MM..YYYY-MM query value.
 */
export function buildMonthRange(startMonth: string, endMonth: string): string {
  const orderedRange = startMonth <= endMonth ? [startMonth, endMonth] : [endMonth, startMonth];
  return orderedRange.join(MONTH_RANGE_SEPARATOR);
}

/**
 * Clamp a valid month key to an available year interval.
 *
 * @param value - Valid month key.
 * @param minYear - Earliest available year.
 * @param maxYear - Latest available year.
 * @returns Original or boundary month key.
 */
function clampMonthKeyToYears(value: string, minYear: number, maxYear: number): string {
  const year = Number(value.slice(0, 4));
  if (year < minYear) {
    return buildMonthKey(minYear, 1);
  }
  if (year > maxYear) {
    return buildMonthKey(maxYear, 12);
  }
  return value;
}

/**
 * Resolve a raw range into an available year interval or its full-range default.
 *
 * @param value - Raw compact month range.
 * @param minYear - Earliest available year.
 * @param maxYear - Latest available year.
 * @returns Ordered, clamped month endpoints.
 */
export function resolveMonthRangeForYears(
  value: string | null | undefined,
  minYear: number,
  maxYear: number,
): MonthRange {
  const orderedMinYear = Math.min(minYear, maxYear);
  const orderedMaxYear = Math.max(minYear, maxYear);
  const fullRange: MonthRange = [
    buildMonthKey(orderedMinYear, 1),
    buildMonthKey(orderedMaxYear, 12),
  ];
  const parsedRange = parseMonthRange(value);
  if (!parsedRange) {
    return fullRange;
  }
  const startMonth = clampMonthKeyToYears(parsedRange[0], orderedMinYear, orderedMaxYear);
  const endMonth = clampMonthKeyToYears(parsedRange[1], orderedMinYear, orderedMaxYear);
  return startMonth <= endMonth ? [startMonth, endMonth] : [endMonth, startMonth];
}

/**
 * Convert a month key into the first ISO date in that month.
 *
 * @param value - Candidate month key.
 * @returns First-day ISO date, or null when invalid.
 */
export function monthKeyToDateFrom(value: string | null | undefined): string | null {
  return isMonthKey(value) ? `${value}-01` : null;
}

/**
 * Convert a month key into the last ISO date in that month.
 *
 * @param value - Candidate month key.
 * @returns Last-day ISO date, or null when invalid.
 */
export function monthKeyToDateTo(value: string | null | undefined): string | null {
  if (!isMonthKey(value)) {
    return null;
  }
  const year = Number(value.slice(0, 4));
  const month = Number(value.slice(5, 7));
  const lastDay = new Date(year, month, 0).getDate();
  return `${value}-${String(lastDay).padStart(2, '0')}`;
}

/**
 * Convert a compact month range into API date boundaries.
 *
 * @param value - Compact month-range query value.
 * @returns Inclusive date boundaries, or null when the range is invalid.
 */
export function getMonthRangeDateBounds(
  value: string | null | undefined,
): { dateFrom: string; dateTo: string } | null {
  const range = parseMonthRange(value);
  if (!range) {
    return null;
  }
  const dateFrom = monthKeyToDateFrom(range[0]);
  const dateTo = monthKeyToDateTo(range[1]);
  return dateFrom && dateTo ? { dateFrom, dateTo } : null;
}

/**
 * Format one month key for Chinese filter UI.
 *
 * @param value - Valid month key.
 * @returns Human-readable year-month label.
 */
export function formatMonthLabel(value: string): string {
  return `${value.slice(0, 4)}年${value.slice(5, 7)}月`;
}

/**
 * Format a compact month range for filter feedback.
 *
 * @param value - Compact month-range query value.
 * @returns Human-readable range, or null when invalid.
 */
export function formatMonthRangeLabel(value: string | null | undefined): string | null {
  const range = parseMonthRange(value);
  return range ? `${formatMonthLabel(range[0])} - ${formatMonthLabel(range[1])}` : null;
}

/**
 * Derive finite minimum and maximum years from metadata.
 *
 * @param values - Metadata records containing year values.
 * @returns Year bounds, or null for an empty or invalid collection.
 */
export function getYearBounds(values: readonly { year: number }[]): YearBounds | null {
  const years = values.map((item) => item.year).filter((year) => Number.isInteger(year));
  if (years.length === 0) {
    return null;
  }
  return {
    max: Math.max(...years),
    min: Math.min(...years),
  };
}

/**
 * Build descending year labels for a bounded date selector.
 *
 * @param bounds - Available year interval.
 * @returns Descending year labels.
 */
export function buildYearOptions(bounds: YearBounds): string[] {
  const result: string[] = [];
  for (let year = bounds.max; year >= bounds.min; year -= 1) {
    result.push(String(year));
  }
  return result;
}

/**
 * Convert a zero-based absolute month index into a month key.
 *
 * @param value - Absolute month index.
 * @returns Month key.
 */
function monthIndexToKey(value: number): string {
  const year = Math.floor(value / 12);
  const month = (value % 12) + 1;
  return buildMonthKey(year, month);
}

/**
 * Build an inclusive recent-year shortcut within available metadata bounds.
 *
 * @param yearCount - Number of trailing twelve-month intervals.
 * @param minYear - Earliest available year.
 * @param maxYear - Latest available year.
 * @param currentDate - Clock value used when the latest year is current.
 * @returns Clamped recent month range.
 */
export function buildRecentMonthRange(
  yearCount: number,
  minYear: number,
  maxYear: number,
  currentDate = new Date(),
): MonthRange {
  const bounds = {
    max: Math.max(minYear, maxYear),
    min: Math.min(minYear, maxYear),
  };
  const endMonth = bounds.max === currentDate.getFullYear() ? currentDate.getMonth() + 1 : 12;
  const endIndex = bounds.max * 12 + endMonth - 1;
  const minIndex = bounds.min * 12;
  const monthCount = Math.max(1, Math.floor(yearCount)) * 12;
  const startIndex = Math.max(minIndex, endIndex - monthCount + 1);
  return [monthIndexToKey(startIndex), monthIndexToKey(endIndex)];
}
