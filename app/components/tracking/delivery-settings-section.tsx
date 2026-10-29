'use client';

/**
 * Delivery method and PushPlus configuration for tracking notifications.
 */

import { FolderInput, Send, type LucideIcon } from 'lucide-react';

import {
  SettingsGroup,
  SettingsRow,
  SettingsSection,
  SettingsSectionContent,
  SettingsSectionDescription,
  SettingsSectionHeader,
  SettingsSectionTitle,
  StatusPill,
} from '@/components/settings/settings-section';
import type { TrackingPageViewModel } from '@/components/tracking/use-tracking-page';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import {
  COLLAPSE_VARIANTS,
  MOTION_DURATION_SECONDS,
  MotionDiv,
  useMotionTransition,
} from '@/components/ui/motion';
import { Switch } from '@/components/ui/switch';
import { cn } from '@/lib/utils';

type DeliveryOption = {
  description: string;
  icon: LucideIcon;
  label: string;
  value: 'folder' | 'pushplus';
};

const DELIVERY_OPTIONS: readonly DeliveryOption[] = [
  {
    description: '推荐文章直接收藏到追踪文件夹，在「我的收藏」查看',
    icon: FolderInput,
    label: '追踪文件夹推送',
    value: 'folder',
  },
  {
    description: '通过 PushPlus 发送到微信等外部渠道',
    icon: Send,
    label: 'PushPlus 外部推送',
    value: 'pushplus',
  },
];

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
  const isPushplus = model.delivery.method === 'pushplus';
  const panelTransition = useMotionTransition(MOTION_DURATION_SECONDS.base);

  const state = { model, pushplus, isPushplus, panelTransition };
  return (
    <SettingsSection>
      <SettingsSectionHeader>
        <SettingsSectionTitle>推送方式</SettingsSectionTitle>
        <SettingsSectionDescription>
          选择自动推荐的投递位置，并在需要时配置 PushPlus 外部通知。
        </SettingsSectionDescription>
      </SettingsSectionHeader>
      <SettingsSectionContent className="space-y-4">
        <div role="radiogroup" aria-label="推送方式" className="grid gap-3 sm:grid-cols-2">
          {DELIVERY_OPTIONS.map((option) => {
            const Icon = option.icon;
            const isSelected = model.delivery.method === option.value;
            return (
              <button
                key={option.value}
                type="button"
                role="radio"
                aria-checked={isSelected}
                aria-label={option.label}
                className={cn(
                  'motion-control relative flex items-start gap-3 rounded-xl border bg-background p-4 text-left transition-[border-color,box-shadow] outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50',
                  isSelected
                    ? 'border-primary shadow-[0_0_0_1px_var(--primary)]'
                    : 'hover:border-foreground/30',
                )}
                onClick={() =>
                  model.updateSettings((current) => ({
                    ...current,
                    delivery_method: option.value,
                  }))
                }
              >
                <span
                  aria-hidden="true"
                  className={cn(
                    'flex size-10 shrink-0 items-center justify-center rounded-xl',
                    isSelected
                      ? 'bg-primary text-primary-foreground'
                      : 'bg-muted text-muted-foreground',
                  )}
                >
                  <Icon className="size-5" />
                </span>
                <span className="min-w-0 flex-1">
                  <span className="block text-sm leading-6 font-bold">{option.label}</span>
                  <span className="block text-xs leading-relaxed text-muted-foreground">
                    {option.description}
                  </span>
                </span>
                <span
                  aria-hidden="true"
                  className={cn(
                    'mt-0.5 flex size-4 shrink-0 items-center justify-center rounded-full border',
                    isSelected && 'border-[5px] border-primary',
                  )}
                />
              </button>
            );
          })}
        </div>

        <MotionDiv
          data-motion-delivery-panel="pushplus"
          aria-hidden={!isPushplus}
          inert={!isPushplus ? true : undefined}
          initial={false}
          animate={isPushplus ? 'visible' : 'hidden'}
          variants={COLLAPSE_VARIANTS}
          transition={panelTransition}
          className="overflow-hidden"
          style={{ pointerEvents: isPushplus ? 'auto' : 'none' }}
        >
          {renderPushplusSettings(state)}
        </MotionDiv>
      </SettingsSectionContent>
    </SettingsSection>
  );
}

/** Retain PushPlus secret display, current-value toggle and saved-presence hint. */
function renderPushplusToken(state: DeliveryViewState) {
  const { model, pushplus } = state;
  const hasStoredToken = model.storedSettings?.has_pushplus_token;

  return (
    <SettingsRow title="PushPlus 令牌" htmlFor="pp-token" className="sm:items-start">
      <div className="w-full space-y-2 sm:w-72">
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
        <div className="flex items-center justify-between gap-2">
          <StatusPill
            tone={hasStoredToken ? (pushplus.token === null ? 'warning' : 'success') : 'neutral'}
            className="h-auto min-h-6 py-0.5 whitespace-normal"
          >
            {hasStoredToken
              ? pushplus.token === null
                ? '保存后清除当前令牌'
                : '已安全保存；留空不会覆盖'
              : '尚未配置'}
          </StatusPill>
          {hasStoredToken && (
            <Button
              type="button"
              variant="ghost"
              size="sm"
              className="h-7 px-2 text-xs"
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
    </SettingsRow>
  );
}

/** Retain delivery settings inside the permanently mounted collapsed panel. */
function renderPushplusSettings(state: DeliveryViewState) {
  const { model, pushplus } = state;

  return (
    <SettingsGroup>
      {renderPushplusToken(state)}
      <SettingsRow title="模板" htmlFor="pp-template">
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
          className="w-full sm:w-72"
        />
      </SettingsRow>
      <SettingsRow title="主题" htmlFor="pp-topic">
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
          className="w-full sm:w-72"
        />
      </SettingsRow>
      <SettingsRow
        title="渠道"
        htmlFor="pp-channel"
        description="填写 PushPlus 渠道，例如 `wechat`。"
      >
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
          className="w-full sm:w-72"
        />
      </SettingsRow>
      <SettingsRow
        title="同步写入追踪文件夹"
        htmlFor="pp-sync-tracking"
        description={
          model.trackingFolder
            ? `发送 PushPlus 时，同时写入“${model.trackingFolder.name}”`
            : '需要先设置追踪文件夹后才能开启'
        }
      >
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
      </SettingsRow>
    </SettingsGroup>
  );
}

type DeliveryViewState = DeliverySettingsSectionProps & {
  pushplus: DeliverySettingsSectionProps['model']['delivery']['pushplus'];
  isPushplus: boolean;
  panelTransition: ReturnType<typeof useMotionTransition>;
};
