import { useMemo, useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { AlertTriangle, Clock3, Pencil, Plus, Trash2, Clock } from '@/components/ui/animated-icons';

import {
  adminCreateScheduledTask,
  adminDeleteScheduledTask,
  adminGetSchedulerStatus,
  adminGetScheduledTasks,
  adminUpdateScheduledTask,
  type ScheduledJobSpec,
  type SchedulerRunState,
  type ScheduledTaskCreate,
  type ScheduledTaskInfo,
} from '@/lib/api';
import { AdminListHeader } from '@/components/admin/admin-list-header';
import { SettingsGroup, StatusPill } from '@/components/settings/settings-section';
import { Button } from '@/components/ui/button';
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';
import { ConfirmDialog } from '@/components/ui/confirm-dialog';
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
import {
  DISCLOSURE_VARIANTS,
  MotionDiv,
  MotionParagraph,
  MotionPresence,
  REVEAL_VARIANTS,
  SWAP_VARIANTS,
} from '@/components/ui/motion';
import { Switch } from '@/components/ui/switch';
import { RevealItem } from '@/components/ui/reveal';
import { LoadingText } from '@/components/ui/loading-text';
import * as stylex from '@stylexjs/stylex';
import { colors, lineHeights } from '@/lib/theme.stylex';

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
 * Format every generated scheduler state for administrator display.
 *
 * @param status - Generated scheduler state.
 * @returns Stable localized state label.
 */
function formatSchedulerState(status: SchedulerRunState): string {
  return SCHEDULER_STATE_LABELS.get(status) ?? status;
}

/**
 * Render the admin scheduled task management card.
 *
 * @param props - Whether the retained panel is active and should fetch scheduler data.
 * @returns Scheduled task management UI.
 */
export function ScheduledTasksCard({ isActive = true }: { isActive?: boolean } = {}) {
  const state = useSchedulerViewState(isActive);
  const { error, isLoading } = state;
  return (
    <Card>
      <CardHeader>
        <CardTitle xstyle={styles.scheduledTasksCardCardTitle}>
          <Clock3 {...stylex.props(styles.scheduledTasksCardClock3)} />
          定时任务
        </CardTitle>
        <CardDescription>管理后台自动执行的类型化任务</CardDescription>
      </CardHeader>
      <CardContent xstyle={styles.scheduledTasksCardCardContent}>
        {renderSchedulerStatus(state)}

        <AdminListHeader summary={`共 ${state.tasks.length} 个任务`}>
          {renderScheduledTaskEditor(state)}
        </AdminListHeader>

        {error instanceof Error && (
          <p role="alert" {...stylex.props(styles.scheduledTasksCardP)}>
            {error.message}
          </p>
        )}

        <RevealItem
          isReady={!isLoading}
          placeholder={
            <p role="status" {...stylex.props(styles.scheduledTasksCardP2)}>
              <LoadingText text="加载中…" />
            </p>
          }
        >
          <SettingsGroup>{renderScheduledTaskList(state)}</SettingsGroup>
        </RevealItem>

        {renderRecentSchedulerRuns(state)}
        {renderScheduledTaskDeletion(state)}
      </CardContent>
    </Card>
  );
}
/** Retain editor state while limiting task and status queries to the active panel. */
function useSchedulerViewState(isActive: boolean) {
  const queryClient = useQueryClient();
  const [dialogOpen, setDialogOpen] = useState(false);
  const [editingTask, setEditingTask] = useState<ScheduledTaskInfo | null>(null);
  const [taskToDelete, setTaskToDelete] = useState<ScheduledTaskInfo | null>(null);
  const [form, setForm] = useState<TaskFormState>(DEFAULT_FORM);
  const [jobPreset, setJobPreset] = useState<JobPresetId>(DEFAULT_PRESET);

  const {
    data: tasks = [],
    error,
    isLoading,
  } = useQuery({
    queryKey: ['admin-scheduled-tasks'],
    queryFn: () => adminGetScheduledTasks(),
    enabled: isActive,
  });
  const {
    data: schedulerStatus,
    error: schedulerStatusError,
    isLoading: isSchedulerStatusLoading,
  } = useQuery({
    queryKey: ['admin-scheduler-status'],
    queryFn: () => adminGetSchedulerStatus(),
    enabled: isActive,
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
    onSuccess: (_data, taskId) => {
      queryClient.invalidateQueries({ queryKey: ['admin-scheduled-tasks'] });
      queryClient.invalidateQueries({ queryKey: ['admin-stats'] });
      queryClient.invalidateQueries({ queryKey: ['admin-scheduler-status'] });
      setTaskToDelete((current) => (current?.id === taskId ? null : current));
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
    setForm(getScheduledTaskForm(task));
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
  const isIndexJobPreset = isIndexPreset(jobPreset);
  const schedulerStatusKey = getSchedulerStatusKey(
    isSchedulerStatusLoading,
    schedulerStatusError,
    healthyWorkerCount,
    schedulerStatus,
  );

  return {
    dialogOpen,
    setDialogOpen,
    editingTask,
    taskToDelete,
    setTaskToDelete,
    form,
    setForm,
    jobPreset,
    setJobPreset,
    tasks,
    error,
    isLoading,
    schedulerStatus,
    schedulerStatusError,
    isSchedulerStatusLoading,
    saveMutation,
    toggleMutation,
    deleteMutation,
    mutationError,
    openCreateDialog,
    openEditDialog,
    isFormValid,
    healthyWorkerCount,
    isIndexJobPreset,
    schedulerStatusKey,
  };
}
type SchedulerViewState = ReturnType<typeof useSchedulerViewState>;

/** Keep hidden index arguments mounted with identical inert and motion guards. */
function renderSchedulerIndexFields(
  state: Pick<SchedulerViewState, 'form' | 'setForm' | 'isIndexJobPreset'>,
) {
  const { form, setForm, isIndexJobPreset } = state;

  return (
    <MotionDiv
      data-motion-scheduled-fields="index"
      aria-hidden={!isIndexJobPreset}
      inert={!isIndexJobPreset ? true : undefined}
      initial={false}
      animate={isIndexJobPreset ? 'visible' : 'hidden'}
      variants={DISCLOSURE_VARIANTS}
      {...stylex.props(
        styles.renderSchedulerIndexFieldsMotionDiv,
        !isIndexJobPreset && styles.inert,
      )}
    >
      <div>
        <Label xstyle={styles.renderSchedulerIndexFieldsDivItem} htmlFor="scheduled-task-metadata">
          元数据 CSV 文件名（可选）
        </Label>
        <Input
          xstyle={styles.renderSchedulerIndexFieldsDivItem}
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
          <p
            role="alert"
            {...stylex.props(styles.renderSchedulerIndexFieldsDivItem, styles.scheduledTasksCardP)}
          >
            请输入不含路径或特殊符号的 .csv 文件名。
          </p>
        )}
      </div>
    </MotionDiv>
  );
}

/** Keep hidden delivery arguments mounted with identical inert and validation guards. */
function renderSchedulerDeliveryFields(
  state: Pick<SchedulerViewState, 'form' | 'setForm' | 'isIndexJobPreset'>,
) {
  const { form, setForm, isIndexJobPreset } = state;

  return (
    <MotionDiv
      data-motion-scheduled-fields="delivery"
      aria-hidden={isIndexJobPreset}
      inert={isIndexJobPreset ? true : undefined}
      initial={false}
      animate={isIndexJobPreset ? 'hidden' : 'visible'}
      variants={DISCLOSURE_VARIANTS}
      {...stylex.props(
        styles.renderSchedulerIndexFieldsMotionDiv,
        isIndexJobPreset && styles.inert,
      )}
    >
      <div {...stylex.props(styles.renderSchedulerDeliveryFieldsDiv)}>
        <div>
          <Label
            xstyle={styles.renderSchedulerIndexFieldsDivItem}
            htmlFor="scheduled-task-database"
          >
            索引数据库（可选）
          </Label>
          <Input
            xstyle={styles.renderSchedulerIndexFieldsDivItem}
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
            <p
              role="alert"
              {...stylex.props(
                styles.renderSchedulerIndexFieldsDivItem,
                styles.scheduledTasksCardP,
              )}
            >
              请输入不含路径或特殊符号的 .sqlite 文件名。
            </p>
          )}
        </div>
        <div>
          <Label
            xstyle={styles.renderSchedulerIndexFieldsDivItem}
            htmlFor="scheduled-task-candidates"
          >
            候选上限（可选）
          </Label>
          <Input
            xstyle={styles.renderSchedulerIndexFieldsDivItem}
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
    </MotionDiv>
  );
}

