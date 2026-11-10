/**
 * Administrator load section: interface load, latency, hourly trends, and search shape.
 */

import { keepPreviousData, useQuery } from '@tanstack/react-query';
import { useState } from 'react';

import { Clock, Layers, List, Search } from '@/components/ui/animated-icons';
import { AdminListHeader } from '@/components/admin/admin-list-header';
import {
  SettingsSection,
  SettingsSectionDescription,
  SettingsSectionHeader,
  SettingsSectionTitle,
} from '@/components/settings/settings-section';
import { Button } from '@/components/ui/button';
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';
import { LoadingText } from '@/components/ui/loading-text';
import { Progress } from '@/components/ui/progress';
import { RevealItem } from '@/components/ui/reveal';
import { StateMessage } from '@/components/ui/state-message';
import { ToggleGroup, ToggleGroupItem } from '@/components/ui/toggle-group';
import { ApiError, adminGetTelemetry } from '@/lib/api';
import type {
  TelemetryKind,
  TelemetryPeriod,
  TelemetryResponse,
  TelemetrySeries,
} from '@/lib/api/types';
import {
  buildHitsByBasis,
  buildKindTrends,
  groupSearchShapes,
  type DistributionValue,
  type HitCountBasis,
  type KindTrend,
  type SearchShapeFeature,
} from '@/lib/load-telemetry';
import * as stylex from '@stylexjs/stylex';
import { colors, lineHeights } from '@/lib/theme.stylex';

type KindFilter = TelemetryKind | 'all';

const DEFAULT_PERIOD: TelemetryPeriod = '24h';
const SERIES_PREVIEW_LIMIT = 20;
const NUMBER_FORMATTER = new Intl.NumberFormat('zh-CN');
const PERCENT_FORMATTER = new Intl.NumberFormat('zh-CN', {
  maximumFractionDigits: 1,
  style: 'percent',
});
const HOUR_FORMATTER = new Intl.DateTimeFormat('zh-CN', {
  day: '2-digit',
  hour: '2-digit',
  minute: '2-digit',
  month: '2-digit',
});

const PERIOD_OPTIONS: readonly { label: string; value: TelemetryPeriod }[] = [
  { label: '24 小时', value: '24h' },
  { label: '7 天', value: '7d' },
  { label: '30 天', value: '30d' },
];

const KIND_OPTIONS: readonly { label: string; value: KindFilter }[] = [
  { label: '全部', value: 'all' },
  { label: 'HTTP', value: 'http' },
  { label: 'MCP', value: 'mcp' },
  { label: '检索', value: 'search' },
];

const KIND_LABELS: Readonly<Record<TelemetryKind, string>> = {
  http: 'HTTP 接口',
  mcp: 'MCP 工具',
  search: '检索',
};

const FEATURE_LABELS: Readonly<Record<SearchShapeFeature, string>> = {
  database: '数据库',
  endpoint: '检索入口',
  hit_count_basis: '命中计数口径',
  mode: '检索模式',
  term_count: '检索词数',
  uses_boolean: '使用布尔或 NEAR 运算',
  uses_prefix: '使用前缀通配',
};

const HIT_BASIS_LABELS: Readonly<Record<HitCountBasis, string>> = {
  candidates: '相关推荐候选数（去重前）',
  returned: '本页返回条数（无总数）',
  total: '总命中数',
};

const MODE_LABELS: Readonly<Record<string, string>> = {
  advanced: '高级检索',
  simple: '简单检索',
};

/**
 * Format a search-shape value for display without hiding unknown server values.
 *
 * @param feature - Feature the value belongs to.
 * @param value - Raw server value.
 * @returns Localized display label.
 */
function formatShapeValue(feature: SearchShapeFeature, value: string): string {
  if (feature === 'uses_prefix' || feature === 'uses_boolean') {
    return value === 'true' ? '是' : value === 'false' ? '否' : value;
  }
  if (feature === 'mode') {
    return MODE_LABELS[value] ?? value;
  }
  if (feature === 'hit_count_basis') {
    return HIT_BASIS_LABELS[value as HitCountBasis] ?? value;
  }
  if (feature === 'term_count') {
    return `${value} 个词`;
  }
  return value;
}

