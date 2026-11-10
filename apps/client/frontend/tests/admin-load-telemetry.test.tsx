/**
 * Administrator load-telemetry section: query parameters, report rendering, and explicit states.
 */

import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { http, HttpResponse } from 'msw';
import { describe, expect, test } from 'vitest';

import { LoadTelemetryCard } from '@/components/admin/load-telemetry-card';
import type { TelemetryResponse, TelemetrySeries } from '@/lib/api';
import { buildHitsByBasis, buildKindTrends, groupSearchShapes } from '@/lib/load-telemetry';
import { server } from '@/tests/mocks/server';
import { renderWithQuery } from '@/tests/render';

const TELEMETRY_URL = 'http://localhost/api/admin/telemetry';

/**
 * Build one search series with defaults for the flattened search dimensions.
 *
 * @param overrides - Fields that differ from the default search series.
 * @returns Search telemetry series.
 */
function createSearchSeries(overrides: Partial<TelemetrySeries>): TelemetrySeries {
  return {
    kind: 'search',
    name: 'articles',
    is_error: false,
    mode: 'advanced',
    database: 'catalog.sqlite',
    term_count: '2-4',
    uses_prefix: true,
    uses_boolean: true,
    hits: '10-99',
    hit_count_basis: 'total',
    calls: 25,
    errors: 0,
    error_rate: 0,
    p50_ms: 8.192,
    p95_ms: 16.384,
    p99_ms: 32.768,
    ...overrides,
  };
}

/**
 * Build a three-hour report with HTTP, MCP and search load.
 *
 * @returns Enabled telemetry report.
 */
function createReport(): TelemetryResponse {
  return {
    enabled: true,
    from: '2026-11-10T09:00:00Z',
    to: '2026-11-10T12:00:00Z',
    series: [
      {
        kind: 'http',
        name: '/api/articles',
        method: 'GET',
        status_class: '2xx',
        is_error: false,
        uses_prefix: false,
        uses_boolean: false,
        calls: 40,
        errors: 0,
        error_rate: 0,
        p50_ms: 4.096,
        p95_ms: 8.192,
        p99_ms: 16.384,
      },
      createSearchSeries({}),
      createSearchSeries({ hits: '0', hit_count_basis: 'returned', calls: 5 }),
      {
        kind: 'mcp',
        name: 'search_articles',
        is_error: true,
        uses_prefix: false,
        uses_boolean: false,
        calls: 4,
        errors: 4,
        error_rate: 1,
        p50_ms: 131.072,
        p95_ms: 262.144,
        p99_ms: 262.144,
      },
    ],
    hourly: [
      { hour: '2026-11-10T09:00:00Z', kind: 'http', calls: 10, errors: 0 },
      { hour: '2026-11-10T11:00:00Z', kind: 'http', calls: 30, errors: 0 },
      { hour: '2026-11-10T11:00:00Z', kind: 'mcp', calls: 4, errors: 4 },
      { hour: '2026-11-10T11:00:00Z', kind: 'search', calls: 30, errors: 0 },
    ],
    search_shapes: [
      { feature: 'database', value: 'catalog.sqlite', calls: 30 },
      { feature: 'endpoint', value: 'articles', calls: 30 },
      { feature: 'hit_count_basis', value: 'returned', calls: 5 },
      { feature: 'hit_count_basis', value: 'total', calls: 25 },
      { feature: 'hits', value: '0', calls: 5 },
      { feature: 'hits', value: '10-99', calls: 25 },
      { feature: 'mode', value: 'advanced', calls: 30 },
      { feature: 'term_count', value: '2-4', calls: 30 },
      { feature: 'uses_boolean', value: 'true', calls: 30 },
      { feature: 'uses_prefix', value: 'true', calls: 30 },
    ],
  };
}

/**
 * Install a telemetry handler that records each query string.
 *
 * @param respond - Response factory.
 * @returns Recorded `period` and `kind` query values in request order.
 */
function installTelemetryHandler(respond: () => Response): string[] {
  const queries: string[] = [];
  server.use(
    http.get(TELEMETRY_URL, ({ request }) => {
      queries.push(new URL(request.url).search);
      return respond();
    }),
  );
  return queries;
}

/** Verify missing UTC hours are zero-filled per kind across the server window. */
function zeroFillsMissingHours(): void {
  const trends = buildKindTrends(createReport(), null);
  expect(trends.map((trend) => trend.kind)).toEqual(['http', 'mcp', 'search']);
  expect(trends[0].hours.map((hour) => hour.calls)).toEqual([10, 0, 30]);
  expect(trends[0].hours.map((hour) => new Date(hour.hourMs).toISOString())).toEqual([
    '2026-11-10T09:00:00.000Z',
    '2026-11-10T10:00:00.000Z',
    '2026-11-10T11:00:00.000Z',
  ]);
  expect(trends[1]).toMatchObject({ maxCalls: 4, totalCalls: 4, totalErrors: 4 });
  expect(buildKindTrends({ ...createReport(), hourly: [] }, 'search')).toEqual([
    expect.objectContaining({ kind: 'search', maxCalls: 0, totalCalls: 0 }),
  ]);
  expect(buildKindTrends({ ...createReport(), hourly: [] }, 'search')[0].hours).toHaveLength(3);
}