/** Retain typed task fields, legacy warning and exact create/save admission. */
function renderScheduledTaskEditor(state: SchedulerViewState) {
  const {
    dialogOpen,
    setDialogOpen,
    editingTask,
    form,
    setForm,
    jobPreset,
    setJobPreset,
    saveMutation,
    mutationError,
    openCreateDialog,
    isFormValid,
  } = state;

  return (
    <Dialog open={dialogOpen} onOpenChange={setDialogOpen}>
      <DialogTrigger asChild>
        <Button size="sm" onClick={openCreateDialog}>
          <Plus {...stylex.props(styles.renderScheduledTaskEditorPlus)} />
          新建任务
        </Button>
      </DialogTrigger>
      <DialogContent xstyle={styles.renderScheduledTaskEditorDialogContent}>
        <DialogHeader>
          <DialogTitle>{editingTask ? '编辑定时任务' : '新建定时任务'}</DialogTitle>
          <DialogDescription>
            使用五段 crontab 表达式和明确的 IANA 时区，例如 `0 8 * * *` 与 `Asia/Shanghai`。
          </DialogDescription>
        </DialogHeader>
        <div {...stylex.props(styles.renderScheduledTaskEditorDiv)}>
          {editingTask?.job === null && (
            <div role="alert" {...stylex.props(styles.renderScheduledTaskEditorDiv2)}>
              <div
                {...stylex.props(
                  styles.renderSchedulerIndexFieldsDivItem,
                  styles.renderScheduledTaskEditorDiv3,
                )}
              >
                <AlertTriangle {...stylex.props(styles.renderScheduledTaskEditorAlertTriangle)} />
                旧任务已自动停用
              </div>
              <p
                {...stylex.props(
                  styles.renderSchedulerIndexFieldsDivItem,
                  styles.scheduledTasksCardP2,
                )}
              >
                保存后会用当前类型化配置替换旧命令，旧命令不会被执行。
              </p>
              <div
                {...stylex.props(
                  styles.renderSchedulerIndexFieldsDivItem,
                  styles.renderScheduledTaskEditorDiv4,
                )}
              >
                {editingTask.legacy_command}
              </div>
            </div>
          )}
          <div>
            <Label xstyle={styles.renderSchedulerIndexFieldsDivItem} htmlFor="scheduled-task-name">
              任务名称
            </Label>
            <Input
              xstyle={styles.renderSchedulerIndexFieldsDivItem}
              id="scheduled-task-name"
              name="scheduled_task_name"
              autoComplete="off"
              value={form.name}
              onChange={(event) => setForm((current) => ({ ...current, name: event.target.value }))}
              placeholder="任务名称"
            />
          </div>
          <div>
            <Label xstyle={styles.renderSchedulerIndexFieldsDivItem} htmlFor="scheduled-task-cron">
              Cron 表达式
            </Label>
            <Input
              xstyle={styles.renderSchedulerIndexFieldsDivItem}
              id="scheduled-task-cron"
              name="scheduled_task_cron"
              autoComplete="off"
              spellCheck={false}
              value={form.cron}
              onChange={(event) => setForm((current) => ({ ...current, cron: event.target.value }))}
              placeholder="Cron 表达式"
            />
          </div>
          <div {...stylex.props(styles.renderSchedulerDeliveryFieldsDiv)}>
            <div>
              <Label
                xstyle={styles.renderSchedulerIndexFieldsDivItem}
                htmlFor="scheduled-task-timezone"
              >
                IANA 时区
              </Label>
              <Input
                xstyle={styles.renderSchedulerIndexFieldsDivItem}
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
                <p
                  role="alert"
                  {...stylex.props(
                    styles.renderSchedulerIndexFieldsDivItem,
                    styles.scheduledTasksCardP,
                  )}
                >
                  请输入有效的 IANA 时区，例如 Asia/Shanghai。
                </p>
              )}
            </div>
            <div>
              <Label
                xstyle={styles.renderSchedulerIndexFieldsDivItem}
                htmlFor="scheduled-task-timeout"
              >
                超时秒数
              </Label>
              <Input
                xstyle={styles.renderSchedulerIndexFieldsDivItem}
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
                <p
                  role="alert"
                  {...stylex.props(
                    styles.renderSchedulerIndexFieldsDivItem,
                    styles.scheduledTasksCardP,
                  )}
                >
                  超时必须为 1 到 86400 秒的整数。
                </p>
              )}
            </div>
          </div>
          <div {...stylex.props(styles.renderScheduledTaskEditorDiv5)}>
            <Label htmlFor="scheduled-task-preset">任务预设</Label>
            <Select value={jobPreset} onValueChange={(value: JobPresetId) => setJobPreset(value)}>
              <SelectTrigger
                id="scheduled-task-preset"
                xstyle={styles.renderScheduledTaskEditorSelectTrigger}
              >
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
          <div>
            {renderSchedulerIndexFields(state)}
            {renderSchedulerDeliveryFields(state)}
          </div>
          <div {...stylex.props(styles.renderScheduledTaskEditorDiv6)}>
            将执行：{describeJob(buildScheduledJob(form, jobPreset))}
          </div>
          <div {...stylex.props(styles.renderScheduledTaskEditorDiv7)}>
            <Label htmlFor="scheduled-task-enabled" xstyle={styles.renderScheduledTaskEditorLabel}>
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
          <div {...stylex.props(styles.renderScheduledTaskEditorDiv7)}>
            <div>
              <Label
                htmlFor="scheduled-task-coalesce"
                xstyle={[
                  styles.renderScheduledTaskEditorDivItem,
                  styles.renderScheduledTaskEditorLabel,
                ]}
              >
                合并补跑
              </Label>
              <p
                {...stylex.props(
                  styles.renderScheduledTaskEditorDivItem,
                  styles.renderScheduledTaskEditorP,
                )}
              >
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
          <MotionPresence>
            {mutationError && (
              <MotionParagraph
                key="scheduled-task-error"
                data-motion-feedback="scheduled-task"
                role="alert"
                {...stylex.props(styles.scheduledTasksCardP)}
                initial="hidden"
                animate="visible"
                exit="exit"
                variants={REVEAL_VARIANTS}
              >
                {mutationError}
              </MotionParagraph>
            )}
          </MotionPresence>
          <div {...stylex.props(styles.renderScheduledTaskEditorDiv8)}>
            <Button
              variant="outline"
              xstyle={styles.renderScheduledTaskEditorButton}
              onClick={() => setDialogOpen(false)}
            >
              取消
            </Button>
            <Button
              xstyle={styles.renderScheduledTaskEditorButton}
              disabled={!isFormValid || saveMutation.isPending}
              onClick={() => saveMutation.mutate()}
            >
              {editingTask ? '保存' : '创建'}
            </Button>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}

