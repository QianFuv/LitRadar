/**
 * Date and time calculations on Temporal.
 *
 * Browsers in the compatibility baseline (Chromium 132, Firefox 134, Safari 18.3) predate native
 * Temporal, so it is imported explicitly from `@js-temporal/polyfill`, which does not register a
 * global. Formatting for display stays on `Intl.DateTimeFormat` or `toLocaleString`.
 */

import { Temporal } from '@js-temporal/polyfill';

/** Hours in the fixed-length days used for expiry and dismissal windows. */
const HOURS_PER_DAY = 24;

/** `YYYY-MM-DD` calendar dates, as opposed to timestamps with a time and offset. */
const CALENDAR_DATE_PATTERN = /^\d{4}-\d{2}-\d{2}$/;

/**
 * Read the last millisecond of the current day in the local time zone.
 *
 * @returns Epoch milliseconds one millisecond before the next local midnight.
 */
export function endOfLocalDayEpochMs(): number {
  return Temporal.Now.zonedDateTimeISO().add({ days: 1 }).startOfDay().epochMilliseconds - 1;
}

/**
 * Read the instant a number of 24-hour days from now.
 *
 * @param days - Whole or fractional 24-hour days.
 * @returns Epoch milliseconds.
 */
export function epochMsAfterDays(days: number): number {
  return Temporal.Now.instant().add({ hours: days * HOURS_PER_DAY }).epochMilliseconds;
}

/**
 * Count the started 24-hour days left before an expiry, never below zero.
 *
 * @param epochSeconds - Expiry in Unix seconds.
 * @returns Remaining days rounded up.
 */
export function daysUntilEpochSeconds(epochSeconds: number): number {
  const remaining = Temporal.Instant.fromEpochMilliseconds(Math.round(epochSeconds * 1000)).since(
    Temporal.Now.instant(),
  );
  return Math.max(0, Math.ceil(remaining.total({ unit: 'hours' }) / HOURS_PER_DAY));
}

/**
 * Read the browser's IANA time zone.
 *
 * @returns Time-zone identifier, such as `Asia/Shanghai`.
 */
export function localTimeZoneId(): string {
  return Temporal.Now.timeZoneId();
}

/**
 * Return whether a name is an IANA time zone or offset Temporal can resolve.
 *
 * @param value - Candidate time-zone name.
 * @returns Whether the time zone is valid.
 */
export function isTimeZoneId(value: string): boolean {
  try {
    Temporal.Now.zonedDateTimeISO(value);
    return true;
  } catch {
    return false;
  }
}

/**
 * Build a `YYYY-MM` month key.
 *
 * @param year - Calendar year.
 * @param month - Month from 1 to 12.
 * @returns Month key.
 */
export function monthKey(year: number, month: number): string {
  return Temporal.PlainYearMonth.from({ year, month }).toString();
}

/**
 * Read the year of a month key.
 *
 * @param value - Month key.
 * @returns Calendar year.
 */
export function monthKeyYear(value: string): number {
  return Temporal.PlainYearMonth.from(value).year;
}

/**
 * Order two month keys chronologically.
 *
 * @param first - First month key.
 * @param second - Second month key.
 * @returns Negative, zero, or positive like `Array.prototype.sort` comparators.
 */
export function compareMonthKeys(first: string, second: string): number {
  return Temporal.PlainYearMonth.compare(first, second);
}

/**
 * Read the last calendar date of a month.
 *
 * @param value - Month key.
 * @returns `YYYY-MM-DD` date of the month's last day.
 */
export function lastDateOfMonthKey(value: string): string {
  const yearMonth = Temporal.PlainYearMonth.from(value);
  return yearMonth.toPlainDate({ day: yearMonth.daysInMonth }).toString();
}

/**
 * Move a month key by whole months.
 *
 * @param value - Month key.
 * @param months - Months to add; negative moves back.
 * @returns Shifted month key.
 */
export function addMonthsToMonthKey(value: string, months: number): string {
  return Temporal.PlainYearMonth.from(value).add({ months }).toString();
}

/**
 * Read the current local month as a month key.
 *
 * @returns Month key of today in the local time zone.
 */
export function currentMonthKey(): string {
  return Temporal.Now.plainDateISO().toPlainYearMonth().toString();
}

/**
 * List the start of every whole hour in a UTC window.
 *
 * @param from - Window start, an RFC 3339 timestamp with an offset.
 * @param to - Window end, exclusive.
 * @returns Epoch milliseconds of each hour in `[from, to)`.
 */
export function hourStartsBetween(from: string, to: string): number[] {
  const start = Temporal.Instant.from(from);
  const hourCount = Math.max(0, Math.round(start.until(to).total({ unit: 'hours' })));
  return Array.from(
    { length: hourCount },
    (_, index) => start.add({ hours: index }).epochMilliseconds,
  );
}

/**
 * Read the epoch milliseconds of an RFC 3339 timestamp.
 *
 * @param value - Timestamp with an offset.
 * @returns Epoch milliseconds.
 */
export function timestampEpochMs(value: string): number {
  return Temporal.Instant.from(value).epochMilliseconds;
}

/**
 * Read the UTC calendar date of a calendar date or a timestamp.
 *
 * @param value - `YYYY-MM-DD` date or RFC 3339 timestamp.
 * @returns Calendar date, or null when the value is neither.
 */
export function utcCalendarDate(value: string): Temporal.PlainDate | null {
  try {
    return CALENDAR_DATE_PATTERN.test(value)
      ? Temporal.PlainDate.from(value)
      : Temporal.Instant.from(value).toZonedDateTimeISO('UTC').toPlainDate();
  } catch {
    return null;
  }
}
