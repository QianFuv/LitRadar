'use client';

import { useQuery } from '@tanstack/react-query';
import { BellRing, Database, Folder, Key, Shield, Star, Ticket, Users } from 'lucide-react';

import { adminGetStats } from '@/lib/api';
import { StatusPill } from '@/components/settings/settings-section';
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';

const NUMBER_FORMATTER = new Intl.NumberFormat('zh-CN');

/**
 * Render one aggregate statistic as a tile.
 *
 * @param props - Statistic label, value, and optional icon.
 * @returns Statistic tile.
 */
function StatCard({
  label,
  value,
  icon,
}: {
  label: string;
  value: string | number;
  icon?: React.ReactNode;
}) {
  return (
    <div className="flex flex-col justify-between gap-4 bg-background px-4 py-4 sm:px-5">
      <div className="flex items-start justify-between gap-2">
        <dt className="text-xs text-muted-foreground">{label}</dt>
        <span
          aria-hidden="true"
          className="flex size-7 shrink-0 items-center justify-center rounded-lg bg-muted text-muted-foreground [&_svg]:size-3.5"
        >
          {icon}
        </span>
      </div>
      <dd className="text-[28px] leading-none font-bold text-foreground tabular-nums">{value}</dd>
    </div>
  );
}

/**
 * Render a titled grid of statistic tiles.
 *
 * @param props - Group title and statistic tiles.
 * @returns Statistic tile grid.
 */
function StatGroup({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section className="space-y-3">
      <h3 className="text-sm font-bold">{title}</h3>
      <dl className="-mx-4 grid grid-cols-2 gap-px border-y bg-border sm:-mx-5 lg:grid-cols-4">
        {children}
      </dl>
    </section>
  );
}

/**
 * Render administrator aggregate and database statistics.
 *
 * @param props - Whether administrator queries may run.
 * @returns System overview card.
 */
export function AdminOverviewCard(props: { isEnabled: boolean }) {
  const state = useOverviewViewState(props);
  const { statsLoading } = state;
  return (
    <Card>
      <CardHeader>
        <CardTitle>系统概览</CardTitle>
        <CardDescription>全局统计信息</CardDescription>
      </CardHeader>
      <CardContent>
        {statsLoading ? (
          <div className="text-muted-foreground" role="status">
            加载中…
          </div>
        ) : (
          <div className="space-y-10">
            {renderIndexHero(state)}

            {renderAuthStatistics(state)}

            {/* Index stats */}
            {renderIndexStatistics(state)}

            {/* Push stats */}
            {renderPushStatistics(state)}
          </div>
        )}
      </CardContent>
    </Card>
  );
}
/** Own the administrator statistics query and conditional metadata. */
function useOverviewViewState({ isEnabled }: { isEnabled: boolean }) {
  const { data: stats, isLoading: statsLoading } = useQuery({
    queryKey: ['admin-stats'],
    queryFn: () => adminGetStats(),
    enabled: isEnabled,
  });
  const authStats = stats?.auth;
  const indexStats = stats?.index;
  const pushStats = stats?.push;

  return { statsLoading, authStats, indexStats, pushStats };
}
type OverviewViewState = ReturnType<typeof useOverviewViewState>;

/** Retain every authentication statistic and exact nullish zero fallback. */
function renderAuthStatistics(state: Pick<OverviewViewState, 'authStats'>) {
  const { authStats } = state;
  return (
    <>
      <StatGroup title="账号与访问">
        <StatCard
          label="用户总数"
          value={getAuthStatisticValue(authStats?.total_users)}
          icon={<Users />}
        />
        <StatCard
          label="管理员"
          value={getAuthStatisticValue(authStats?.admin_count)}
          icon={<Shield />}
        />
        <StatCard
          label="活跃令牌"
          value={getAuthStatisticValue(authStats?.active_tokens)}
          icon={<Key />}
        />
        <StatCard
          label="推送订阅"
          value={getAuthStatisticValue(authStats?.notification_subscribers)}
          icon={<BellRing />}
        />
      </StatGroup>
      <StatGroup title="收藏与邀请">
        <StatCard
          label="收藏夹"
          value={getAuthStatisticValue(authStats?.total_folders)}
          icon={<Folder />}
        />
        <StatCard
          label="收藏文章"
          value={getAuthStatisticValue(authStats?.total_favorites)}
          icon={<Star />}
        />
        <StatCard
          label="邀请码 (未使用)"
          value={getAuthStatisticValue(authStats?.unused_invite_codes)}
          icon={<Ticket />}
        />
        <StatCard
          label="邀请码 (已使用)"
          value={getAuthStatisticValue(authStats?.used_invite_codes)}
          icon={<Ticket />}
        />
      </StatGroup>
    </>
  );
}