/** Retain loading-before-error state and direct keyed worker-status presence. */
function renderSchedulerStatus(state: SchedulerViewState) {
  const {
    schedulerStatus,
    schedulerStatusError,
    isSchedulerStatusLoading,
    healthyWorkerCount,
    schedulerStatusKey,
  } = state;

  return (
    <div aria-label="调度器状态" {...stylex.props(styles.renderSchedulerStatusDiv)}>
      <RevealItem
        isReady={!isSchedulerStatusLoading}
        placeholder={<LoadingText text="正在读取调度器状态…" />}
      >
        <MotionPresence mode="wait">
          <MotionDiv
            key={schedulerStatusKey}
            data-motion-scheduler-state={schedulerStatusKey}
            initial="hidden"
            animate="visible"
            exit="exit"
            variants={SWAP_VARIANTS}
          >
            {isSchedulerStatusLoading ? (
              <LoadingText text="正在读取调度器状态…" />
            ) : schedulerStatusError instanceof Error ? (
              <span role="alert" {...stylex.props(styles.renderSchedulerStatusSpan)}>
                {schedulerStatusError.message}
              </span>
            ) : (
              <div {...stylex.props(styles.renderSchedulerStatusDiv2)}>
                <div {...stylex.props(styles.renderSchedulerStatusDiv3)}>
                  <span aria-hidden="true" {...stylex.props(styles.renderSchedulerStatusSpan2)}>
                    {healthyWorkerCount > 0 && (
                      <span {...stylex.props(styles.renderSchedulerStatusSpan3)} />
                    )}
                    <span
                      {...stylex.props(
                        styles.renderSchedulerStatusSpan4,
                        healthyWorkerCount > 0
                          ? styles.renderSchedulerStatusSpanHealthyWorkerCount
                          : styles.renderSchedulerStatusSpanNotHealthyWorkerCount,
                      )}
                    />
                  </span>
                  <div>
                    <p {...stylex.props(styles.renderScheduledTaskEditorP)}>调度器</p>
                    <p {...stylex.props(styles.renderSchedulerStatusP)}>
                      健康 worker：{healthyWorkerCount}/{schedulerStatus?.workers.length ?? 0}
                    </p>
                  </div>
                </div>
                <span {...stylex.props(styles.renderSchedulerStatusSpan5)}>
                  最近检查：{formatDateTime(schedulerStatus?.last_checked_at ?? null)}
                </span>
              </div>
            )}
          </MotionDiv>
        </MotionPresence>
      </RevealItem>
    </div>
  );
}

