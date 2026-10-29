/**
 * Static tracking workflow overview.
 */

import { Bot, FolderHeart, Send, Tags } from 'lucide-react';

import {
  SettingsSection,
  SettingsSectionHeader,
  SettingsSectionTitle,
} from '@/components/settings/settings-section';

const TRACKING_STEPS = [
  {
    icon: FolderHeart,
    title: '选择追踪文件夹',
    detail: '创建或选择一个收藏夹，设为「追踪文件夹」',
  },
  { icon: Tags, title: '描述兴趣', detail: '配置关键词、研究方向' },
  {
    icon: Bot,
    title: '连接 AI',
    detail: '至少一套可用的 OpenAI 兼容 AI 服务；主配置失败后会自动切换到备用配置',
  },
  {
    icon: Send,
    title: '接收推送',
    detail: '推送到追踪文件夹或通过 PushPlus 外部推送，也可以手动触发同步',
  },
] as const;

/**
 * Render the tracking workflow as a four-step flow.
 *
 * @returns Tracking workflow overview.
 */
export function TrackingHelpCard() {
  return (
    <SettingsSection>
      <SettingsSectionHeader className="sr-only">
        <SettingsSectionTitle>文献追踪说明</SettingsSectionTitle>
      </SettingsSectionHeader>
      <ol className="-mx-4 grid grid-cols-2 gap-px border-y bg-border sm:-mx-5 sm:grid-cols-4">
        {TRACKING_STEPS.map((step, index) => {
          const Icon = step.icon;
          return (
            <li key={step.title} className="flex flex-col gap-2 bg-background p-4">
              <div className="flex items-center justify-between">
                <span className="flex size-8 items-center justify-center rounded-lg bg-info text-primary">
                  <Icon className="size-4" aria-hidden="true" />
                </span>
                <span className="text-xs font-bold text-muted-foreground tabular-nums">
                  0{index + 1}
                </span>
              </div>
              <p className="text-sm font-bold">{step.title}</p>
              <p className="text-xs leading-relaxed text-muted-foreground">{step.detail}</p>
            </li>
          );
        })}
      </ol>
      <p className="px-1 text-xs text-muted-foreground">
        系统只会推送 AI 推荐出的文章；未配置偏好或 AI
        不可用时会跳过。在「我的收藏」中查看追踪到的文章。
      </p>
    </SettingsSection>
  );
}
