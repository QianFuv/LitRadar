'use client';

/**
 * Recommendation, delivery, database, and AI configuration section.
 */

import { Plus, Save, X } from 'lucide-react';

import type { TrackingPageViewModel } from '@/components/tracking/use-tracking-page';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';
import { Checkbox } from '@/components/ui/checkbox';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select';
import { Switch } from '@/components/ui/switch';
import { Textarea } from '@/components/ui/textarea';

type RecommendationSettingsCardProps = {
  model: TrackingPageViewModel['recommendation'];
};

/**
 * Render recommendation preferences, database scope, delivery, and AI settings.
 *
 * @param props - Recommendation-specific tracking view model.
 * @returns Recommendation settings card.
 */
export function RecommendationSettingsCard({ model }: RecommendationSettingsCardProps) {
  const { backup, primary, retryAttempts } = model.ai;
  const { directions, keywords } = model.preferences;
  const databaseSelection = model.databaseSelection;
  const pushplus = model.delivery.pushplus;

  return (
    <Card>
      <CardHeader>
        <CardTitle>AI 推荐配置</CardTitle>
        <CardDescription>
          只有在启用推荐、填写关键词或研究方向、且至少有一套可用 AI 配置时，系统才会推送文章
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-5">
        {model.notificationQuery.isPending && !model.hasDraft ? (
          <div role="status" className="rounded-md border px-3 py-4 text-sm text-muted-foreground">
            正在加载已保存的推荐配置…
          </div>
        ) : model.notificationQuery.isError && !model.hasDraft ? (
          <div
            role="alert"
            className="rounded-md border border-destructive/50 px-3 py-4 text-sm text-destructive"
          >
            {model.notificationQuery.error instanceof Error
              ? model.notificationQuery.error.message
              : '加载推荐配置失败'}
          </div>
        ) : (
          <>
            <div className="flex items-start justify-between gap-3">
              <Label htmlFor="notify-enabled">启用推荐</Label>
              <Switch
                id="notify-enabled"
                name="notification_enabled"
                checked={model.enabled}
                onCheckedChange={(checked: boolean) =>
                  model.updateSettings((current) => ({
                    ...current,
                    enabled: checked,
                  }))
                }
              />
            </div>

            <div className="space-y-2">
              <Label htmlFor="keyword-input">关键词</Label>
              <div className="flex min-h-[2rem] flex-wrap gap-1.5">
                {keywords.items.map((keyword) => (
                  <Badge key={keyword} variant="secondary" className="gap-1 pr-1">
                    {keyword}
                    <button
                      type="button"
                      aria-label={`移除关键词 ${keyword}`}
                      onClick={() =>
                        model.updateSettings((current) => ({
                          ...current,
                          keywords: current.keywords.filter((item) => item !== keyword),
                        }))
                      }
                      className="rounded-full p-0.5 hover:bg-muted"
                    >
                      <X className="h-3 w-3" />
                    </button>
                  </Badge>
                ))}
              </div>
              <div className="flex flex-col gap-2 sm:flex-row">
                <Input
                  id="keyword-input"
                  name="notification_keyword"
                  autoComplete="off"
                  spellCheck={false}
                  value={keywords.input}
                  onChange={(event) => keywords.setInput(event.target.value)}
                  onKeyDown={(event) => {
                    if (event.key === 'Enter') {
                      event.preventDefault();
                      keywords.add();
                    }
                  }}
                  placeholder="输入关键词后回车添加"
                  className="flex-1"
                />
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  className="w-full sm:w-auto"
                  aria-label="添加关键词"
                  onClick={keywords.add}
                  disabled={!keywords.input.trim()}
                >
                  <Plus className="h-4 w-4" />
                </Button>
              </div>
            </div>

            <div className="space-y-2">
              <Label htmlFor="direction-input">研究方向</Label>
              <div className="flex min-h-[2rem] flex-wrap gap-1.5">
                {directions.items.map((direction) => (
                  <Badge key={direction} variant="secondary" className="gap-1 pr-1">
                    {direction}
                    <button
                      type="button"
                      aria-label={`移除研究方向 ${direction}`}
                      onClick={() =>
                        model.updateSettings((current) => ({
                          ...current,
                          directions: current.directions.filter((item) => item !== direction),
                        }))
                      }
                      className="rounded-full p-0.5 hover:bg-muted"
                    >
                      <X className="h-3 w-3" />
                    </button>
                  </Badge>
                ))}
              </div>
              <div className="flex flex-col gap-2 sm:flex-row">
                <Input
                  id="direction-input"
                  name="notification_direction"
                  autoComplete="off"
                  spellCheck={false}
                  value={directions.input}
                  onChange={(event) => directions.setInput(event.target.value)}
                  onKeyDown={(event) => {
                    if (event.key === 'Enter') {
                      event.preventDefault();
                      directions.add();
                    }
                  }}
                  placeholder="输入研究方向后回车添加"
                  className="flex-1"
                />
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  className="w-full sm:w-auto"
                  aria-label="添加研究方向"
                  onClick={directions.add}
                  disabled={!directions.input.trim()}
                >
                  <Plus className="h-4 w-4" />
                </Button>
              </div>
            </div>

            <div
              className="space-y-3 rounded-md border p-3"
              role="group"
              aria-labelledby="push-databases-label"
            >
              <div className="space-y-1">
                <div id="push-databases-label" className="text-base font-medium">
                  推送数据库
                </div>
                <p className="text-xs text-muted-foreground">
                  手动推送和自动推送都会按这里的数据库范围执行；不限制时表示全部数据库。
                </p>
              </div>
              {databaseSelection.query.isPending ? (
                <div
                  role="status"
                  className="rounded-md border border-dashed px-3 py-4 text-sm text-muted-foreground"
                >
                  正在加载数据库列表…
                </div>
              ) : databaseSelection.query.isError ? (
                <div
                  role="alert"
                  className="rounded-md border border-destructive/50 px-3 py-4 text-sm text-destructive"
                >
                  {databaseSelection.query.error instanceof Error
                    ? databaseSelection.query.error.message
                    : '加载数据库列表失败'}
                </div>
              ) : databaseSelection.available.length === 0 ? (
                <div className="rounded-md border border-dashed px-3 py-4 text-sm text-muted-foreground">
                  当前没有可用数据库。
                </div>
              ) : (
                <div className="space-y-3">
                  <div className="flex flex-col gap-2 rounded-md border border-dashed p-3 sm:flex-row sm:items-center sm:justify-between">
                    <div className="space-y-1">
                      <div className="text-sm font-medium">全部数据库</div>
                      <p className="text-xs text-muted-foreground">
                        选中后，新增加的数据库也会自动纳入推送范围。
                      </p>
                    </div>
                    <Button
                      type="button"
                      variant={databaseSelection.allSelected ? 'default' : 'outline'}
                      size="sm"
                      className="w-full sm:w-auto"
                      onClick={databaseSelection.selectAll}
                    >
                      设为全部数据库
                    </Button>
                  </div>
                  <div className="grid gap-2 sm:grid-cols-2">
                    {databaseSelection.available.map((databaseName) => {
                      const isChecked =
                        databaseSelection.allSelected ||
                        databaseSelection.effectiveSelected.includes(databaseName);
                      return (
                        <label
                          key={databaseName}
                          className="content-visibility-row flex items-start gap-3 rounded-md border px-3 py-2 text-sm"
                        >
                          <Checkbox
                            checked={isChecked}
                            onCheckedChange={(checked: boolean | 'indeterminate') =>
                              databaseSelection.setSelected(databaseName, Boolean(checked))
                            }
                          />
                          <span className="break-all">{databaseName}</span>
                        </label>
                      );
                    })}
                  </div>
                  <p className="text-xs text-muted-foreground">
                    当前范围:{' '}
                    {databaseSelection.allSelected
                      ? `全部数据库（${databaseSelection.available.length} 个）`
                      : `已选 ${databaseSelection.effectiveSelected.length} / ${databaseSelection.available.length} 个数据库`}
                  </p>
                </div>
              )}
            </div>

            <div className="space-y-2">
              <Label htmlFor="delivery-method">推送方式</Label>
              <Select
                name="delivery_method"
                value={model.delivery.method}
                onValueChange={(value: string) =>
                  model.updateSettings((current) => ({
                    ...current,
                    delivery_method: value as 'folder' | 'pushplus',
                  }))
                }
              >
                <SelectTrigger id="delivery-method" className="w-full sm:w-60">
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
                    <div className="text-base font-medium">主 AI 配置</div>
                    <p className="mt-1 text-xs text-muted-foreground">
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
                    name="ai_base_url"
                    type="url"
                    autoComplete="off"
                    spellCheck={false}
                    value={primary.baseUrl}
                    onChange={(event) =>
                      model.updateSettings((current) => ({
                        ...current,
                        ai_base_url: event.target.value,
                      }))
                    }
                    placeholder="https://api.openai.com/v1"
                  />
                </div>
                <div className="space-y-1">
                  <Label htmlFor="ai-model">Model</Label>
                  <Input
                    id="ai-model"
                    name="ai_model"
                    autoComplete="off"
                    spellCheck={false}
                    value={primary.model}
                    onChange={(event) =>
                      model.updateSettings((current) => ({
                        ...current,
                        ai_model: event.target.value,
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
                  name="ai_api_key"
                  type="password"
                  autoComplete="off"
                  spellCheck={false}
                  value={primary.apiKey ?? ''}
                  onChange={(event) =>
                    model.updateSettings((current) => ({
                      ...current,
                      ai_api_key: event.target.value,
                    }))
                  }
                  placeholder="sk-…"
                />
                <div className="flex items-center justify-between gap-2 text-xs text-muted-foreground">
                  <span>
                    {model.storedSettings?.has_ai_api_key
                      ? primary.apiKey === null
                        ? '保存后清除当前密钥'
                        : '已安全保存；留空不会覆盖'
                      : '尚未配置'}
                  </span>
                  {model.storedSettings?.has_ai_api_key && (
                    <Button
                      type="button"
                      variant="ghost"
                      size="sm"
                      onClick={() =>
                        model.updateSettings((current) => ({
                          ...current,
                          ai_api_key: current.ai_api_key === null ? undefined : null,
                        }))
                      }
                    >
                      {primary.apiKey === null ? '保留原密钥' : '清除密钥'}
                    </Button>
                  )}
                </div>
              </div>
              <div className="space-y-1">
                <Label htmlFor="ai-system-prompt">System Prompt</Label>
                <Textarea
                  id="ai-system-prompt"
                  name="ai_system_prompt"
                  autoComplete="off"
                  spellCheck={false}
                  value={primary.systemPrompt}
                  onChange={(event) =>
                    model.updateSettings((current) => ({
                      ...current,
                      ai_system_prompt: event.target.value,
                    }))
                  }
                  placeholder="Describe how the model should evaluate article relevance."
                  className="min-h-28"
                />
              </div>
              <div className="grid gap-3 md:grid-cols-2">
                <div className="space-y-1">
                  <Label htmlFor="ai-retry-attempts">失败重试次数</Label>
                  <Input
                    id="ai-retry-attempts"
                    name="ai_retry_attempts"
                    type="number"
                    autoComplete="off"
                    inputMode="numeric"
                    min={1}
                    max={10}
                    value={retryAttempts}
                    onChange={(event) =>
                      model.updateSettings((current) => ({
                        ...current,
                        ai_retry_attempts: Math.max(
                          1,
                          Math.min(10, Number(event.target.value) || 1),
                        ),
                      }))
                    }
                  />
                </div>
              </div>
              <div className="space-y-3 rounded-md border border-dashed p-3">
                <div>
                  <div className="text-base font-medium">备用 AI 配置</div>
                  <p className="mt-1 text-xs text-muted-foreground">
                    当主配置连续失败后，系统会自动切换到这套备用配置重试。
                  </p>
                </div>
                <div className="grid gap-3 md:grid-cols-2">
                  <div className="space-y-1">
                    <Label htmlFor="ai-backup-base-url">Backup Base URL</Label>
                    <Input
                      id="ai-backup-base-url"
                      name="ai_backup_base_url"
                      type="url"
                      autoComplete="off"
                      spellCheck={false}
                      value={backup.baseUrl}
                      onChange={(event) =>
                        model.updateSettings((current) => ({
                          ...current,
                          ai_backup_base_url: event.target.value,
                        }))
                      }
                      placeholder="https://api.openai.com/v1"
                    />
                  </div>
                  <div className="space-y-1">
                    <Label htmlFor="ai-backup-model">Backup Model</Label>
                    <Input
                      id="ai-backup-model"
                      name="ai_backup_model"
                      autoComplete="off"
                      spellCheck={false}
                      value={backup.model}
                      onChange={(event) =>
                        model.updateSettings((current) => ({
                          ...current,
                          ai_backup_model: event.target.value,
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
                    name="ai_backup_api_key"
                    type="password"
                    autoComplete="off"
                    spellCheck={false}
                    value={backup.apiKey ?? ''}
                    onChange={(event) =>
                      model.updateSettings((current) => ({
                        ...current,
                        ai_backup_api_key: event.target.value,
                      }))
                    }
                    placeholder="sk-…"
                  />
                  <div className="flex items-center justify-between gap-2 text-xs text-muted-foreground">
                    <span>
                      {model.storedSettings?.has_ai_backup_api_key
                        ? backup.apiKey === null
                          ? '保存后清除当前密钥'
                          : '已安全保存；留空不会覆盖'
                        : '尚未配置'}
                    </span>
                    {model.storedSettings?.has_ai_backup_api_key && (
                      <Button
                        type="button"
                        variant="ghost"
                        size="sm"
                        onClick={() =>
                          model.updateSettings((current) => ({
                            ...current,
                            ai_backup_api_key:
                              current.ai_backup_api_key === null ? undefined : null,
                          }))
                        }
                      >
                        {backup.apiKey === null ? '保留原密钥' : '清除密钥'}
                      </Button>
                    )}
                  </div>
                </div>
                <div className="space-y-1">
                  <Label htmlFor="ai-backup-system-prompt">Backup System Prompt</Label>
                  <Textarea
                    id="ai-backup-system-prompt"
                    name="ai_backup_system_prompt"
                    autoComplete="off"
                    spellCheck={false}
                    value={backup.systemPrompt}
                    onChange={(event) =>
                      model.updateSettings((current) => ({
                        ...current,
                        ai_backup_system_prompt: event.target.value,
                      }))
                    }
                    placeholder="Optional backup prompt override."
                    className="min-h-28"
                  />
                </div>
              </div>
              <p className="text-xs text-muted-foreground">
                未配置关键词或研究方向时不会推送；主备 AI 都不可用时同样会跳过推送。
              </p>
            </div>

            {model.delivery.method === 'pushplus' && (
              <div className="space-y-3 rounded-md border p-3">
                <div className="space-y-1">
                  <Label htmlFor="pp-token">PushPlus 令牌</Label>
                  <Input
                    id="pp-token"
                    name="pushplus_token"
                    autoComplete="off"
                    spellCheck={false}
                    type="password"
                    value={pushplus.token ?? ''}
                    onChange={(event) =>
                      model.updateSettings((current) => ({
                        ...current,
                        pushplus_token: event.target.value,
                      }))
                    }
                    placeholder="输入你的 PushPlus 令牌"
                  />
                  <div className="flex items-center justify-between gap-2 text-xs text-muted-foreground">
                    <span>
                      {model.storedSettings?.has_pushplus_token
                        ? pushplus.token === null
                          ? '保存后清除当前令牌'
                          : '已安全保存；留空不会覆盖'
                        : '尚未配置'}
                    </span>
                    {model.storedSettings?.has_pushplus_token && (
                      <Button
                        type="button"
                        variant="ghost"
                        size="sm"
                        onClick={() =>
                          model.updateSettings((current) => ({
                            ...current,
                            pushplus_token: current.pushplus_token === null ? undefined : null,
                          }))
                        }
                      >
                        {pushplus.token === null ? '保留原令牌' : '清除令牌'}
                      </Button>
                    )}
                  </div>
                </div>
                <div className="grid gap-3 sm:grid-cols-2">
                  <div className="space-y-1">
                    <Label htmlFor="pp-template">模板</Label>
                    <Input
                      id="pp-template"
                      name="pushplus_template"
                      autoComplete="off"
                      spellCheck={false}
                      value={pushplus.template}
                      onChange={(event) =>
                        model.updateSettings((current) => ({
                          ...current,
                          pushplus_template: event.target.value,
                        }))
                      }
                      placeholder="markdown"
                    />
                  </div>
                  <div className="space-y-1">
                    <Label htmlFor="pp-topic">主题</Label>
                    <Input
                      id="pp-topic"
                      name="pushplus_topic"
                      autoComplete="off"
                      spellCheck={false}
                      value={pushplus.topic}
                      onChange={(event) =>
                        model.updateSettings((current) => ({
                          ...current,
                          pushplus_topic: event.target.value,
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
                    name="pushplus_channel"
                    autoComplete="off"
                    spellCheck={false}
                    value={pushplus.channel}
                    onChange={(event) =>
                      model.updateSettings((current) => ({
                        ...current,
                        pushplus_channel: event.target.value,
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
                      {model.trackingFolder
                        ? `发送 PushPlus 时，同时写入“${model.trackingFolder.name}”`
                        : '需要先设置追踪文件夹后才能开启'}
                    </p>
                  </div>
                  <Switch
                    id="pp-sync-tracking"
                    name="sync_to_tracking_folder"
                    checked={model.delivery.syncToTrackingFolder}
                    disabled={!model.trackingFolder}
                    onCheckedChange={(checked: boolean) =>
                      model.updateSettings((current) => ({
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
                onClick={() => model.save.mutation.mutate()}
                disabled={model.save.mutation.isPending}
              >
                <Save className="mr-1 h-4 w-4" />
                {model.save.mutation.isPending ? '保存中…' : '保存配置'}
              </Button>
              {model.save.didSave && (
                <span role="status" className="text-sm text-green-600">
                  已保存
                </span>
              )}
              {model.save.mutation.isError && (
                <span role="alert" className="text-sm text-destructive">
                  {model.save.mutation.error instanceof Error
                    ? model.save.mutation.error.message
                    : '保存失败'}
                </span>
              )}
            </div>
          </>
        )}
      </CardContent>
    </Card>
  );
}