/** Retain directly keyed tasks, disabled legacy toggles and captured deletion targets. */
function renderScheduledTaskList(state: SchedulerViewState) {
  const { setTaskToDelete, tasks, toggleMutation, deleteMutation, openEditDialog } = state;

  return (
    <MotionPresence>
      {tasks.length === 0 ? (
        <MotionParagraph
          key="empty-scheduled-tasks"
          {...stylex.props(styles.renderScheduledTaskListMotionParagraph)}
          initial="hidden"
          animate="visible"
          exit="exit"
          variants={REVEAL_VARIANTS}
        >
          暂无定时任务
        </MotionParagraph>
      ) : (
        tasks.map((task) => (
          <MotionDiv
            key={task.id}
            data-motion-scheduled-task-key={task.id}
            {...stylex.props(styles.renderScheduledTaskListMotionDiv)}
            initial="hidden"
            animate="visible"
            exit="exit"
            variants={DISCLOSURE_VARIANTS}
          >
            <div {...stylex.props(styles.renderScheduledTaskListDiv)}>
              <span
                aria-hidden="true"
                {...stylex.props(
                  styles.renderScheduledTaskListSpan,
                  task.enabled
                    ? styles.renderScheduledTaskListSpanEnabled
                    : styles.renderScheduledTaskListSpanNotEnabled,
                )}
              >
                <Clock3 {...stylex.props(styles.renderScheduledTaskListClock3)} />
              </span>
              <div {...stylex.props(styles.renderScheduledTaskListDiv2)}>
                <div
                  {...stylex.props(
                    styles.renderScheduledTaskListDivItem,
                    styles.renderScheduledTaskListDiv3,
                  )}
                >
                  <span {...stylex.props(styles.renderScheduledTaskListSpan2)}>{task.name}</span>
                  <StatusPill tone={getSchedulerStateTone(task.last_status)}>
                    {formatSchedulerState(task.last_status)}
                  </StatusPill>
                </div>
                <div
                  {...stylex.props(
                    styles.renderScheduledTaskListDivItem,
                    styles.renderScheduledTaskListDiv4,
                  )}
                >
                  <code {...stylex.props(styles.renderScheduledTaskListCode)}>{task.cron}</code>
                  <span {...stylex.props(styles.renderScheduledTaskListSpan3)}>
                    {task.timezone} · 超时 {task.timeout_seconds} 秒 ·
                    {task.coalesce ? ' 合并补跑' : ' 逐次补跑'}
                  </span>
                </div>
                <div
                  {...stylex.props(
                    styles.renderScheduledTaskListDivItem,
                    styles.renderScheduledTaskListDiv5,
                  )}
                >
                  {describeJob(task.job)}
                </div>
                {task.legacy_command && (
                  <div
                    {...stylex.props(
                      styles.renderScheduledTaskListDivItem,
                      styles.renderScheduledTaskListDiv6,
                    )}
                  >
                    旧命令（只读）：{task.legacy_command}
                  </div>
                )}
                <div
                  {...stylex.props(
                    styles.renderScheduledTaskListDivItem,
                    styles.renderSchedulerStatusSpan5,
                  )}
                >
                  最近执行: {formatDateTime(task.last_run_at)}
                </div>
              </div>
              <div {...stylex.props(styles.renderScheduledTaskListDiv7)}>
                <Switch
                  checked={task.enabled}
                  disabled={task.job === null}
                  xstyle={styles.renderScheduledTaskListSwitch}
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
                  xstyle={styles.renderScheduledTaskListButton}
                  aria-label={`编辑定时任务 ${task.name}`}
                  onClick={() => openEditDialog(task)}
                >
                  <Pencil {...stylex.props(styles.renderScheduledTaskEditorPlus)} />
                </Button>
                <Button
                  variant="ghost"
                  size="icon"
                  xstyle={styles.renderScheduledTaskListButton2}
                  aria-label={`删除定时任务 ${task.name}`}
                  disabled={deleteMutation.isPending}
                  onClick={() => {
                    deleteMutation.reset();
                    setTaskToDelete(task);
                  }}
                >
                  <Trash2 {...stylex.props(styles.renderScheduledTaskEditorPlus)} />
                </Button>
              </div>
            </div>
          </MotionDiv>
        ))
      )}
    </MotionPresence>
  );
}

