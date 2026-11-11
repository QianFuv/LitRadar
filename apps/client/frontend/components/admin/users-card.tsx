import { useMemo, useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { KeyRound, ShieldCheck, Trash2, Users, ShieldOff } from '@/components/ui/animated-icons';

import {
  adminDeleteUser,
  adminResetPassword,
  adminSetAdmin,
  adminStatsQuery,
  adminUsersQuery,
} from '@/lib/api';
import { AdminListHeader } from '@/components/admin/admin-list-header';
import {
  SettingsGroup,
  StatusPill,
  settingsGroupItemStyle,
} from '@/components/settings/settings-section';
import { UserAvatar } from '@/components/shell/user-avatar';
import { Button } from '@/components/ui/button';
import { IconTooltip } from '@/components/ui/tooltip';
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import { Input } from '@/components/ui/input';
import { RevealItem } from '@/components/ui/reveal';
import { LoadingText } from '@/components/ui/loading-text';
import * as stylex from '@stylexjs/stylex';
import { colors, lineHeights } from '@/lib/theme.stylex';

/**
 * Format a Unix timestamp for administrator tables.
 *
 * @param ts - Unix timestamp in seconds.
 * @returns Localized date and time.
 */
function formatDate(ts: number): string {
  return new Date(ts * 1000).toLocaleDateString('zh-CN', {
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
  });
}

/**
 * Render administrator user management and confirmation dialogs.
 *
 * @param props - Current administrator id and query enablement.
 * @returns User management card and dialogs.
 */
export function AdminUsersCard({
  currentUserId,
  isEnabled,
}: {
  currentUserId: number;
  isEnabled: boolean;
}) {
  const queryClient = useQueryClient();
  const [resetPwUserId, setResetPwUserId] = useState<number | null>(null);
  const [resetPwValue, setResetPwValue] = useState('');
  const [resetDialogOpen, setResetDialogOpen] = useState(false);
  const [deleteUserId, setDeleteUserId] = useState<number | null>(null);
  const [deleteDialogOpen, setDeleteDialogOpen] = useState(false);
  const {
    data: users,
    error: usersError,
    isLoading: isUsersLoading,
  } = useQuery({
    ...adminUsersQuery(),
    enabled: isEnabled,
  });
  const userRows = useMemo(
    () => (users ?? []).map((user) => ({ user, registrationDate: formatDate(user.created_at) })),
    [users],
  );
  const toggleAdminMut = useMutation({
    mutationFn: ({ userId, isAdmin }: { userId: number; isAdmin: boolean }) =>
      adminSetAdmin(userId, isAdmin),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: adminUsersQuery().queryKey });
      void queryClient.invalidateQueries({ queryKey: adminStatsQuery().queryKey });
    },
  });
  const resetPwMut = useMutation({
    mutationFn: ({ userId, pw }: { userId: number; pw: string }) => adminResetPassword(userId, pw),
    onSuccess: () => {
      setResetDialogOpen(false);
      setResetPwValue('');
      setResetPwUserId(null);
    },
  });
  const deleteUserMut = useMutation({
    mutationFn: (userId: number) => adminDeleteUser(userId),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: adminUsersQuery().queryKey });
      void queryClient.invalidateQueries({ queryKey: adminStatsQuery().queryKey });
      setDeleteDialogOpen(false);
      setDeleteUserId(null);
    },
  });

  return (
    <>
      <Card>
        <CardHeader>
          <CardTitle xstyle={styles.adminUsersCardCardTitle}>
            <Users xstyle={styles.adminUsersCardUsers} />
            账号管理
          </CardTitle>
          <CardDescription>查看和管理所有用户账号</CardDescription>
        </CardHeader>
        <CardContent>
          {usersError instanceof Error && (
            <p
              role="alert"
              {...stylex.props(styles.adminUsersCardCardContentItem, styles.adminUsersCardP)}
            >
              {usersError.message}
            </p>
          )}
          {toggleAdminMut.isError && (
            <p
              role="alert"
              {...stylex.props(styles.adminUsersCardCardContentItem, styles.adminUsersCardP)}
            >
              {toggleAdminMut.error instanceof Error
                ? toggleAdminMut.error.message
                : '更新管理员状态失败'}
            </p>
          )}
          <RevealItem
            isReady={!isUsersLoading}
            xstyle={[styles.adminUsersCardCardContentItem, styles.adminUsersCardRevealItem]}
            placeholder={
              <output {...stylex.props(styles.statusOutput, styles.adminUsersCardP2)}>
                <LoadingText text="加载中…" />
              </output>
            }
          >
            {users && (
              <AdminListHeader
                summary={`共 ${users.length} 位用户 · ${users.filter((entry) => entry.is_admin).length} 位管理员`}
              />
            )}
            <SettingsGroup>
              {userRows.map(({ user: u, registrationDate }) => (
                <div key={u.id} {...stylex.props(settingsGroupItemStyle, styles.adminUsersCardDiv)}>
                  <div {...stylex.props(styles.adminUsersCardDiv2)}>
                    <UserAvatar
                      username={u.username}
                      xstyle={u.is_admin ? null : styles.adminUsersCardUserAvatarNotIsAdmin}
                    />
                    <div {...stylex.props(styles.adminUsersCardDiv3)}>
                      <div {...stylex.props(styles.adminUsersCardDiv4)}>
                        <span {...stylex.props(styles.adminUsersCardSpan)}>{u.username}</span>
                        {u.is_admin ? (
                          <StatusPill tone="info">管理员</StatusPill>
                        ) : (
                          <StatusPill>用户</StatusPill>
                        )}
                        {u.auth_kind === 'oidc' ? (
                          <StatusPill tone="success">SSO 账号</StatusPill>
                        ) : (
                          u.is_oidc_linked && <StatusPill>已绑定 SSO</StatusPill>
                        )}
                        {u.id === currentUserId && (
                          <span {...stylex.props(styles.adminUsersCardSpan2)}>（你）</span>
                        )}
                      </div>
                      <div {...stylex.props(styles.adminUsersCardDiv5)}>
                        #{u.id} · <span>注册时间: {registrationDate}</span>
                      </div>
                      <div {...stylex.props(styles.adminUsersCardDiv6)}>
                        {u.email ?? '未设置邮箱'}
                      </div>
                    </div>
                  </div>
                  <dl {...stylex.props(styles.adminUsersCardDl)}>
                    <div {...stylex.props(styles.adminUsersCardDiv7)}>
                      <dt {...stylex.props(styles.adminUsersCardDt)}>收藏夹</dt>
                      <dd {...stylex.props(styles.adminUsersCardDd)}>{u.folder_count}</dd>
                    </div>
                    <div {...stylex.props(styles.adminUsersCardDiv7)}>
                      <dt {...stylex.props(styles.adminUsersCardDt)}>收藏</dt>
                      <dd {...stylex.props(styles.adminUsersCardDd)}>{u.favorite_count}</dd>
                    </div>
                    <div {...stylex.props(styles.adminUsersCardDiv8)}>
                      <dt {...stylex.props(styles.adminUsersCardDt)}>推送</dt>
                      <dd
                        {...stylex.props(
                          u.notify_enabled
                            ? styles.adminUsersCardDdNotifyEnabled
                            : styles.adminUsersCardP2,
                        )}
                      >
                        {u.notify_enabled ? '已订阅' : '未订阅'}
                      </dd>
                    </div>
                  </dl>
                  <div {...stylex.props(styles.adminUsersCardDiv9)}>
                    <IconTooltip label={u.is_admin ? '取消管理员' : '设为管理员'}>
                      <Button
                        variant="ghost"
                        size="icon"
                        xstyle={styles.adminUsersCardButton}
                        aria-label={
                          u.is_admin ? `取消 ${u.username} 的管理员` : `设为 ${u.username} 为管理员`
                        }
                        disabled={toggleAdminMut.isPending || u.id === currentUserId}
                        onClick={() =>
                          toggleAdminMut.mutate({
                            userId: u.id,
                            isAdmin: !u.is_admin,
                          })
                        }
                      >
                        {u.is_admin ? (
                          <ShieldOff xstyle={styles.adminUsersCardShieldOff} />
                        ) : (
                          <ShieldCheck xstyle={styles.adminUsersCardShieldOff} />
                        )}
                      </Button>
                    </IconTooltip>
                    <IconTooltip
                      label={u.auth_kind === 'oidc' ? 'SSO 账号无法重置密码' : '重置密码'}
                    >
                      <Button
                        variant="ghost"
                        size="icon"
                        xstyle={styles.adminUsersCardButton}
                        aria-label={`重置 ${u.username} 的密码`}
                        disabled={u.auth_kind === 'oidc'}
                        onClick={() => {
                          resetPwMut.reset();
                          setResetPwUserId(u.id);
                          setResetPwValue('');
                          setResetDialogOpen(true);
                        }}
                      >
                        <KeyRound xstyle={styles.adminUsersCardShieldOff} />
                      </Button>
                    </IconTooltip>
                    <IconTooltip label="删除用户">
                      <Button
                        variant="ghost"
                        size="icon"
                        aria-label={`删除用户 ${u.username}`}
                        disabled={u.id === currentUserId}
                        xstyle={styles.adminUsersCardButton2}
                        onClick={() => {
                          deleteUserMut.reset();
                          setDeleteUserId(u.id);
                          setDeleteDialogOpen(true);
                        }}
                      >
                        <Trash2 xstyle={styles.adminUsersCardShieldOff} />
                      </Button>
                    </IconTooltip>
                  </div>
                </div>
              ))}
            </SettingsGroup>
          </RevealItem>
        </CardContent>
      </Card>

      {/* Password reset dialog */}
      <Dialog open={resetDialogOpen} onOpenChange={setResetDialogOpen}>
        <DialogContent xstyle={styles.adminUsersCardDialogContent}>
          <DialogHeader>
            <DialogTitle>重置密码</DialogTitle>
            <DialogDescription>为用户 #{resetPwUserId} 设置新密码</DialogDescription>
          </DialogHeader>
          <div {...stylex.props(styles.adminUsersCardDiv10)}>
            <Input
              xstyle={styles.adminUsersCardDivItem}
              name="admin_new_password"
              type="password"
              aria-label="新密码"
              autoComplete="new-password"
              value={resetPwValue}
              onChange={(e) => setResetPwValue(e.target.value)}
              placeholder="新密码 (至少12位)"
              minLength={12}
            />
            <div {...stylex.props(styles.adminUsersCardDivItem, styles.adminUsersCardDiv11)}>
              <Button
                variant="outline"
                xstyle={styles.adminUsersCardButton3}
                onClick={() => setResetDialogOpen(false)}
              >
                取消
              </Button>
              <Button
                xstyle={styles.adminUsersCardButton3}
                disabled={resetPwValue.length < 12 || resetPwMut.isPending}
                onClick={() => {
                  if (resetPwUserId != null) {
                    resetPwMut.mutate({
                      userId: resetPwUserId,
                      pw: resetPwValue,
                    });
                  }
                }}
              >
                确认重置
              </Button>
            </div>
            {resetPwMut.isError && (
              <p
                role="alert"
                {...stylex.props(styles.adminUsersCardDivItem, styles.adminUsersCardP)}
              >
                {resetPwMut.error instanceof Error ? resetPwMut.error.message : '重置失败'}
              </p>
            )}
          </div>
        </DialogContent>
      </Dialog>

      {/* Delete user dialog */}
      <Dialog open={deleteDialogOpen} onOpenChange={setDeleteDialogOpen}>
        <DialogContent xstyle={styles.adminUsersCardDialogContent}>
          <DialogHeader>
            <DialogTitle>确认删除用户</DialogTitle>
            <DialogDescription>
              删除用户 #{deleteUserId} 及其所有数据（收藏夹、收藏、设置）。此操作不可恢复。
            </DialogDescription>
          </DialogHeader>
          <div {...stylex.props(styles.adminUsersCardDiv12)}>
            <Button
              variant="outline"
              xstyle={styles.adminUsersCardButton3}
              onClick={() => setDeleteDialogOpen(false)}
            >
              取消
            </Button>
            <Button
              variant="destructive"
              xstyle={styles.adminUsersCardButton3}
              disabled={deleteUserMut.isPending}
              onClick={() => {
                if (deleteUserId != null) {
                  deleteUserMut.mutate(deleteUserId);
                }
              }}
            >
              确认删除
            </Button>
          </div>
          {deleteUserMut.isError && (
            <p role="alert" {...stylex.props(styles.adminUsersCardP)}>
              {deleteUserMut.error instanceof Error ? deleteUserMut.error.message : '删除失败'}
            </p>
          )}
        </DialogContent>
      </Dialog>
    </>
  );
}

