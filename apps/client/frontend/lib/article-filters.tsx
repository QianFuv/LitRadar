/**
 * Shared article month-range parsing, display, and API-bound conversion helpers.
 */

import {
  addMonthsToMonthKey,
  compareMonthKeys,
  currentMonthKey,
  lastDateOfMonthKey,
  monthKey,
  monthKeyYear,
} from '@/lib/date';

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
 * Order two month keys chronologically.
 *
 * @param first - One month endpoint.
 * @param second - The other month endpoint.
 * @returns Earlier and later month.
 */
function orderMonthKeys(first: string, second: string): MonthRange {
  return compareMonthKeys(first, second) <= 0 ? [first, second] : [second, first];
}

/**
 * Check whether a value is a supported YYYY-MM key.
 *
 * @param value - Candidate query value.
 * @returns Whether the value is a valid month key.
 */
function isMonthKey(value: string | null | undefined): value is string {
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
  return orderMonthKeys(startMonth, endMonth);
}

/**
 * Build an ordered compact month-range query value.
 *
 * @param startMonth - First month endpoint.
 * @param endMonth - Second month endpoint.
 * @returns Ordered YYYY-MM..YYYY-MM query value.
 */
export function buildMonthRange(startMonth: string, endMonth: string): string {
  return orderMonthKeys(startMonth, endMonth).join(MONTH_RANGE_SEPARATOR);
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
  const year = monthKeyYear(value);
  if (year < minYear) {
    return monthKey(minYear, 1);
  }
  if (year > maxYear) {
    return monthKey(maxYear, 12);
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
  const fullRange: MonthRange = [monthKey(orderedMinYear, 1), monthKey(orderedMaxYear, 12)];
  const parsedRange = parseMonthRange(value);
  if (!parsedRange) {
    return fullRange;
  }
  const startMonth = clampMonthKeyToYears(parsedRange[0], orderedMinYear, orderedMaxYear);
  const endMonth = clampMonthKeyToYears(parsedRange[1], orderedMinYear, orderedMaxYear);
  return orderMonthKeys(startMonth, endMonth);
}

/**
 * Convert a month key into the first ISO date in that month.
 *
 * @param value - Candidate month key.
 * @returns First-day ISO date, or null when invalid.
 */
function monthKeyToDateFrom(value: string | null | undefined): string | null {
  return isMonthKey(value) ? `${value}-01` : null;
}

/**
 * Convert a month key into the last ISO date in that month.
 *
 * @param value - Candidate month key.
 * @returns Last-day ISO date, or null when invalid.
 */
function monthKeyToDateTo(value: string | null | undefined): string | null {
  return isMonthKey(value) ? lastDateOfMonthKey(value) : null;
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
function formatMonthLabel(value: string): string {
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
 * Build an inclusive recent-year shortcut within available metadata bounds.
 *
 * @param yearCount - Number of trailing twelve-month intervals.
 * @param minYear - Earliest available year.
 * @param maxYear - Latest available year.
 * @param currentMonth - Local month key used when the latest year is current.
 * @returns Clamped recent month range.
 */
export function buildRecentMonthRange(
  yearCount: number,
  minYear: number,
  maxYear: number,
  currentMonth = currentMonthKey(),
): MonthRange {
  const latestYear = Math.max(minYear, maxYear);
  const earliestMonth = monthKey(Math.min(minYear, maxYear), 1);
  const endMonth =
    latestYear === monthKeyYear(currentMonth) ? currentMonth : monthKey(latestYear, 12);
  const monthCount = Math.max(1, Math.floor(yearCount)) * 12;
  const startMonth = addMonthsToMonthKey(endMonth, 1 - monthCount);
  return [compareMonthKeys(startMonth, earliestMonth) < 0 ? earliestMonth : startMonth, endMonth];
}
