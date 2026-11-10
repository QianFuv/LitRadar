/**
 * Pure shaping helpers for the administrator load-telemetry section.
 */

import type {
  TelemetryHour,
  TelemetryKind,
  TelemetryResponse,
  TelemetrySearchShape,
  TelemetrySeries,
} from '@/lib/api/types';

/** Interface kinds in their fixed display order. */
export const TELEMETRY_KINDS = [
  'http',
  'mcp',
  'search',
] as const satisfies readonly TelemetryKind[];

const HOUR_MS = 3_600_000;

/** Search-shape features in display order; hits are rendered per hit-count basis instead. */
export const SEARCH_SHAPE_FEATURES = [
  'endpoint',
  'mode',
  'database',
  'term_count',
  'uses_prefix',
  'uses_boolean',
  'hit_count_basis',
] as const satisfies readonly TelemetrySearchShape['feature'][];

/** Search-shape feature shown as a marginal distribution. */
export type SearchShapeFeature = (typeof SEARCH_SHAPE_FEATURES)[number];

/** Hit-count bases in display order. */
export const HIT_COUNT_BASES = [
  'total',
  'returned',
  'candidates',
] as const satisfies readonly NonNullable<TelemetrySeries['hit_count_basis']>[];

/** Hit-count basis that a hits bucket was measured against. */
export type HitCountBasis = (typeof HIT_COUNT_BASES)[number];

const BUCKET_ORDERS: Readonly<Record<string, readonly string[]>> = {
  hits: ['0', '1-9', '10-99', '100-999', '1000+'],
  term_count: ['0', '1', '2-4', '5-9', '10+'],
  uses_boolean: ['true', 'false'],
  uses_prefix: ['true', 'false'],
  hit_count_basis: HIT_COUNT_BASES,
};

/** One zero-filled hour of calls and errors. */
export type TrendHour = {
  calls: number;
  errors: number;
  /** UTC hour start in epoch milliseconds. */
  hourMs: number;
};

/** Zero-filled hourly trend of one interface kind. */
export type KindTrend = {
  hours: TrendHour[];
  kind: TelemetryKind;
  maxCalls: number;
  totalCalls: number;
  totalErrors: number;
};

/** One value of a call-weighted distribution. */
export type DistributionValue = {
  calls: number;
  value: string;
};

/** Call-weighted marginal distribution of one search-shape feature. */
export type FeatureDistribution = {
  feature: SearchShapeFeature;
  totalCalls: number;
  values: DistributionValue[];
};

/** Call-weighted hits distribution measured against one hit-count basis. */
export type HitsDistribution = {
  basis: HitCountBasis;
  totalCalls: number;
  values: DistributionValue[];
};

/**
 * Sort distribution values by a known bucket order, or by calls for open-ended values.
 *
 * @param feature - Feature whose values are sorted.
 * @param values - Unsorted distribution values.
 * @returns Values in display order.
 */
function sortDistributionValues(feature: string, values: DistributionValue[]): DistributionValue[] {
  const order = BUCKET_ORDERS[feature];
  if (order) {
    const rank = (value: string) => {
      const index = order.indexOf(value);
      return index === -1 ? order.length : index;
    };
    return [...values].sort((first, second) => rank(first.value) - rank(second.value));
  }
  return [...values].sort(
    (first, second) => second.calls - first.calls || first.value.localeCompare(second.value),
  );
}

/**
 * Expand sparse hourly rows into one zero-filled UTC hour series per displayed kind.
 *
 * @param response - Telemetry report whose from/to bound the window.
 * @param kind - Selected kind filter, or null to show every kind.
 * @returns One trend per displayed kind covering every hour in [from, to).
 */
export function buildKindTrends(
  response: Pick<TelemetryResponse, 'from' | 'hourly' | 'to'>,
  kind: TelemetryKind | null,
): KindTrend[] {
  const fromMs = Date.parse(response.from);
  const toMs = Date.parse(response.to);
  const hourCount = Math.max(0, Math.round((toMs - fromMs) / HOUR_MS));
  const observed = new Map<string, TelemetryHour>();
  for (const row of response.hourly) {
    observed.set(`${row.kind}|${Date.parse(row.hour)}`, row);
  }
  const kinds = kind ? [kind] : TELEMETRY_KINDS;
  return kinds.map((trendKind) => {
    const hours: TrendHour[] = [];
    for (let index = 0; index < hourCount; index += 1) {
      const hourMs = fromMs + index * HOUR_MS;
      const row = observed.get(`${trendKind}|${hourMs}`);
      hours.push({ calls: row?.calls ?? 0, errors: row?.errors ?? 0, hourMs });
    }
    return {
      hours,
      kind: trendKind,
      maxCalls: hours.reduce((max, hour) => Math.max(max, hour.calls), 0),
      totalCalls: hours.reduce((sum, hour) => sum + hour.calls, 0),
      totalErrors: hours.reduce((sum, hour) => sum + hour.errors, 0),
    };
  });
}

/**
 * Group marginal search-shape rows by feature in display order.
 *
 * @param shapes - Server marginal distributions.
 * @returns Non-empty feature distributions, excluding hits which depend on their basis.
 */
export function groupSearchShapes(shapes: TelemetrySearchShape[]): FeatureDistribution[] {
  return SEARCH_SHAPE_FEATURES.flatMap((feature) => {
    const values = shapes
      .filter((shape) => shape.feature === feature)
      .map(({ calls, value }) => ({ calls, value }));
    if (values.length === 0) {
      return [];
    }
    return [
      {
        feature,
        totalCalls: values.reduce((sum, value) => sum + value.calls, 0),
        values: sortDistributionValues(feature, values),
      },
    ];
  });
}

/**
 * Split the search hits distribution by hit-count basis so different populations never merge.
 *
 * @param series - Telemetry series; only search series with a basis contribute.
 * @returns Non-empty hits distributions per basis in display order.
 */
export function buildHitsByBasis(series: TelemetrySeries[]): HitsDistribution[] {
  return HIT_COUNT_BASES.flatMap((basis) => {
    const callsByBucket = new Map<string, number>();
    for (const row of series) {
      if (row.kind === 'search' && row.hit_count_basis === basis && row.hits) {
        callsByBucket.set(row.hits, (callsByBucket.get(row.hits) ?? 0) + row.calls);
      }
    }
    if (callsByBucket.size === 0) {
      return [];
    }
    const values = [...callsByBucket].map(([value, calls]) => ({ calls, value }));
    return [
      {
        basis,
        totalCalls: values.reduce((sum, value) => sum + value.calls, 0),
        values: sortDistributionValues('hits', values),
      },
    ];
  });
}
