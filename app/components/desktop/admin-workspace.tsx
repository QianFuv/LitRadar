'use client';

/**
 * Desktop administration workspace.
 */

import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import {
  Bell,
  Copy,
  Database,
  KeyRound,
  Plus,
  RefreshCw,
  Save,
  Shield,
  ShieldOff,
  Trash2,
  Users,
} from 'lucide-react';
import Link from 'next/link';
import { useState } from 'react';
import { ShellConfigurator } from '@/components/desktop/shell';
import {
  Badge,
  Button,
  EmptyState,
  Field,
  IconButton,
  Modal,
  Notice,
  Panel,
  SelectInput,
  SwitchRow,
  TextArea,
  TextInput,
} from '@/components/desktop/ui';
import {
  adminCreateAnnouncement,
  adminCreateInviteCode,
  adminCreateScheduledTask,
  adminDeleteAnnouncement,
  adminDeleteInviteCode,
  adminDeleteScheduledTask,
  adminDeleteUser,
  adminGetAnnouncements,
  adminGetInviteCodes,
  adminGetRuntimeSettings,
  adminGetScheduledTasks,
  adminGetStats,
  adminGetUsers,
  adminResetPassword,
  adminSetAdmin,
  adminUpdateAnnouncement,
  adminUpdateRuntimeSettings,
  adminUpdateScheduledTask,
  type AdminUserInfo,
  type AnnouncementCreate,
  type AnnouncementInfo,
  type ScheduledTaskCreate,
  type ScheduledTaskInfo,
} from '@/lib/client-api';
import { useAuthSession } from '@/lib/auth-session';
import { formatCount, formatTimestamp } from '@/lib/format';

type AdminTab = 'overview' | 'users' | 'invites' | 'runtime' | 'tasks' | 'announcements';

interface TaskFormState {
  name: string;
  command: string;
  cron: string;
  enabled: boolean;
}

interface AnnouncementFormState {
  title: string;
  message: string;
  priority: AnnouncementCreate['priority'];
  enabled: boolean;
}

const ADMIN_TABS: Array<{ value: AdminTab; label: string }> = [
  { label: '系统概览', value: 'overview' },
  { label: '账号管理', value: 'users' },
  { label: '邀请码', value: 'invites' },
  { label: '运行配置', value: 'runtime' },
  { label: '定时任务', value: 'tasks' },
  { label: '公告', value: 'announcements' },
];

const EMPTY_TASK_FORM: TaskFormState = {
  command: '',
  cron: '',
  enabled: true,
  name: '',
};

const EMPTY_ANNOUNCEMENT_FORM: AnnouncementFormState = {
  enabled: true,
  message: '',
  priority: 'normal',
  title: '',
};

/**
 * Copy text to the clipboard.
 *
 * @param value - Value to copy.
 */
async function copyText(value: string): Promise<void> {
  await navigator.clipboard.writeText(value);
}

/**
 * Render a stat tile.
 *
 * @param props - Stat tile props.
 * @returns Stat tile.
 */
function StatTile({
  icon,
  label,
  value,
}: {
  icon?: React.ReactNode;
  label: string;
  value: string | number;
}) {
  return (
    <div className="stat-tile">
      <div className="stat-tile__label">
        <span className="toolbar">
          {icon}
          {label}
        </span>
      </div>
      <div className="stat-tile__value">{value}</div>
    </div>
  );
}

/**
 * Convert a task form into an API payload.
 *
 * @param form - Task form state.
 * @returns Scheduled task payload.
 */
function buildTaskPayload(form: TaskFormState): ScheduledTaskCreate {
  return {
    command: form.command.trim(),
    cron: form.cron.trim(),
    enabled: form.enabled,
    name: form.name.trim(),
  };
}

/**
 * Convert an announcement form into an API payload.
 *
 * @param form - Announcement form state.
 * @returns Announcement payload.
 */
function buildAnnouncementPayload(form: AnnouncementFormState): AnnouncementCreate {
  return {
    enabled: form.enabled,
    message: form.message.trim(),
    priority: form.priority,
    title: form.title.trim(),
  };
}

/**
 * Render the admin overview panel.
 *
 * @param props - Overview props.
 * @returns Overview panel.
 */