/**
 * Format an approximate histogram percentile in milliseconds.
 *
 * @param milliseconds - Percentile upper bound in milliseconds.
 * @returns Display text with precision scaled to magnitude.
 */
function formatMilliseconds(milliseconds: number): string {
  const fractionDigits = milliseconds < 10 ? 2 : milliseconds < 100 ? 1 : 0;
  return milliseconds.toLocaleString('zh-CN', { maximumFractionDigits: fractionDigits });
}

/**
 * Format a UTC hour start in the browser's local time.
 *
 * @param hourMs - Hour start in epoch milliseconds.
 * @returns Localized month, day, and time.
 */
function formatHour(hourMs: number): string {
  return HOUR_FORMATTER.format(hourMs);
}

/**
 * List the identity dimensions that distinguish one series from its siblings.
 *
 * @param series - Telemetry series.
 * @returns Short dimension labels in display order.
 */
function describeSeriesDimensions(series: TelemetrySeries): string[] {
  if (series.kind === 'http') {
    return [series.method, series.status_class].filter((value): value is string => !!value);
  }
  const outcome = series.is_error ? '失败' : '成功';
  if (series.kind === 'mcp') {
    return [outcome];
  }
  return [
    series.mode ? (MODE_LABELS[series.mode] ?? series.mode) : null,
    series.database ?? null,
    series.term_count ? `${series.term_count} 个词` : null,
    series.uses_prefix ? '前缀通配' : null,
    series.uses_boolean ? '布尔/NEAR' : null,
    series.hits && series.hit_count_basis
      ? `命中 ${series.hits}（${HIT_BASIS_LABELS[series.hit_count_basis]}）`
      : null,
    outcome,
  ].filter((value): value is string => value !== null);
}

/**
 * Describe a failed telemetry read with explicit authentication and availability states.
 *
 * @param error - Query error.
 * @returns State message title and description.
 */
function describeTelemetryError(error: Error): { description: string; title: string } {
  const status = error instanceof ApiError ? error.status : null;
  if (status === 401) {
    return { description: '登录状态已失效，请重新登录后再查看负载统计。', title: '需要登录' };
  }
  if (status === 403) {
    return { description: '当前账号没有管理员权限，无法查看负载统计。', title: '需要管理员权限' };
  }
  if (status === 503) {
    return { description: '读取遥测数据库失败，请稍后重试。', title: '负载统计暂时不可用' };
  }
  return { description: error.message, title: '获取负载统计失败' };
}

/**
 * Render the administrator load-telemetry section.
 *
 * @param props - Whether administrator queries may run.
 * @returns Load telemetry card.
 */