const styles = stylex.create({
  statusOutput: {
    display: 'block',
  },
  adminUsersCardCardTitle: {
    display: 'flex',
    alignItems: 'center',
    gap: '0.5rem',
  },
  adminUsersCardUsers: {
    height: '1.25rem',
    width: '1.25rem',
  },
  adminUsersCardCardContentItem: {
    marginBottom: {
      default: null,
      ':not(:last-child)': '1rem',
    },
  },
  adminUsersCardP: {
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
    color: colors.destructive,
  },
  adminUsersCardRevealItem: {
    display: 'flex',
    flexDirection: 'column',
    rowGap: '1rem',
  },
  adminUsersCardP2: {
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
    color: colors.mutedForeground,
  },
  adminUsersCardDiv: {
    display: 'flex',
    flexDirection: {
      default: 'column',
      '@media (min-width: 48rem)': 'row',
    },
    gap: {
      default: '0.75rem',
      '@media (min-width: 48rem)': '1.5rem',
    },
    paddingInline: {
      default: '1rem',
      '@media (min-width: 40rem)': '1.25rem',
    },
    paddingBlock: '1rem',
    contentVisibility: 'auto',
    containIntrinsicSize: '0 220px',
    alignItems: {
      default: null,
      '@media (min-width: 48rem)': 'center',
    },
  },
  adminUsersCardDiv2: {
    display: 'flex',
    minWidth: '0px',
    flex: '1',
    alignItems: 'center',
    gap: '0.75rem',
  },
  adminUsersCardUserAvatarNotIsAdmin: {
    backgroundColor: colors.muted,
    color: colors.foreground,
  },
  adminUsersCardDiv3: {
    minWidth: '0px',
  },
  adminUsersCardDiv4: {
    display: 'flex',
    flexWrap: 'wrap',
    alignItems: 'center',
    gap: '0.5rem',
  },
  adminUsersCardSpan: {
    overflow: 'hidden',
    textOverflow: 'ellipsis',
    whiteSpace: 'nowrap',
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
    fontWeight: 700,
  },
  adminUsersCardSpan2: {
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    color: colors.mutedForeground,
  },
  adminUsersCardDiv5: {
    marginTop: '0.125rem',
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    color: colors.mutedForeground,
    fontVariantNumeric: 'tabular-nums',
  },
  adminUsersCardDiv6: {
    marginTop: '0.125rem',
    overflow: 'hidden',
    textOverflow: 'ellipsis',
    whiteSpace: 'nowrap',
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    color: colors.mutedForeground,
  },
  adminUsersCardDl: {
    display: {
      default: 'grid',
      '@media (min-width: 48rem)': 'flex',
    },
    gridTemplateColumns: 'repeat(3, minmax(0, 1fr))',
    gap: {
      default: '0.5rem',
      '@media (min-width: 48rem)': '1.5rem',
    },
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
  },
  adminUsersCardDiv7: {
    borderRadius: '8px',
    backgroundColor: {
      default: `color-mix(in oklab, ${colors.muted} 60%, transparent)`,
      '@media (min-width: 48rem)': 'transparent',
    },
    paddingInline: {
      default: '0.75rem',
      '@media (min-width: 48rem)': '0px',
    },
    paddingBlock: {
      default: '0.5rem',
      '@media (min-width: 48rem)': '0px',
    },
    textAlign: {
      default: null,
      '@media (min-width: 48rem)': 'right',
    },
  },
  adminUsersCardDt: {
    color: colors.mutedForeground,
  },
  adminUsersCardDd: {
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
    fontWeight: 700,
    fontVariantNumeric: 'tabular-nums',
  },
  adminUsersCardDiv8: {
    borderRadius: '8px',
    backgroundColor: {
      default: `color-mix(in oklab, ${colors.muted} 60%, transparent)`,
      '@media (min-width: 48rem)': 'transparent',
    },
    paddingInline: {
      default: '0.75rem',
      '@media (min-width: 48rem)': '0px',
    },
    paddingBlock: {
      default: '0.5rem',
      '@media (min-width: 48rem)': '0px',
    },
    width: {
      default: null,
      '@media (min-width: 48rem)': '3.5rem',
    },
    textAlign: {
      default: null,
      '@media (min-width: 48rem)': 'right',
    },
  },
  adminUsersCardDdNotifyEnabled: {
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
    fontWeight: 700,
    color: colors.primary,
  },
  adminUsersCardDiv9: {
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'flex-end',
    gap: '0.25rem',
    borderTopStyle: 'solid',
    borderTopWidth: {
      default: '1px',
      '@media (min-width: 48rem)': '0px',
    },
    paddingTop: {
      default: '0.75rem',
      '@media (min-width: 48rem)': '0px',
    },
  },
  adminUsersCardButton: {
    width: '2.25rem',
    height: '2.25rem',
  },
  adminUsersCardShieldOff: {
    width: '1rem',
    height: '1rem',
  },
  adminUsersCardButton2: {
    width: '2.25rem',
    height: '2.25rem',
    color: {
      default: colors.mutedForeground,
      ':hover': {
        default: null,
        '@media (hover: hover)': colors.destructive,
      },
    },
    backgroundColor: {
      default: null,
      ':active': {
        default: colors.accentPressed,
        ':hover': {
          default: null,
          '@media (hover: hover)': colors.accentPressed,
        },
      },
      ':hover': {
        default: null,
        '@media (hover: hover)': `color-mix(in oklab, ${colors.destructive} 10%, transparent)`,
      },
    },
  },
  adminUsersCardDialogContent: {
    maxHeight: '90vh',
    overflowY: 'auto',
    maxWidth: {
      default: 'none',
      '@media (min-width: 40rem)': '32rem',
      '@media (min-width: 48rem)': '36rem',
    },
  },
  adminUsersCardDivItem: {
    marginBottom: {
      default: null,
      ':not(:last-child)': '0.75rem',
    },
  },
  adminUsersCardDiv10: {
    paddingBlock: '0.5rem',
  },
  adminUsersCardDiv11: {
    display: 'flex',
    flexDirection: {
      default: 'column-reverse',
      '@media (min-width: 40rem)': 'row',
    },
    gap: '0.5rem',
    justifyContent: {
      default: null,
      '@media (min-width: 40rem)': 'flex-end',
    },
  },
  adminUsersCardButton3: {
    width: {
      default: '100%',
      '@media (min-width: 40rem)': 'auto',
    },
  },
  adminUsersCardDiv12: {
    display: 'flex',
    flexDirection: {
      default: 'column-reverse',
      '@media (min-width: 40rem)': 'row',
    },
    gap: '0.5rem',
    paddingBlock: '0.5rem',
    justifyContent: {
      default: null,
      '@media (min-width: 40rem)': 'flex-end',
    },
  },
});
