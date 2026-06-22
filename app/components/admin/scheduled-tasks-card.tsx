'use client';

import { useMemo, useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Clock3, Pencil, Plus, Trash2 } from 'lucide-react';

import {
  adminCreateScheduledTask,
  adminDeleteScheduledTask,
  adminGetScheduledTasks,
  adminUpdateScheduledTask,
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

type ScheduledTasksCardProps = {
  token: string;
};

type TaskFormState = {
  command: string;
  cron: string;
  enabled: boolean;
  name: string;
};

type CommandPresetId =
  | 'index-update'
  | 'index-update-folder'
  | 'index-update-external'
  | 'index-update-both'
  | 'folder-only'
  | 'external-only'
  | 'custom';

const COMMAND_PRESETS: {
  command: string;
  label: string;
  value: Exclude<CommandPresetId, 'custom'>;
}[] = [
  { value: 'index-update', label: '索引更新', command: 'index --update' },
  {
    value: 'index-update-folder',
    label: '索引更新 + 文件夹推送',
    command: 'index --update && push',
  },
  {
    value: 'index-update-external',
    label: '索引更新 + 外部推送',
    command: 'index --update && notify',
  },
  {
    value: 'index-update-both',
    label: '索引更新 + 双推送',
    command: 'index --update && notify && push',
  },
  { value: 'folder-only', label: '仅文件夹推送', command: 'push' },
  { value: 'external-only', label: '仅外部推送', command: 'notify' },
];

const DEFAULT_FORM: TaskFormState = {
  command: 'index --update && notify',
  cron: '0 8 * * *',
  enabled: true,
  name: '',
};

const DEFAULT_PRESET: CommandPresetId = 'index-update-external';

function getPresetForCommand(command: string): CommandPresetId {
  return COMMAND_PRESETS.find((preset) => preset.command === command)?.value ?? 'custom';
}

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

export function ScheduledTasksCard({ token }: ScheduledTasksCardProps) {
  const queryClient = useQueryClient();
  const [dialogOpen, setDialogOpen] = useState(false);
  const [editingTask, setEditingTask] = useState<ScheduledTaskInfo | null>(null);
  const [form, setForm] = useState<TaskFormState>(DEFAULT_FORM);
  const [commandPreset, setCommandPreset] = useState<CommandPresetId>(DEFAULT_PRESET);

  const {
    data: tasks = [],
    error,
    isLoading,
  } = useQuery({
    queryKey: ['admin-scheduled-tasks'],
    queryFn: () => adminGetScheduledTasks(token),
  });

  const saveMutation = useMutation({
    mutationFn: async () => {
      if (editingTask) {
        return adminUpdateScheduledTask(token, editingTask.id, form);
      }
      return adminCreateScheduledTask(token, form);
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['admin-scheduled-tasks'] });
      queryClient.invalidateQueries({ queryKey: ['admin-stats'] });
      setDialogOpen(false);
      setEditingTask(null);
      setForm(DEFAULT_FORM);
    },
  });

  const toggleMutation = useMutation({
    mutationFn: ({ enabled, taskId }: { enabled: boolean; taskId: number }) =>
      adminUpdateScheduledTask(token, taskId, { enabled }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['admin-scheduled-tasks'] });
      queryClient.invalidateQueries({ queryKey: ['admin-stats'] });
    },
  });

  const deleteMutation = useMutation({
    mutationFn: (taskId: number) => adminDeleteScheduledTask(token, taskId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['admin-scheduled-tasks'] });
      queryClient.invalidateQueries({ queryKey: ['admin-stats'] });
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
    setForm(DEFAULT_FORM);
    setCommandPreset(DEFAULT_PRESET);
    setDialogOpen(true);
  };

  const openEditDialog = (task: ScheduledTaskInfo) => {
    const preset = getPresetForCommand(task.command);
    setEditingTask(task);
    setForm({
      command: task.command,
      cron: task.cron,
      enabled: task.enabled,
      name: task.name,
    });
    setCommandPreset(preset);
    setDialogOpen(true);
  };

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <Clock3 className="h-5 w-5" />
          定时任务
        </CardTitle>
        <CardDescription>管理后台自动执行的命令任务</CardDescription>
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
              <DialogDescription>使用五段 crontab 表达式，例如 `0 8 * * *`。</DialogDescription>
            </DialogHeader>
            <div className="space-y-4 py-2">
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
              <div className="space-y-2">
                <Label htmlFor="scheduled-task-preset">任务预设</Label>
                <Select
                  value={commandPreset}
                  onValueChange={(value: CommandPresetId) => {
                    setCommandPreset(value);
                    if (value === 'custom') {
                      setForm((current) => ({
                        ...current,
                        command: editingTask?.command ?? current.command,
                      }));
                      return;
                    }

                    const preset = COMMAND_PRESETS.find((item) => item.value === value);
                    if (preset) {
                      setForm((current) => ({ ...current, command: preset.command }));
                    }
                  }}
                >
                  <SelectTrigger id="scheduled-task-preset" className="w-full">
                    <SelectValue placeholder="选择任务预设" />
                  </SelectTrigger>
                  <SelectContent>
                    {COMMAND_PRESETS.map((preset) => (
                      <SelectItem key={preset.value} value={preset.value}>
                        {preset.label}
                      </SelectItem>
                    ))}
                    <SelectItem value="custom">自定义</SelectItem>
                  </SelectContent>
                </Select>
                {commandPreset === 'custom' ? (
                  <Input
                    name="scheduled_task_command"
                    autoComplete="off"
                    spellCheck={false}
                    aria-label="自定义执行命令"
                    value={form.command}
                    onChange={(event) =>
                      setForm((current) => ({ ...current, command: event.target.value }))
                    }
                    placeholder="执行命令"
                  />
                ) : (
                  <div className="rounded-md border bg-muted/40 px-3 py-2 font-mono text-sm text-muted-foreground break-all whitespace-pre-wrap">
                    {form.command}
                  </div>
                )}
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
                  disabled={
                    !form.name.trim() ||
                    !form.cron.trim() ||
                    !form.command.trim() ||
                    saveMutation.isPending
                  }
                  onClick={() => saveMutation.mutate()}
                >
                  {editingTask ? '保存' : '创建'}
                </Button>
              </div>
            </div>
          </DialogContent>
        </Dialog>

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
                    <div className="text-sm text-muted-foreground break-all">{task.command}</div>
                    <div className="text-xs text-muted-foreground">
                      最近执行: {formatDateTime(task.last_run_at)}
                      {task.last_status && ` · ${task.last_status}`}
                    </div>
                  </div>
                  <div className="flex w-full flex-wrap items-center justify-end gap-2 sm:w-auto sm:flex-nowrap">
                    <Switch
                      checked={task.enabled}
                      aria-label={`${task.enabled ? '停用' : '启用'}定时任务 ${task.name}`}
                      onCheckedChange={(checked: boolean) =>
                        toggleMutation.mutate({ enabled: checked, taskId: task.id })
                      }
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
      </CardContent>
    </Card>
  );
}