/** Verify hit buckets stay split by basis while other features keep their marginal totals. */
function keepsHitBasesSeparate(): void {
  const report = createReport();
  expect(buildHitsByBasis(report.series)).toEqual([
    { basis: 'total', totalCalls: 25, values: [{ calls: 25, value: '10-99' }] },
    { basis: 'returned', totalCalls: 5, values: [{ calls: 5, value: '0' }] },
  ]);
  const features = groupSearchShapes(report.search_shapes);
  expect(features.map((feature) => feature.feature)).toEqual([
    'endpoint',
    'mode',
    'database',
    'term_count',
    'uses_prefix',
    'uses_boolean',
    'hit_count_basis',
  ]);
  expect(features.at(-1)?.values.map((value) => value.value)).toEqual(['total', 'returned']);
}

/** Verify the report renders identities, approximate percentiles, trends and shapes. */
async function rendersReportAndRequestsFilters(): Promise<void> {
  const queries = installTelemetryHandler(() => HttpResponse.json(createReport()));
  const user = userEvent.setup();
  renderWithQuery(<LoadTelemetryCard isEnabled />);

  const table = await screen.findByRole('table', { name: /调用排行/ });
  expect(queries).toEqual(['?period=24h']);
  const rows = within(table).getAllByRole('row');
  expect(rows).toHaveLength(5);
  expect(rows[1]).toHaveTextContent('/api/articles');
  expect(rows[1]).toHaveTextContent('GET');
  expect(rows[1]).toHaveTextContent('2xx');
  expect(rows[4]).toHaveTextContent('search_articles');
  expect(rows[4]).toHaveTextContent('失败');
  expect(rows[4]).toHaveTextContent('100%');
  expect(rows[2]).toHaveTextContent('命中 10-99（总命中数）');
  expect(rows[3]).toHaveTextContent('命中 0（本页返回条数（无总数）');
  expect(within(table).getByRole('columnheader', { name: 'P95 ≈ ms' })).toBeInTheDocument();
  expect(rows[1]).toHaveTextContent('8.19');

  expect(screen.getByRole('img', { name: /HTTP 接口每小时调用趋势/ })).toHaveAccessibleName(
    /共 40 次调用 · 0 次错误 · 峰值 30 次\/小时/,
  );
  expect(screen.getByRole('img', { name: /MCP 工具每小时调用趋势/ })).toBeInTheDocument();
  expect(screen.getByRole('table', { name: /HTTP 接口每小时调用与错误/ })).toBeInTheDocument();
  expect(screen.getByRole('region', { name: '命中量级 · 总命中数' })).toBeInTheDocument();
  expect(
    screen.getByRole('region', { name: '命中量级 · 本页返回条数（无总数）' }),
  ).toBeInTheDocument();
  expect(screen.getByRole('region', { name: '命中计数口径' })).toHaveTextContent('总命中数');

  await user.click(screen.getByRole('radio', { name: '7 天' }));
  await waitFor(() => expect(queries.at(-1)).toBe('?period=7d'));
  await user.click(screen.getByRole('radio', { name: '检索' }));
  await waitFor(() => expect(queries.at(-1)).toBe('?period=7d&kind=search'));
}

/** Verify a disabled collector explains how to enable it instead of showing empty charts. */
async function explainsDisabledTelemetry(): Promise<void> {
  installTelemetryHandler(() =>
    HttpResponse.json({
      ...createReport(),
      enabled: false,
      series: [],
      hourly: [],
      search_shapes: [],
    }),
  );
  renderWithQuery(<LoadTelemetryCard isEnabled />);

  expect(await screen.findByText('负载统计未启用')).toBeInTheDocument();
  expect(screen.getByText(/telemetry_enabled/)).toHaveTextContent('重启服务');
  expect(screen.queryByRole('table')).not.toBeInTheDocument();
}

/** Verify an enabled collector without observations shows an explicit empty state. */
async function showsEmptyWindow(): Promise<void> {
  installTelemetryHandler(() =>
    HttpResponse.json({ ...createReport(), series: [], hourly: [], search_shapes: [] }),
  );
  renderWithQuery(<LoadTelemetryCard isEnabled />);

  expect(await screen.findByText('暂无负载数据')).toBeInTheDocument();
  expect(screen.queryByRole('img')).not.toBeInTheDocument();
}

/**
 * Verify one failed status maps to its explicit error state.
 *
 * @param status - HTTP status returned by the telemetry endpoint.
 * @param title - Expected state title.
 */
async function showsErrorState(status: number, title: string): Promise<void> {
  installTelemetryHandler(() =>
    HttpResponse.json({ detail: 'Load telemetry temporarily unavailable' }, { status }),
  );
  renderWithQuery(<LoadTelemetryCard isEnabled />);

  const alert = await screen.findByRole('alert');
  expect(alert).toHaveTextContent(title);
  expect(within(alert).getByRole('button', { name: '重试' })).toBeInTheDocument();
}

describe('LoadTelemetryCard', () => {
  test('zero-fills missing UTC hours per kind', zeroFillsMissingHours);
  test('keeps hit-count bases separate', keepsHitBasesSeparate);
  test('renders the report and requests selected filters', rendersReportAndRequestsFilters);
  test('explains a disabled collector', explainsDisabledTelemetry);
  test('shows an empty window', showsEmptyWindow);
  test.each([
    [401, '需要登录'],
    [403, '需要管理员权限'],
    [503, '负载统计暂时不可用'],
  ])('shows the %i state', showsErrorState);
});