/** Retain the first five server-ordered runs and empty-record feedback. */
function renderRecentSchedulerRuns(
  state: Pick<SchedulerViewState, 'schedulerStatus' | 'isSchedulerStatusLoading'>,
) {
  const { schedulerStatus, isSchedulerStatusLoading } = state;

  return (
    <RevealItem isReady={!isSchedulerStatusLoading} placeholder={null}>
      {schedulerStatus && (
        <section {...stylex.props(styles.renderRecentSchedulerRunsSection)}>
          <h3
            {...stylex.props(
              styles.renderRecentSchedulerRunsSectionItem,
              styles.renderRecentSchedulerRunsH3,
            )}
          >
            <Clock
              animateOnView
              {...stylex.props(styles.renderRecentSchedulerRunsClock)}
              aria-hidden="true"
            />
            最近调度运行
          </h3>
          {schedulerStatus.recent_runs.length === 0 ? (
            <p
              {...stylex.props(
                styles.renderRecentSchedulerRunsSectionItem,
                styles.renderRecentSchedulerRunsP,
              )}
            >
              暂无调度运行记录
            </p>
          ) : (
            <ol
              {...stylex.props(
                styles.renderRecentSchedulerRunsSectionItem,
                styles.renderRecentSchedulerRunsOl,
              )}
            >
              {schedulerStatus.recent_runs.slice(0, 5).map((run) => (
                <li
                  key={run.id}
                  {...stylex.props(
                    styles.renderRecentSchedulerRunsOlItem,
                    styles.renderRecentSchedulerRunsLi,
                  )}
                >
                  <span
                    aria-hidden="true"
                    {...stylex.props(
                      styles.schedulerStateDot,
                      SCHEDULER_STATE_DOT_STYLES[getSchedulerStateTone(run.status)],
                    )}
                  />
                  <span {...stylex.props(styles.renderRecentSchedulerRunsSpan2)}>
                    {run.task_name}
                  </span>
                  <span {...stylex.props(styles.renderRecentSchedulerRunsSpan3)}>
                    {formatSchedulerState(run.status)} · {formatDateTime(run.scheduled_for)}
                  </span>
                </li>
              ))}
            </ol>
          )}
        </section>
      )}
    </RevealItem>
  );
}

/**
 * Map a scheduler run state to a status tone.
 *
 * @param status - Scheduler run state.
 * @returns Status pill tone.
 */
function getSchedulerStateTone(status: SchedulerRunState): keyof typeof SCHEDULER_STATE_DOT_STYLES {
  if (status === 'success') return 'success';
  if (status === 'failed' || status === 'timed_out' || status === 'error') return 'danger';
  if (status === 'pending' || status === 'claimed' || status === 'running') return 'info';
  if (status === 'unknown') return 'warning';
  return 'neutral';
}

/** Retain confirmation target identity and pending-dismissal safeguards. */
function renderScheduledTaskDeletion(
  state: Pick<SchedulerViewState, 'taskToDelete' | 'setTaskToDelete' | 'deleteMutation'>,
) {
  const { taskToDelete, setTaskToDelete, deleteMutation } = state;

  return (
    <ConfirmDialog
      open={taskToDelete !== null}
      onOpenChange={(nextOpen) => {
        if (!nextOpen && !deleteMutation.isPending) {
          setTaskToDelete(null);
        }
      }}
      title="删除定时任务？"
      description={`确认删除定时任务“${taskToDelete?.name ?? ''}”？`}
      actionLabel="确认删除"
      pendingLabel="删除中…"
      isPending={deleteMutation.isPending}
      error={deleteMutation.error instanceof Error ? deleteMutation.error.message : null}
      onConfirm={() => {
        if (taskToDelete) {
          deleteMutation.mutate(taskToDelete.id);
        }
      }}
    />
  );
}

