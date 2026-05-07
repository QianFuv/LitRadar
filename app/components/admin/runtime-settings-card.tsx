'use client';

import { useMemo, useState } from 'react';
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

type RuntimeSettingsCardProps = {
  token: string;
};

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
  if (source === 'environment') {
    return '环境变量';
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
function RuntimePoolEditor({ id, inputType, label, value, onChange }: RuntimePoolEditorProps) {
  const rows = splitPoolValue(value);
  const poolInputType = getPoolInputType(inputType);

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
            type={poolInputType}
            value={row}
            onChange={(event) => updateRow(index, event.target.value)}
            aria-label={`${label} ${index + 1}`}
          />
          <Button
            type="button"
            variant="ghost"
            size="icon-sm"
            className="shrink-0 text-destructive hover:text-destructive"
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
export function RuntimeSettingsCard({ token }: RuntimeSettingsCardProps) {
  const queryClient = useQueryClient();
  const [formOverrides, setFormOverrides] = useState<RuntimeSettingsForm>({});

  const {
    data: settings = EMPTY_RUNTIME_SETTINGS,
    error,
    isLoading,
  } = useQuery({
    queryKey: ['admin-runtime-settings'],
    queryFn: () => adminGetRuntimeSettings(token),
  });

  const baseForm = useMemo(() => buildForm(settings), [settings]);
  const form = useMemo(() => {
    return { ...baseForm, ...formOverrides };
  }, [baseForm, formOverrides]);

  const saveMutation = useMutation({
    mutationFn: () => adminUpdateRuntimeSettings(token, { values: form }),
    onSuccess: (updatedSettings) => {
      setFormOverrides({});
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

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <DatabaseZap className="h-5 w-5" />
          运行配置
        </CardTitle>
        <CardDescription>管理索引服务使用的外部 API 配置</CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        {isLoading ? (
          <div className="text-sm text-muted-foreground">加载中...</div>
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
                        checked={value !== 'false'}
                        onCheckedChange={(checked) =>
                          setFormOverrides((current) => ({
                            ...current,
                            [setting.field]: checked ? 'true' : 'false',
                          }))
                        }
                      />
                    </div>
                  ) : isPoolSetting(setting) ? (
                    <RuntimePoolEditor
                      id={`runtime-${setting.field}`}
                      inputType={setting.input_type}
                      label={setting.label}
                      value={value}
                      onChange={(nextValue) =>
                        setFormOverrides((current) => ({
                          ...current,
                          [setting.field]: nextValue,
                        }))
                      }
                    />
                  ) : (
                    <Input
                      id={`runtime-${setting.field}`}
                      type={setting.input_type}
                      value={value}
                      onChange={(event) =>
                        setFormOverrides((current) => ({
                          ...current,
                          [setting.field]: event.target.value,
                        }))
                      }
                      placeholder={setting.description}
                    />
                  )}
                  {setting.input_type !== 'boolean' && (
                    <div className="text-xs text-muted-foreground">{setting.description}</div>
                  )}
                </div>
              );
            })}
          </div>
        )}
        {mutationError && <p className="text-sm text-destructive">{mutationError}</p>}
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
