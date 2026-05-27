'use client';

/**
 * Desktop tracking and notification configuration workspace.
 */

import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Download, FolderPlus, Plus, Save, X } from 'lucide-react';
import { useCallback, useEffect, useState } from 'react';
import { ShellConfigurator } from '@/components/desktop/shell';
import {
  Badge,
  Button,
  CheckboxRow,
  Field,
  Notice,
  Panel,
  SelectInput,
  SwitchRow,
  TextArea,
  TextInput,
} from '@/components/desktop/ui';
import {
  createFolder,
  getDatabases,
  getFolders,
  getNotificationSettings,
  getPushWeeklyStatus,
  getTrackingStatus,
  pushWeeklyToTracking,
  setTrackingFolder,
  updateNotificationSettings,
  type ManualPushStatus,
  type NotificationSettings,
  type NotificationSettingsUpdate,
} from '@/lib/client-api';
import { useAuthSession } from '@/lib/auth-session';

/**
 * Convert saved notification settings into an editable update payload.
 *
 * @param settings - Saved settings.
 * @returns Editable payload.
 */
function normalizeSettings(
  settings: NotificationSettings | null | undefined,
): NotificationSettingsUpdate {
  return {
    ai_api_key: settings?.ai_api_key ?? '',
    ai_backup_api_key: settings?.ai_backup_api_key ?? '',
    ai_backup_base_url: settings?.ai_backup_base_url ?? '',
    ai_backup_model: settings?.ai_backup_model ?? '',
    ai_backup_system_prompt: settings?.ai_backup_system_prompt ?? '',
    ai_base_url: settings?.ai_base_url ?? '',
    ai_model: settings?.ai_model ?? '',
    ai_retry_attempts: settings?.ai_retry_attempts ?? 3,
    ai_system_prompt: settings?.ai_system_prompt ?? '',
    delivery_method: settings?.delivery_method ?? 'folder',
    directions: settings?.directions ?? [],
    enabled: settings?.enabled ?? true,
    keywords: settings?.keywords ?? [],
    pushplus_channel: settings?.pushplus_channel ?? 'wechat',
    pushplus_template: settings?.pushplus_template ?? 'markdown',
    pushplus_token: settings?.pushplus_token ?? '',
    pushplus_topic: settings?.pushplus_topic ?? '',
    selected_databases: settings?.selected_databases ?? [],
    sync_to_tracking_folder: settings?.sync_to_tracking_folder ?? false,
  };
}

/**
 * Format a manual push response for display.
 *
 * @param status - Push status.
 * @returns Display text.
 */
function formatPushStatus(status: ManualPushStatus): string {
  if (status.message) {
    return status.pushed > 0 ? `${status.message}（已推送 ${status.pushed} 篇）` : status.message;
  }
  if (status.status === 'failed') {
    return '推送失败';
  }
  return `成功推送 ${status.pushed} 篇文章`;
}

/**
 * Normalize selected databases; empty array means all databases.
 *
 * @param availableDatabases - Available databases.
 * @param selectedDatabases - Draft selected databases.
 * @returns Normalized database selection.
 */
function normalizeDatabaseSelection(
  availableDatabases: string[],
  selectedDatabases: string[],
): string[] {
  const selectedSet = new Set(selectedDatabases);
  const normalized = availableDatabases.filter((dbName) => selectedSet.has(dbName));
  if (normalized.length === 0 || normalized.length === availableDatabases.length) {
    return [];
  }
  return normalized;
}

/**
 * Render editable tag input section.
 *
 * @param props - Tag editor props.
 * @returns Tag editor.
 */
