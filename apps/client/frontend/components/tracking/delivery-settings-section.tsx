/**
 * Delivery method and PushPlus configuration for tracking notifications.
 */

import { Folder, Send, type AppIcon } from '@/components/ui/animated-icons';

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
import { DISCLOSURE_VARIANTS, MotionDiv } from '@/components/ui/motion';
import { Switch } from '@/components/ui/switch';
import { RadioGroup, RadioGroupIndicator, RadioGroupItem } from '@/components/ui/radio-group';
import * as stylex from '@stylexjs/stylex';
import { colors, lineHeights } from '@/lib/theme.stylex';

type DeliveryOption = {
  description: string;
  icon: AppIcon;
  label: string;
  value: 'folder' | 'pushplus';
};

const DELIVERY_OPTIONS: readonly DeliveryOption[] = [
  {
    description: '推荐文章直接收藏到追踪文件夹，在「我的收藏」查看',
    icon: Folder,
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

  const state = { model, pushplus, isPushplus };
  return (
    <SettingsSection>
      <SettingsSectionHeader>
        <SettingsSectionTitle icon={Send}>推送方式</SettingsSectionTitle>
        <SettingsSectionDescription>
          选择自动推荐的投递位置，并在需要时配置 PushPlus 外部通知。
        </SettingsSectionDescription>
      </SettingsSectionHeader>
      <SettingsSectionContent xstyle={styles.deliverySettingsSectionSettingsSectionContent}>
        <RadioGroup
          aria-label="推送方式"
          xstyle={styles.deliverySettingsSectionRadioGroup}
          value={model.delivery.method}
          onValueChange={(method) =>
            model.updateSettings((current) => ({
              ...current,
              delivery_method: method as typeof current.delivery_method,
            }))
          }
        >
          {DELIVERY_OPTIONS.map((option) => {
            const Icon = option.icon;
            const isSelected = model.delivery.method === option.value;
            return (
              <RadioGroupItem
                key={option.value}
                value={option.value}
                aria-label={option.label}
                {...stylex.props(
                  styles.deliverySettingsSectionRadioGroupItem,
                  isSelected
                    ? styles.deliverySettingsSectionRadioGroupItemSelected
                    : styles.deliverySettingsSectionRadioGroupItemNotSelected,
                )}
              >
                <span
                  aria-hidden="true"
                  {...stylex.props(
                    styles.deliverySettingsSectionSpan,
                    isSelected
                      ? styles.deliverySettingsSectionSpanSelected
                      : styles.deliverySettingsSectionSpanNotSelected,
                  )}
                >
                  <Icon xstyle={styles.deliverySettingsSectionIcon} />
                </span>
                <span {...stylex.props(styles.deliverySettingsSectionSpan2)}>
                  <span {...stylex.props(styles.deliverySettingsSectionSpan3)}>{option.label}</span>
                  <span {...stylex.props(styles.deliverySettingsSectionSpan4)}>
                    {option.description}
                  </span>
                </span>
                <span
                  aria-hidden="true"
                  {...stylex.props(
                    styles.deliverySettingsSectionSpan5,
                    isSelected && styles.deliverySettingsSectionSpanSelected2,
                  )}
                >
                  <RadioGroupIndicator
                    {...stylex.props(styles.deliverySettingsSectionRadioGroupIndicator)}
                  />
                </span>
              </RadioGroupItem>
            );
          })}
        </RadioGroup>

        <MotionDiv
          data-motion-delivery-panel="pushplus"
          aria-hidden={!isPushplus}
          inert={!isPushplus ? true : undefined}
          initial={false}
          animate={isPushplus ? 'visible' : 'hidden'}
          variants={DISCLOSURE_VARIANTS}
          {...stylex.props(styles.deliverySettingsSectionMotionDiv, !isPushplus && styles.inert)}
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
    <SettingsRow
      title="PushPlus 令牌"
      htmlFor="pp-token"
      xstyle={styles.renderPushplusTokenSettingsRow}
    >
      <div {...stylex.props(styles.renderPushplusTokenDiv)}>
        <Input
          xstyle={styles.renderPushplusTokenDivItem}
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
        <div {...stylex.props(styles.renderPushplusTokenDivItem, styles.renderPushplusTokenDiv2)}>
          <StatusPill
            tone={hasStoredToken ? (pushplus.token === null ? 'warning' : 'success') : 'neutral'}
            xstyle={styles.renderPushplusTokenStatusPill}
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
              xstyle={styles.renderPushplusTokenButton}
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
          xstyle={styles.renderPushplusTokenDiv}
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
          xstyle={styles.renderPushplusTokenDiv}
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
          xstyle={styles.renderPushplusTokenDiv}
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
};

const styles = stylex.create({
  inert: {
    pointerEvents: 'none',
  },
  deliverySettingsSectionSettingsSectionContent: {
    display: 'flex',
    flexDirection: 'column',
    rowGap: '1rem',
  },
  deliverySettingsSectionRadioGroup: {
    display: 'grid',
    gap: '0.75rem',
    gridTemplateColumns: {
      default: null,
      '@media (min-width: 40rem)': 'repeat(2, minmax(0, 1fr))',
    },
  },
  deliverySettingsSectionRadioGroupItem: {
    position: 'relative',
    display: 'flex',
    alignItems: 'flex-start',
    gap: '0.75rem',
    borderRadius: '12px',
    borderStyle: 'solid',
    borderWidth: '1px',
    backgroundColor: colors.background,
    padding: '1rem',
    textAlign: 'left',
    transitionProperty: 'border-color, box-shadow',
    transitionTimingFunction: 'cubic-bezier(0.25, 0.1, 0.25, 1)',
    transitionDuration: '200ms',
    outlineStyle: 'none',
    boxShadow: {
      default: null,
      ':focus-visible': `0 0 0 3px color-mix(in oklab, ${colors.ring} 50%, transparent)`,
    },
  },
  deliverySettingsSectionRadioGroupItemSelected: {
    borderColor: colors.primary,
    boxShadow: {
      default: `0 0 0 1px ${colors.primary}`,
      ':focus-visible': `0 0 0 3px color-mix(in oklab, ${colors.ring} 50%, transparent), 0 0 0 1px ${colors.primary}`,
    },
  },
  deliverySettingsSectionRadioGroupItemNotSelected: {
    borderColor: {
      default: null,
      ':hover': {
        default: null,
        '@media (hover: hover)': `color-mix(in oklab, ${colors.foreground} 30%, transparent)`,
      },
    },
  },
  deliverySettingsSectionSpan: {
    display: 'flex',
    width: '2.5rem',
    height: '2.5rem',
    flexShrink: 0,
    alignItems: 'center',
    justifyContent: 'center',
    borderRadius: '12px',
  },
  deliverySettingsSectionSpanSelected: {
    backgroundColor: colors.primary,
    color: colors.primaryForeground,
  },
  deliverySettingsSectionSpanNotSelected: {
    backgroundColor: colors.muted,
    color: colors.mutedForeground,
  },
  deliverySettingsSectionIcon: {
    width: '1.25rem',
    height: '1.25rem',
  },
  deliverySettingsSectionSpan2: {
    minWidth: '0px',
    flex: '1',
  },
  deliverySettingsSectionSpan3: {
    display: 'block',
    fontSize: '0.875rem',
    lineHeight: '1.5rem',
    fontWeight: 700,
  },
  deliverySettingsSectionSpan4: {
    display: 'block',
    fontSize: '0.75rem',
    lineHeight: 1.625,
    color: colors.mutedForeground,
  },
  deliverySettingsSectionSpan5: {
    marginTop: '0.125rem',
    display: 'flex',
    width: '1rem',
    height: '1rem',
    flexShrink: 0,
    alignItems: 'center',
    justifyContent: 'center',
    borderRadius: 'calc(infinity * 1px)',
    borderStyle: 'solid',
    borderWidth: '1px',
  },
  deliverySettingsSectionSpanSelected2: {
    borderColor: colors.primary,
  },
  deliverySettingsSectionRadioGroupIndicator: {
    width: '0.5rem',
    height: '0.5rem',
    borderRadius: 'calc(infinity * 1px)',
    backgroundColor: colors.primary,
  },
  deliverySettingsSectionMotionDiv: {
    overflow: 'hidden',
  },
  renderPushplusTokenSettingsRow: {
    alignItems: {
      default: null,
      '@media (min-width: 40rem)': 'flex-start',
    },
  },
  renderPushplusTokenDivItem: {
    marginBottom: {
      default: null,
      ':not(:last-child)': '0.5rem',
    },
  },
  renderPushplusTokenDiv: {
    width: {
      default: '100%',
      '@media (min-width: 40rem)': '18rem',
    },
  },
  renderPushplusTokenDiv2: {
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'space-between',
    gap: '0.5rem',
  },
  renderPushplusTokenStatusPill: {
    height: 'auto',
    minHeight: '1.5rem',
    paddingBlock: '0.125rem',
    whiteSpace: 'normal',
  },
  renderPushplusTokenButton: {
    height: '1.75rem',
    paddingInline: {
      default: '0.5rem',
      ':has( > svg)': '0.625rem',
    },
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
  },
});
