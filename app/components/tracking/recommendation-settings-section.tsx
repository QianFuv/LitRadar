'use client';

/**
 * Recommendation preferences, database scope, and AI configuration.
 */

import { Plus, X } from 'lucide-react';

import {
  SettingsSection,
  SettingsSectionContent,
  SettingsSectionDescription,
  SettingsSectionHeader,
  SettingsSectionTitle,
} from '@/components/settings/settings-section';
import type { TrackingPageViewModel } from '@/components/tracking/use-tracking-page';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Checkbox } from '@/components/ui/checkbox';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { Switch } from '@/components/ui/switch';
import { Textarea } from '@/components/ui/textarea';

type RecommendationSettingsSectionProps = {
  model: TrackingPageViewModel['recommendation'];
};

/**
 * Render recommendation preferences, database scope, and primary/backup AI controls.
 *
 * @param props - Recommendation-specific tracking view model.
 * @returns Recommendation settings section.
 */
export function RecommendationSettingsSection({ model }: RecommendationSettingsSectionProps) {
  const { backup, primary, retryAttempts } = model.ai;
  const { directions, keywords } = model.preferences;
  const databaseSelection = model.databaseSelection;

  return (
    <SettingsSection>
      <SettingsSectionHeader>
        <SettingsSectionTitle>AI 推荐配置</SettingsSectionTitle>
        <SettingsSectionDescription>
          只有在启用推荐、填写关键词或研究方向、且至少有一套可用 AI 配置时，系统才会推送文章
        </SettingsSectionDescription>
      </SettingsSectionHeader>
      <SettingsSectionContent className="space-y-5">
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

        <div className="space-y-4 rounded-md border p-3">
          <div>
            <div className="text-base font-medium">主 AI 配置</div>
            <p className="mt-1 text-xs text-muted-foreground">
              优先使用这套配置进行筛选；留空字段会回退到服务端默认值。
            </p>
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
                    ai_retry_attempts: Math.max(1, Math.min(10, Number(event.target.value) || 1)),
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
                        ai_backup_api_key: current.ai_backup_api_key === null ? undefined : null,
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
      </SettingsSectionContent>
    </SettingsSection>
  );
}
