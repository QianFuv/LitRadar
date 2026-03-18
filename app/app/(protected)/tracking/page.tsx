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

  const { data: notifySettings } = useQuery({
    queryKey: ['notification-settings'],
    queryFn: () => getNotificationSettings(token!),
    enabled: !!token,
  });

  const normalizeSettings = useCallback(
    (settings: NotificationSettings | null | undefined): NotificationSettingsUpdate => ({
      keywords: settings?.keywords || [],
      directions: settings?.directions || [],
      delivery_method: settings?.delivery_method || 'folder',
      pushplus_token: settings?.pushplus_token || '',
      pushplus_template: settings?.pushplus_template || 'markdown',
      pushplus_topic: settings?.pushplus_topic || '',
      pushplus_to: settings?.pushplus_to || '',
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
    pushplus_to: pushplusTo,
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
      queryClient.setQueryData(['notification-settings'], savedSettings);
      setDraftSettings(null);
      queryClient.invalidateQueries({ queryKey: ['notification-settings'] });
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
    <div className="max-w-3xl mx-auto p-6 space-y-6">
      <div className="flex items-center gap-3">
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
            <div className="flex items-center gap-2">
              <Select
                value={status?.tracking_folder?.id?.toString() || ''}
                onValueChange={(val) => setTrackMut.mutate(Number(val))}
              >
                <SelectTrigger className="w-60">
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

          <div className="flex items-center gap-2">
            <Input
              value={newFolderName}
              onChange={(e) => setNewFolderName(e.target.value)}
              placeholder="新建追踪文件夹"
              className="w-60"
            />
            <Button
              variant="outline"
              size="sm"
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
            将最近一周的推送文章手动同步到追踪文件夹
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="flex items-center gap-4">
            <div className="text-sm text-muted-foreground">
              可推送文章: {status?.weekly_articles_available ?? '...'} 篇
            </div>
            <Button
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
            配置关键词和研究方向，AI 将根据你的偏好筛选和推荐文章
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-5">
          <div className="flex items-center justify-between">
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
            <div className="flex gap-2">
              <Input
                value={keywordInput}
                onChange={(e) => setKeywordInput(e.target.value)}
                onKeyDown={(e) => { if (e.key === 'Enter') { e.preventDefault(); addKeyword(); } }}
                placeholder="输入关键词后回车添加"
                className="flex-1"
              />
              <Button type="button" variant="outline" size="sm" onClick={addKeyword} disabled={!keywordInput.trim()}>
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
            <div className="flex gap-2">
              <Input
                value={directionInput}
                onChange={(e) => setDirectionInput(e.target.value)}
                onKeyDown={(e) => { if (e.key === 'Enter') { e.preventDefault(); addDirection(); } }}
                placeholder="输入研究方向后回车添加"
                className="flex-1"
              />
              <Button type="button" variant="outline" size="sm" onClick={addDirection} disabled={!directionInput.trim()}>
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
              <SelectTrigger className="w-60">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="folder">收藏到追踪文件夹</SelectItem>
                <SelectItem value="pushplus">PushPlus 推送</SelectItem>
              </SelectContent>
            </Select>
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
              <div className="grid grid-cols-2 gap-3">
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
                <Label htmlFor="pp-to">接收方</Label>
                <Input
                  id="pp-to"
                  value={pushplusTo}
                  onChange={(e) =>
                    updateDraftSettings((current) => ({
                      ...current,
                      pushplus_to: e.target.value,
                    }))
                  }
                  placeholder="可选，指定接收方"
                />
              </div>
            </div>
          )}

          <div className="flex items-center gap-3">
            <Button
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
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>文献追踪说明</CardTitle>
        </CardHeader>
        <CardContent className="text-sm text-muted-foreground space-y-2">
          <p>1. 创建或选择一个收藏夹，设为「追踪文件夹」</p>
          <p>2. 配置关键词和研究方向，AI 将自动筛选推荐相关文献</p>
          <p>3. 选择推送方式：收藏到文件夹或通过 PushPlus 推送</p>
          <p>4. 系统每周推送的新文章会根据你的偏好自动筛选和推荐</p>
          <p>5. 也可以手动触发推送同步</p>
          <p>6. 在「我的收藏」中查看追踪到的文章</p>
        </CardContent>
      </Card>
    </div>
  );
}
