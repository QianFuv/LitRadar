import { useQuery } from '@tanstack/react-query';
import {
  BellRing,
  Folder,
  Gift,
  Key,
  Layers,
  ShieldCheck,
  Star,
  Users,
  type AnimatedIcon,
} from '@/components/ui/animated-icons';

import { adminGetStats } from '@/lib/api';
import { StatusPill } from '@/components/settings/settings-section';
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';
import { Progress } from '@/components/ui/progress';
import { SlidingNumber } from '@animate-ui/react/primitives/texts/sliding-number';
import { RevealItem } from '@/components/ui/reveal';
import { LoadingText } from '@/components/ui/loading-text';
import * as stylex from '@stylexjs/stylex';
import { colors, lineHeights } from '@/lib/theme.stylex';

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
  icon: Icon,
}: {
  label: string;
  value: string | number;
  icon?: AnimatedIcon;
}) {
  return (
    <div {...stylex.props(styles.statCardDiv)}>
      <div {...stylex.props(styles.statCardDiv2)}>
        <dt {...stylex.props(styles.statCardDt)}>{label}</dt>
        <span aria-hidden="true" {...stylex.props(styles.statCardSpan)}>
          {Icon && <Icon xstyle={styles.statCardIcon} />}
        </span>
      </div>
      <dd {...stylex.props(styles.statCardDd)}>
        {typeof value === 'number' ? <SlidingNumber number={value} /> : value}
      </dd>
    </div>
  );
}

/**
 * Render a titled grid of statistic tiles.
 *
 * @param props - Group title and statistic tiles.
 * @returns Statistic tile grid.
 */
