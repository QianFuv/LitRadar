'use client';

import { useMemo, useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { AlertTriangle, Clock3, Pencil, Plus, Trash2 } from 'lucide-react';

import {
  adminCreateScheduledTask,
  adminDeleteScheduledTask,
  adminGetSchedulerStatus,
  adminGetScheduledTasks,
  adminUpdateScheduledTask,
  type ScheduledJobSpec,
  type ScheduledTaskCreate,
  type ScheduledTaskInfo,
} from '@/lib/api';
import { Button } from '@/components/ui/button';
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from '@/components/ui/dialog';
import { Input } from '@/components/ui/input';
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select';
import { Label } from '@/components/ui/label';
import { Switch } from '@/components/ui/switch';

type TaskFormState = {
  coalesce: boolean;
  cron: string;
  database: string;
  enabled: boolean;
  maxCandidates: string;
  metadataFile: string;
  name: string;
  timeoutSeconds: string;
  timezone: string;
};

type JobPresetId =
  | 'index-update'
  | 'index-update-folder'
  | 'index-update-external'
  | 'index-update-both'
  | 'folder-only'
  | 'external-only';

const JOB_PRESETS: { label: string; value: JobPresetId }[] = [
  { value: 'index-update', label: '索引更新' },
  { value: 'index-update-folder', label: '索引更新 + 文件夹推送' },
  { value: 'index-update-external', label: '索引更新 + 外部推送' },
  { value: 'index-update-both', label: '索引更新 + 双推送' },
  { value: 'folder-only', label: '仅文件夹推送' },
  { value: 'external-only', label: '仅外部推送' },
];

const DEFAULT_FORM: TaskFormState = {
  coalesce: true,
  cron: '0 8 * * *',
  database: '',
  enabled: true,
  maxCandidates: '',
  metadataFile: '',
  name: '',
  timeoutSeconds: '3600',
  timezone: 'UTC',
};

const DEFAULT_PRESET: JobPresetId = 'index-update-external';

/**
 * Resolve the browser's IANA time zone for new task defaults.
 *
 * @returns Browser time zone, or UTC when the runtime does not expose one.
 */
function getBrowserTimeZone(): string {
  return Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC';
}

/**
 * Return whether a preset starts with an index refresh.
 *
 * @param preset - Selected structured job preset.
 * @returns Whether index-only arguments should be displayed.
 */
function isIndexPreset(preset: JobPresetId): boolean {
  return preset.startsWith('index-update');
}

/**
 * Resolve a typed job to its structured UI preset.
 *
 * @param job - Stored typed job, or null for a migrated legacy task.
 * @returns Matching preset identifier.
 */
function getPresetForJob(job: ScheduledJobSpec | null): JobPresetId {
  if (!job) {
    return DEFAULT_PRESET;
  }
  if (job.kind === 'notify') {
    return 'external-only';
  }
  if (job.kind === 'push') {
    return 'folder-only';
  }
  if (job.notify && job.push) {
    return 'index-update-both';
  }
  if (job.push) {
    return 'index-update-folder';
  }
  if (job.notify) {
    return 'index-update-external';
  }
  return 'index-update';
}

/**
 * Build the allowlisted job payload represented by the form.
 *
 * @param form - Structured task form values.
 * @param preset - Selected job preset.
 * @returns Generated API job specification.
 */
function buildScheduledJob(form: TaskFormState, preset: JobPresetId): ScheduledJobSpec {
  if (isIndexPreset(preset)) {
    const metadataFile = form.metadataFile.trim();
    return {
      kind: 'index',
      notify: preset === 'index-update-external' || preset === 'index-update-both',
      push: preset === 'index-update-folder' || preset === 'index-update-both',
      ...(metadataFile ? { metadata_file: metadataFile } : {}),
    };
  }

  const database = form.database.trim();
  const maxCandidates = form.maxCandidates.trim();
  return {
    kind: preset === 'folder-only' ? 'push' : 'notify',
    ...(database ? { database } : {}),
    ...(maxCandidates ? { max_candidates: Number(maxCandidates) } : {}),
  };
}

/**
 * Return whether an optional filename matches the backend basename allowlist.
 *
 * @param value - Candidate filename.
 * @param extension - Required filename extension.
 * @returns Whether the optional value is empty or a safe basename.
 */
function isSafeBasename(value: string, extension: '.csv' | '.sqlite'): boolean {
  const trimmed = value.trim();
  return (
    !trimmed ||
    (trimmed.length <= 128 &&
      /^[A-Za-z0-9][A-Za-z0-9._-]*$/.test(trimmed) &&
      !trimmed.includes('..') &&
      trimmed.endsWith(extension))
  );
}

/**
 * Return whether all structured job arguments are valid.
 *
 * @param form - Structured task form values.
 * @param preset - Selected job preset.
 * @returns Whether the form can be submitted.
 */
function isJobFormValid(form: TaskFormState, preset: JobPresetId): boolean {
  if (isIndexPreset(preset)) {
    return isSafeBasename(form.metadataFile, '.csv');
  }
  const maxCandidates = form.maxCandidates.trim();
  const parsedMaxCandidates = Number(maxCandidates);
  return (
    isSafeBasename(form.database, '.sqlite') &&
    (!maxCandidates ||
      (Number.isInteger(parsedMaxCandidates) &&
        parsedMaxCandidates >= 1 &&
        parsedMaxCandidates <= 1000))
  );
}

/**
 * Return whether a scheduler time zone can be resolved by the browser runtime.
 *
 * @param value - Candidate IANA time-zone name.
 * @returns Whether the time-zone name is valid.
 */
function isValidTimeZone(value: string): boolean {
  const timezone = value.trim();
  if (!timezone) {
    return false;
  }
  try {
    Intl.DateTimeFormat('en-US', { timeZone: timezone }).format();
    return true;
  } catch {
    return false;
  }
}

/**
 * Return whether a scheduler timeout is a whole number within the backend limit.
 *
 * @param value - Timeout input in seconds.
 * @returns Whether the timeout can be submitted.
 */
function isValidTimeout(value: string): boolean {
  const timeoutSeconds = Number(value);
  return Number.isInteger(timeoutSeconds) && timeoutSeconds >= 1 && timeoutSeconds <= 86_400;
}

/**
 * Format a typed job for administrator review.
 *
 * @param job - Typed job, or null for a migrated legacy task.
 * @returns Human-readable job summary.
 */
function describeJob(job: ScheduledJobSpec | null): string {
  if (!job) {
    return '旧版自由命令（已禁用，需替换）';
  }
  if (job.kind === 'index') {
    const steps = ['索引更新'];
    if (job.notify) {
      steps.push('外部推送');
    }
    if (job.push) {
      steps.push('文件夹推送');
    }
    const metadata = job.metadata_file ? ` · CSV: ${job.metadata_file}` : '';
    return `${steps.join(' → ')}${metadata}`;
  }
  const workflow = job.kind === 'push' ? '文件夹推送' : '外部推送';
  const database = job.database ? ` · 数据库: ${job.database}` : '';
  const limit = job.max_candidates ? ` · 候选上限: ${job.max_candidates}` : '';
  return `${workflow}${database}${limit}`;
}

/**
 * Format a scheduler timestamp for display.
 *
 * @param value - Unix timestamp in seconds, or null when the task never ran.
 * @returns Localized display text.
 */
function formatDateTime(value: number | null): string {
  if (!value) {
    return '从未执行';
  }

  return new Date(value * 1000).toLocaleString('zh-CN', {
    hour: '2-digit',
    minute: '2-digit',
    month: '2-digit',
    day: '2-digit',
    year: 'numeric',
  });
}

/**
 * Render the admin scheduled task management card.
 *
 * @returns Scheduled task management UI.
 */
export function ScheduledTasksCard() {
  const queryClient = useQueryClient();
  const [dialogOpen, setDialogOpen] = useState(false);
  const [editingTask, setEditingTask] = useState<ScheduledTaskInfo | null>(null);
  const [form, setForm] = useState<TaskFormState>(DEFAULT_FORM);
  const [jobPreset, setJobPreset] = useState<JobPresetId>(DEFAULT_PRESET);

  const {
    data: tasks = [],
    error,
    isLoading,
  } = useQuery({
    queryKey: ['admin-scheduled-tasks'],
    queryFn: () => adminGetScheduledTasks(),
  });
  const {
    data: schedulerStatus,
    error: schedulerStatusError,
    isLoading: isSchedulerStatusLoading,
  } = useQuery({
    queryKey: ['admin-scheduler-status'],
    queryFn: () => adminGetSchedulerStatus(),
    refetchInterval: 30_000,
  });

  const saveMutation = useMutation({
    mutationFn: async () => {
      const payload: ScheduledTaskCreate = {
        coalesce: form.coalesce,
        cron: form.cron,
        enabled: form.enabled,
        job: buildScheduledJob(form, jobPreset),
        name: form.name,
        timeout_seconds: Number(form.timeoutSeconds),
        timezone: form.timezone.trim(),
      };
      if (editingTask) {
        return adminUpdateScheduledTask(editingTask.id, payload);
      }
      return adminCreateScheduledTask(payload);
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['admin-scheduled-tasks'] });
      queryClient.invalidateQueries({ queryKey: ['admin-stats'] });
      queryClient.invalidateQueries({ queryKey: ['admin-scheduler-status'] });
      setDialogOpen(false);
      setEditingTask(null);
      setForm(DEFAULT_FORM);
      setJobPreset(DEFAULT_PRESET);
    },
  });

  const toggleMutation = useMutation({
    mutationFn: ({ enabled, taskId }: { enabled: boolean; taskId: number }) =>
      adminUpdateScheduledTask(taskId, { enabled }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['admin-scheduled-tasks'] });
      queryClient.invalidateQueries({ queryKey: ['admin-stats'] });
      queryClient.invalidateQueries({ queryKey: ['admin-scheduler-status'] });
    },
  });

  const deleteMutation = useMutation({
    mutationFn: (taskId: number) => adminDeleteScheduledTask(taskId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['admin-scheduled-tasks'] });
      queryClient.invalidateQueries({ queryKey: ['admin-stats'] });
      queryClient.invalidateQueries({ queryKey: ['admin-scheduler-status'] });
    },
  });

  const mutationError = useMemo(() => {
    if (saveMutation.error instanceof Error) {
      return saveMutation.error.message;
    }
    if (toggleMutation.error instanceof Error) {
      return toggleMutation.error.message;
    }
    if (deleteMutation.error instanceof Error) {
      return deleteMutation.error.message;
    }
    return '';
  }, [deleteMutation.error, saveMutation.error, toggleMutation.error]);

  const openCreateDialog = () => {
    setEditingTask(null);
    setForm({ ...DEFAULT_FORM, timezone: getBrowserTimeZone() });
    setJobPreset(DEFAULT_PRESET);
    setDialogOpen(true);
  };

  const openEditDialog = (task: ScheduledTaskInfo) => {
    const preset = getPresetForJob(task.job);
    setEditingTask(task);
    setForm({
      coalesce: task.coalesce,
      cron: task.cron,
      database:
        task.job?.kind === 'notify' || task.job?.kind === 'push' ? (task.job.database ?? '') : '',
      enabled: task.enabled,
      maxCandidates:
        task.job?.kind === 'notify' || task.job?.kind === 'push'
          ? (task.job.max_candidates?.toString() ?? '')
          : '',
      metadataFile: task.job?.kind === 'index' ? (task.job.metadata_file ?? '') : '',
      name: task.name,
      timeoutSeconds: task.timeout_seconds.toString(),
      timezone: task.timezone,
    });
    setJobPreset(preset);
    setDialogOpen(true);
  };

  const isFormValid =
    Boolean(form.name.trim()) &&
    Boolean(form.cron.trim()) &&
    isValidTimeZone(form.timezone) &&
    isValidTimeout(form.timeoutSeconds) &&
    isJobFormValid(form, jobPreset);
  const healthyWorkerCount =
    schedulerStatus?.workers.filter((worker) => worker.is_healthy).length ?? 0;

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <Clock3 className="h-5 w-5" />
          定时任务
        </CardTitle>
        <CardDescription>管理后台自动执行的类型化任务</CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        <Dialog open={dialogOpen} onOpenChange={setDialogOpen}>
          <DialogTrigger asChild>
            <Button
              variant="outline"
              size="sm"
              className="w-full sm:w-auto"
              onClick={openCreateDialog}
            >
              <Plus className="mr-2 h-4 w-4" />
              新建任务
            </Button>
          </DialogTrigger>
          <DialogContent className="max-h-[90vh] overflow-y-auto sm:max-w-2xl">
            <DialogHeader>
              <DialogTitle>{editingTask ? '编辑定时任务' : '新建定时任务'}</DialogTitle>
              <DialogDescription>
                使用五段 crontab 表达式和明确的 IANA 时区，例如 `0 8 * * *` 与 `Asia/Shanghai`。
              </DialogDescription>
            </DialogHeader>
            <div className="space-y-4 py-2">
              {editingTask?.job === null && (
                <div role="alert" className="space-y-2 rounded-md border border-amber-500/40 p-3">
                  <div className="flex items-center gap-2 text-sm font-medium">
                    <AlertTriangle className="h-4 w-4 text-amber-600" />
                    旧任务已自动停用
                  </div>
                  <p className="text-sm text-muted-foreground">
                    保存后会用当前类型化配置替换旧命令，旧命令不会被执行。
                  </p>
                  <div className="rounded bg-muted px-2 py-1 font-mono text-xs break-all">
                    {editingTask.legacy_command}
                  </div>
                </div>
              )}
              <div className="space-y-2">
                <Label htmlFor="scheduled-task-name">任务名称</Label>
                <Input
                  id="scheduled-task-name"
                  name="scheduled_task_name"
                  autoComplete="off"
                  value={form.name}
                  onChange={(event) =>
                    setForm((current) => ({ ...current, name: event.target.value }))
                  }
                  placeholder="任务名称"
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="scheduled-task-cron">Cron 表达式</Label>
                <Input
                  id="scheduled-task-cron"
                  name="scheduled_task_cron"
                  autoComplete="off"
                  spellCheck={false}
                  value={form.cron}
                  onChange={(event) =>
                    setForm((current) => ({ ...current, cron: event.target.value }))
                  }
                  placeholder="Cron 表达式"
                />
              </div>
              <div className="grid gap-4 sm:grid-cols-2">
                <div className="space-y-2">
                  <Label htmlFor="scheduled-task-timezone">IANA 时区</Label>
                  <Input
                    id="scheduled-task-timezone"
                    name="scheduled_task_timezone"
                    autoComplete="off"
                    spellCheck={false}
                    value={form.timezone}
                    onChange={(event) =>
                      setForm((current) => ({ ...current, timezone: event.target.value }))
                    }
                    placeholder="Asia/Shanghai"
                  />
                  {!isValidTimeZone(form.timezone) && (
                    <p role="alert" className="text-sm text-destructive">
                      请输入有效的 IANA 时区，例如 Asia/Shanghai。
                    </p>
                  )}
                </div>
                <div className="space-y-2">
                  <Label htmlFor="scheduled-task-timeout">超时秒数</Label>
                  <Input
                    id="scheduled-task-timeout"
                    name="scheduled_task_timeout_seconds"
                    type="number"
                    min={1}
                    max={86_400}
                    step={1}
                    value={form.timeoutSeconds}
                    onChange={(event) =>
                      setForm((current) => ({ ...current, timeoutSeconds: event.target.value }))
                    }
                  />
                  {!isValidTimeout(form.timeoutSeconds) && (
                    <p role="alert" className="text-sm text-destructive">
                      超时必须为 1 到 86400 秒的整数。
                    </p>
                  )}
                </div>
              </div>
              <div className="space-y-2">
                <Label htmlFor="scheduled-task-preset">任务预设</Label>
                <Select
                  value={jobPreset}
                  onValueChange={(value: JobPresetId) => setJobPreset(value)}
                >
                  <SelectTrigger id="scheduled-task-preset" className="w-full">
                    <SelectValue placeholder="选择任务预设" />
                  </SelectTrigger>
                  <SelectContent>
                    {JOB_PRESETS.map((preset) => (
                      <SelectItem key={preset.value} value={preset.value}>
                        {preset.label}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
              {isIndexPreset(jobPreset) ? (
                <div className="space-y-2">
                  <Label htmlFor="scheduled-task-metadata">元数据 CSV 文件名（可选）</Label>
                  <Input
                    id="scheduled-task-metadata"
                    name="scheduled_task_metadata_file"
                    autoComplete="off"
                    spellCheck={false}
                    value={form.metadataFile}
                    onChange={(event) =>
                      setForm((current) => ({ ...current, metadataFile: event.target.value }))
                    }
                    placeholder="journals.csv"
                  />
                  {!isSafeBasename(form.metadataFile, '.csv') && (
                    <p role="alert" className="text-sm text-destructive">
                      请输入不含路径或特殊符号的 .csv 文件名。
                    </p>
                  )}
                </div>
              ) : (
                <div className="grid gap-4 sm:grid-cols-2">
                  <div className="space-y-2">
                    <Label htmlFor="scheduled-task-database">索引数据库（可选）</Label>
                    <Input
                      id="scheduled-task-database"
                      name="scheduled_task_database"
                      autoComplete="off"
                      spellCheck={false}
                      value={form.database}
                      onChange={(event) =>
                        setForm((current) => ({ ...current, database: event.target.value }))
                      }
                      placeholder="journals.sqlite"
                    />
                    {!isSafeBasename(form.database, '.sqlite') && (
                      <p role="alert" className="text-sm text-destructive">
                        请输入不含路径或特殊符号的 .sqlite 文件名。
                      </p>
                    )}
                  </div>
                  <div className="space-y-2">
                    <Label htmlFor="scheduled-task-candidates">候选上限（可选）</Label>
                    <Input
                      id="scheduled-task-candidates"
                      name="scheduled_task_max_candidates"
                      type="number"
                      min={1}
                      max={1000}
                      step={1}
                      value={form.maxCandidates}
                      onChange={(event) =>
                        setForm((current) => ({ ...current, maxCandidates: event.target.value }))
                      }
                      placeholder="100"
                    />
                  </div>
                </div>
              )}
              <div className="rounded-md border bg-muted/40 px-3 py-2 text-sm text-muted-foreground">
                将执行：{describeJob(buildScheduledJob(form, jobPreset))}
              </div>
              <div className="flex items-start justify-between gap-3 rounded-md border px-3 py-2">
                <Label htmlFor="scheduled-task-enabled" className="text-sm">
                  启用任务
                </Label>
                <Switch
                  id="scheduled-task-enabled"
                  checked={form.enabled}
                  onCheckedChange={(checked: boolean) =>
                    setForm((current) => ({ ...current, enabled: checked }))
                  }
                />
              </div>
              <div className="flex items-start justify-between gap-3 rounded-md border px-3 py-2">
                <div className="space-y-1">
                  <Label htmlFor="scheduled-task-coalesce" className="text-sm">
                    合并补跑
                  </Label>
                  <p className="text-xs text-muted-foreground">
                    worker 离线后仅补跑最近一次错过的执行。
                  </p>
                </div>
                <Switch
                  id="scheduled-task-coalesce"
                  checked={form.coalesce}
                  onCheckedChange={(checked: boolean) =>
                    setForm((current) => ({ ...current, coalesce: checked }))
                  }
                />
              </div>
              {mutationError && (
                <p role="alert" className="text-sm text-destructive">
                  {mutationError}
                </p>
              )}
              <div className="flex flex-col-reverse gap-2 sm:flex-row sm:justify-end">
                <Button
                  variant="outline"
                  className="w-full sm:w-auto"
                  onClick={() => setDialogOpen(false)}
                >
                  取消
                </Button>
                <Button
                  className="w-full sm:w-auto"
                  disabled={!isFormValid || saveMutation.isPending}
                  onClick={() => saveMutation.mutate()}
                >
                  {editingTask ? '保存' : '创建'}
                </Button>
              </div>
            </div>
          </DialogContent>
        </Dialog>

        <div aria-label="调度器状态" className="rounded-lg border bg-muted/30 p-3 text-sm">
          {isSchedulerStatusLoading ? (
            <span className="text-muted-foreground">正在读取调度器状态…</span>
          ) : schedulerStatusError instanceof Error ? (
            <span role="alert" className="text-destructive">
              {schedulerStatusError.message}
            </span>
          ) : (
            <div className="flex flex-col gap-1 sm:flex-row sm:items-center sm:justify-between">
              <span>
                健康 worker：{healthyWorkerCount}/{schedulerStatus?.workers.length ?? 0}
              </span>
              <span className="text-xs text-muted-foreground">
                最近检查：{formatDateTime(schedulerStatus?.last_checked_at ?? null)}
              </span>
            </div>
          )}
        </div>

        {error instanceof Error && (
          <p role="alert" className="text-sm text-destructive">
            {error.message}
          </p>
        )}

        {isLoading ? (
          <p role="status" className="text-sm text-muted-foreground">
            加载中…
          </p>
        ) : tasks.length === 0 ? (
          <p className="text-sm text-muted-foreground">暂无定时任务</p>
        ) : (
          <div className="space-y-3">
            {tasks.map((task) => (
              <div key={task.id} className="rounded-lg border p-4">
                <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
                  <div className="min-w-0 flex-1 space-y-1">
                    <div className="font-medium">{task.name}</div>
                    <div className="font-mono text-xs text-muted-foreground">{task.cron}</div>
                    <div className="text-xs text-muted-foreground">
                      {task.timezone} · 超时 {task.timeout_seconds} 秒 ·
                      {task.coalesce ? ' 合并补跑' : ' 逐次补跑'}
                    </div>
                    <div className="text-sm text-muted-foreground break-all">
                      {describeJob(task.job)}
                    </div>
                    {task.legacy_command && (
                      <div className="rounded border border-amber-500/40 px-2 py-1 text-xs text-muted-foreground break-all">
                        旧命令（只读）：{task.legacy_command}
                      </div>
                    )}
                    <div className="text-xs text-muted-foreground">
                      最近执行: {formatDateTime(task.last_run_at)}
                      {task.last_status && ` · ${task.last_status}`}
                    </div>
                  </div>
                  <div className="flex w-full flex-wrap items-center justify-end gap-2 sm:w-auto sm:flex-nowrap">
                    <Switch
                      checked={task.enabled}
                      disabled={task.job === null}
                      aria-label={
                        task.job
                          ? `${task.enabled ? '停用' : '启用'}定时任务 ${task.name}`
                          : `旧定时任务 ${task.name} 需替换`
                      }
                      onCheckedChange={(checked: boolean) => {
                        if (task.job) {
                          toggleMutation.mutate({ enabled: checked, taskId: task.id });
                        }
                      }}
                    />
                    <Button
                      variant="ghost"
                      size="icon"
                      aria-label={`编辑定时任务 ${task.name}`}
                      onClick={() => openEditDialog(task)}
                    >
                      <Pencil className="h-4 w-4" />
                    </Button>
                    <Button
                      variant="ghost"
                      size="icon"
                      className="text-destructive hover:text-destructive"
                      aria-label={`删除定时任务 ${task.name}`}
                      onClick={() => {
                        if (window.confirm(`确认删除定时任务“${task.name}”？`)) {
                          deleteMutation.mutate(task.id);
                        }
                      }}
                    >
                      <Trash2 className="h-4 w-4" />
                    </Button>
                  </div>
                </div>
              </div>
            ))}
          </div>
        )}

        {schedulerStatus && (
          <div className="space-y-2">
            <h3 className="text-sm font-medium">最近调度运行</h3>
            {schedulerStatus.recent_runs.length === 0 ? (
              <p className="text-sm text-muted-foreground">暂无调度运行记录</p>
            ) : (
              <div className="divide-y rounded-lg border">
                {schedulerStatus.recent_runs.slice(0, 5).map((run) => (
                  <div
                    key={run.id}
                    className="flex flex-col gap-1 px-3 py-2 text-sm sm:flex-row sm:items-center sm:justify-between"
                  >
                    <span className="min-w-0 truncate">{run.task_name}</span>
                    <span className="text-xs text-muted-foreground">
                      {run.status} · {formatDateTime(run.scheduled_for)}
                    </span>
                  </div>
                ))}
              </div>
            )}
          </div>
        )}
      </CardContent>
    </Card>
  );
}
