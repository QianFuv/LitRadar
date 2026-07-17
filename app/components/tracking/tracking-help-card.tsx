/**
 * Static tracking workflow help section.
 */

import {
  SettingsSection,
  SettingsSectionContent,
  SettingsSectionHeader,
  SettingsSectionTitle,
} from '@/components/settings/settings-section';

/**
 * Render the tracking workflow help card.
 *
 * @returns Tracking help card.
 */
export function TrackingHelpCard() {
  return (
    <SettingsSection>
      <SettingsSectionHeader>
        <SettingsSectionTitle>文献追踪说明</SettingsSectionTitle>
      </SettingsSectionHeader>
      <SettingsSectionContent className="space-y-2 text-sm text-muted-foreground">
        <p>1. 创建或选择一个收藏夹，设为「追踪文件夹」</p>
        <p>2. 配置关键词、研究方向和至少一套可用的 OpenAI 兼容 AI 服务</p>
        <p>3. 选择推送方式：推送到追踪文件夹或通过 PushPlus 外部推送</p>
        <p>4. 系统只会推送 AI 推荐出的文章；未配置偏好或 AI 不可用时会跳过</p>
        <p>5. 主配置失败后会自动切换到备用 AI 配置并重试</p>
        <p>6. 也可以手动触发推送同步</p>
        <p>7. 在「我的收藏」中查看追踪到的文章</p>
      </SettingsSectionContent>
    </SettingsSection>
  );
}