function StatGroup({
  icon: Icon,
  title,
  children,
}: {
  icon: AnimatedIcon;
  title: string;
  children: React.ReactNode;
}) {
  return (
    <section>
      <h3 {...stylex.props(styles.statGroupSectionItem, styles.statGroupH3)}>
        <Icon animateOnView xstyle={styles.statGroupIcon} aria-hidden="true" />
        {title}
      </h3>
      <dl {...stylex.props(styles.statGroupSectionItem, styles.statGroupDl)}>{children}</dl>
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
        <RevealItem
          isReady={!statsLoading}
          xstyle={styles.adminOverviewCardRevealItem}
          placeholder={
            <div {...stylex.props(styles.adminOverviewCardDiv)} role="status">
              <LoadingText text="加载中…" />
            </div>
          }
        >
          {renderIndexHero(state)}

          {renderAuthStatistics(state)}

          {/* Index stats */}
          {renderIndexStatistics(state)}

          {/* Push stats */}
          {renderPushStatistics(state)}
        </RevealItem>
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
      <StatGroup icon={Users} title="账号与访问">
        <StatCard
          label="用户总数"
          value={getAuthStatisticValue(authStats?.total_users)}
          icon={Users}
        />
        <StatCard
          label="管理员"
          value={getAuthStatisticValue(authStats?.admin_count)}
          icon={ShieldCheck}
        />
        <StatCard
          label="活跃令牌"
          value={getAuthStatisticValue(authStats?.active_tokens)}
          icon={Key}
        />
        <StatCard
          label="推送订阅"
          value={getAuthStatisticValue(authStats?.notification_subscribers)}
          icon={BellRing}
        />
      </StatGroup>
      <StatGroup icon={Star} title="收藏与邀请">
        <StatCard
          label="收藏夹"
          value={getAuthStatisticValue(authStats?.total_folders)}
          icon={Folder}
        />
        <StatCard
          label="收藏文章"
          value={getAuthStatisticValue(authStats?.total_favorites)}
          icon={Star}
        />
        <StatCard
          label="邀请码 (未使用)"
          value={getAuthStatisticValue(authStats?.unused_invite_codes)}
          icon={Gift}
        />
        <StatCard
          label="邀请码 (已使用)"
          value={getAuthStatisticValue(authStats?.used_invite_codes)}
          icon={Gift}
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
      <div {...stylex.props(styles.renderIndexHeroDiv)}>
        <p {...stylex.props(styles.statCardDt)}>索引文献总量</p>
        <p {...stylex.props(styles.renderIndexHeroP)}>
          {NUMBER_FORMATTER.format(indexStats.total_articles)}
        </p>
        <p {...stylex.props(styles.renderIndexHeroP2)}>
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
      <section>
        <div {...stylex.props(styles.statGroupSectionItem, styles.renderIndexStatisticsDiv)}>
          <h3 {...stylex.props(styles.statGroupH3)}>
            <Layers animateOnView xstyle={styles.statGroupIcon} aria-hidden="true" />
            索引数据库
          </h3>
          <p {...stylex.props(styles.renderIndexStatisticsP)}>
            共 {NUMBER_FORMATTER.format(indexStats.total_articles)} 篇文章，
            {NUMBER_FORMATTER.format(indexStats.total_journals)} 本期刊
          </p>
        </div>
        <ul {...stylex.props(styles.statGroupSectionItem, styles.renderIndexStatisticsUl)}>
          {indexStats.databases.map((db) => {
            const share =
              indexStats.total_articles > 0 ? db.articles / indexStats.total_articles : 0;
            return (
              <li
                key={db.db_name}
                {...stylex.props(
                  styles.renderIndexStatisticsUlItem,
                  styles.renderIndexStatisticsLi,
                )}
              >
                <div
                  {...stylex.props(
                    styles.renderIndexStatisticsLiItem,
                    styles.renderIndexStatisticsDiv2,
                  )}
                >
                  <span {...stylex.props(styles.renderIndexStatisticsSpan)}>
                    <Layers xstyle={styles.renderIndexStatisticsLayers} />
                    <span {...stylex.props(styles.renderIndexStatisticsSpan2)}>{db.db_name}</span>
                  </span>
                  <span {...stylex.props(styles.renderIndexStatisticsP)}>
                    {NUMBER_FORMATTER.format(db.articles)} 文章 ·{' '}
                    {NUMBER_FORMATTER.format(db.journals)} 期刊 ·{' '}
                    {NUMBER_FORMATTER.format(db.issues)} 期
                  </span>
                </div>
                <div
                  {...stylex.props(
                    styles.renderIndexStatisticsLiItem,
                    styles.renderIndexStatisticsDiv3,
                  )}
                >
                  <Progress
                    aria-hidden="true"
                    xstyle={styles.renderIndexStatisticsProgress}
                    value={share * 100}
                  />
                  <span {...stylex.props(styles.renderIndexStatisticsSpan3)}>
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
      <section>
        <h3 {...stylex.props(styles.statGroupSectionItem, styles.statGroupH3)}>
          <BellRing animateOnView xstyle={styles.statGroupIcon} aria-hidden="true" />
          推送状态
        </h3>
        <ul {...stylex.props(styles.statGroupSectionItem, styles.renderIndexStatisticsUl)}>
          {pushStats.map((ps) => (
            <li
              key={ps.db_name}
              {...stylex.props(styles.renderIndexStatisticsUlItem, styles.renderPushStatisticsLi)}
            >
              <span {...stylex.props(styles.renderPushStatisticsSpan)}>
                <span {...stylex.props(styles.renderPushStatisticsSpan2)}>{ps.db_name}</span>
                <StatusPill>{ps.status}</StatusPill>
              </span>
              <span {...stylex.props(styles.renderIndexStatisticsP)}>
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

const styles = stylex.create({
  statCardIcon: {
    width: '0.875rem',
    height: '0.875rem',
  },
  statCardDiv: {
    display: 'flex',
    flexDirection: 'column',
    justifyContent: 'space-between',
    gap: '1rem',
    backgroundColor: colors.background,
    paddingInline: {
      default: '1rem',
      '@media (min-width: 40rem)': '1.25rem',
    },
    paddingBlock: '1rem',
  },
  statCardDiv2: {
    display: 'flex',
    alignItems: 'flex-start',
    justifyContent: 'space-between',
    gap: '0.5rem',
  },
  statCardDt: {
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    color: colors.mutedForeground,
  },
  statCardSpan: {
    display: 'flex',
    width: '1.75rem',
    height: '1.75rem',
    flexShrink: 0,
    alignItems: 'center',
    justifyContent: 'center',
    borderRadius: '8px',
    backgroundColor: colors.muted,
    color: colors.mutedForeground,
  },
  statCardDd: {
    fontSize: '28px',
    lineHeight: 1,
    fontWeight: 700,
    color: colors.foreground,
    fontVariantNumeric: 'tabular-nums',
  },
  statGroupSectionItem: {
    marginBottom: {
      default: null,
      ':not(:last-child)': '0.75rem',
    },
  },
  statGroupH3: {
    display: 'flex',
    alignItems: 'center',
    gap: '0.5rem',
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
    fontWeight: 700,
  },
  statGroupIcon: {
    width: '0.875rem',
    height: '0.875rem',
    flexShrink: 0,
  },
  statGroupDl: {
    marginInline: {
      default: '-1rem',
      '@media (min-width: 40rem)': '-1.25rem',
    },
    display: 'grid',
    gridTemplateColumns: {
      default: 'repeat(2, minmax(0, 1fr))',
      '@media (min-width: 64rem)': 'repeat(4, minmax(0, 1fr))',
    },
    gap: '1px',
    borderBlockStyle: 'solid',
    borderBlockWidth: '1px',
    backgroundColor: colors.border,
  },
  adminOverviewCardRevealItem: {
    display: 'flex',
    flexDirection: 'column',
    rowGap: '2.5rem',
  },
  adminOverviewCardDiv: {
    color: colors.mutedForeground,
  },
  renderIndexHeroDiv: {
    borderBottomStyle: 'solid',
    borderBottomWidth: '1px',
    paddingBottom: '2rem',
  },
  renderIndexHeroP: {
    marginTop: '0.5rem',
    fontSize: {
      default: '2.25rem',
      '@media (min-width: 40rem)': '3rem',
    },
    lineHeight: {
      default: 1,
      '@media (min-width: 40rem)': 1,
    },
    fontWeight: 700,
    fontVariantNumeric: 'tabular-nums',
  },
  renderIndexHeroP2: {
    marginTop: '0.75rem',
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    color: colors.mutedForeground,
    fontVariantNumeric: 'tabular-nums',
  },
  renderIndexStatisticsDiv: {
    display: 'flex',
    flexWrap: 'wrap',
    alignItems: 'baseline',
    justifyContent: 'space-between',
    columnGap: '1rem',
    rowGap: '0.25rem',
  },
  renderIndexStatisticsP: {
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    color: colors.mutedForeground,
    fontVariantNumeric: 'tabular-nums',
  },
  renderIndexStatisticsUlItem: {
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
  renderIndexStatisticsUl: {
    marginInline: {
      default: '-1rem',
      '@media (min-width: 40rem)': '-1.25rem',
    },
    borderBlockStyle: 'solid',
    borderBlockWidth: '1px',
  },
  renderIndexStatisticsLiItem: {
    marginBottom: {
      default: null,
      ':not(:last-child)': '0.625rem',
    },
  },
  renderIndexStatisticsLi: {
    paddingInline: '1.25rem',
    paddingBlock: '1rem',
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
  },
  renderIndexStatisticsDiv2: {
    display: 'flex',
    flexWrap: 'wrap',
    alignItems: 'center',
    justifyContent: 'space-between',
    columnGap: '1rem',
    rowGap: '0.25rem',
  },
  renderIndexStatisticsSpan: {
    display: 'flex',
    minWidth: '0px',
    alignItems: 'center',
    gap: '0.5rem',
    fontWeight: 700,
  },
  renderIndexStatisticsLayers: {
    width: '1rem',
    height: '1rem',
    flexShrink: 0,
    color: colors.mutedForeground,
  },
  renderIndexStatisticsSpan2: {
    wordBreak: 'break-all',
  },
  renderIndexStatisticsDiv3: {
    display: 'flex',
    alignItems: 'center',
    gap: '0.75rem',
  },
  renderIndexStatisticsProgress: {
    flex: '1',
  },
  renderIndexStatisticsSpan3: {
    width: '3rem',
    textAlign: 'right',
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    color: colors.mutedForeground,
    fontVariantNumeric: 'tabular-nums',
  },
  renderPushStatisticsLi: {
    display: 'flex',
    flexWrap: 'wrap',
    alignItems: 'center',
    justifyContent: 'space-between',
    columnGap: '1rem',
    rowGap: '0.5rem',
    paddingInline: '1.25rem',
    paddingBlock: '0.875rem',
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
  },
  renderPushStatisticsSpan: {
    display: 'flex',
    minWidth: '0px',
    alignItems: 'center',
    gap: '0.5rem',
  },
  renderPushStatisticsSpan2: {
    fontWeight: 700,
    wordBreak: 'break-all',
  },
});
