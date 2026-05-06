'use client';

import { useEffect, useMemo, useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { DatabaseZap, Save } from 'lucide-react';

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
 * Render the admin runtime settings editor.
 *
 * @param props - Component props.
 * @returns Runtime settings card.
 */
export function RuntimeSettingsCard({ token }: RuntimeSettingsCardProps) {
  const queryClient = useQueryClient();
  const [form, setForm] = useState<RuntimeSettingsForm>({});

  const { data: settings = [], error, isLoading } = useQuery({
    queryKey: ['admin-runtime-settings'],
    queryFn: () => adminGetRuntimeSettings(token),
  });

  useEffect(() => {
    setForm(buildForm(settings));
  }, [settings]);

  const saveMutation = useMutation({
    mutationFn: () => adminUpdateRuntimeSettings(token, { values: form }),
    onSuccess: (updatedSettings) => {
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
                          setForm((current) => ({
                            ...current,
                            [setting.field]: checked ? 'true' : 'false',
                          }))
                        }
                      />
                    </div>
                  ) : (
                    <Input
                      id={`runtime-${setting.field}`}
                      type={setting.input_type}
                      value={value}
                      onChange={(event) =>
                        setForm((current) => ({
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
        {mutationError && (
          <p className="text-sm text-destructive">{mutationError}</p>
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