/** Render the corpus totals as the headline of the overview. */
function renderIndexHero(state: Pick<OverviewViewState, 'indexStats'>) {
  const { indexStats } = state;
  return (
    indexStats && (
      <div className="border-b pb-8">
        <p className="text-xs text-muted-foreground">索引文献总量</p>
        <p className="mt-2 text-4xl leading-none font-bold tabular-nums sm:text-5xl">
          {NUMBER_FORMATTER.format(indexStats.total_articles)}
        </p>
        <p className="mt-3 text-xs text-muted-foreground tabular-nums">
          {NUMBER_FORMATTER.format(indexStats.total_journals)} 本期刊 ·{' '}
          {indexStats.databases.length} 个数据库
        </p>
      </div>
    )
  );
}

/** Retain conditional index totals, database order and localized values. */
function renderIndexStatistics(state: Pick<OverviewViewState, 'indexStats'>) {
  const { indexStats } = state;
  return (
    indexStats && (
      <section className="space-y-3">
        <div className="flex flex-wrap items-baseline justify-between gap-x-4 gap-y-1">
          <h3 className="text-sm font-bold">索引数据库</h3>
          <p className="text-xs text-muted-foreground tabular-nums">
            共 {NUMBER_FORMATTER.format(indexStats.total_articles)} 篇文章，
            {NUMBER_FORMATTER.format(indexStats.total_journals)} 本期刊
          </p>
        </div>
        <ul className="-mx-4 divide-y border-y sm:-mx-5">
          {indexStats.databases.map((db) => {
            const share =
              indexStats.total_articles > 0 ? db.articles / indexStats.total_articles : 0;
            return (
              <li key={db.db_name} className="space-y-2.5 px-5 py-4 text-sm">
                <div className="flex flex-wrap items-center justify-between gap-x-4 gap-y-1">
                  <span className="flex min-w-0 items-center gap-2 font-bold">
                    <Database className="size-4 shrink-0 text-muted-foreground" />
                    <span className="break-all">{db.db_name}</span>
                  </span>
                  <span className="text-xs text-muted-foreground tabular-nums">
                    {NUMBER_FORMATTER.format(db.articles)} 文章 ·{' '}
                    {NUMBER_FORMATTER.format(db.journals)} 期刊 ·{' '}
                    {NUMBER_FORMATTER.format(db.issues)} 期
                  </span>
                </div>
                <div className="flex items-center gap-3">
                  <span
                    aria-hidden="true"
                    className="h-1.5 flex-1 overflow-hidden rounded-full bg-muted"
                  >
                    <span
                      className="block h-full rounded-full bg-primary"
                      style={{ width: `${share * 100}%` }}
                    />
                  </span>
                  <span className="w-12 text-right text-xs text-muted-foreground tabular-nums">
                    {Math.round(share * 100)}%
                  </span>
                </div>
              </li>
            );
          })}
        </ul>
      </section>
    )
  );
}

/** Retain delivered zero counts and server-ordered completion metadata. */
function renderPushStatistics(state: Pick<OverviewViewState, 'pushStats'>) {
  const { pushStats } = state;
  return (
    pushStats &&
    pushStats.length > 0 && (
      <section className="space-y-3">
        <h3 className="text-sm font-bold">推送状态</h3>
        <ul className="-mx-4 divide-y border-y sm:-mx-5">
          {pushStats.map((ps) => (
            <li
              key={ps.db_name}
              className="flex flex-wrap items-center justify-between gap-x-4 gap-y-2 px-5 py-3.5 text-sm"
            >
              <span className="flex min-w-0 items-center gap-2">
                <span className="font-bold break-all">{ps.db_name}</span>
                <StatusPill>{ps.status}</StatusPill>
              </span>
              <span className="text-xs text-muted-foreground tabular-nums">
                {ps.delivered_count != null && `已推送 ${ps.delivered_count} 篇`}
                {ps.delivered_count != null && ps.last_completed && ' · '}
                {ps.last_completed && `最近完成: ${ps.last_completed}`}
              </span>
            </li>
          ))}
        </ul>
      </section>
    )
  );
}

/** Default an absent authentication count to zero without changing valid counts. */
function getAuthStatisticValue(value: number | null | undefined): number {
  return value ?? 0;
}