function TagEditor({
  input,
  label,
  onAdd,
  onInput,
  onRemove,
  placeholder,
  values,
}: {
  input: string;
  label: string;
  values: string[];
  placeholder: string;
  onInput: (value: string) => void;
  onAdd: () => void;
  onRemove: (value: string) => void;
}) {
  return (
    <Field label={label}>
      <div className="form-grid">
        <div className="chip-list">
          {values.map((value) => (
            <span key={value} className="chip">
              {value}
              <button aria-label={`删除 ${value}`} type="button" onClick={() => onRemove(value)}>
                <X size={12} />
              </button>
            </span>
          ))}
        </div>
        <div className="toolbar">
          <TextInput
            value={input}
            onChange={(event) => onInput(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === 'Enter') {
                event.preventDefault();
                onAdd();
              }
            }}
            placeholder={placeholder}
          />
          <Button icon={<Plus size={15} />} variant="secondary" onClick={onAdd}>
            添加
          </Button>
        </div>
      </div>
    </Field>
  );
}

/**
 * Render the tracking workspace.
 *
 * @returns Tracking workspace.
 */
export function TrackingWorkspace() {
  const { token, user } = useAuthSession();
  const queryClient = useQueryClient();
  const [newFolderName, setNewFolderName] = useState('');
  const [keywordInput, setKeywordInput] = useState('');
  const [directionInput, setDirectionInput] = useState('');
  const [draftSettings, setDraftSettings] = useState<NotificationSettingsUpdate | null>(null);
  const [pushMessage, setPushMessage] = useState<string | null>(null);
  const [isPollingPush, setIsPollingPush] = useState(false);
  const [savedMessage, setSavedMessage] = useState<string | null>(null);

  const statusQuery = useQuery({
    queryKey: ['tracking-status'],
    queryFn: () => getTrackingStatus(token!),
    enabled: Boolean(token),
  });
  const foldersQuery = useQuery({
    queryKey: ['folders', user?.id],
    queryFn: () => getFolders(token!),
    enabled: Boolean(token),
  });
  const databasesQuery = useQuery({
    queryKey: ['databases'],
    queryFn: () => getDatabases(token!),
    enabled: Boolean(token),
  });
  const settingsQuery = useQuery({
    queryKey: ['notification-settings', user?.id],
    queryFn: () => getNotificationSettings(token!),
    enabled: Boolean(token),
  });

  const availableDatabases = databasesQuery.data ?? [];
  const settings = draftSettings ?? normalizeSettings(settingsQuery.data);
  const effectiveSelectedDatabases = normalizeDatabaseSelection(
    availableDatabases,
    settings.selected_databases,
  );
  const allDatabasesSelected = effectiveSelectedDatabases.length === 0;
  const requiresTrackingFolder =
    settings.delivery_method === 'folder' || settings.sync_to_tracking_folder;

  const updateDraft = useCallback(
    (updater: (current: NotificationSettingsUpdate) => NotificationSettingsUpdate) => {
      setDraftSettings((current) => updater(current ?? normalizeSettings(settingsQuery.data)));
      setSavedMessage(null);
    },
    [settingsQuery.data],
  );

  const setFolderMutation = useMutation({
    mutationFn: (folderId: number) => setTrackingFolder(token!, folderId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['tracking-status'] });
      queryClient.invalidateQueries({ queryKey: ['folders'] });
    },
  });

  const createTrackingFolderMutation = useMutation({
    mutationFn: (name: string) => createFolder(token!, name, true),
    onSuccess: (folder) => {
      setNewFolderName('');
      queryClient.invalidateQueries({ queryKey: ['folders'] });
      queryClient.invalidateQueries({ queryKey: ['tracking-status'] });
      setFolderMutation.mutate(folder.id);
    },
  });

  const pushMutation = useMutation({
    mutationFn: () => pushWeeklyToTracking(token!),
    onSuccess: (status) => {
      setPushMessage(formatPushStatus(status));
      if (status.status === 'running') {
        setIsPollingPush(true);
      }
    },
    onError: (error) => {
      setPushMessage(error instanceof Error ? error.message : '推送失败');
    },
  });

  const saveMutation = useMutation({
    mutationFn: () =>
      updateNotificationSettings(token!, {
        ...settings,
        selected_databases: effectiveSelectedDatabases,
      }),
    onSuccess: (savedSettings) => {
      setDraftSettings(null);
      queryClient.setQueryData(['notification-settings', user?.id], savedSettings);
      queryClient.invalidateQueries({ queryKey: ['notification-settings', user?.id] });
      queryClient.invalidateQueries({ queryKey: ['tracking-status'] });
      setSavedMessage('配置已保存');
    },
  });

  useEffect(() => {
    if (!token || !isPollingPush) {
      return;
    }
    let cancelled = false;
    const poll = async () => {
      try {
        const status = await getPushWeeklyStatus(token);
        if (cancelled) {
          return;
        }
        setPushMessage(formatPushStatus(status));
        if (status.status !== 'running') {
          setIsPollingPush(false);
          queryClient.invalidateQueries({ queryKey: ['tracking-status'] });
          queryClient.invalidateQueries({ queryKey: ['folders'] });
        }
      } catch (error) {
        if (!cancelled) {
          setPushMessage(error instanceof Error ? error.message : '获取推送状态失败');
          setIsPollingPush(false);
        }
      }
    };
    void poll();
    const intervalId = window.setInterval(() => void poll(), 2000);
    return () => {
      cancelled = true;
      window.clearInterval(intervalId);
    };
  }, [isPollingPush, queryClient, token]);

  const addTag = (field: 'keywords' | 'directions', value: string) => {
    const trimmed = value.trim();
    if (!trimmed) {
      return;
    }
    updateDraft((current) => ({
      ...current,
      [field]: current[field].includes(trimmed) ? current[field] : [...current[field], trimmed],
    }));
  };

  const removeTag = (field: 'keywords' | 'directions', value: string) => {
    updateDraft((current) => ({
      ...current,
      [field]: current[field].filter((item) => item !== value),
    }));
  };

  const setDatabaseSelected = (dbName: string, checked: boolean) => {
    updateDraft((current) => {
      const normalizedCurrent = normalizeDatabaseSelection(
        availableDatabases,
        current.selected_databases,
      );
      const base = normalizedCurrent.length === 0 ? availableDatabases : normalizedCurrent;
      const next = checked ? [...base, dbName] : base.filter((item) => item !== dbName);
      return {
        ...current,
        selected_databases: normalizeDatabaseSelection(availableDatabases, next),
      };
    });
  };

  return (
    <>
      <ShellConfigurator
        kicker="Tracking"
        title="文献追踪"
        actions={
          <>
            <Badge tone={settings.enabled ? 'teal' : 'neutral'}>
              {settings.enabled ? '推荐已启用' : '推荐已暂停'}
            </Badge>
            <Badge tone="violet">{statusQuery.data?.weekly_articles_available ?? 0} 篇可推送</Badge>
          </>
        }
      />
      <div className="workspace-grid workspace-grid--two">
        <div className="list-stack">
          <Panel title="追踪文件夹" meta="每周推荐文章会同步到这里">
            <div className="form-grid">
              {statusQuery.data?.tracking_folder ? (
                <Badge tone="teal">当前追踪：{statusQuery.data.tracking_folder.name}</Badge>
              ) : (
                <Notice>尚未设置追踪文件夹。</Notice>
              )}
              <Field label="选择收藏夹">
                <SelectInput
                  value={statusQuery.data?.tracking_folder?.id ?? ''}
                  onChange={(event) => setFolderMutation.mutate(Number(event.target.value))}
                >
                  <option value="">请选择</option>
                  {(foldersQuery.data ?? []).map((folder) => (
                    <option key={folder.id} value={folder.id}>
                      {folder.name} ({folder.article_count})
                    </option>
                  ))}
                </SelectInput>
              </Field>
              <div className="toolbar">
                <TextInput
                  value={newFolderName}
                  onChange={(event) => setNewFolderName(event.target.value)}
                  placeholder="新建追踪文件夹"
                />
                <Button
                  icon={<FolderPlus size={15} />}
                  disabled={!newFolderName.trim()}
                  onClick={() => createTrackingFolderMutation.mutate(newFolderName.trim())}
                >
                  创建并设为追踪
                </Button>
              </div>
            </div>
          </Panel>

          <Panel title="手动推送" meta="按当前规则立即执行">
            <div className="form-grid">
              <Notice>
                {settings.delivery_method === 'pushplus'
                  ? settings.sync_to_tracking_folder
                    ? '推送到 PushPlus 并同步写入追踪文件夹。'
                    : '推送到 PushPlus。'
                  : '推送到追踪文件夹。'}
              </Notice>
              <Button
                icon={<Download size={15} />}
                disabled={
                  pushMutation.isPending ||
                  isPollingPush ||
                  (requiresTrackingFolder && !statusQuery.data?.tracking_folder)
                }
                onClick={() => pushMutation.mutate()}
              >
                {pushMutation.isPending || isPollingPush ? '推送中...' : '启动推送'}
              </Button>
              {pushMessage ? <Notice>{pushMessage}</Notice> : null}
            </div>
          </Panel>

          <Panel title="推送数据库" meta="空选择表示全部数据库">
            <div className="form-grid">
              <Button
                variant={allDatabasesSelected ? 'primary' : 'secondary'}
                onClick={() => updateDraft((current) => ({ ...current, selected_databases: [] }))}
              >
                全部数据库
              </Button>
              <div className="list-stack">
                {availableDatabases.map((dbName) => (
                  <CheckboxRow
                    key={dbName}
                    checked={allDatabasesSelected || effectiveSelectedDatabases.includes(dbName)}
                    label={dbName}
                    onChange={(event) => setDatabaseSelected(dbName, event.currentTarget.checked)}
                  />
                ))}
              </div>
            </div>
          </Panel>
        </div>

        <div className="list-stack">
          <Panel
            title="推荐规则"
            meta="关键词、方向与投递方式"
            actions={
              <Button
                icon={<Save size={15} />}
                disabled={saveMutation.isPending}
                onClick={() => saveMutation.mutate()}
              >
                保存配置
              </Button>
            }
          >
            <div className="form-grid">
              <SwitchRow
                checked={settings.enabled}
                label="启用推荐"
                onChange={(event) =>
                  updateDraft((current) => ({ ...current, enabled: event.currentTarget.checked }))
                }
              />
              <TagEditor
                input={keywordInput}
                label="关键词"
                placeholder="输入关键词后回车"
                values={settings.keywords}
                onInput={setKeywordInput}
                onAdd={() => {
                  addTag('keywords', keywordInput);
                  setKeywordInput('');
                }}
                onRemove={(value) => removeTag('keywords', value)}
              />
              <TagEditor
                input={directionInput}
                label="研究方向"
                placeholder="输入研究方向后回车"
                values={settings.directions}
                onInput={setDirectionInput}
                onAdd={() => {
                  addTag('directions', directionInput);
                  setDirectionInput('');
                }}
                onRemove={(value) => removeTag('directions', value)}
              />
              <Field label="推送方式">
                <SelectInput
                  value={settings.delivery_method}
                  onChange={(event) =>
                    updateDraft((current) => ({
                      ...current,
                      delivery_method: event.target.value as 'folder' | 'pushplus',
                    }))
                  }
                >
                  <option value="folder">追踪文件夹</option>
                  <option value="pushplus">PushPlus</option>
                </SelectInput>
              </Field>
            </div>
          </Panel>

          <Panel title="AI 配置" meta="主配置与备用配置">
            <div className="form-grid">
              <div className="form-grid form-grid--two">
                <Field label="主 Base URL">
                  <TextInput
                    value={settings.ai_base_url}
                    onChange={(event) =>
                      updateDraft((current) => ({ ...current, ai_base_url: event.target.value }))
                    }
                  />
                </Field>
                <Field label="主 Model">
                  <TextInput
                    value={settings.ai_model}
                    onChange={(event) =>
                      updateDraft((current) => ({ ...current, ai_model: event.target.value }))
                    }
                  />
                </Field>
              </div>
              <Field label="主 API Key">
                <TextInput
                  type="password"
                  value={settings.ai_api_key}
                  onChange={(event) =>
                    updateDraft((current) => ({ ...current, ai_api_key: event.target.value }))
                  }
                />
              </Field>
              <Field label="主 System Prompt">
                <TextArea
                  value={settings.ai_system_prompt}
                  onChange={(event) =>
                    updateDraft((current) => ({ ...current, ai_system_prompt: event.target.value }))
                  }
                />
              </Field>
              <div className="form-grid form-grid--two">
                <Field label="备用 Base URL">
                  <TextInput
                    value={settings.ai_backup_base_url}
                    onChange={(event) =>
                      updateDraft((current) => ({
                        ...current,
                        ai_backup_base_url: event.target.value,
                      }))
                    }
                  />
                </Field>
                <Field label="备用 Model">
                  <TextInput
                    value={settings.ai_backup_model}
                    onChange={(event) =>
                      updateDraft((current) => ({
                        ...current,
                        ai_backup_model: event.target.value,
                      }))
                    }
                  />
                </Field>
              </div>
              <Field label="备用 API Key">
                <TextInput
                  type="password"
                  value={settings.ai_backup_api_key}
                  onChange={(event) =>
                    updateDraft((current) => ({
                      ...current,
                      ai_backup_api_key: event.target.value,
                    }))
                  }
                />
              </Field>
              <Field label="备用 System Prompt">
                <TextArea
                  value={settings.ai_backup_system_prompt}
                  onChange={(event) =>
                    updateDraft((current) => ({
                      ...current,
                      ai_backup_system_prompt: event.target.value,
                    }))
                  }
                />
              </Field>
              <Field label="失败重试次数">
                <TextInput
                  max={10}
                  min={1}
                  type="number"
                  value={settings.ai_retry_attempts}
                  onChange={(event) =>
                    updateDraft((current) => ({
                      ...current,
                      ai_retry_attempts: Math.max(1, Math.min(10, Number(event.target.value) || 1)),
                    }))
                  }
                />
              </Field>
            </div>
          </Panel>

          {settings.delivery_method === 'pushplus' ? (
            <Panel title="PushPlus" meta="外部推送配置">
              <div className="form-grid">
                <Field label="PushPlus Token">
                  <TextInput
                    value={settings.pushplus_token}
                    onChange={(event) =>
                      updateDraft((current) => ({ ...current, pushplus_token: event.target.value }))
                    }
                  />
                </Field>
                <div className="form-grid form-grid--two">
                  <Field label="模板">
                    <TextInput
                      value={settings.pushplus_template}
                      onChange={(event) =>
                        updateDraft((current) => ({
                          ...current,
                          pushplus_template: event.target.value,
                        }))
                      }
                    />
                  </Field>
                  <Field label="主题">
                    <TextInput
                      value={settings.pushplus_topic}
                      onChange={(event) =>
                        updateDraft((current) => ({
                          ...current,
                          pushplus_topic: event.target.value,
                        }))
                      }
                    />
                  </Field>
                </div>
                <Field label="渠道">
                  <TextInput
                    value={settings.pushplus_channel}
                    onChange={(event) =>
                      updateDraft((current) => ({
                        ...current,
                        pushplus_channel: event.target.value,
                      }))
                    }
                  />
                </Field>
                <SwitchRow
                  checked={settings.sync_to_tracking_folder}
                  disabled={!statusQuery.data?.tracking_folder}
                  label="同步写入追踪文件夹"
                  onChange={(event) =>
                    updateDraft((current) => ({
                      ...current,
                      sync_to_tracking_folder: event.currentTarget.checked,
                    }))
                  }
                />
              </div>
            </Panel>
          ) : null}

          {savedMessage ? <Notice>{savedMessage}</Notice> : null}
          {saveMutation.isError ? (
            <Notice tone="error">
              {saveMutation.error instanceof Error ? saveMutation.error.message : '保存失败'}
            </Notice>
          ) : null}
          {settingsQuery.isError ? (
            <Notice tone="error">
              {settingsQuery.error instanceof Error ? settingsQuery.error.message : '加载配置失败'}
            </Notice>
          ) : null}
        </div>
      </div>
    </>
  );
}
