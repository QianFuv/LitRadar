'use client';

import { useEffect, useMemo, useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { DatabaseZap, Plus, Save, Trash2 } from 'lucide-react';

import {
  adminGetRuntimeSettings,
  adminUpdateRuntimeSettings,
  type RuntimeSettingInfo,
} from '@/lib/api';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { Switch } from '@/components/ui/switch';

type RuntimeSettingsForm = Record<string, string>;

const EMPTY_RUNTIME_SETTINGS: RuntimeSettingInfo[] = [];

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
  return setting.field.endsWith('_pool');
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
            onChange={(event) => updateRow(index, event.target.value)}
            aria-label={`${label} ${index + 1}`}
          />
          <Button
            type="button"
            variant="ghost"
            size="icon-sm"
            className="shrink-0 text-destructive hover:text-destructive"
            aria-label={`删除${label}第 ${index + 1} 行`}
            onClick={() => deleteRow(index)}
          >
            <Trash2 className="h-4 w-4" />
          </Button>
        </div>
      ))}
      <Button type="button" variant="outline" size="sm" onClick={addRow}>
        <Plus className="mr-2 h-4 w-4" />
        添加
      </Button>
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

  const {
    data: settings = EMPTY_RUNTIME_SETTINGS,
    error,
    isLoading,
  } = useQuery({
    queryKey: ['admin-runtime-settings'],
    queryFn: () => adminGetRuntimeSettings(),
  });

  const baseForm = useMemo(() => buildForm(settings), [settings]);
  const form = useMemo(() => {
    return { ...baseForm, ...formOverrides };
  }, [baseForm, formOverrides]);
  const hasFormOverrides = Object.keys(formOverrides).length > 0 || clearedSecrets.size > 0;

  useEffect(() => {
    if (!hasFormOverrides) {
      return;
    }
    const handleBeforeUnload = (event: BeforeUnloadEvent) => {
      event.preventDefault();
      event.returnValue = '';
    };
    window.addEventListener('beforeunload', handleBeforeUnload);
    return () => window.removeEventListener('beforeunload', handleBeforeUnload);
  }, [hasFormOverrides]);

  const saveMutation = useMutation({
    mutationFn: () => {
      const values: Record<string, string | null> = { ...form };
      for (const field of clearedSecrets) {
        values[field] = null;
      }
      return adminUpdateRuntimeSettings({ values });
    },
    onSuccess: (updatedSettings) => {
      setFormOverrides({});
      setClearedSecrets(new Set());
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
    return '';
  }, [error, saveMutation.error]);

  const updateFormValue = (field: string, value: string) => {
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

  const toggleSecretClear = (field: string) => {
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
          <div className="grid gap-4">
            {settings.map((setting) => {
              const value = form[setting.field] ?? '';
              return (
                <div key={setting.field} className="grid gap-2 rounded-md border p-3">
                  <div className="flex flex-wrap items-center justify-between gap-2">
                    <Label htmlFor={`runtime-${setting.field}`}>{setting.label}</Label>
                    <Badge variant="secondary">{getSourceLabel(setting.source)}</Badge>
                  </div>
                  {setting.input_type === 'boolean' ? (
                    <div className="flex items-center justify-between gap-3">
                      <span className="text-sm text-muted-foreground">{setting.description}</span>
                      <Switch
                        id={`runtime-${setting.field}`}
                        name={`runtime_${setting.field}`}
                        checked={value !== 'false'}
                        onCheckedChange={(checked: boolean) =>
                          updateFormValue(setting.field, checked ? 'true' : 'false')
                        }
                      />
                    </div>
                  ) : isPoolSetting(setting) ? (
                    <RuntimePoolEditor
                      field={setting.field}
                      id={`runtime-${setting.field}`}
                      inputType={setting.input_type}
                      label={setting.label}
                      value={value}
                      onChange={(nextValue) => updateFormValue(setting.field, nextValue)}
                    />
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
                  {setting.input_type !== 'boolean' && (
                    <div className="flex flex-wrap items-center justify-between gap-2 text-xs text-muted-foreground">
                      <span>
                        {setting.description}
                        {setting.is_secret && setting.has_value
                          ? clearedSecrets.has(setting.field)
                            ? '（保存后清除）'
                            : '（已安全保存，留空会保留）'
                          : ''}
                      </span>
                      {setting.is_secret && setting.has_value && (
                        <Button
                          type="button"
                          variant="ghost"
                          size="sm"
                          onClick={() => toggleSecretClear(setting.field)}
                        >
                          {clearedSecrets.has(setting.field) ? '保留原密钥' : '清除密钥'}
                        </Button>
                      )}
                    </div>
                  )}
                </div>
              );
            })}
          </div>
        )}
        {mutationError && (
          <p role="alert" className="text-sm text-destructive">
            {mutationError}
          </p>
        )}
        <div className="flex justify-end">
          <Button
            disabled={isLoading || saveMutation.isPending}
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
