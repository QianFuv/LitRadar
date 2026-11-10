/**
 * Static tracking workflow overview.
 */

import { Bot, CircleQuestionMark, Folder, Send, Tag } from '@/components/ui/animated-icons';

import {
  SettingsSection,
  SettingsSectionHeader,
  SettingsSectionTitle,
} from '@/components/settings/settings-section';
import * as stylex from '@stylexjs/stylex';
import { colors, lineHeights } from '@/lib/theme.stylex';

const TRACKING_STEPS = [
  {
    icon: Folder,
    title: '选择追踪文件夹',
    detail: '创建或选择一个收藏夹，设为「追踪文件夹」',
  },
  { icon: Tag, title: '描述兴趣', detail: '配置关键词、研究方向' },
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
      <SettingsSectionHeader xstyle={styles.trackingHelpCardSettingsSectionHeader}>
        <SettingsSectionTitle icon={CircleQuestionMark}>文献追踪说明</SettingsSectionTitle>
      </SettingsSectionHeader>
      <ol {...stylex.props(styles.trackingHelpCardOl)}>
        {TRACKING_STEPS.map((step, index) => {
          const Icon = step.icon;
          return (
            <li key={step.title} {...stylex.props(styles.trackingHelpCardLi)}>
              <div {...stylex.props(styles.trackingHelpCardDiv)}>
                <span {...stylex.props(styles.trackingHelpCardSpan)}>
                  <Icon xstyle={styles.trackingHelpCardIcon} aria-hidden="true" />
                </span>
                <span {...stylex.props(styles.trackingHelpCardSpan2)}>0{index + 1}</span>
              </div>
              <p {...stylex.props(styles.trackingHelpCardP)}>{step.title}</p>
              <p {...stylex.props(styles.trackingHelpCardP2)}>{step.detail}</p>
            </li>
          );
        })}
      </ol>
      <p {...stylex.props(styles.trackingHelpCardP3)}>
        系统只会推送 AI 推荐出的文章；未配置偏好或 AI
        不可用时会跳过。在「我的收藏」中查看追踪到的文章。
      </p>
    </SettingsSection>
  );
}

const styles = stylex.create({
  trackingHelpCardSettingsSectionHeader: {
    position: 'absolute',
    width: '1px',
    height: '1px',
    padding: 0,
    margin: '-1px',
    overflow: 'hidden',
    clipPath: 'inset(50%)',
    whiteSpace: 'nowrap',
    borderWidth: 0,
  },
  trackingHelpCardOl: {
    marginInline: {
      default: '-1rem',
      '@media (min-width: 40rem)': '-1.25rem',
    },
    display: 'grid',
    gridTemplateColumns: {
      default: 'repeat(2, minmax(0, 1fr))',
      '@media (min-width: 40rem)': 'repeat(4, minmax(0, 1fr))',
    },
    gap: '1px',
    borderBlockStyle: 'solid',
    borderBlockWidth: '1px',
    backgroundColor: colors.border,
  },
  trackingHelpCardLi: {
    display: 'flex',
    flexDirection: 'column',
    gap: '0.5rem',
    backgroundColor: colors.background,
    padding: '1rem',
  },
  trackingHelpCardDiv: {
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'space-between',
  },
  trackingHelpCardSpan: {
    display: 'flex',
    width: '2rem',
    height: '2rem',
    alignItems: 'center',
    justifyContent: 'center',
    borderRadius: '8px',
    backgroundColor: colors.info,
    color: colors.primary,
  },
  trackingHelpCardIcon: {
    width: '1rem',
    height: '1rem',
  },
  trackingHelpCardSpan2: {
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    fontWeight: 700,
    color: colors.mutedForeground,
    fontVariantNumeric: 'tabular-nums',
  },
  trackingHelpCardP: {
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
    fontWeight: 700,
  },
  trackingHelpCardP2: {
    fontSize: '0.75rem',
    lineHeight: 1.625,
    color: colors.mutedForeground,
  },
  trackingHelpCardP3: {
    paddingInline: '0.25rem',
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    color: colors.mutedForeground,
  },
});
