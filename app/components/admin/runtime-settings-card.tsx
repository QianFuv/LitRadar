'use client';

import { useEffect, useMemo, useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { DatabaseZap, Plus, Save, Trash2 } from 'lucide-react';

import {
  adminGetProviderCatalog,
  adminGetRuntimeSettings,
  adminUpdateRuntimeSettings,
  type RuntimeSettingApplyMode,
  type RuntimeSettingGroup,
  type RuntimeSettingInfo,
  type RuntimeSettingsUpdate,
} from '@/lib/api';
import { ProviderConfigurationEditor } from '@/components/admin/provider-configuration-editor';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';
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

type RuntimeSettingsForm = Record<string, string>;
type RuntimeSecretPoolRemovals = Record<string, Set<string>>;

const EMPTY_RUNTIME_SETTINGS: RuntimeSettingInfo[] = [];
const EMPTY_SECRET_REFERENCES = new Set<string>();

const RUNTIME_GROUP_LABELS: Record<RuntimeSettingGroup, string> = {
  source_access: '来源访问',
  provider_routing: 'Provider 路由',
  server_security: '服务与安全',
  observability: '可观测性',
};
const PROVIDER_SETTING_FIELDS = new Set([
  'index_provider_routes',
  'article_abstract_provider_orders',
  'article_fulltext_provider_orders',
]);

/**
 * Convert settings into editable form state.
 *
 * @param settings - Runtime settings returned by the API.
 * @returns Runtime setting values keyed by field name.
 */
function buildForm(settings: RuntimeSettingInfo[]): RuntimeSettingsForm {
  return Object.fromEntries(settings.map((item) => [item.field, item.value]));
}

/**
 * Render a short source label for a runtime setting.
 *
 * @param source - Runtime setting source.
 * @returns Source label.
 */
function getSourceLabel(source: RuntimeSettingInfo['source']): string {
  if (source === 'database') {
    return '数据库';
  }
  return '默认值';
}

/**
 * Check whether a setting should use the pool editor.
 *
 * @param setting - Runtime setting metadata.
 * @returns Whether the setting stores a pool value.
 */
function isPoolSetting(setting: RuntimeSettingInfo): boolean {
  return setting.control === 'string_list';
}

/**
 * Check whether a setting is an encrypted value pool.
 *
 * @param setting - Runtime setting metadata.
 * @returns Whether the setting needs the stored-secret pool editor.
 */
function isSecretPoolSetting(setting: RuntimeSettingInfo): boolean {
  return setting.is_secret && setting.control === 'secret_pool';
}

/**
 * Render a short apply-mode label for a runtime setting.
 *
 * @param applyMode - Backend-declared lifecycle point.
 * @returns Administrator-facing apply timing.
 */
function getApplyModeLabel(applyMode: RuntimeSettingApplyMode): string {
  if (applyMode === 'next_request') {
    return '下次请求生效';
  }
  if (applyMode === 'next_command') {
    return '下次命令生效';
  }
  return '重启后生效';
}

/**
 * Check whether a runtime setting is rendered by the grouped Provider editor.
 *
 * @param setting - Runtime setting descriptor.
 * @returns Whether the setting belongs to the specialized Provider controls.
 */
function isProviderSetting(setting: RuntimeSettingInfo): boolean {
  return PROVIDER_SETTING_FIELDS.has(setting.field);
}

/**
 * Check whether a runtime setting stores URL-like text.
 *
 * @param field - Runtime setting field name.
 * @returns Whether the field should use URL input hints.
 */
function isUrlSetting(field: string): boolean {
  return field.toLowerCase().includes('url');
}

/**
 * Check whether a runtime setting should avoid browser spellcheck.
 *
 * @param field - Runtime setting field name.
 * @param inputType - Runtime setting input type.
 * @returns Whether spellcheck should be disabled.
 */
function shouldDisableRuntimeSpellCheck(
  field: string,
  inputType: RuntimeSettingInfo['input_type'],
): boolean {
  if (inputType === 'email' || inputType === 'password') {
    return true;
  }
  const normalizedField = field.toLowerCase();
  return ['api', 'command', 'endpoint', 'key', 'model', 'pool', 'secret', 'token', 'url'].some(
    (marker) => normalizedField.includes(marker),
  );
}

/**
 * Split a stored pool value into editable rows.
 *
 * @param value - Stored pool value.
 * @returns Editable pool rows.
 */
function splitPoolValue(value: string): string[] {
  if (!value) {
    return [''];
  }
  const parts = value.includes('\n') ? value.split('\n') : value.split(/[,;]+/);
  return parts.map((part) => part.trim());
}

/**
 * Normalize newly entered pool values for an incremental update.
 *
 * @param value - Editable pool text.
 * @returns Unique non-empty values in first-seen order.
 */
function normalizePoolValues(value: string): string[] {
  const normalized: string[] = [];
  for (const part of value.split(/[,;\n]+/)) {
    const item = part.trim();
    if (item && !normalized.includes(item)) {
      normalized.push(item);
    }
  }
  return normalized;
}

/**
 * Render the input type used for one pool row.
 *
 * @param inputType - Runtime setting input type.
 * @returns Input type for an editable pool row.
 */
function getPoolInputType(
  inputType: RuntimeSettingInfo['input_type'],
): 'email' | 'password' | 'text' {
  if (inputType === 'email' || inputType === 'password') {
    return inputType;
  }
  return 'text';
}

type RuntimePoolEditorProps = {
  field: string;
  id: string;
  inputType: RuntimeSettingInfo['input_type'];
  label: string;
  value: string;
  disabled?: boolean;
  onChange: (value: string) => void;
};

/**
 * Render a line-based editor for runtime pool values.
 *
 * @param props - Component props.
 * @returns Runtime pool editor.
 */
function RuntimePoolEditor({
  field,
  id,
  inputType,
  label,
  value,
  disabled = false,
  onChange,
}: RuntimePoolEditorProps) {
  const rows = splitPoolValue(value);
  const poolInputType = getPoolInputType(inputType);
  const shouldDisableSpellCheck = shouldDisableRuntimeSpellCheck(field, inputType);

  const updateRow = (index: number, nextValue: string) => {
    const nextRows = [...rows];
    nextRows[index] = nextValue;
    onChange(nextRows.join('\n'));
  };

  const addRow = () => {
    onChange([...rows, ''].join('\n'));
  };

  const deleteRow = (index: number) => {
    const nextRows = rows.filter((_, rowIndex) => rowIndex !== index);
    onChange(nextRows.length > 0 ? nextRows.join('\n') : '');
  };

  return (
    <div className="space-y-2">
      {rows.map((row, index) => (
        <div key={`${index}-${rows.length}`} className="flex items-center gap-2">
          <Input
            id={index === 0 ? id : undefined}
            name={`runtime_${field}_${index + 1}`}
            type={poolInputType}
            autoComplete="off"
            inputMode={isUrlSetting(field) ? 'url' : undefined}
            spellCheck={shouldDisableSpellCheck ? false : undefined}
            value={row}
            disabled={disabled}
            onChange={(event) => updateRow(index, event.target.value)}
            aria-label={`${label} ${index + 1}`}
          />
          <Button
            type="button"
            variant="ghost"
            size="icon-sm"
            className="shrink-0 text-destructive hover:text-destructive"
            disabled={disabled}
            aria-label={`删除${label}第 ${index + 1} 行`}
            onClick={() => deleteRow(index)}
          >
            <Trash2 className="h-4 w-4" />
          </Button>
        </div>
      ))}
      <Button type="button" variant="outline" size="sm" disabled={disabled} onClick={addRow}>
        <Plus className="mr-2 h-4 w-4" />
        添加
      </Button>
    </div>
  );
}

type RuntimeSecretPoolEditorProps = {
  setting: RuntimeSettingInfo;
  value: string;
  removedReferences: Set<string>;
  isCleared: boolean;
  onChange: (value: string) => void;
  onToggleRemoval: (reference: string) => void;
};

/**
 * Render stored masked secret rows separately from new plaintext inputs.
 *
 * @param props - Component props.
 * @returns Secret-pool editor.
 */
function RuntimeSecretPoolEditor({
  setting,
  value,
  removedReferences,
  isCleared,
  onChange,
  onToggleRemoval,
}: RuntimeSecretPoolEditorProps) {
  return (
    <div className="space-y-3">
      <div className="space-y-2">
        {setting.secret_items.length === 0 ? (
          <p className="text-sm text-muted-foreground">尚未保存密钥</p>
        ) : (
          setting.secret_items.map((item, index) => {
            const isPendingRemoval = isCleared || removedReferences.has(item.reference);
            return (
              <div
                key={item.reference}
                className={`flex items-center justify-between gap-3 rounded-md border px-3 py-2 ${
                  isPendingRemoval ? 'bg-muted/50 text-muted-foreground' : ''
                }`}
              >
                <span
                  className={
                    isPendingRemoval ? 'font-mono text-sm line-through' : 'font-mono text-sm'
                  }
                >
                  {item.masked_value}
                </span>
                <div className="flex items-center gap-2">
                  {isPendingRemoval && <Badge variant="outline">保存后删除</Badge>}
                  <Button
                    type="button"
                    variant="ghost"
                    size="sm"
                    disabled={isCleared}
                    className="text-destructive hover:text-destructive"
                    aria-label={
                      removedReferences.has(item.reference)
                        ? `撤销删除${setting.label}第 ${index + 1} 个密钥`
                        : `删除${setting.label}第 ${index + 1} 个密钥`
                    }
                    onClick={() => onToggleRemoval(item.reference)}
                  >
                    {removedReferences.has(item.reference) ? (
                      '撤销删除'
                    ) : (
                      <Trash2 className="h-4 w-4" />
                    )}
                  </Button>
                </div>
              </div>
            );
          })
        )}
      </div>
      <div className="space-y-2">
        <span className="text-xs text-muted-foreground">添加新密钥</span>
        <RuntimePoolEditor
          field={setting.field}
          id={`runtime-${setting.field}`}
          inputType={setting.input_type}
          label={`${setting.label} 新密钥`}
          value={value}
          disabled={isCleared}
          onChange={onChange}
        />
      </div>
    </div>
  );
}

/**
 * Render the admin runtime settings editor.
 *
 * @param props - Component props.
 * @returns Runtime settings card.
 */
export function RuntimeSettingsCard() {
  const queryClient = useQueryClient();
  const [formOverrides, setFormOverrides] = useState<RuntimeSettingsForm>({});
  const [clearedSecrets, setClearedSecrets] = useState<Set<string>>(new Set());
  const [secretPoolAdditions, setSecretPoolAdditions] = useState<RuntimeSettingsForm>({});
  const [secretPoolRemovals, setSecretPoolRemovals] = useState<RuntimeSecretPoolRemovals>({});
  const [saveFeedback, setSaveFeedback] = useState('');

  const {
    data: settings = EMPTY_RUNTIME_SETTINGS,
    error,
    isLoading,
  } = useQuery({
    queryKey: ['admin-runtime-settings'],
    queryFn: () => adminGetRuntimeSettings(),
  });

  const providerSettings = useMemo(
    () => settings.filter((setting) => isProviderSetting(setting)),
    [settings],
  );
  const genericSettingGroups = useMemo(() => {
    const groups = new Map<RuntimeSettingGroup, RuntimeSettingInfo[]>();
    for (const setting of settings) {
      if (isProviderSetting(setting)) {
        continue;
      }
      const group = groups.get(setting.group) ?? [];
      group.push(setting);
      groups.set(setting.group, group);
    }
    return [...groups.entries()];
  }, [settings]);
  const {
    data: providerCatalog,
    error: providerCatalogError,
    isLoading: isProviderCatalogLoading,
  } = useQuery({
    queryKey: ['admin-provider-catalog'],
    queryFn: () => adminGetProviderCatalog(),
    enabled: providerSettings.length > 0,
  });

  const baseForm = useMemo(() => buildForm(settings), [settings]);
  const form = useMemo(() => {
    return { ...baseForm, ...formOverrides };
  }, [baseForm, formOverrides]);
  const hasPendingChanges =
    Object.keys(formOverrides).length > 0 ||
    clearedSecrets.size > 0 ||
    Object.keys(secretPoolAdditions).length > 0 ||
    Object.keys(secretPoolRemovals).length > 0;

  useEffect(() => {
    if (!hasPendingChanges) {
      return;
    }
    const handleBeforeUnload = (event: BeforeUnloadEvent) => {
      event.preventDefault();
      event.returnValue = '';
    };
    window.addEventListener('beforeunload', handleBeforeUnload);
    return () => window.removeEventListener('beforeunload', handleBeforeUnload);
  }, [hasPendingChanges]);

  const saveMutation = useMutation({
    onMutate: () => {
      setSaveFeedback('');
    },
    mutationFn: () => {
      const values: Record<string, string | null> = { ...formOverrides };
      for (const field of clearedSecrets) {
        values[field] = null;
      }
      const secretPoolUpdates: RuntimeSettingsUpdate['secret_pool_updates'] = {};
      for (const setting of settings) {
        if (!isSecretPoolSetting(setting) || clearedSecrets.has(setting.field)) {
          continue;
        }
        const add = normalizePoolValues(secretPoolAdditions[setting.field] ?? '');
        const remove = [...(secretPoolRemovals[setting.field] ?? EMPTY_SECRET_REFERENCES)];
        if (add.length > 0 || remove.length > 0) {
          secretPoolUpdates[setting.field] = { add, remove };
        }
      }
      return adminUpdateRuntimeSettings({ values, secret_pool_updates: secretPoolUpdates });
    },
    onSuccess: (updatedSettings) => {
      setFormOverrides({});
      setClearedSecrets(new Set());
      setSecretPoolAdditions({});
      setSecretPoolRemovals({});
      setSaveFeedback('运行配置已保存。');
      queryClient.setQueryData(['admin-runtime-settings'], updatedSettings);
      queryClient.invalidateQueries({ queryKey: ['admin-runtime-settings'] });
    },
  });

  const mutationError = useMemo(() => {
    if (saveMutation.error instanceof Error) {
      return saveMutation.error.message;
    }
    if (error instanceof Error) {
      return error.message;
    }
    if (providerCatalogError instanceof Error) {
      return providerCatalogError.message;
    }
    return '';
  }, [error, providerCatalogError, saveMutation.error]);

  const updateFormValue = (field: string, value: string) => {
    setSaveFeedback('');
    setFormOverrides((current) => ({ ...current, [field]: value }));
    setClearedSecrets((current) => {
      if (!current.has(field)) {
        return current;
      }
      const next = new Set(current);
      next.delete(field);
      return next;
    });
  };

  const updateSecretPoolAddition = (field: string, value: string) => {
    setSaveFeedback('');
    setSecretPoolAdditions((current) => ({ ...current, [field]: value }));
  };

  const toggleSecretItemRemoval = (field: string, reference: string) => {
    setSaveFeedback('');
    setSecretPoolRemovals((current) => {
      const references = new Set(current[field] ?? EMPTY_SECRET_REFERENCES);
      if (references.has(reference)) {
        references.delete(reference);
      } else {
        references.add(reference);
      }
      const next = { ...current };
      if (references.size === 0) {
        delete next[field];
      } else {
        next[field] = references;
      }
      return next;
    });
  };

  const toggleSecretClear = (field: string) => {
    setSaveFeedback('');
    setClearedSecrets((current) => {
      const next = new Set(current);
      if (next.has(field)) {
        next.delete(field);
      } else {
        next.add(field);
      }
      return next;
    });
  };

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <DatabaseZap className="h-5 w-5" />
          运行配置
        </CardTitle>
        <CardDescription>管理后端共享运行配置</CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        {isLoading ? (
          <div role="status" className="text-sm text-muted-foreground">
            加载中…
          </div>
        ) : (
          <div className="space-y-6">
            {genericSettingGroups.map(([group, groupSettings]) => (
              <section key={group} className="space-y-3" aria-labelledby={`runtime-group-${group}`}>
                <h3 id={`runtime-group-${group}`} className="text-base font-semibold">
                  {RUNTIME_GROUP_LABELS[group]}
                </h3>
                <div className="grid gap-3 lg:grid-cols-2">
                  {groupSettings.map((setting) => {
                    const value = form[setting.field] ?? '';
                    return (
                      <div
                        key={setting.field}
                        data-runtime-setting-field={setting.field}
                        className="grid gap-2 rounded-md border p-3"
                      >
                        <div className="flex flex-wrap items-center justify-between gap-2">
                          <Label htmlFor={`runtime-${setting.field}`}>{setting.label}</Label>
                          <div className="flex flex-wrap items-center gap-2">
                            {isSecretPoolSetting(setting) && (
                              <Badge variant="outline">{setting.secret_items.length} 个密钥</Badge>
                            )}
                            <Badge variant="outline">{getApplyModeLabel(setting.apply_mode)}</Badge>
                            <Badge variant="secondary">{getSourceLabel(setting.source)}</Badge>
                          </div>
                        </div>
                        {setting.control === 'boolean' ? (
                          <div className="flex items-center justify-between gap-3">
                            <span className="text-sm text-muted-foreground">
                              {setting.description}
                            </span>
                            <Switch
                              id={`runtime-${setting.field}`}
                              name={`runtime_${setting.field}`}
                              checked={value !== 'false'}
                              onCheckedChange={(checked: boolean) =>
                                updateFormValue(setting.field, checked ? 'true' : 'false')
                              }
                            />
                          </div>
                        ) : isSecretPoolSetting(setting) ? (
                          <RuntimeSecretPoolEditor
                            setting={setting}
                            value={secretPoolAdditions[setting.field] ?? ''}
                            removedReferences={
                              secretPoolRemovals[setting.field] ?? EMPTY_SECRET_REFERENCES
                            }
                            isCleared={clearedSecrets.has(setting.field)}
                            onChange={(nextValue) =>
                              updateSecretPoolAddition(setting.field, nextValue)
                            }
                            onToggleRemoval={(reference) =>
                              toggleSecretItemRemoval(setting.field, reference)
                            }
                          />
                        ) : isPoolSetting(setting) ? (
                          <RuntimePoolEditor
                            field={setting.field}
                            id={`runtime-${setting.field}`}
                            inputType={setting.input_type}
                            label={setting.label}
                            value={value}
                            onChange={(nextValue) => updateFormValue(setting.field, nextValue)}
                          />
                        ) : setting.control === 'select' ? (
                          <Select
                            value={value}
                            onValueChange={(nextValue) => updateFormValue(setting.field, nextValue)}
                          >
                            <SelectTrigger id={`runtime-${setting.field}`} className="w-full">
                              <SelectValue />
                            </SelectTrigger>
                            <SelectContent>
                              {setting.allowed_values.map((allowedValue) => (
                                <SelectItem key={allowedValue} value={allowedValue}>
                                  {allowedValue}
                                </SelectItem>
                              ))}
                            </SelectContent>
                          </Select>
                        ) : (
                          <Input
                            id={`runtime-${setting.field}`}
                            name={`runtime_${setting.field}`}
                            type={setting.input_type}
                            autoComplete="off"
                            inputMode={isUrlSetting(setting.field) ? 'url' : undefined}
                            spellCheck={
                              shouldDisableRuntimeSpellCheck(setting.field, setting.input_type)
                                ? false
                                : undefined
                            }
                            value={value}
                            onChange={(event) => updateFormValue(setting.field, event.target.value)}
                            placeholder={setting.description}
                          />
                        )}
                        {setting.control !== 'boolean' && (
                          <div className="flex flex-wrap items-center justify-between gap-2 text-xs text-muted-foreground">
                            <span>
                              {setting.description}
                              {setting.is_secret && setting.has_value
                                ? clearedSecrets.has(setting.field)
                                  ? '（保存后清除全部）'
                                  : (secretPoolRemovals[setting.field]?.size ?? 0) > 0
                                    ? `（${secretPoolRemovals[setting.field]?.size} 个保存后删除）`
                                    : '（已安全保存）'
                                : ''}
                            </span>
                            {setting.is_secret && setting.has_value && (
                              <Button
                                type="button"
                                variant="ghost"
                                size="sm"
                                onClick={() => toggleSecretClear(setting.field)}
                              >
                                {clearedSecrets.has(setting.field)
                                  ? '保留全部密钥'
                                  : '清除全部密钥'}
                              </Button>
                            )}
                          </div>
                        )}
                      </div>
                    );
                  })}
                </div>
              </section>
            ))}

            {providerSettings.length > 0 &&
              (isProviderCatalogLoading ? (
                <div role="status" className="text-sm text-muted-foreground">
                  正在加载 Provider 能力目录…
                </div>
              ) : providerCatalog ? (
                <ProviderConfigurationEditor
                  settings={providerSettings}
                  values={form}
                  catalog={providerCatalog}
                  onChange={updateFormValue}
                />
              ) : null)}
          </div>
        )}
        {mutationError && (
          <p role="alert" className="text-sm text-destructive">
            {mutationError}
          </p>
        )}
        {saveFeedback && (
          <p role="status" className="text-sm text-foreground">
            {saveFeedback}
          </p>
        )}
        <div className="flex justify-end">
          <Button
            disabled={
              isLoading ||
              saveMutation.isPending ||
              (providerSettings.length > 0 &&
                (isProviderCatalogLoading || !providerCatalog || Boolean(providerCatalogError)))
            }
            onClick={() => saveMutation.mutate()}
          >
            <Save className="mr-2 h-4 w-4" />
            保存配置
          </Button>
        </div>
      </CardContent>
    </Card>
  );
}
