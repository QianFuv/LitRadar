'use client';

import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import Link from 'next/link';
import { ArrowLeft, Radar, FolderPlus, Download, Save, X, Plus } from 'lucide-react';

import { useAuth } from '@/lib/auth-context';
import {
  getTrackingStatus,
  getFolders,
  createFolder,
  setTrackingFolder,
  pushWeeklyToTracking,
  getNotificationSettings,
  updateNotificationSettings,
  type NotificationSettings,
  type NotificationSettingsUpdate,
} from '@/lib/api';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { Switch } from '@/components/ui/switch';
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from '@/components/ui/card';
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select';
import { Badge } from '@/components/ui/badge';
import { useState, useCallback } from 'react';

export default function TrackingPage() {
  const { user, token } = useAuth();
  const queryClient = useQueryClient();
  const [newFolderName, setNewFolderName] = useState('');
  const [pushResult, setPushResult] = useState<string | null>(null);
  const [draftSettings, setDraftSettings] = useState<NotificationSettingsUpdate | null>(
    null,
  );
  const [keywordInput, setKeywordInput] = useState('');
  const [directionInput, setDirectionInput] = useState('');
  const [settingsSaved, setSettingsSaved] = useState(false);

  const { data: status } = useQuery({
    queryKey: ['tracking-status'],
    queryFn: () => getTrackingStatus(token!),
    enabled: !!token,
  });

  const { data: folders = [] } = useQuery({
    queryKey: ['folders', user?.id],
    queryFn: () => getFolders(token!),
    enabled: !!token,
  });

  const notificationSettingsQuery = useQuery({
    queryKey: ['notification-settings', user?.id],
    queryFn: () => getNotificationSettings(token!),
    enabled: !!token,
  });
  const notifySettings = notificationSettingsQuery.data;

  const normalizeSettings = useCallback(
    (settings: NotificationSettings | null | undefined): NotificationSettingsUpdate => ({
      keywords: settings?.keywords || [],
      directions: settings?.directions || [],
      delivery_method: settings?.delivery_method || 'folder',
      pushplus_token: settings?.pushplus_token || '',
      pushplus_template: settings?.pushplus_template || 'markdown',
      pushplus_topic: settings?.pushplus_topic || '',
      pushplus_channel: settings?.pushplus_channel || 'wechat',
      sync_to_tracking_folder: settings?.sync_to_tracking_folder ?? false,
      ai_base_url: settings?.ai_base_url || '',
      ai_api_key: settings?.ai_api_key || '',
      ai_model: settings?.ai_model || '',
      ai_system_prompt: settings?.ai_system_prompt || '',
      ai_backup_base_url: settings?.ai_backup_base_url || '',
      ai_backup_api_key: settings?.ai_backup_api_key || '',
      ai_backup_model: settings?.ai_backup_model || '',
      ai_backup_system_prompt: settings?.ai_backup_system_prompt || '',
      ai_retry_attempts: settings?.ai_retry_attempts ?? 3,
      enabled: settings?.enabled ?? true,
    }),
    [],
  );

  const formSettings = draftSettings || normalizeSettings(notifySettings);
  const {
    keywords,
    directions,
    delivery_method: deliveryMethod,
    pushplus_token: pushplusToken,
    pushplus_template: pushplusTemplate,
    pushplus_topic: pushplusTopic,
    pushplus_channel: pushplusChannel,
    sync_to_tracking_folder: syncToTrackingFolder,
    ai_base_url: aiBaseUrl,
    ai_api_key: aiApiKey,
    ai_model: aiModel,
    ai_system_prompt: aiSystemPrompt,
    ai_backup_base_url: aiBackupBaseUrl,
    ai_backup_api_key: aiBackupApiKey,
    ai_backup_model: aiBackupModel,
    ai_backup_system_prompt: aiBackupSystemPrompt,
    ai_retry_attempts: aiRetryAttempts,
    enabled: notifyEnabled,
  } = formSettings;

  const updateDraftSettings = useCallback(
    (
      updater: (
        current: NotificationSettingsUpdate,
      ) => NotificationSettingsUpdate,
    ) => {
      setDraftSettings((current) => updater(current || normalizeSettings(notifySettings)));
      setSettingsSaved(false);
    },
    [normalizeSettings, notifySettings],
  );

  const setTrackMut = useMutation({
    mutationFn: (folderId: number) => setTrackingFolder(token!, folderId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['tracking-status'] });
      queryClient.invalidateQueries({ queryKey: ['folders'] });
    },
  });

  const createAndSetMut = useMutation({
    mutationFn: async (name: string) => {
      const folder = await createFolder(token!, name, true);
      return folder;
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['tracking-status'] });
      queryClient.invalidateQueries({ queryKey: ['folders'] });
      setNewFolderName('');
    },
  });

  const pushMut = useMutation({
    mutationFn: () => pushWeeklyToTracking(token!),
    onSuccess: (data) => {
      if (data.message) {
        setPushResult(
          data.pushed > 0
            ? `${data.message}（已推送 ${data.pushed} 篇）`
            : data.message,
        );
      } else {
        setPushResult(`成功推送 ${data.pushed} 篇文章到追踪文件夹`);
      }
      queryClient.invalidateQueries({ queryKey: ['tracking-status'] });
      queryClient.invalidateQueries({ queryKey: ['folders'] });
    },
    onError: (err) => {
      setPushResult(err instanceof Error ? err.message : '推送失败');
    },
  });

  const saveSettingsMut = useMutation({
    mutationFn: () =>
      updateNotificationSettings(token!, formSettings),
    onSuccess: (savedSettings) => {
      queryClient.setQueryData(['notification-settings', user?.id], savedSettings);
      setDraftSettings(null);
      queryClient.invalidateQueries({ queryKey: ['notification-settings', user?.id] });
      queryClient.invalidateQueries({ queryKey: ['tracking-status'] });
      setSettingsSaved(true);
      setTimeout(() => setSettingsSaved(false), 2000);
    },
  });

  function addKeyword() {
    const val = keywordInput.trim();
    if (val && !keywords.includes(val)) {
      updateDraftSettings((current) => ({
        ...current,
        keywords: [...current.keywords, val],
      }));
    }
    setKeywordInput('');
  }

  function addDirection() {
    const val = directionInput.trim();
    if (val && !directions.includes(val)) {
      updateDraftSettings((current) => ({
        ...current,
        directions: [...current.directions, val],
      }));
    }
    setDirectionInput('');
  }

  if (!user) {
    return (
      <div className="flex flex-col items-center justify-center min-h-[60vh] gap-4">
        <p className="text-muted-foreground">请先登录</p>
        <Button asChild>
          <Link href="/login?next=/tracking">登录</Link>
        </Button>
      </div>
    );
  }

  return (
    <div className="mx-auto max-w-3xl space-y-4 p-4 sm:space-y-6 sm:p-6">
      <div className="flex items-start gap-2 sm:gap-3">
        <Button variant="ghost" size="icon" asChild>
          <Link href="/">
            <ArrowLeft className="h-5 w-5" />
          </Link>
        </Button>
        <h1 className="text-2xl font-bold flex items-center gap-2">
          <Radar className="h-6 w-6" />
          文献追踪
        </h1>
      </div>

      <Card>
        <CardHeader>
          <CardTitle>追踪文件夹</CardTitle>
          <CardDescription>
            设置追踪文件夹后，每周推送的新文章将自动收藏到该文件夹中
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          {status?.tracking_folder ? (
            <div className="flex items-center gap-2">
              <Badge variant="secondary" className="text-sm">
                当前追踪: {status.tracking_folder.name}
              </Badge>
            </div>
          ) : (
            <p className="text-sm text-muted-foreground">尚未设置追踪文件夹</p>
          )}

          {folders.length > 0 && (
            <div className="flex flex-col gap-2 sm:flex-row sm:items-center">
              <Select
                value={status?.tracking_folder?.id?.toString() || ''}
                onValueChange={(val) => setTrackMut.mutate(Number(val))}
              >
                <SelectTrigger className="w-full sm:w-60">
                  <SelectValue placeholder="选择追踪文件夹" />
                </SelectTrigger>
                <SelectContent>
                  {folders.map((f) => (
                    <SelectItem key={f.id} value={f.id.toString()}>
                      {f.name} ({f.article_count})
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
          )}

          <div className="flex flex-col gap-2 sm:flex-row sm:items-center">
            <Input
              value={newFolderName}
              onChange={(e) => setNewFolderName(e.target.value)}
              placeholder="新建追踪文件夹"
              className="w-full sm:w-60"
            />
            <Button
              variant="outline"
              size="sm"
              className="w-full sm:w-auto"
              disabled={!newFolderName.trim() || createAndSetMut.isPending}
              onClick={() => createAndSetMut.mutate(newFolderName.trim())}
            >
              <FolderPlus className="h-4 w-4 mr-1" />
              创建并设为追踪
            </Button>
          </div>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>手动推送</CardTitle>
          <CardDescription>
            将最近一周的文章按当前 AI 推荐规则同步到追踪文件夹
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
            <div className="text-sm text-muted-foreground">
              可推送文章: {status?.weekly_articles_available ?? '...'} 篇
            </div>
            <Button
              className="w-full sm:w-auto"
              onClick={() => pushMut.mutate()}
              disabled={pushMut.isPending || !status?.tracking_folder}
            >
              <Download className="h-4 w-4 mr-1" />
              {pushMut.isPending ? '推送中...' : '推送到追踪文件夹'}
            </Button>
          </div>
          {pushResult && (
            <div className="rounded-md border px-3 py-2 text-sm">
              {pushResult}
            </div>
          )}
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>AI 推荐配置</CardTitle>
          <CardDescription>
            只有在启用推荐、填写关键词或研究方向、且至少有一套可用 AI 配置时，系统才会推送文章
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-5">
          {notificationSettingsQuery.isPending && draftSettings === null ? (
            <div className="rounded-md border px-3 py-4 text-sm text-muted-foreground">
              正在加载已保存的推荐配置...
            </div>
          ) : notificationSettingsQuery.isError && draftSettings === null ? (
            <div className="rounded-md border border-destructive/50 px-3 py-4 text-sm text-destructive">
              {notificationSettingsQuery.error instanceof Error
                ? notificationSettingsQuery.error.message
                : '加载推荐配置失败'}
            </div>
          ) : (
            <>
          <div className="flex items-start justify-between gap-3">
            <Label htmlFor="notify-enabled">启用推荐</Label>
            <Switch
              id="notify-enabled"
              checked={notifyEnabled}
              onCheckedChange={(checked) =>
                updateDraftSettings((current) => ({
                  ...current,
                  enabled: checked,
                }))
              }
            />
          </div>

          <div className="space-y-2">
            <Label>关键词</Label>
            <div className="flex flex-wrap gap-1.5 min-h-[2rem]">
              {keywords.map((kw) => (
                <Badge key={kw} variant="secondary" className="gap-1 pr-1">
                  {kw}
                  <button
                    type="button"
                    onClick={() =>
                      updateDraftSettings((current) => ({
                        ...current,
                        keywords: current.keywords.filter((k) => k !== kw),
                      }))
                    }
                    className="rounded-full hover:bg-muted p-0.5"
                  >
                    <X className="h-3 w-3" />
                  </button>
                </Badge>
              ))}
            </div>
            <div className="flex flex-col gap-2 sm:flex-row">
              <Input
                value={keywordInput}
                onChange={(e) => setKeywordInput(e.target.value)}
                onKeyDown={(e) => { if (e.key === 'Enter') { e.preventDefault(); addKeyword(); } }}
                placeholder="输入关键词后回车添加"
                className="flex-1"
              />
              <Button
                type="button"
                variant="outline"
                size="sm"
                className="w-full sm:w-auto"
                onClick={addKeyword}
                disabled={!keywordInput.trim()}
              >
                <Plus className="h-4 w-4" />
              </Button>
            </div>
          </div>

          <div className="space-y-2">
            <Label>研究方向</Label>
            <div className="flex flex-wrap gap-1.5 min-h-[2rem]">
              {directions.map((d) => (
                <Badge key={d} variant="secondary" className="gap-1 pr-1">
                  {d}
                  <button
                    type="button"
                    onClick={() =>
                      updateDraftSettings((current) => ({
                        ...current,
                        directions: current.directions.filter((x) => x !== d),
                      }))
                    }
                    className="rounded-full hover:bg-muted p-0.5"
                  >
                    <X className="h-3 w-3" />
                  </button>
                </Badge>
              ))}
            </div>
            <div className="flex flex-col gap-2 sm:flex-row">
              <Input
                value={directionInput}
                onChange={(e) => setDirectionInput(e.target.value)}
                onKeyDown={(e) => { if (e.key === 'Enter') { e.preventDefault(); addDirection(); } }}
                placeholder="输入研究方向后回车添加"
                className="flex-1"
              />
              <Button
                type="button"
                variant="outline"
                size="sm"
                className="w-full sm:w-auto"
                onClick={addDirection}
                disabled={!directionInput.trim()}
              >
                <Plus className="h-4 w-4" />
              </Button>
            </div>
          </div>

          <div className="space-y-2">
            <Label>推送方式</Label>
            <Select
              value={deliveryMethod}
              onValueChange={(v) =>
                updateDraftSettings((current) => ({
                  ...current,
                  delivery_method: v as 'folder' | 'pushplus',
                }))
              }
            >
              <SelectTrigger className="w-full sm:w-60">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="folder">追踪文件夹推送</SelectItem>
                <SelectItem value="pushplus">PushPlus 外部推送</SelectItem>
              </SelectContent>
            </Select>
          </div>

          <div className="space-y-4 rounded-md border p-3">
            <div className="space-y-2">
              <div className="flex items-center justify-between gap-3">
                <div>
                  <Label className="text-base">主 AI 配置</Label>
                  <p className="text-xs text-muted-foreground mt-1">
                    优先使用这套配置进行筛选；留空字段会回退到服务端默认值。
                  </p>
                </div>
              </div>
            </div>
            <div className="grid gap-3 md:grid-cols-2">
              <div className="space-y-1">
                <Label htmlFor="ai-base-url">Base URL</Label>
                <Input
                  id="ai-base-url"
                  value={aiBaseUrl}
                  onChange={(e) =>
                    updateDraftSettings((current) => ({
                      ...current,
                      ai_base_url: e.target.value,
                    }))
                  }
                  placeholder="https://api.openai.com/v1"
                />
              </div>
              <div className="space-y-1">
                <Label htmlFor="ai-model">Model</Label>
                <Input
                  id="ai-model"
                  value={aiModel}
                  onChange={(e) =>
                    updateDraftSettings((current) => ({
                      ...current,
                      ai_model: e.target.value,
                    }))
                  }
                  placeholder="gpt-4.1-mini"
                />
              </div>
            </div>
            <div className="space-y-1">
              <Label htmlFor="ai-api-key">API Key</Label>
              <Input
                id="ai-api-key"
                type="password"
                value={aiApiKey}
                onChange={(e) =>
                  updateDraftSettings((current) => ({
                    ...current,
                    ai_api_key: e.target.value,
                  }))
                }
                placeholder="sk-..."
              />
            </div>
            <div className="space-y-1">
              <Label htmlFor="ai-system-prompt">System Prompt</Label>
              <textarea
                id="ai-system-prompt"
                value={aiSystemPrompt}
                onChange={(e) =>
                  updateDraftSettings((current) => ({
                    ...current,
                    ai_system_prompt: e.target.value,
                  }))
                }
                placeholder="Describe how the model should evaluate article relevance."
                className="min-h-28 w-full rounded-md border border-input bg-transparent px-3 py-2 text-sm shadow-xs outline-none placeholder:text-muted-foreground focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/50"
              />
            </div>
            <div className="grid gap-3 md:grid-cols-2">
              <div className="space-y-1">
                <Label htmlFor="ai-retry-attempts">失败重试次数</Label>
                <Input
                  id="ai-retry-attempts"
                  type="number"
                  min={1}
                  max={10}
                  value={aiRetryAttempts}
                  onChange={(e) =>
                    updateDraftSettings((current) => ({
                      ...current,
                      ai_retry_attempts: Math.max(1, Math.min(10, Number(e.target.value) || 1)),
                    }))
                  }
                />
              </div>
            </div>
            <div className="space-y-3 rounded-md border border-dashed p-3">
              <div>
                <Label className="text-base">备用 AI 配置</Label>
                <p className="text-xs text-muted-foreground mt-1">
                  当主配置连续失败后，系统会自动切换到这套备用配置重试。
                </p>
              </div>
              <div className="grid gap-3 md:grid-cols-2">
                <div className="space-y-1">
                  <Label htmlFor="ai-backup-base-url">Backup Base URL</Label>
                  <Input
                    id="ai-backup-base-url"
                    value={aiBackupBaseUrl}
                    onChange={(e) =>
                      updateDraftSettings((current) => ({
                        ...current,
                        ai_backup_base_url: e.target.value,
                      }))
                    }
                    placeholder="https://api.openai.com/v1"
                  />
                </div>
                <div className="space-y-1">
                  <Label htmlFor="ai-backup-model">Backup Model</Label>
                  <Input
                    id="ai-backup-model"
                    value={aiBackupModel}
                    onChange={(e) =>
                      updateDraftSettings((current) => ({
                        ...current,
                        ai_backup_model: e.target.value,
                      }))
                    }
                    placeholder="gpt-4.1-mini"
                  />
                </div>
              </div>
              <div className="space-y-1">
                <Label htmlFor="ai-backup-api-key">Backup API Key</Label>
                <Input
                  id="ai-backup-api-key"
                  type="password"
                  value={aiBackupApiKey}
                  onChange={(e) =>
                    updateDraftSettings((current) => ({
                      ...current,
                      ai_backup_api_key: e.target.value,
                    }))
                  }
                  placeholder="sk-..."
                />
              </div>
              <div className="space-y-1">
                <Label htmlFor="ai-backup-system-prompt">Backup System Prompt</Label>
                <textarea
                  id="ai-backup-system-prompt"
                  value={aiBackupSystemPrompt}
                  onChange={(e) =>
                    updateDraftSettings((current) => ({
                      ...current,
                      ai_backup_system_prompt: e.target.value,
                    }))
                  }
                  placeholder="Optional backup prompt override."
                  className="min-h-28 w-full rounded-md border border-input bg-transparent px-3 py-2 text-sm shadow-xs outline-none placeholder:text-muted-foreground focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/50"
                />
              </div>
            </div>
            <p className="text-xs text-muted-foreground">
              未配置关键词或研究方向时不会推送；主备 AI 都不可用时同样会跳过推送。
            </p>
          </div>

          {deliveryMethod === 'pushplus' && (
            <div className="space-y-3 rounded-md border p-3">
              <div className="space-y-1">
                <Label htmlFor="pp-token">PushPlus 令牌</Label>
                <Input
                  id="pp-token"
                  value={pushplusToken}
                  onChange={(e) =>
                    updateDraftSettings((current) => ({
                      ...current,
                      pushplus_token: e.target.value,
                    }))
                  }
                  placeholder="输入你的 PushPlus 令牌"
                />
              </div>
              <div className="grid gap-3 sm:grid-cols-2">
                <div className="space-y-1">
                  <Label htmlFor="pp-template">模板</Label>
                  <Input
                    id="pp-template"
                    value={pushplusTemplate}
                    onChange={(e) =>
                      updateDraftSettings((current) => ({
                        ...current,
                        pushplus_template: e.target.value,
                      }))
                    }
                    placeholder="markdown"
                  />
                </div>
                <div className="space-y-1">
                  <Label htmlFor="pp-topic">主题</Label>
                  <Input
                    id="pp-topic"
                    value={pushplusTopic}
                    onChange={(e) =>
                      updateDraftSettings((current) => ({
                        ...current,
                        pushplus_topic: e.target.value,
                      }))
                    }
                    placeholder="可选"
                  />
                </div>
              </div>
              <div className="space-y-1">
                <Label htmlFor="pp-channel">渠道</Label>
                <Input
                  id="pp-channel"
                  value={pushplusChannel}
                  onChange={(e) =>
                    updateDraftSettings((current) => ({
                      ...current,
                      pushplus_channel: e.target.value,
                    }))
                  }
                  placeholder="wechat"
                />
                <p className="text-xs text-muted-foreground">
                  填写 PushPlus 渠道，例如 `wechat`。
                </p>
              </div>
              <div className="flex flex-col gap-3 rounded-md border border-dashed p-3 sm:flex-row sm:items-start sm:justify-between">
                <div className="space-y-1">
                  <Label htmlFor="pp-sync-tracking">同步写入追踪文件夹</Label>
                  <p className="text-xs text-muted-foreground">
                    {status?.tracking_folder
                      ? `发送 PushPlus 时，同时写入“${status.tracking_folder.name}”`
                      : '需要先设置追踪文件夹后才能开启'}
                  </p>
                </div>
                <Switch
                  id="pp-sync-tracking"
                  checked={syncToTrackingFolder}
                  disabled={!status?.tracking_folder}
                  onCheckedChange={(checked) =>
                    updateDraftSettings((current) => ({
                      ...current,
                      sync_to_tracking_folder: checked,
                    }))
                  }
                />
              </div>
            </div>
          )}

          <div className="flex flex-col gap-3 sm:flex-row sm:items-center">
            <Button
              className="w-full sm:w-auto"
              onClick={() => saveSettingsMut.mutate()}
              disabled={saveSettingsMut.isPending}
            >
              <Save className="h-4 w-4 mr-1" />
              {saveSettingsMut.isPending ? '保存中...' : '保存配置'}
            </Button>
            {settingsSaved && (
              <span className="text-sm text-green-600">已保存</span>
            )}
            {saveSettingsMut.isError && (
              <span className="text-sm text-destructive">
                {saveSettingsMut.error instanceof Error
                  ? saveSettingsMut.error.message
                  : '保存失败'}
              </span>
            )}
          </div>
            </>
          )}
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>文献追踪说明</CardTitle>
        </CardHeader>
        <CardContent className="text-sm text-muted-foreground space-y-2">
          <p>1. 创建或选择一个收藏夹，设为「追踪文件夹」</p>
          <p>2. 配置关键词、研究方向和至少一套可用的 OpenAI 兼容 AI 服务</p>
          <p>3. 选择推送方式：推送到追踪文件夹或通过 PushPlus 外部推送</p>
          <p>4. 系统只会推送 AI 推荐出的文章；未配置偏好或 AI 不可用时会跳过</p>
          <p>5. 主配置失败后会自动切换到备用 AI 配置并重试</p>
          <p>6. 也可以手动触发推送同步</p>
          <p>7. 在「我的收藏」中查看追踪到的文章</p>
        </CardContent>
      </Card>
    </div>
  );
}