const SCHEDULER_STATE_LABELS = new Map<SchedulerRunState, string>(
  Object.entries({
    idle: '未运行',
    pending: '等待中',
    claimed: '已领取',
    running: '运行中',
    success: '成功',
    failed: '失败',
    timed_out: '超时',
    error: '执行错误',
    unknown: '结果未知',
    cancelled: '已取消',
  } satisfies Record<SchedulerRunState, string>) as [SchedulerRunState, string][],
);

/** Retain loading-before-error identity and server worker-count fallback. */
function getSchedulerStatusKey(
  isSchedulerStatusLoading: boolean,
  schedulerStatusError: unknown,
  healthyWorkerCount: number,
  schedulerStatus: Awaited<ReturnType<typeof adminGetSchedulerStatus>> | undefined,
): string {
  return isSchedulerStatusLoading
    ? 'loading'
    : schedulerStatusError instanceof Error
      ? 'error'
      : `workers-${healthyWorkerCount}-${schedulerStatus?.workers.length ?? 0}`;
}

/** Restore the original database field only for delivery jobs. */
function getScheduledTaskDatabase(task: ScheduledTaskInfo): string {
  return task.job?.kind === 'notify' || task.job?.kind === 'push' ? (task.job.database ?? '') : '';
}

/** Restore the original maxCandidates field only for delivery jobs. */
function getScheduledTaskCandidates(task: ScheduledTaskInfo): string {
  return task.job?.kind === 'notify' || task.job?.kind === 'push'
    ? (task.job.max_candidates?.toString() ?? '')
    : '';
}

/** Restore edit fields in their original order without executing legacy commands. */
function getScheduledTaskForm(task: ScheduledTaskInfo): TaskFormState {
  return {
    coalesce: task.coalesce,
    cron: task.cron,
    database: getScheduledTaskDatabase(task),
    enabled: task.enabled,
    maxCandidates: getScheduledTaskCandidates(task),
    metadataFile: task.job?.kind === 'index' ? (task.job.metadata_file ?? '') : '',
    name: task.name,
    timeoutSeconds: task.timeout_seconds.toString(),
    timezone: task.timezone,
  };
}

const pingKeyframes = stylex.keyframes({ '75%, 100%': { transform: 'scale(2)', opacity: 0 } });

