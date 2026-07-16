'use client';

import { useState } from 'react';
import Link from 'next/link';
import { useRouter } from 'next/navigation';
import { ArrowLeft, Download, FolderPlus, Plus, Radar, Save, X } from 'lucide-react';

import { useTrackingPage } from '@/components/tracking/use-tracking-page';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';
import { Checkbox } from '@/components/ui/checkbox';
import { ConfirmDialog } from '@/components/ui/confirm-dialog';
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

/**
 * Render the tracking page from its query and mutation view model.
 *
 * @param props - Authenticated user identifier.
 * @returns Tracking configuration and manual-push UI.
 */
export function TrackingPageContent({ userId }: { userId: number }) {
  const router = useRouter();
  const [isLeaveConfirmOpen, setIsLeaveConfirmOpen] = useState(false);
  const {
    addDirection,
    addKeyword,
    aiApiKey,
    aiBackupApiKey,
    aiBackupBaseUrl,
    aiBackupModel,
    aiBackupSystemPrompt,
    aiBaseUrl,
    aiModel,
    aiRetryAttempts,
    aiSystemPrompt,
    allDatabasesSelected,
    availableDatabases,
    createAndSetMut,
    databasesQuery,
    deliveryMethod,
    directionInput,
    directions,
    draftSettings,
    effectiveSelectedDatabases,
    folders,
    hasUnsavedSettings,
    isPushPolling,
    keywordInput,
    keywords,
    manualPushDescription,
    manualPushLabel,
    newFolderName,
    notificationSettingsQuery,
    notifyEnabled,
    notifySettings,
    pushMut,
    pushResult,
    pushplusChannel,
    pushplusTemplate,
    pushplusToken,
    pushplusTopic,
    requiresTrackingFolder,
    saveSettingsMut,
    selectAllDatabases,
    setDatabaseSelected,
    setDirectionInput,
    setKeywordInput,
    setNewFolderName,
    setTrackMut,
    settingsSaved,
    status,
    syncToTrackingFolder,
    updateDraftSettings,
  } = useTrackingPage(userId);

  return (
    <main id="main-content" className="mx-auto max-w-3xl space-y-4 p-4 sm:space-y-6 sm:p-6">
      <div className="flex items-start gap-2 sm:gap-3">
        <Button variant="ghost" size="icon" aria-label="返回首页" asChild>
          <Link
            href="/"
            onClick={(event) => {
              if (hasUnsavedSettings) {
                event.preventDefault();
                setIsLeaveConfirmOpen(true);
              }
            }}
          >
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
                name="tracking_folder_id"
                value={status?.tracking_folder?.id?.toString() || ''}
                onValueChange={(val: string) => setTrackMut.mutate(Number(val))}
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
              aria-label="新建追踪文件夹名称"
              name="tracking_folder_name"
              autoComplete="off"
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
          <CardDescription>{manualPushDescription}</CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
            <div className="text-sm text-muted-foreground">
              可推送文章: {status?.weekly_articles_available ?? '…'} 篇
            </div>
            <Button
              className="w-full sm:w-auto"
              onClick={() => pushMut.mutate()}
              disabled={
                pushMut.isPending ||
                isPushPolling ||
                (requiresTrackingFolder && !status?.tracking_folder)
              }
            >
              <Download className="h-4 w-4 mr-1" />
              {manualPushLabel}
            </Button>
          </div>
          {pushResult && (
            <div
              role={pushMut.isError ? 'alert' : 'status'}
              className="rounded-md border px-3 py-2 text-sm"
            >
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
            <div
              role="status"
              className="rounded-md border px-3 py-4 text-sm text-muted-foreground"
            >
              正在加载已保存的推荐配置…
            </div>
          ) : notificationSettingsQuery.isError && draftSettings === null ? (
            <div
              role="alert"
              className="rounded-md border border-destructive/50 px-3 py-4 text-sm text-destructive"
            >
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
                  name="notification_enabled"
                  checked={notifyEnabled}
                  onCheckedChange={(checked: boolean) =>
                    updateDraftSettings((current) => ({
                      ...current,
                      enabled: checked,
                    }))
                  }
                />
              </div>

              <div className="space-y-2">
                <Label htmlFor="keyword-input">关键词</Label>
                <div className="flex flex-wrap gap-1.5 min-h-[2rem]">
                  {keywords.map((kw) => (
                    <Badge key={kw} variant="secondary" className="gap-1 pr-1">
                      {kw}
                      <button
                        type="button"
                        aria-label={`移除关键词 ${kw}`}
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
                    id="keyword-input"
                    name="notification_keyword"
                    autoComplete="off"
                    spellCheck={false}
                    value={keywordInput}
                    onChange={(e) => setKeywordInput(e.target.value)}
                    onKeyDown={(e) => {
                      if (e.key === 'Enter') {
                        e.preventDefault();
                        addKeyword();
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
                    onClick={addKeyword}
                    disabled={!keywordInput.trim()}
                  >
                    <Plus className="h-4 w-4" />
                  </Button>
                </div>
              </div>

              <div className="space-y-2">
                <Label htmlFor="direction-input">研究方向</Label>
                <div className="flex flex-wrap gap-1.5 min-h-[2rem]">
                  {directions.map((d) => (
                    <Badge key={d} variant="secondary" className="gap-1 pr-1">
                      {d}
                      <button
                        type="button"
                        aria-label={`移除研究方向 ${d}`}
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
                    id="direction-input"
                    name="notification_direction"
                    autoComplete="off"
                    spellCheck={false}
                    value={directionInput}
                    onChange={(e) => setDirectionInput(e.target.value)}
                    onKeyDown={(e) => {
                      if (e.key === 'Enter') {
                        e.preventDefault();
                        addDirection();
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
                    onClick={addDirection}
                    disabled={!directionInput.trim()}
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
                {databasesQuery.isPending ? (
                  <div
                    role="status"
                    className="rounded-md border border-dashed px-3 py-4 text-sm text-muted-foreground"
                  >
                    正在加载数据库列表…
                  </div>
                ) : databasesQuery.isError ? (
                  <div
                    role="alert"
                    className="rounded-md border border-destructive/50 px-3 py-4 text-sm text-destructive"
                  >
                    {databasesQuery.error instanceof Error
                      ? databasesQuery.error.message
                      : '加载数据库列表失败'}
                  </div>
                ) : availableDatabases.length === 0 ? (
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
                        variant={allDatabasesSelected ? 'default' : 'outline'}
                        size="sm"
                        className="w-full sm:w-auto"
                        onClick={selectAllDatabases}
                      >
                        设为全部数据库
                      </Button>
                    </div>
                    <div className="grid gap-2 sm:grid-cols-2">
                      {availableDatabases.map((dbName) => {
                        const checked =
                          allDatabasesSelected || effectiveSelectedDatabases.includes(dbName);
                        return (
                          <label
                            key={dbName}
                            className="flex items-start gap-3 rounded-md border px-3 py-2 text-sm [content-visibility:auto] [contain-intrinsic-size:0_40px]"
                          >
                            <Checkbox
                              checked={checked}
                              onCheckedChange={(nextChecked: boolean | 'indeterminate') =>
                                setDatabaseSelected(dbName, Boolean(nextChecked))
                              }
                            />
                            <span className="break-all">{dbName}</span>
                          </label>
                        );
                      })}
                    </div>
                    <p className="text-xs text-muted-foreground">
                      当前范围:{' '}
                      {allDatabasesSelected
                        ? `全部数据库（${availableDatabases.length} 个）`
                        : `已选 ${effectiveSelectedDatabases.length} / ${availableDatabases.length} 个数据库`}
                    </p>
                  </div>
                )}
              </div>

              <div className="space-y-2">
                <Label htmlFor="delivery-method">推送方式</Label>
                <Select
                  name="delivery_method"
                  value={deliveryMethod}
                  onValueChange={(v: string) =>
                    updateDraftSettings((current) => ({
                      ...current,
                      delivery_method: v as 'folder' | 'pushplus',
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
                      name="ai_base_url"
                      type="url"
                      autoComplete="off"
                      spellCheck={false}
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
                      name="ai_model"
                      autoComplete="off"
                      spellCheck={false}
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
                    name="ai_api_key"
                    type="password"
                    autoComplete="off"
                    spellCheck={false}
                    value={aiApiKey ?? ''}
                    onChange={(e) =>
                      updateDraftSettings((current) => ({
                        ...current,
                        ai_api_key: e.target.value,
                      }))
                    }
                    placeholder="sk-…"
                  />
                  <div className="flex items-center justify-between gap-2 text-xs text-muted-foreground">
                    <span>
                      {notifySettings?.has_ai_api_key
                        ? aiApiKey === null
                          ? '保存后清除当前密钥'
                          : '已安全保存；留空不会覆盖'
                        : '尚未配置'}
                    </span>
                    {notifySettings?.has_ai_api_key && (
                      <Button
                        type="button"
                        variant="ghost"
                        size="sm"
                        onClick={() =>
                          updateDraftSettings((current) => ({
                            ...current,
                            ai_api_key: current.ai_api_key === null ? undefined : null,
                          }))
                        }
                      >
                        {aiApiKey === null ? '保留原密钥' : '清除密钥'}
                      </Button>
                    )}
                  </div>
                </div>
                <div className="space-y-1">
                  <Label htmlFor="ai-system-prompt">System Prompt</Label>
                  <textarea
                    id="ai-system-prompt"
                    name="ai_system_prompt"
                    autoComplete="off"
                    spellCheck={false}
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
                      name="ai_retry_attempts"
                      type="number"
                      autoComplete="off"
                      inputMode="numeric"
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
                    <div className="text-base font-medium">备用 AI 配置</div>
                    <p className="text-xs text-muted-foreground mt-1">
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
                        name="ai_backup_model"
                        autoComplete="off"
                        spellCheck={false}
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
                      name="ai_backup_api_key"
                      type="password"
                      autoComplete="off"
                      spellCheck={false}
                      value={aiBackupApiKey ?? ''}
                      onChange={(e) =>
                        updateDraftSettings((current) => ({
                          ...current,
                          ai_backup_api_key: e.target.value,
                        }))
                      }
                      placeholder="sk-…"
                    />
                    <div className="flex items-center justify-between gap-2 text-xs text-muted-foreground">
                      <span>
                        {notifySettings?.has_ai_backup_api_key
                          ? aiBackupApiKey === null
                            ? '保存后清除当前密钥'
                            : '已安全保存；留空不会覆盖'
                          : '尚未配置'}
                      </span>
                      {notifySettings?.has_ai_backup_api_key && (
                        <Button
                          type="button"
                          variant="ghost"
                          size="sm"
                          onClick={() =>
                            updateDraftSettings((current) => ({
                              ...current,
                              ai_backup_api_key:
                                current.ai_backup_api_key === null ? undefined : null,
                            }))
                          }
                        >
                          {aiBackupApiKey === null ? '保留原密钥' : '清除密钥'}
                        </Button>
                      )}
                    </div>
                  </div>
                  <div className="space-y-1">
                    <Label htmlFor="ai-backup-system-prompt">Backup System Prompt</Label>
                    <textarea
                      id="ai-backup-system-prompt"
                      name="ai_backup_system_prompt"
                      autoComplete="off"
                      spellCheck={false}
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
                      name="pushplus_token"
                      autoComplete="off"
                      spellCheck={false}
                      type="password"
                      value={pushplusToken ?? ''}
                      onChange={(e) =>
                        updateDraftSettings((current) => ({
                          ...current,
                          pushplus_token: e.target.value,
                        }))
                      }
                      placeholder="输入你的 PushPlus 令牌"
                    />
                    <div className="flex items-center justify-between gap-2 text-xs text-muted-foreground">
                      <span>
                        {notifySettings?.has_pushplus_token
                          ? pushplusToken === null
                            ? '保存后清除当前令牌'
                            : '已安全保存；留空不会覆盖'
                          : '尚未配置'}
                      </span>
                      {notifySettings?.has_pushplus_token && (
                        <Button
                          type="button"
                          variant="ghost"
                          size="sm"
                          onClick={() =>
                            updateDraftSettings((current) => ({
                              ...current,
                              pushplus_token: current.pushplus_token === null ? undefined : null,
                            }))
                          }
                        >
                          {pushplusToken === null ? '保留原令牌' : '清除令牌'}
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
                        name="pushplus_topic"
                        autoComplete="off"
                        spellCheck={false}
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
                      name="pushplus_channel"
                      autoComplete="off"
                      spellCheck={false}
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
                      name="sync_to_tracking_folder"
                      checked={syncToTrackingFolder}
                      disabled={!status?.tracking_folder}
                      onCheckedChange={(checked: boolean) =>
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
                  {saveSettingsMut.isPending ? '保存中…' : '保存配置'}
                </Button>
                {settingsSaved && (
                  <span role="status" className="text-sm text-green-600">
                    已保存
                  </span>
                )}
                {saveSettingsMut.isError && (
                  <span role="alert" className="text-sm text-destructive">
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
      <ConfirmDialog
        open={isLeaveConfirmOpen}
        onOpenChange={setIsLeaveConfirmOpen}
        title="离开未保存的配置？"
        description="当前推荐配置尚未保存，确认离开？未保存的更改将会丢失。"
        actionLabel="确认离开"
        onConfirm={() => {
          setIsLeaveConfirmOpen(false);
          router.push('/');
        }}
      />
    </main>
  );
}