export function LoadTelemetryCard({ isEnabled }: { isEnabled: boolean }) {
  const [period, setPeriod] = useState<TelemetryPeriod>(DEFAULT_PERIOD);
  const [kindFilter, setKindFilter] = useState<KindFilter>('all');
  const kind = kindFilter === 'all' ? null : kindFilter;
  const { data, error, isLoading, refetch } = useQuery({
    queryKey: ['admin-telemetry', period, kind],
    queryFn: () => adminGetTelemetry(period, kind),
    enabled: isEnabled,
    placeholderData: keepPreviousData,
  });

  return (
    <Card>
      <CardHeader>
        <CardTitle>负载统计</CardTitle>
        <CardDescription>接口调用量、错误率、延迟与检索形态</CardDescription>
      </CardHeader>
      <CardContent>
        <div
          {...stylex.props(styles.loadTelemetryCardCardContentItem, styles.loadTelemetryCardDiv)}
        >
          <ToggleGroup
            type="single"
            aria-label="统计时段"
            value={period}
            onValueChange={(value) => {
              if (value) setPeriod(value as TelemetryPeriod);
            }}
          >
            {PERIOD_OPTIONS.map((option) => (
              <ToggleGroupItem key={option.value} value={option.value}>
                {option.label}
              </ToggleGroupItem>
            ))}
          </ToggleGroup>
          <ToggleGroup
            type="single"
            aria-label="调用类型"
            value={kindFilter}
            onValueChange={(value) => {
              if (value) setKindFilter(value as KindFilter);
            }}
          >
            {KIND_OPTIONS.map((option) => (
              <ToggleGroupItem key={option.value} value={option.value}>
                {option.label}
              </ToggleGroupItem>
            ))}
          </ToggleGroup>
        </div>

        <RevealItem
          isReady={!isLoading}
          xstyle={styles.loadTelemetryCardCardContentItem}
          placeholder={
            <p role="status" {...stylex.props(styles.loadTelemetryCardP)}>
              <LoadingText text="加载中…" />
            </p>
          }
        >
          {error instanceof Error ? (
            <StateMessage
              tone="danger"
              {...describeTelemetryError(error)}
              action={
                <Button variant="outline" size="sm" onClick={() => void refetch()}>
                  重试
                </Button>
              }
            />
          ) : (
            data && <TelemetryReport report={data} kind={kind} />
          )}
        </RevealItem>

        <p {...stylex.props(styles.loadTelemetryCardCardContentItem, styles.loadTelemetryCardP2)}>
          统计为尽力而为的近似值：百分位取对数分桶的上界，高并发争用或存储失败时可能少计。
          仅记录接口模板、工具名与检索形态，不记录用户身份、IP 与检索文本。
        </p>
      </CardContent>
    </Card>
  );
}

/**
 * Render one successful telemetry report, including the disabled and empty states.
 *
 * @param props - Report and the selected kind filter.
 * @returns Report sections or an explicit state message.
 */
function TelemetryReport({
  kind,
  report,
}: {
  kind: TelemetryKind | null;
  report: TelemetryResponse;
}) {
  if (!report.enabled) {
    return (
      <StateMessage
        tone="warning"
        title="负载统计未启用"
        description="在「运行配置」中将 telemetry_enabled 设为 true，重启服务后开始采集；已有数据会保留。"
      />
    );
  }
  if (report.series.length === 0) {
    return (
      <StateMessage
        title="暂无负载数据"
        description={`${formatHour(Date.parse(report.from))} 至 ${formatHour(Date.parse(report.to))} 之间没有记录到所选类型的调用。`}
      />
    );
  }
  return (
    <>
      <SeriesSection report={report} />
      <TrendSection report={report} kind={kind} />
      {report.search_shapes.length > 0 && <SearchShapeSection report={report} />}
    </>
  );
}

/**
 * Render the top series table with approximate latency percentiles.
 *
 * @param props - Telemetry report.
 * @returns Series summary section.
 */