const styles = stylex.create({
  inert: {
    pointerEvents: 'none',
  },
  schedulerStateDot: {
    position: 'absolute',
    top: 'calc(1 / 2 * 100%)',
    left: '-1rem',
    width: '11px',
    height: '11px',
    translate: '0 calc(calc(1 / 2 * 100%) * -1)',
    borderRadius: 'calc(infinity * 1px)',
    borderStyle: 'solid',
    borderWidth: '2px',
    borderColor: colors.background,
  },
  schedulerStateDotDanger: {
    backgroundColor: colors.destructive,
  },
  schedulerStateDotInfo: {
    backgroundColor: colors.primary,
  },
  schedulerStateDotNeutral: {
    backgroundColor: `color-mix(in oklab, ${colors.mutedForeground} 50%, transparent)`,
  },
  schedulerStateDotSuccess: {
    backgroundColor: colors.successForeground,
  },
  schedulerStateDotWarning: {
    backgroundColor: colors.warningForeground,
  },
  scheduledTasksCardCardTitle: {
    display: 'flex',
    alignItems: 'center',
    gap: '0.5rem',
  },
  scheduledTasksCardClock3: {
    height: '1.25rem',
    width: '1.25rem',
  },
  scheduledTasksCardCardContent: {
    display: 'flex',
    flexDirection: 'column',
    rowGap: '0.75rem',
  },
  scheduledTasksCardP: {
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
    color: colors.destructive,
  },
  scheduledTasksCardP2: {
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
    color: colors.mutedForeground,
  },
  renderSchedulerIndexFieldsMotionDiv: {
    overflow: 'hidden',
  },
  renderSchedulerIndexFieldsDivItem: {
    marginBottom: {
      default: null,
      ':not(:last-child)': '0.5rem',
    },
  },
  renderSchedulerDeliveryFieldsDiv: {
    display: 'grid',
    gap: '1rem',
    gridTemplateColumns: {
      default: null,
      '@media (min-width: 40rem)': 'repeat(2, minmax(0, 1fr))',
    },
  },
  renderScheduledTaskEditorPlus: {
    width: '1rem',
    height: '1rem',
  },
  renderScheduledTaskEditorDialogContent: {
    maxHeight: '90vh',
    overflowY: 'auto',
    maxWidth: {
      default: 'none',
      '@media (min-width: 40rem)': '42rem',
      '@media (min-width: 48rem)': '36rem',
    },
  },
  renderScheduledTaskEditorDiv: {
    display: 'flex',
    flexDirection: 'column',
    rowGap: '1rem',
    paddingBlock: '0.5rem',
  },
  renderScheduledTaskEditorDiv2: {
    borderRadius: '6px',
    borderStyle: 'solid',
    borderWidth: '1px',
    borderColor: colors.warningBorder,
    padding: '0.75rem',
  },
  renderScheduledTaskEditorDiv3: {
    display: 'flex',
    alignItems: 'center',
    gap: '0.5rem',
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
    fontWeight: 500,
  },
  renderScheduledTaskEditorAlertTriangle: {
    height: '1rem',
    width: '1rem',
    color: colors.warningForeground,
  },
  renderScheduledTaskEditorDiv4: {
    borderRadius: '0.25rem',
    backgroundColor: colors.muted,
    paddingInline: '0.5rem',
    paddingBlock: '0.25rem',
    fontFamily: "'JetBrainsLxgwNerdMono', monospace",
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    wordBreak: 'break-all',
  },
  renderScheduledTaskEditorDiv5: {
    display: 'flex',
    flexDirection: 'column',
    rowGap: '0.5rem',
  },
  renderScheduledTaskEditorSelectTrigger: {
    width: '100%',
  },
  renderScheduledTaskEditorDiv6: {
    borderRadius: '6px',
    borderStyle: 'solid',
    borderWidth: '1px',
    backgroundColor: `color-mix(in oklab, ${colors.muted} 40%, transparent)`,
    paddingInline: '0.75rem',
    paddingBlock: '0.5rem',
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
    color: colors.mutedForeground,
  },
  renderScheduledTaskEditorDiv7: {
    display: 'flex',
    alignItems: 'flex-start',
    justifyContent: 'space-between',
    gap: '0.75rem',
    borderRadius: '6px',
    borderStyle: 'solid',
    borderWidth: '1px',
    paddingInline: '0.75rem',
    paddingBlock: '0.5rem',
  },
  renderScheduledTaskEditorLabel: {
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
  },
  renderScheduledTaskEditorDivItem: {
    marginBottom: {
      default: null,
      ':not(:last-child)': '0.25rem',
    },
  },
  renderScheduledTaskEditorP: {
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    color: colors.mutedForeground,
  },
  renderScheduledTaskEditorDiv8: {
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
  renderScheduledTaskEditorButton: {
    width: {
      default: '100%',
      '@media (min-width: 40rem)': 'auto',
    },
  },
  renderSchedulerStatusDiv: {
    overflow: 'hidden',
    borderBlockStyle: 'solid',
    borderBlockWidth: '1px',
    paddingBlock: '1rem',
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
  },
  renderSchedulerStatusSpan: {
    color: colors.destructive,
  },
  renderSchedulerStatusDiv2: {
    display: 'flex',
    flexDirection: {
      default: 'column',
      '@media (min-width: 40rem)': 'row',
    },
    gap: '0.75rem',
    alignItems: {
      default: null,
      '@media (min-width: 40rem)': 'center',
    },
    justifyContent: {
      default: null,
      '@media (min-width: 40rem)': 'space-between',
    },
  },
  renderSchedulerStatusDiv3: {
    display: 'flex',
    alignItems: 'center',
    gap: '0.75rem',
  },
  renderSchedulerStatusSpan2: {
    position: 'relative',
    display: 'flex',
    width: '0.75rem',
    height: '0.75rem',
  },
  renderSchedulerStatusSpan3: {
    position: 'absolute',
    display: 'inline-flex',
    width: '100%',
    height: '100%',
    animationName: pingKeyframes,
    animationDuration: '1s',
    animationTimingFunction: 'cubic-bezier(0, 0, 0.2, 1)',
    animationIterationCount: 'infinite',
    borderRadius: 'calc(infinity * 1px)',
    backgroundColor: 'oklch(76.5% 0.177 163.223)',
    opacity: '60%',
  },
  renderSchedulerStatusSpan4: {
    position: 'relative',
    display: 'inline-flex',
    width: '0.75rem',
    height: '0.75rem',
    borderRadius: 'calc(infinity * 1px)',
  },
  renderSchedulerStatusSpanHealthyWorkerCount: {
    backgroundColor: 'oklch(76.5% 0.177 163.223)',
  },
  renderSchedulerStatusSpanNotHealthyWorkerCount: {
    backgroundColor: 'oklch(70.4% 0.191 22.216)',
  },
  renderSchedulerStatusP: {
    fontWeight: 700,
    fontVariantNumeric: 'tabular-nums',
  },
  renderSchedulerStatusSpan5: {
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    color: colors.mutedForeground,
    fontVariantNumeric: 'tabular-nums',
  },
  renderScheduledTaskListMotionParagraph: {
    paddingInline: '1.25rem',
    paddingBlock: '2.5rem',
    textAlign: 'center',
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
    color: colors.mutedForeground,
  },
  renderScheduledTaskListMotionDiv: {
    overflow: 'hidden',
    paddingInline: {
      default: '1rem',
      '@media (min-width: 40rem)': '1.25rem',
    },
    paddingBlock: '1rem',
  },
  renderScheduledTaskListDiv: {
    display: 'flex',
    flexDirection: {
      default: 'column',
      '@media (min-width: 40rem)': 'row',
    },
    gap: {
      default: '0.75rem',
      '@media (min-width: 40rem)': '1rem',
    },
    alignItems: {
      default: null,
      '@media (min-width: 40rem)': 'flex-start',
    },
  },
  renderScheduledTaskListSpan: {
    display: {
      default: 'none',
      '@media (min-width: 40rem)': 'flex',
    },
    width: '2.5rem',
    height: '2.5rem',
    flexShrink: 0,
    alignItems: 'center',
    justifyContent: 'center',
    borderRadius: '12px',
  },
  renderScheduledTaskListSpanEnabled: {
    backgroundColor: colors.info,
    color: colors.primary,
  },
  renderScheduledTaskListSpanNotEnabled: {
    backgroundColor: colors.muted,
    color: colors.mutedForeground,
  },
  renderScheduledTaskListClock3: {
    width: '1.25rem',
    height: '1.25rem',
  },
  renderScheduledTaskListDivItem: {
    marginBottom: {
      default: null,
      ':not(:last-child)': '0.375rem',
    },
  },
  renderScheduledTaskListDiv2: {
    minWidth: '0px',
    flex: '1',
  },
  renderScheduledTaskListDiv3: {
    display: 'flex',
    flexWrap: 'wrap',
    alignItems: 'center',
    gap: '0.5rem',
  },
  renderScheduledTaskListSpan2: {
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
    fontWeight: 700,
  },
  renderScheduledTaskListDiv4: {
    display: 'flex',
    flexWrap: 'wrap',
    alignItems: 'center',
    gap: '0.375rem',
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
  },
  renderScheduledTaskListCode: {
    borderRadius: '6px',
    backgroundColor: colors.foreground,
    paddingInline: '0.375rem',
    paddingBlock: '0.125rem',
    fontWeight: 700,
    color: colors.background,
  },
  renderScheduledTaskListSpan3: {
    color: colors.mutedForeground,
  },
  renderScheduledTaskListDiv5: {
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
    wordBreak: 'break-all',
    color: colors.mutedForeground,
  },
  renderScheduledTaskListDiv6: {
    borderRadius: '8px',
    borderStyle: 'solid',
    borderWidth: '1px',
    borderColor: colors.warningBorder,
    backgroundColor: colors.warning,
    paddingInline: '0.625rem',
    paddingBlock: '0.375rem',
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    wordBreak: 'break-all',
    color: colors.warningForeground,
  },
  renderScheduledTaskListDiv7: {
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'flex-end',
    gap: '0.25rem',
    borderTopStyle: 'solid',
    borderTopWidth: {
      default: '1px',
      '@media (min-width: 40rem)': '0px',
    },
    paddingTop: {
      default: '0.75rem',
      '@media (min-width: 40rem)': '0px',
    },
  },
  renderScheduledTaskListSwitch: {
    marginRight: '0.5rem',
  },
  renderScheduledTaskListButton: {
    width: '2.25rem',
    height: '2.25rem',
  },
  renderScheduledTaskListButton2: {
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
  renderRecentSchedulerRunsSectionItem: {
    marginBottom: {
      default: null,
      ':not(:last-child)': '0.75rem',
    },
  },
  renderRecentSchedulerRunsSection: {
    paddingTop: '1.75rem',
  },
  renderRecentSchedulerRunsH3: {
    display: 'flex',
    alignItems: 'center',
    gap: '0.5rem',
    paddingInline: '0.25rem',
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
    fontWeight: 700,
  },
  renderRecentSchedulerRunsClock: {
    width: '0.875rem',
    height: '0.875rem',
    flexShrink: 0,
  },
  renderRecentSchedulerRunsP: {
    paddingInline: '0.25rem',
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
    color: colors.mutedForeground,
  },
  renderRecentSchedulerRunsOlItem: {
    marginBottom: {
      default: null,
      ':not(:last-child)': '0px',
    },
  },
  renderRecentSchedulerRunsOl: {
    position: 'relative',
    paddingLeft: '1rem',
    '::before': {
      content: '""',
      position: 'absolute',
      top: '0.5rem',
      bottom: '0.5rem',
      left: '5px',
      width: '1px',
      backgroundColor: colors.border,
    },
  },
  renderRecentSchedulerRunsLi: {
    position: 'relative',
    display: 'flex',
    alignItems: 'center',
    gap: '0.75rem',
    paddingBlock: '0.5rem',
    paddingLeft: '0.75rem',
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
  },
  renderRecentSchedulerRunsSpan2: {
    minWidth: '0px',
    flex: '1',
    overflow: 'hidden',
    textOverflow: 'ellipsis',
    whiteSpace: 'nowrap',
  },
  renderRecentSchedulerRunsSpan3: {
    flexShrink: 0,
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    color: colors.mutedForeground,
    fontVariantNumeric: 'tabular-nums',
  },
});

const SCHEDULER_STATE_DOT_STYLES = {
  danger: styles.schedulerStateDotDanger,
  info: styles.schedulerStateDotInfo,
  neutral: styles.schedulerStateDotNeutral,
  success: styles.schedulerStateDotSuccess,
  warning: styles.schedulerStateDotWarning,
};