function OverviewPanel({
  statsPending,
  stats,
}: {
  statsPending: boolean;
  stats: Awaited<ReturnType<typeof adminGetStats>> | undefined;
}) {
  if (statsPending) {
    return (
      <Panel title="系统概览">
        <Notice>正在加载统计信息...</Notice>
      </Panel>
    );
  }

  return (
    <div className="list-stack">
      <Panel title="系统概览" meta="账号、索引、推送">
        <div className="stat-grid">
          <StatTile
            icon={<Users size={15} />}
            label="用户总数"
            value={stats?.auth.total_users ?? 0}
          />
          <StatTile
            icon={<Shield size={15} />}
            label="管理员"
            value={stats?.auth.admin_count ?? 0}
          />
          <StatTile
            icon={<KeyRound size={15} />}
            label="活跃令牌"
            value={stats?.auth.active_tokens ?? 0}
          />
          <StatTile
            icon={<Bell size={15} />}
            label="订阅用户"
            value={stats?.auth.notification_subscribers ?? 0}
          />
          <StatTile label="收藏夹" value={stats?.auth.total_folders ?? 0} />
          <StatTile label="收藏文章" value={stats?.auth.total_favorites ?? 0} />
          <StatTile label="未使用邀请码" value={stats?.auth.unused_invite_codes ?? 0} />
          <StatTile label="活动公告" value={stats?.auth.active_announcements ?? 0} />
        </div>
      </Panel>
      <Panel
        title="索引数据库"
        meta={`总文章 ${formatCount(stats?.index.total_articles)} · 总期刊 ${formatCount(stats?.index.total_journals)}`}
      >
        <div className="table-wrap">
          <table className="data-table">
            <thead>
              <tr>
                <th>数据库</th>
                <th>文章</th>
                <th>期刊</th>
                <th>期数</th>
              </tr>
            </thead>
            <tbody>
              {stats?.index.databases.map((database) => (
                <tr key={database.db_name}>
                  <td>
                    <span className="toolbar">
                      <Database size={15} />
                      {database.db_name}
                    </span>
                  </td>
                  <td>{formatCount(database.articles)}</td>
                  <td>{formatCount(database.journals)}</td>
                  <td>{formatCount(database.issues)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </Panel>
      <Panel title="推送状态">
        <div className="table-wrap">
          <table className="data-table">
            <thead>
              <tr>
                <th>数据库</th>
                <th>状态</th>
                <th>已推送</th>
                <th>最近完成</th>
              </tr>
            </thead>
            <tbody>
              {stats?.push.map((push) => (
                <tr key={push.db_name}>
                  <td>{push.db_name}</td>
                  <td>
                    <Badge tone={push.status === 'completed' ? 'teal' : 'neutral'}>
                      {push.status}
                    </Badge>
                  </td>
                  <td>{formatCount(push.delivered_count)}</td>
                  <td>{push.last_completed || '从未'}</td>
                </tr>
              ))}
              {stats?.push.length === 0 ? (
                <tr>
                  <td colSpan={4}>暂无推送记录</td>
                </tr>
              ) : null}
            </tbody>
          </table>
        </div>
      </Panel>
    </div>
  );
}

/**
 * Render user administration.
 *
 * @param props - User panel props.
 * @returns User panel.
 */
function UsersPanel({
  currentUserId,
  onDelete,
  onReset,
  onToggleAdmin,
  users,
}: {
  currentUserId: number | undefined;
  users: AdminUserInfo[];
  onToggleAdmin: (userInfo: AdminUserInfo) => void;
  onReset: (userInfo: AdminUserInfo) => void;
  onDelete: (userInfo: AdminUserInfo) => void;
}) {
  return (
    <Panel title="账号管理" meta="角色、密码、账号数据">
      <div className="table-wrap">
        <table className="data-table">
          <thead>
            <tr>
              <th>用户</th>
              <th>角色</th>
              <th>收藏夹</th>
              <th>收藏</th>
              <th>推送</th>
              <th>注册时间</th>
              <th>操作</th>
            </tr>
          </thead>
          <tbody>
            {users.map((userInfo) => (
              <tr key={userInfo.id}>
                <td>
                  <strong>{userInfo.username}</strong>
                  <div className="panel__meta">#{userInfo.id}</div>
                </td>
                <td>
                  <Badge tone={userInfo.is_admin ? 'violet' : 'neutral'}>
                    {userInfo.is_admin ? '管理员' : '用户'}
                  </Badge>
                </td>
                <td>{formatCount(userInfo.folder_count)}</td>
                <td>{formatCount(userInfo.favorite_count)}</td>
                <td>
                  {userInfo.notify_enabled ? (
                    <Badge tone="teal">已订阅</Badge>
                  ) : (
                    <span className="panel__meta">未订阅</span>
                  )}
                </td>
                <td>{formatTimestamp(userInfo.created_at)}</td>
                <td>
                  <div className="toolbar">
                    <IconButton
                      aria-label={userInfo.is_admin ? '取消管理员' : '设为管理员'}
                      disabled={userInfo.id === currentUserId}
                      title={userInfo.is_admin ? '取消管理员' : '设为管理员'}
                      onClick={() => onToggleAdmin(userInfo)}
                    >
                      {userInfo.is_admin ? <ShieldOff size={15} /> : <Shield size={15} />}
                    </IconButton>
                    <IconButton
                      aria-label="重置密码"
                      title="重置密码"
                      onClick={() => onReset(userInfo)}
                    >
                      <RefreshCw size={15} />
                    </IconButton>
                    <IconButton
                      danger
                      aria-label="删除用户"
                      disabled={userInfo.id === currentUserId}
                      title="删除用户"
                      onClick={() => onDelete(userInfo)}
                    >
                      <Trash2 size={15} />
                    </IconButton>
                  </div>
                </td>
              </tr>
            ))}
            {users.length === 0 ? (
              <tr>
                <td colSpan={7}>暂无用户</td>
              </tr>
            ) : null}
          </tbody>
        </table>
      </div>
    </Panel>
  );
}

/**
 * Render the admin workspace.
 *
 * @returns Admin workspace.
 */
export function AdminWorkspace() {
  const { token, user } = useAuthSession();
  const queryClient = useQueryClient();
  const [activeTab, setActiveTab] = useState<AdminTab>('overview');
  const [resetTarget, setResetTarget] = useState<AdminUserInfo | null>(null);
  const [resetPassword, setResetPassword] = useState('');
  const [deleteTarget, setDeleteTarget] = useState<AdminUserInfo | null>(null);
  const [runtimeEdits, setRuntimeEdits] = useState<Record<string, string>>({});
  const [taskTarget, setTaskTarget] = useState<ScheduledTaskInfo | null>(null);
  const [taskForm, setTaskForm] = useState<TaskFormState>(EMPTY_TASK_FORM);
  const [taskModalOpen, setTaskModalOpen] = useState(false);
  const [announcementTarget, setAnnouncementTarget] = useState<AnnouncementInfo | null>(null);
  const [announcementForm, setAnnouncementForm] =
    useState<AnnouncementFormState>(EMPTY_ANNOUNCEMENT_FORM);
  const [announcementModalOpen, setAnnouncementModalOpen] = useState(false);
  const [feedback, setFeedback] = useState<string | null>(null);

  const statsQuery = useQuery({
    queryKey: ['admin-stats'],
    queryFn: () => adminGetStats(token!),
    enabled: Boolean(token && user?.is_admin),
  });

  const usersQuery = useQuery({
    queryKey: ['admin-users'],
    queryFn: () => adminGetUsers(token!),
    enabled: Boolean(token && user?.is_admin),
  });

  const inviteCodesQuery = useQuery({
    queryKey: ['admin-invite-codes'],
    queryFn: () => adminGetInviteCodes(token!),
    enabled: Boolean(token && user?.is_admin),
  });

  const runtimeSettingsQuery = useQuery({
    queryKey: ['admin-runtime-settings'],
    queryFn: () => adminGetRuntimeSettings(token!),
    enabled: Boolean(token && user?.is_admin),
  });

  const scheduledTasksQuery = useQuery({
    queryKey: ['admin-scheduled-tasks'],
    queryFn: () => adminGetScheduledTasks(token!),
    enabled: Boolean(token && user?.is_admin),
  });

  const announcementsQuery = useQuery({
    queryKey: ['admin-announcements'],
    queryFn: () => adminGetAnnouncements(token!),
    enabled: Boolean(token && user?.is_admin),
  });

  const invalidateAdminQueries = () => {
    void queryClient.invalidateQueries({ queryKey: ['admin-stats'] });
    void queryClient.invalidateQueries({ queryKey: ['admin-users'] });
    void queryClient.invalidateQueries({ queryKey: ['admin-invite-codes'] });
    void queryClient.invalidateQueries({ queryKey: ['admin-runtime-settings'] });
    void queryClient.invalidateQueries({ queryKey: ['admin-scheduled-tasks'] });
    void queryClient.invalidateQueries({ queryKey: ['admin-announcements'] });
  };

  const toggleAdminMutation = useMutation({
    mutationFn: (target: AdminUserInfo) => adminSetAdmin(token!, target.id, !target.is_admin),
    onSuccess: invalidateAdminQueries,
  });

  const resetPasswordMutation = useMutation({
    mutationFn: () => adminResetPassword(token!, resetTarget!.id, resetPassword),
    onSuccess: () => {
      setResetTarget(null);
      setResetPassword('');
      setFeedback('密码已重置');
    },
  });

  const deleteUserMutation = useMutation({
    mutationFn: () => adminDeleteUser(token!, deleteTarget!.id),
    onSuccess: () => {
      setDeleteTarget(null);
      invalidateAdminQueries();
    },
  });

  const createInviteMutation = useMutation({
    mutationFn: () => adminCreateInviteCode(token!),
    onSuccess: (data) => {
      invalidateAdminQueries();
      setFeedback(`邀请码 ${data.code} 已生成`);
    },
  });

  const deleteInviteMutation = useMutation({
    mutationFn: (codeId: number) => adminDeleteInviteCode(token!, codeId),
    onSuccess: invalidateAdminQueries,
  });

  const saveRuntimeMutation = useMutation({
    mutationFn: () =>
      adminUpdateRuntimeSettings(token!, {
        values: Object.fromEntries(
          (runtimeSettingsQuery.data ?? []).map((setting) => [
            setting.field,
            runtimeEdits[setting.field] ?? setting.value ?? '',
          ]),
        ),
      }),
    onSuccess: (settings) => {
      queryClient.setQueryData(['admin-runtime-settings'], settings);
      setRuntimeEdits({});
      setFeedback('运行配置已保存');
    },
  });

  const saveTaskMutation = useMutation({
    mutationFn: () =>
      taskTarget
        ? adminUpdateScheduledTask(token!, taskTarget.id, buildTaskPayload(taskForm))
        : adminCreateScheduledTask(token!, buildTaskPayload(taskForm)),
    onSuccess: () => {
      setTaskModalOpen(false);
      setTaskTarget(null);
      setTaskForm(EMPTY_TASK_FORM);
      invalidateAdminQueries();
    },
  });

  const deleteTaskMutation = useMutation({
    mutationFn: (taskId: number) => adminDeleteScheduledTask(token!, taskId),
    onSuccess: invalidateAdminQueries,
  });

  const toggleTaskMutation = useMutation({
    mutationFn: (task: ScheduledTaskInfo) =>
      adminUpdateScheduledTask(token!, task.id, { enabled: !task.enabled }),
    onSuccess: invalidateAdminQueries,
  });

  const saveAnnouncementMutation = useMutation({
    mutationFn: () =>
      announcementTarget
        ? adminUpdateAnnouncement(
            token!,
            announcementTarget.id,
            buildAnnouncementPayload(announcementForm),
          )
        : adminCreateAnnouncement(token!, buildAnnouncementPayload(announcementForm)),
    onSuccess: () => {
      setAnnouncementModalOpen(false);
      setAnnouncementTarget(null);
      setAnnouncementForm(EMPTY_ANNOUNCEMENT_FORM);
      invalidateAdminQueries();
    },
  });

  const deleteAnnouncementMutation = useMutation({
    mutationFn: (announcementId: number) => adminDeleteAnnouncement(token!, announcementId),
    onSuccess: invalidateAdminQueries,
  });

  const toggleAnnouncementMutation = useMutation({
    mutationFn: (announcement: AnnouncementInfo) =>
      adminUpdateAnnouncement(token!, announcement.id, { enabled: !announcement.enabled }),
    onSuccess: invalidateAdminQueries,
  });

  const openTaskModal = (task?: ScheduledTaskInfo) => {
    setTaskTarget(task ?? null);
    setTaskForm(
      task
        ? { command: task.command, cron: task.cron, enabled: task.enabled, name: task.name }
        : EMPTY_TASK_FORM,
    );
    setTaskModalOpen(true);
  };

  const openAnnouncementModal = (announcement?: AnnouncementInfo) => {
    setAnnouncementTarget(announcement ?? null);
    setAnnouncementForm(
      announcement
        ? {
            enabled: announcement.enabled,
            message: announcement.message,
            priority: announcement.priority,
            title: announcement.title,
          }
        : EMPTY_ANNOUNCEMENT_FORM,
    );
    setAnnouncementModalOpen(true);
  };

  if (!user?.is_admin) {
    return (
      <>
        <ShellConfigurator title="管理面板" />
        <Panel title="权限不足">
          <div className="form-grid">
            <EmptyState>当前账号没有管理员权限。</EmptyState>
            <Link href="/">
              <Button variant="secondary">返回搜索工作台</Button>
            </Link>
          </div>
        </Panel>
      </>
    );
  }

  return (
    <>
      <ShellConfigurator
        title="管理面板"
        actions={
          <>
            <Badge tone="violet">管理员</Badge>
            {feedback ? <Badge tone="teal">{feedback}</Badge> : null}
          </>
        }
      />
      <div className="workspace-grid workspace-grid--two">
        <Panel title="管理导航" meta="选择一个维护区域">
          <div className="list-stack">
            {ADMIN_TABS.map((tab) => (
              <Button
                key={tab.value}
                variant={activeTab === tab.value ? 'primary' : 'secondary'}
                onClick={() => setActiveTab(tab.value)}
                wide
              >
                {tab.label}
              </Button>
            ))}
          </div>
        </Panel>

        <div className="list-stack">
          {activeTab === 'overview' ? (
            <OverviewPanel stats={statsQuery.data} statsPending={statsQuery.isPending} />
          ) : null}

          {activeTab === 'users' ? (
            <UsersPanel
              currentUserId={user.id}
              users={usersQuery.data ?? []}
              onToggleAdmin={(target) => toggleAdminMutation.mutate(target)}
              onReset={(target) => {
                setResetTarget(target);
                setResetPassword('');
              }}
              onDelete={(target) => setDeleteTarget(target)}
            />
          ) : null}

          {activeTab === 'invites' ? (
            <Panel
              title="邀请码管理"
              actions={
                <Button icon={<Plus size={15} />} onClick={() => createInviteMutation.mutate()}>
                  生成邀请码
                </Button>
              }
            >
              <div className="table-wrap">
                <table className="data-table">
                  <thead>
                    <tr>
                      <th>邀请码</th>
                      <th>状态</th>
                      <th>创建者</th>
                      <th>使用者</th>
                      <th>创建时间</th>
                      <th>操作</th>
                    </tr>
                  </thead>
                  <tbody>
                    {(inviteCodesQuery.data ?? []).map((inviteCode) => (
                      <tr key={inviteCode.id}>
                        <td>
                          <code>{inviteCode.code}</code>
                        </td>
                        <td>
                          <Badge tone={inviteCode.used_by ? 'neutral' : 'teal'}>
                            {inviteCode.used_by ? '已使用' : '可用'}
                          </Badge>
                        </td>
                        <td>{inviteCode.created_by_name || '系统'}</td>
                        <td>{inviteCode.used_by_name || '—'}</td>
                        <td>{formatTimestamp(inviteCode.created_at)}</td>
                        <td>
                          <div className="toolbar">
                            <IconButton
                              aria-label="复制邀请码"
                              title="复制邀请码"
                              onClick={() => void copyText(inviteCode.code)}
                            >
                              <Copy size={15} />
                            </IconButton>
                            {!inviteCode.used_by ? (
                              <IconButton
                                danger
                                aria-label="删除邀请码"
                                title="删除邀请码"
                                onClick={() => deleteInviteMutation.mutate(inviteCode.id)}
                              >
                                <Trash2 size={15} />
                              </IconButton>
                            ) : null}
                          </div>
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            </Panel>
          ) : null}

          {activeTab === 'runtime' ? (
            <Panel
              title="运行配置"
              meta="保存后立即写入运行配置"
              actions={
                <Button
                  icon={<Save size={15} />}
                  disabled={saveRuntimeMutation.isPending}
                  onClick={() => saveRuntimeMutation.mutate()}
                >
                  保存配置
                </Button>
              }
            >
              <div className="form-grid">
                {(runtimeSettingsQuery.data ?? []).map((setting) =>
                  setting.input_type === 'boolean' ? (
                    <SwitchRow
                      key={setting.field}
                      checked={(runtimeEdits[setting.field] ?? setting.value) === 'true'}
                      detail={setting.description}
                      label={setting.label}
                      onChange={(event) =>
                        setRuntimeEdits((current) => ({
                          ...current,
                          [setting.field]: String(event.currentTarget.checked),
                        }))
                      }
                    />
                  ) : (
                    <Field key={setting.field} label={`${setting.label} · ${setting.source}`}>
                      <TextInput
                        type={
                          setting.input_type === 'password' || setting.is_secret
                            ? 'password'
                            : 'text'
                        }
                        value={runtimeEdits[setting.field] ?? setting.value ?? ''}
                        onChange={(event) =>
                          setRuntimeEdits((current) => ({
                            ...current,
                            [setting.field]: event.target.value,
                          }))
                        }
                        placeholder={setting.description}
                      />
                    </Field>
                  ),
                )}
              </div>
            </Panel>
          ) : null}

          {activeTab === 'tasks' ? (
            <Panel
              title="定时任务"
              actions={
                <Button icon={<Plus size={15} />} onClick={() => openTaskModal()}>
                  新建任务
                </Button>
              }
            >
              <div className="table-wrap">
                <table className="data-table">
                  <thead>
                    <tr>
                      <th>名称</th>
                      <th>命令</th>
                      <th>Cron</th>
                      <th>状态</th>
                      <th>上次运行</th>
                      <th>操作</th>
                    </tr>
                  </thead>
                  <tbody>
                    {(scheduledTasksQuery.data ?? []).map((task) => (
                      <tr key={task.id}>
                        <td>{task.name}</td>
                        <td>
                          <code>{task.command}</code>
                        </td>
                        <td>{task.cron}</td>
                        <td>
                          <Badge tone={task.enabled ? 'teal' : 'neutral'}>
                            {task.enabled ? '启用' : '停用'}
                          </Badge>
                        </td>
                        <td>{formatTimestamp(task.last_run_at)}</td>
                        <td>
                          <div className="toolbar">
                            <Button
                              size="small"
                              variant="secondary"
                              onClick={() => openTaskModal(task)}
                            >
                              编辑
                            </Button>
                            <Button
                              size="small"
                              variant="secondary"
                              onClick={() => toggleTaskMutation.mutate(task)}
                            >
                              {task.enabled ? '停用' : '启用'}
                            </Button>
                            <IconButton
                              danger
                              aria-label="删除任务"
                              title="删除任务"
                              onClick={() => deleteTaskMutation.mutate(task.id)}
                            >
                              <Trash2 size={15} />
                            </IconButton>
                          </div>
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            </Panel>
          ) : null}

          {activeTab === 'announcements' ? (
            <Panel
              title="公告"
              actions={
                <Button icon={<Plus size={15} />} onClick={() => openAnnouncementModal()}>
                  新建公告
                </Button>
              }
            >
              <div className="list-stack">
                {(announcementsQuery.data ?? []).map((announcement) => (
                  <div key={announcement.id} className="article-row">
                    <div className="toolbar toolbar--wrap">
                      <strong>{announcement.title}</strong>
                      <Badge
                        tone={
                          announcement.priority === 'high'
                            ? 'coral'
                            : announcement.priority === 'normal'
                              ? 'violet'
                              : 'neutral'
                        }
                      >
                        {announcement.priority}
                      </Badge>
                      <Badge tone={announcement.enabled ? 'teal' : 'neutral'}>
                        {announcement.enabled ? '启用' : '停用'}
                      </Badge>
                    </div>
                    <p className="article-row__abstract">{announcement.message}</p>
                    <div className="toolbar">
                      <Button
                        size="small"
                        variant="secondary"
                        onClick={() => openAnnouncementModal(announcement)}
                      >
                        编辑
                      </Button>
                      <Button
                        size="small"
                        variant="secondary"
                        onClick={() => toggleAnnouncementMutation.mutate(announcement)}
                      >
                        {announcement.enabled ? '停用' : '启用'}
                      </Button>
                      <IconButton
                        danger
                        aria-label="删除公告"
                        title="删除公告"
                        onClick={() => deleteAnnouncementMutation.mutate(announcement.id)}
                      >
                        <Trash2 size={15} />
                      </IconButton>
                    </div>
                  </div>
                ))}
              </div>
            </Panel>
          ) : null}
        </div>
      </div>

      <Modal
        narrow
        open={Boolean(resetTarget)}
        title="重置密码"
        description={resetTarget ? `用户：${resetTarget.username}` : undefined}
        onClose={() => setResetTarget(null)}
        footer={
          <>
            <Button variant="secondary" onClick={() => setResetTarget(null)}>
              取消
            </Button>
            <Button
              disabled={resetPassword.length < 6 || resetPasswordMutation.isPending}
              onClick={() => resetPasswordMutation.mutate()}
            >
              确认重置
            </Button>
          </>
        }
      >
        <Field label="新密码">
          <TextInput
            type="password"
            value={resetPassword}
            onChange={(event) => setResetPassword(event.target.value)}
          />
        </Field>
      </Modal>

      <Modal
        narrow
        open={Boolean(deleteTarget)}
        title="删除用户"
        description={deleteTarget ? `将删除 ${deleteTarget.username} 及其数据。` : undefined}
        onClose={() => setDeleteTarget(null)}
        footer={
          <>
            <Button variant="secondary" onClick={() => setDeleteTarget(null)}>
              取消
            </Button>
            <Button
              variant="danger"
              disabled={deleteUserMutation.isPending}
              onClick={() => deleteUserMutation.mutate()}
            >
              确认删除
            </Button>
          </>
        }
      >
        <Notice tone="error">此操作不可恢复。</Notice>
      </Modal>

      <Modal
        open={taskModalOpen}
        title={taskTarget ? '编辑定时任务' : '新建定时任务'}
        onClose={() => setTaskModalOpen(false)}
        footer={
          <>
            <Button variant="secondary" onClick={() => setTaskModalOpen(false)}>
              取消
            </Button>
            <Button
              disabled={
                !taskForm.name.trim() || !taskForm.command.trim() || saveTaskMutation.isPending
              }
              onClick={() => saveTaskMutation.mutate()}
            >
              保存任务
            </Button>
          </>
        }
      >
        <div className="form-grid">
          <Field label="名称">
            <TextInput
              value={taskForm.name}
              onChange={(event) =>
                setTaskForm((current) => ({ ...current, name: event.target.value }))
              }
            />
          </Field>
          <Field label="命令">
            <TextInput
              value={taskForm.command}
              onChange={(event) =>
                setTaskForm((current) => ({ ...current, command: event.target.value }))
              }
            />
          </Field>
          <Field label="Cron">
            <TextInput
              value={taskForm.cron}
              onChange={(event) =>
                setTaskForm((current) => ({ ...current, cron: event.target.value }))
              }
            />
          </Field>
          <SwitchRow
            checked={taskForm.enabled}
            label="启用任务"
            onChange={(event) =>
              setTaskForm((current) => ({ ...current, enabled: event.currentTarget.checked }))
            }
          />
        </div>
      </Modal>

      <Modal
        open={announcementModalOpen}
        title={announcementTarget ? '编辑公告' : '新建公告'}
        onClose={() => setAnnouncementModalOpen(false)}
        footer={
          <>
            <Button variant="secondary" onClick={() => setAnnouncementModalOpen(false)}>
              取消
            </Button>
            <Button
              disabled={
                !announcementForm.title.trim() ||
                !announcementForm.message.trim() ||
                saveAnnouncementMutation.isPending
              }
              onClick={() => saveAnnouncementMutation.mutate()}
            >
              保存公告
            </Button>
          </>
        }
      >
        <div className="form-grid">
          <Field label="标题">
            <TextInput
              value={announcementForm.title}
              onChange={(event) =>
                setAnnouncementForm((current) => ({ ...current, title: event.target.value }))
              }
            />
          </Field>
          <Field label="优先级">
            <SelectInput
              value={announcementForm.priority}
              onChange={(event) =>
                setAnnouncementForm((current) => ({
                  ...current,
                  priority: event.target.value as AnnouncementCreate['priority'],
                }))
              }
            >
              <option value="high">high</option>
              <option value="normal">normal</option>
              <option value="low">low</option>
            </SelectInput>
          </Field>
          <Field label="内容">
            <TextArea
              value={announcementForm.message}
              onChange={(event) =>
                setAnnouncementForm((current) => ({ ...current, message: event.target.value }))
              }
            />
          </Field>
          <SwitchRow
            checked={announcementForm.enabled}
            label="启用公告"
            onChange={(event) =>
              setAnnouncementForm((current) => ({
                ...current,
                enabled: event.currentTarget.checked,
              }))
            }
          />
        </div>
      </Modal>
    </>
  );
}
