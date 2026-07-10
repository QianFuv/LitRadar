'use client';

import { useQuery } from '@tanstack/react-query';
import { Database, Key, Shield, Ticket, Users } from 'lucide-react';

import { adminGetStats } from '@/lib/api';
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';

const NUMBER_FORMATTER = new Intl.NumberFormat('zh-CN');

/**
 * Render one aggregate statistic.
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
    <div className="rounded-lg border bg-card p-4">
      <div className="flex items-center gap-2 text-sm text-muted-foreground">
        {icon}
        {label}
      </div>
      <div className="mt-1 text-2xl font-bold">{value}</div>
    </div>
  );
}

/**
 * Render administrator aggregate and database statistics.
 *
 * @param props - Whether administrator queries may run.
 * @returns System overview card.
 */
export function AdminOverviewCard({ isEnabled }: { isEnabled: boolean }) {
  const { data: stats, isLoading: statsLoading } = useQuery({
    queryKey: ['admin-stats'],
    queryFn: () => adminGetStats(),
    enabled: isEnabled,
  });
  const authStats = stats?.auth;
  const indexStats = stats?.index;
  const pushStats = stats?.push;

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
          <div className="space-y-4">
            <div className="grid grid-cols-2 sm:grid-cols-3 lg:grid-cols-4 gap-3">
              <StatCard
                label="用户总数"
                value={authStats?.total_users ?? 0}
                icon={<Users className="h-4 w-4" />}
              />
              <StatCard
                label="管理员"
                value={authStats?.admin_count ?? 0}
                icon={<Shield className="h-4 w-4" />}
              />
              <StatCard label="收藏夹" value={authStats?.total_folders ?? 0} />
              <StatCard label="收藏文章" value={authStats?.total_favorites ?? 0} />
              <StatCard
                label="活跃令牌"
                value={authStats?.active_tokens ?? 0}
                icon={<Key className="h-4 w-4" />}
              />
              <StatCard label="推送订阅" value={authStats?.notification_subscribers ?? 0} />
              <StatCard
                label="邀请码 (未使用)"
                value={authStats?.unused_invite_codes ?? 0}
                icon={<Ticket className="h-4 w-4" />}
              />
              <StatCard label="邀请码 (已使用)" value={authStats?.used_invite_codes ?? 0} />
            </div>

            {/* Index stats */}
            {indexStats && (
              <div className="space-y-2">
                <h3 className="text-sm font-medium">
                  索引数据库
                  <span className="ml-2 text-muted-foreground font-normal">
                    共 {NUMBER_FORMATTER.format(indexStats.total_articles)} 篇文章，
                    {NUMBER_FORMATTER.format(indexStats.total_journals)} 本期刊
                  </span>
                </h3>
                <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-2">
                  {indexStats.databases.map((db) => (
                    <div key={db.db_name} className="rounded-md border px-3 py-2 text-sm">
                      <div className="flex items-center gap-1.5 font-medium">
                        <Database className="h-3.5 w-3.5" />
                        {db.db_name}
                      </div>
                      <div className="text-muted-foreground mt-0.5">
                        {NUMBER_FORMATTER.format(db.articles)} 文章 ·{' '}
                        {NUMBER_FORMATTER.format(db.journals)} 期刊 ·{' '}
                        {NUMBER_FORMATTER.format(db.issues)} 期
                      </div>
                    </div>
                  ))}
                </div>
              </div>
            )}

            {/* Push stats */}
            {pushStats && pushStats.length > 0 && (
              <div className="space-y-2">
                <h3 className="text-sm font-medium">推送状态</h3>
                <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-2">
                  {pushStats.map((ps) => (
                    <div key={ps.db_name} className="rounded-md border px-3 py-2 text-sm">
                      <div className="font-medium">{ps.db_name}</div>
                      <div className="text-muted-foreground">
                        状态: {ps.status}
                        {ps.delivered_count != null && ` · 已推送 ${ps.delivered_count} 篇`}
                        {ps.last_completed && (
                          <span className="block">最近完成: {ps.last_completed}</span>
                        )}
                      </div>
                    </div>
                  ))}
                </div>
              </div>
            )}
          </div>
        )}
      </CardContent>
    </Card>
  );
}