function SeriesSection({ report }: { report: TelemetryResponse }) {
  const [isExpanded, setIsExpanded] = useState(false);
  const totalCalls = report.series.reduce((sum, series) => sum + series.calls, 0);
  const totalErrors = report.series.reduce((sum, series) => sum + series.errors, 0);
  const visibleSeries = isExpanded ? report.series : report.series.slice(0, SERIES_PREVIEW_LIMIT);
  const hiddenCount = report.series.length - SERIES_PREVIEW_LIMIT;

  return (
    <SettingsSection>
      <SettingsSectionHeader>
        <SettingsSectionTitle icon={List}>调用排行</SettingsSectionTitle>
        <SettingsSectionDescription>
          按调用次数排序。P50/P95/P99 为对数分桶上界的近似值（毫秒），不是精确观测值。
        </SettingsSectionDescription>
      </SettingsSectionHeader>
      <AdminListHeader
        summary={`共 ${NUMBER_FORMATTER.format(totalCalls)} 次调用 · ${NUMBER_FORMATTER.format(totalErrors)} 次错误 · ${report.series.length} 个序列`}
      >
        {hiddenCount > 0 && (
          <Button variant="ghost" size="sm" onClick={() => setIsExpanded((value) => !value)}>
            {isExpanded ? '收起' : `显示全部 ${report.series.length} 个序列`}
          </Button>
        )}
      </AdminListHeader>
      <div {...stylex.props(styles.seriesSectionDiv)}>
        <table {...stylex.props(styles.seriesSectionTable)}>
          <caption {...stylex.props(styles.seriesSectionCaption)}>
            调用排行（延迟百分位为近似值，单位毫秒）
          </caption>
          <thead {...stylex.props(styles.seriesSectionThead)}>
            <tr {...stylex.props(styles.seriesSectionTr)}>
              <th scope="col" {...stylex.props(styles.seriesSectionTh)}>
                类型
              </th>
              <th scope="col" {...stylex.props(styles.seriesSectionTh2)}>
                标识
              </th>
              <th scope="col" {...stylex.props(styles.seriesSectionTh3)}>
                调用
              </th>
              <th scope="col" {...stylex.props(styles.seriesSectionTh3)}>
                错误率
              </th>
              <th scope="col" {...stylex.props(styles.seriesSectionTh3)}>
                P50 ≈ ms
              </th>
              <th scope="col" {...stylex.props(styles.seriesSectionTh3)}>
                P95 ≈ ms
              </th>
              <th scope="col" {...stylex.props(styles.seriesSectionTh4)}>
                P99 ≈ ms
              </th>
            </tr>
          </thead>
          <tbody {...stylex.props(styles.seriesSectionTbody)}>
            {visibleSeries.map((series, index) => (
              <tr
                {...stylex.props(styles.seriesSectionTbodyItem)}
                key={`${index}:${JSON.stringify(series)}`}
              >
                <td {...stylex.props(styles.seriesSectionTd)}>{KIND_LABELS[series.kind]}</td>
                <td {...stylex.props(styles.seriesSectionTd2)}>
                  <div {...stylex.props(styles.seriesSectionDiv2)}>{series.name}</div>
                  <div {...stylex.props(styles.seriesSectionDiv3)}>
                    {describeSeriesDimensions(series).map((dimension) => (
                      <span key={dimension} {...stylex.props(styles.seriesSectionSpan)}>
                        {dimension}
                      </span>
                    ))}
                  </div>
                </td>
                <td {...stylex.props(styles.seriesSectionTd3)}>
                  {NUMBER_FORMATTER.format(series.calls)}
                </td>
                <td
                  {...stylex.props(
                    styles.seriesSectionTd3,
                    series.errors > 0 && styles.seriesSectionTdErrors,
                  )}
                >
                  {PERCENT_FORMATTER.format(series.error_rate)}
                </td>
                <td {...stylex.props(styles.seriesSectionTd3)}>
                  {formatMilliseconds(series.p50_ms)}
                </td>
                <td {...stylex.props(styles.seriesSectionTd3)}>
                  {formatMilliseconds(series.p95_ms)}
                </td>
                <td {...stylex.props(styles.seriesSectionTd4)}>
                  {formatMilliseconds(series.p99_ms)}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </SettingsSection>
  );
}

/**
 * Render zero-filled hourly call and error trends per kind.
 *
 * @param props - Telemetry report and the selected kind filter.
 * @returns Trend section with one small chart per kind.
 */
function TrendSection({ kind, report }: { kind: TelemetryKind | null; report: TelemetryResponse }) {
  const trends = buildKindTrends(report, kind);
  return (
    <SettingsSection>
      <SettingsSectionHeader>
        <SettingsSectionTitle icon={Clock}>每小时趋势</SettingsSectionTitle>
        <SettingsSectionDescription>
          按 UTC 整点分桶，以本地时间显示：{formatHour(Date.parse(report.from))} 至{' '}
          {formatHour(Date.parse(report.to))}。每个类型使用各自的纵轴峰值。
        </SettingsSectionDescription>
      </SettingsSectionHeader>
      <div {...stylex.props(styles.trendSectionDiv)}>
        <span {...stylex.props(styles.trendSectionSpan)}>
          <span aria-hidden="true" {...stylex.props(styles.trendSectionSpan2)} />
          调用
        </span>
        <span {...stylex.props(styles.trendSectionSpan)}>
          <span aria-hidden="true" {...stylex.props(styles.trendSectionSpan3)} />
          错误
        </span>
      </div>
      <div {...stylex.props(styles.trendSectionDiv2)}>
        {trends.map((trend) => (
          <KindTrendChart key={trend.kind} trend={trend} />
        ))}
      </div>
    </SettingsSection>
  );
}

/**
 * Render one kind's hourly bars with a native tooltip per hour and a data-table alternative.
 *
 * @param props - Zero-filled kind trend.
 * @returns Accessible bar chart figure.
 */
function KindTrendChart({ trend }: { trend: KindTrend }) {
  const { hours, maxCalls } = trend;
  const label = KIND_LABELS[trend.kind];
  const gap = hours.length <= 48 ? 0.2 : 0;
  const scale = (value: number) =>
    maxCalls > 0 && value > 0 ? Math.max((value / maxCalls) * 100, 2) : 0;
  const observedHours = hours.filter((hour) => hour.calls > 0 || hour.errors > 0);
  const summary = `共 ${NUMBER_FORMATTER.format(trend.totalCalls)} 次调用 · ${NUMBER_FORMATTER.format(trend.totalErrors)} 次错误 · 峰值 ${NUMBER_FORMATTER.format(maxCalls)} 次/小时`;

  return (
    <figure data-telemetry-trend={trend.kind}>
      <figcaption
        {...stylex.props(styles.kindTrendChartFigureItem, styles.kindTrendChartFigcaption)}
      >
        <span {...stylex.props(styles.kindTrendChartSpan)}>{label}</span>
        <span {...stylex.props(styles.kindTrendChartSpan2)}>{summary}</span>
      </figcaption>
      <svg
        role="img"
        aria-label={`${label}每小时调用趋势：${summary}`}
        viewBox={`0 0 ${Math.max(hours.length, 1)} 100`}
        preserveAspectRatio="none"
        {...stylex.props(styles.kindTrendChartFigureItem, styles.kindTrendChartSvg)}
      >
        {hours.map((hour, index) => {
          const callsHeight = scale(hour.calls);
          const errorsHeight = Math.min(scale(hour.errors), callsHeight);
          return (
            <g key={hour.hourMs}>
              <title>{`${formatHour(hour.hourMs)} · 调用 ${NUMBER_FORMATTER.format(hour.calls)} · 错误 ${NUMBER_FORMATTER.format(hour.errors)}`}</title>
              <rect
                x={index}
                y={0}
                width={1}
                height={100}
                {...stylex.props(styles.kindTrendChartRect)}
              />
              <rect
                x={index + gap / 2}
                y={100 - callsHeight}
                width={1 - gap}
                height={callsHeight}
                {...stylex.props(styles.kindTrendChartRect2)}
              />
              <rect
                x={index + gap / 2}
                y={100 - errorsHeight}
                width={1 - gap}
                height={errorsHeight}
                {...stylex.props(styles.kindTrendChartRect3)}
              />
            </g>
          );
        })}
      </svg>
      <div {...stylex.props(styles.kindTrendChartFigureItem, styles.kindTrendChartDiv)}>
        <span>{hours.length > 0 && formatHour(hours[0].hourMs)}</span>
        <span>{hours.length > 0 && formatHour(hours[hours.length - 1].hourMs)}</span>
      </div>
      {observedHours.length > 0 && (
        <details {...stylex.props(styles.kindTrendChartFigureItem, styles.kindTrendChartDetails)}>
          <summary {...stylex.props(styles.kindTrendChartSummary)}>查看{label}每小时数据</summary>
          <table {...stylex.props(styles.kindTrendChartTable)}>
            <caption {...stylex.props(styles.seriesSectionCaption)}>
              {label}每小时调用与错误（仅列出有记录的小时）
            </caption>
            <thead {...stylex.props(styles.kindTrendChartThead)}>
              <tr>
                <th scope="col" {...stylex.props(styles.kindTrendChartTh)}>
                  时间（本地）
                </th>
                <th scope="col" {...stylex.props(styles.kindTrendChartTh2)}>
                  调用
                </th>
                <th scope="col" {...stylex.props(styles.kindTrendChartTh2)}>
                  错误
                </th>
              </tr>
            </thead>
            <tbody>
              {observedHours.map((hour) => (
                <tr key={hour.hourMs}>
                  <td {...stylex.props(styles.kindTrendChartTd)}>{formatHour(hour.hourMs)}</td>
                  <td {...stylex.props(styles.kindTrendChartTd2)}>
                    {NUMBER_FORMATTER.format(hour.calls)}
                  </td>
                  <td {...stylex.props(styles.kindTrendChartTd2)}>
                    {NUMBER_FORMATTER.format(hour.errors)}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </details>
      )}
    </figure>
  );
}

/**
 * Render call-weighted marginal search-shape distributions, keeping hit bases separate.
 *
 * @param props - Telemetry report.
 * @returns Search shape section.
 */
function SearchShapeSection({ report }: { report: TelemetryResponse }) {
  const features = groupSearchShapes(report.search_shapes);
  const hitsByBasis = buildHitsByBasis(report.series);
  return (
    <SettingsSection>
      <SettingsSectionHeader>
        <SettingsSectionTitle icon={Search}>检索形态</SettingsSectionTitle>
        <SettingsSectionDescription>
          各特征的边际分布，按调用次数加权；不同特征之间不是交叉组合。命中量级按计数口径分别统计，
          不同口径不可直接比较。
        </SettingsSectionDescription>
      </SettingsSectionHeader>
      <div {...stylex.props(styles.searchShapeSectionDiv)}>
        {features.map((distribution) => (
          <DistributionList
            key={distribution.feature}
            title={FEATURE_LABELS[distribution.feature]}
            totalCalls={distribution.totalCalls}
            values={distribution.values}
            formatValue={(value) => formatShapeValue(distribution.feature, value)}
          />
        ))}
        {hitsByBasis.map((distribution) => (
          <DistributionList
            key={`hits-${distribution.basis}`}
            title={`命中量级 · ${HIT_BASIS_LABELS[distribution.basis]}`}
            totalCalls={distribution.totalCalls}
            values={distribution.values}
            formatValue={(value) => value}
          />
        ))}
      </div>
    </SettingsSection>
  );
}

/**
 * Render one distribution as labelled share bars with exact counts.
 *
 * @param props - Title, total weight, ordered values, and value formatter.
 * @returns Distribution list.
 */
function DistributionList({
  formatValue,
  title,
  totalCalls,
  values,
}: {
  formatValue: (value: string) => string;
  title: string;
  totalCalls: number;
  values: DistributionValue[];
}) {
  return (
    <section aria-label={title}>
      <h4 {...stylex.props(styles.kindTrendChartFigureItem, styles.distributionListH4)}>
        <Layers {...stylex.props(styles.distributionListLayers)} aria-hidden="true" />
        {title}
      </h4>
      <ul {...stylex.props(styles.kindTrendChartFigureItem)}>
        {values.map((entry) => {
          const share = totalCalls > 0 ? entry.calls / totalCalls : 0;
          return (
            <li
              key={entry.value}
              {...stylex.props(styles.kindTrendChartFigureItem, styles.kindTrendChartDetails)}
            >
              <div {...stylex.props(styles.distributionListLiItem, styles.distributionListDiv)}>
                <span {...stylex.props(styles.distributionListSpan)}>
                  {formatValue(entry.value)}
                </span>
                <span {...stylex.props(styles.distributionListSpan2)}>
                  {NUMBER_FORMATTER.format(entry.calls)} · {PERCENT_FORMATTER.format(share)}
                </span>
              </div>
              <Progress
                xstyle={styles.distributionListLiItem}
                aria-hidden="true"
                value={share * 100}
              />
            </li>
          );
        })}
      </ul>
    </section>
  );
}

const styles = stylex.create({
  loadTelemetryCardCardContentItem: {
    marginBottom: {
      default: null,
      ':not(:last-child)': '2rem',
    },
  },
  loadTelemetryCardDiv: {
    display: 'flex',
    flexWrap: 'wrap',
    alignItems: 'center',
    gap: '0.75rem',
  },
  loadTelemetryCardP: {
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
    color: colors.mutedForeground,
  },
  loadTelemetryCardP2: {
    fontSize: '0.75rem',
    lineHeight: 1.625,
    color: colors.mutedForeground,
  },
  seriesSectionDiv: {
    marginInline: {
      default: '-1rem',
      '@media (min-width: 40rem)': '-1.25rem',
    },
    overflowX: 'auto',
    borderBlockStyle: 'solid',
    borderBlockWidth: '1px',
  },
  seriesSectionTable: {
    width: '100%',
    minWidth: '640px',
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
  },
  seriesSectionCaption: {
    position: 'absolute',
    width: '1px',
    height: '1px',
    padding: 0,
    margin: '-1px',
    overflow: 'hidden',
    clipPath: 'inset(50%)',
    whiteSpace: 'nowrap',
    borderWidth: 0,
  },
  seriesSectionThead: {
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    color: colors.mutedForeground,
  },
  seriesSectionTr: {
    borderBottomStyle: 'solid',
    borderBottomWidth: '1px',
  },
  seriesSectionTh: {
    paddingInline: {
      default: '1rem',
      '@media (min-width: 40rem)': '1.25rem',
    },
    paddingBlock: '0.625rem',
    textAlign: 'left',
    fontWeight: 500,
  },
  seriesSectionTh2: {
    paddingInline: '0.5rem',
    paddingBlock: '0.625rem',
    textAlign: 'left',
    fontWeight: 500,
  },
  seriesSectionTh3: {
    paddingInline: '0.5rem',
    paddingBlock: '0.625rem',
    textAlign: 'right',
    fontWeight: 500,
  },
  seriesSectionTh4: {
    paddingInline: {
      default: '1rem',
      '@media (min-width: 40rem)': '1.25rem',
    },
    paddingBlock: '0.625rem',
    textAlign: 'right',
    fontWeight: 500,
  },
  seriesSectionTbodyItem: {
    borderTopStyle: {
      default: null,
      ':not(:last-child)': 'solid',
    },
    borderTopWidth: {
      default: null,
      ':not(:last-child)': '0px',
    },
    borderBottomStyle: {
      default: null,
      ':not(:last-child)': 'solid',
    },
    borderBottomWidth: {
      default: null,
      ':not(:last-child)': '1px',
    },
  },
  seriesSectionTbody: {
    fontVariantNumeric: 'tabular-nums',
  },
  seriesSectionTd: {
    paddingInline: {
      default: '1rem',
      '@media (min-width: 40rem)': '1.25rem',
    },
    paddingBlock: '0.75rem',
    verticalAlign: 'top',
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    color: colors.mutedForeground,
  },
  seriesSectionTd2: {
    paddingInline: '0.5rem',
    paddingBlock: '0.75rem',
    verticalAlign: 'top',
  },
  seriesSectionDiv2: {
    fontFamily: "'JetBrainsLxgwNerdMono', monospace",
    fontSize: '13px',
    wordBreak: 'break-all',
  },
  seriesSectionDiv3: {
    marginTop: '0.25rem',
    display: 'flex',
    flexWrap: 'wrap',
    gap: '0.25rem',
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    color: colors.mutedForeground,
  },
  seriesSectionSpan: {
    borderRadius: '6px',
    backgroundColor: colors.muted,
    paddingInline: '0.375rem',
    paddingBlock: '0.125rem',
  },
  seriesSectionTd3: {
    paddingInline: '0.5rem',
    paddingBlock: '0.75rem',
    textAlign: 'right',
    verticalAlign: 'top',
  },
  seriesSectionTdErrors: {
    color: colors.destructive,
  },
  seriesSectionTd4: {
    paddingInline: {
      default: '1rem',
      '@media (min-width: 40rem)': '1.25rem',
    },
    paddingBlock: '0.75rem',
    textAlign: 'right',
    verticalAlign: 'top',
  },
  trendSectionDiv: {
    display: 'flex',
    alignItems: 'center',
    gap: '1rem',
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    color: colors.mutedForeground,
  },
  trendSectionSpan: {
    display: 'flex',
    alignItems: 'center',
    gap: '0.375rem',
  },
  trendSectionSpan2: {
    width: '0.625rem',
    height: '0.625rem',
    borderRadius: '4px',
    backgroundColor: colors.primary,
  },
  trendSectionSpan3: {
    width: '0.625rem',
    height: '0.625rem',
    borderRadius: '4px',
    backgroundColor: colors.destructive,
  },
  trendSectionDiv2: {
    display: 'flex',
    flexDirection: 'column',
    rowGap: '1.5rem',
  },
  kindTrendChartFigureItem: {
    marginBottom: {
      default: null,
      ':not(:last-child)': '0.5rem',
    },
  },
  kindTrendChartFigcaption: {
    display: 'flex',
    flexWrap: 'wrap',
    alignItems: 'baseline',
    justifyContent: 'space-between',
    columnGap: '1rem',
    rowGap: '0.25rem',
  },
  kindTrendChartSpan: {
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
    fontWeight: 700,
  },
  kindTrendChartSpan2: {
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    color: colors.mutedForeground,
    fontVariantNumeric: 'tabular-nums',
  },
  kindTrendChartSvg: {
    height: '6rem',
    width: '100%',
    borderRadius: '6px',
    backgroundColor: `color-mix(in oklab, ${colors.muted} 50%, transparent)`,
  },
  kindTrendChartRect: {
    fill: 'transparent',
  },
  kindTrendChartRect2: {
    fill: colors.primary,
  },
  kindTrendChartRect3: {
    fill: colors.destructive,
  },
  kindTrendChartDiv: {
    display: 'flex',
    justifyContent: 'space-between',
    fontSize: '11px',
    color: colors.mutedForeground,
    fontVariantNumeric: 'tabular-nums',
  },
  kindTrendChartDetails: {
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
  },
  kindTrendChartSummary: {
    cursor: 'pointer',
    color: {
      default: colors.mutedForeground,
      ':hover': {
        default: null,
        '@media (hover: hover)': colors.foreground,
      },
    },
  },
  kindTrendChartTable: {
    marginTop: '0.5rem',
    width: '100%',
    maxWidth: '28rem',
    fontVariantNumeric: 'tabular-nums',
  },
  kindTrendChartThead: {
    color: colors.mutedForeground,
  },
  kindTrendChartTh: {
    paddingBlock: '0.25rem',
    textAlign: 'left',
    fontWeight: 500,
  },
  kindTrendChartTh2: {
    paddingBlock: '0.25rem',
    textAlign: 'right',
    fontWeight: 500,
  },
  kindTrendChartTd: {
    paddingBlock: '0.125rem',
  },
  kindTrendChartTd2: {
    paddingBlock: '0.125rem',
    textAlign: 'right',
  },
  searchShapeSectionDiv: {
    display: 'grid',
    columnGap: '2rem',
    rowGap: '1.5rem',
    gridTemplateColumns: {
      default: null,
      '@media (min-width: 40rem)': 'repeat(2, minmax(0, 1fr))',
    },
  },
  distributionListH4: {
    display: 'flex',
    alignItems: 'center',
    gap: '0.5rem',
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    fontWeight: 700,
  },
  distributionListLayers: {
    width: '0.875rem',
    height: '0.875rem',
    flexShrink: 0,
    color: colors.mutedForeground,
  },
  distributionListLiItem: {
    marginBottom: {
      default: null,
      ':not(:last-child)': '0.25rem',
    },
  },
  distributionListDiv: {
    display: 'flex',
    alignItems: 'baseline',
    justifyContent: 'space-between',
    gap: '0.75rem',
  },
  distributionListSpan: {
    minWidth: '0px',
    wordBreak: 'break-all',
  },
  distributionListSpan2: {
    flexShrink: 0,
    color: colors.mutedForeground,
    fontVariantNumeric: 'tabular-nums',
  },
});
