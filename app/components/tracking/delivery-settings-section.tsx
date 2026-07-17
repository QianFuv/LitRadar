'use client';

/**
 * Delivery method and PushPlus configuration for tracking notifications.
 */

import {
  SettingsSection,
  SettingsSectionContent,
  SettingsSectionDescription,
  SettingsSectionHeader,
  SettingsSectionTitle,
} from '@/components/settings/settings-section';
import type { TrackingPageViewModel } from '@/components/tracking/use-tracking-page';
import { Button } from '@/components/ui/button';
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

type DeliverySettingsSectionProps = {
  model: TrackingPageViewModel['recommendation'];
};

/**
 * Render folder/PushPlus delivery selection and external notification controls.
 *
 * @param props - Recommendation model containing delivery settings.
 * @returns Delivery settings section.
 */
export function DeliverySettingsSection({ model }: DeliverySettingsSectionProps) {
  const pushplus = model.delivery.pushplus;

  return (
    <SettingsSection>
      <SettingsSectionHeader>
        <SettingsSectionTitle>通知与推送</SettingsSectionTitle>
        <SettingsSectionDescription>
          选择自动推荐的投递位置，并在需要时配置 PushPlus 外部通知。
        </SettingsSectionDescription>
      </SettingsSectionHeader>
      <SettingsSectionContent className="space-y-5">
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
              <p className="text-xs text-muted-foreground">填写 PushPlus 渠道，例如 `wechat`。</p>
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
      </SettingsSectionContent>
    </SettingsSection>
  );
}
