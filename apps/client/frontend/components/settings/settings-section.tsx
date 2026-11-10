/**
 * Grouped-list primitives used inside the settings and administration centers.
 *
 * A section is a captioned stack; each group is a rounded surface whose rows place
 * the explanation on the left and the control on the right.
 */

import type { ComponentProps, ReactNode } from 'react';

import type { AnimatedIcon } from '@/components/ui/animated-icons';
import * as stylex from '@stylexjs/stylex';
import { colors, lineHeights } from '@/lib/theme.stylex';

/**
 * Render one captioned settings section.
 *
 * @param props - Native section properties.
 * @returns Settings section container.
 */
export function SettingsSection({
  xstyle,
  ...props
}: Omit<ComponentProps<'section'>, 'className'> & { xstyle?: stylex.StyleXStyles }) {
  return <section data-slot="settings-section" {...stylex.props(xstyle)} {...props} />;
}

/**
 * Render the caption block above one settings section.
 *
 * @param props - Native header properties.
 * @returns Settings section caption container.
 */
export function SettingsSectionHeader({
  xstyle,
  ...props
}: Omit<ComponentProps<'header'>, 'className'> & { xstyle?: stylex.StyleXStyles }) {
  return (
    <header
      data-slot="settings-section-header"
      {...stylex.props(styles.settingsSectionHeader, xstyle)}
      {...props}
    />
  );
}

/**
 * Render an accessible caption for one settings section, optionally led by an icon that plays once
 * when the title scrolls into view.
 *
 * @param props - Native heading properties and an optional leading icon.
 * @returns Settings section title.
 */
export function SettingsSectionTitle({
  xstyle,
  icon: Icon,
  children,
  ...props
}: Omit<ComponentProps<'h3'> & { icon?: AnimatedIcon }, 'className'> & {
  xstyle?: stylex.StyleXStyles;
}) {
  return (
    <h3
      data-slot="settings-section-title"
      {...stylex.props(
        styles.settingsSectionTitle,
        Icon && styles.settingsSectionTitleIcon,
        xstyle,
      )}
      {...props}
    >
      {Icon && (
        <Icon
          animateOnView
          {...stylex.props(styles.settingsSectionTitleIcon2)}
          aria-hidden="true"
        />
      )}
      {children}
    </h3>
  );
}

/**
 * Render supporting text for one settings section.
 *
 * @param props - Native paragraph properties.
 * @returns Settings section description.
 */
export function SettingsSectionDescription({
  xstyle,
  ...props
}: Omit<ComponentProps<'p'>, 'className'> & { xstyle?: stylex.StyleXStyles }) {
  return (
    <p
      data-slot="settings-section-description"
      {...stylex.props(styles.settingsSectionDescription, xstyle)}
      {...props}
    />
  );
}

/**
 * Render the controls and status content for one settings section.
 *
 * @param props - Native container properties.
 * @returns Settings section content container.
 */
export function SettingsSectionContent({
  xstyle,
  ...props
}: Omit<ComponentProps<'div'>, 'className'> & { xstyle?: stylex.StyleXStyles }) {
  return <div data-slot="settings-section-content" {...stylex.props(xstyle)} {...props} />;
}

/**
 * Render a rounded surface whose children are separated by hairlines.
 *
 * @param props - Native container properties.
 * @returns Grouped settings surface.
 */
export function SettingsGroup({
  xstyle,
  ...props
}: Omit<ComponentProps<'div'>, 'className'> & { xstyle?: stylex.StyleXStyles }) {
  return (
    <div data-slot="settings-group" {...stylex.props(styles.settingsGroup, xstyle)} {...props} />
  );
}

type SettingsRowProps = Omit<ComponentProps<'div'>, 'title'> & {
  /** Short row label. */
  title: ReactNode;
  /** Optional secondary explanation under the label. */
  description?: ReactNode;
  /** Optional leading icon or avatar. */
  leading?: ReactNode;
  /** Control element id that the label should describe. */
  htmlFor?: string;
  /** Keep the control under the label on every viewport. */
  isStacked?: boolean;
};

/**
 * Render a labelled settings row with its control aligned to the trailing edge.
 *
 * @param props - Label, description, optional leading slot, and the trailing control.
 * @returns Settings row.
 */
export function SettingsRow({
  children,
  xstyle,
  description,
  htmlFor,
  isStacked = false,
  leading,
  title,
  ...props
}: Omit<SettingsRowProps, 'className'> & { xstyle?: stylex.StyleXStyles }) {
  const LabelElement = htmlFor ? 'label' : 'div';
  return (
    <div
      data-slot="settings-row"
      {...stylex.props(styles.settingsRow, !isStacked && styles.settingsRowInline, xstyle)}
      {...props}
    >
      <div {...stylex.props(styles.settingsRowDiv)}>
        {leading}
        <div {...stylex.props(styles.settingsRowDiv2)}>
          <LabelElement
            {...(htmlFor ? { htmlFor } : {})}
            {...stylex.props(styles.settingsRowLabelElement)}
          >
            {title}
          </LabelElement>
          {description && <div {...stylex.props(styles.settingsRowDiv3)}>{description}</div>}
        </div>
      </div>
      {children !== undefined && children !== null && (
        <div
          {...stylex.props(
            styles.settingsRowDiv4,
            isStacked ? styles.settingsRowDivStacked : styles.settingsRowDivNotStacked,
          )}
        >
          {children}
        </div>
      )}
    </div>
  );
}

/**
 * Render free-form content that spans a settings group row.
 *
 * @param props - Native container properties.
 * @returns Padded group body.
 */
export function SettingsGroupBody({
  xstyle,
  ...props
}: Omit<ComponentProps<'div'>, 'className'> & { xstyle?: stylex.StyleXStyles }) {
  return (
    <div
      data-slot="settings-group-body"
      {...stylex.props(styles.settingsGroupBody, xstyle)}
      {...props}
    />
  );
}

type StatusPillProps = Omit<ComponentProps<'span'>, 'className'> & {
  xstyle?: stylex.StyleXStyles;
  /** Visual tone of the status. */
  tone?: 'neutral' | 'success' | 'warning' | 'danger' | 'info';
};

/**
 * Render a compact status label with a leading dot.
 *
 * @param props - Tone and label content.
 * @returns Status pill.
 */
export function StatusPill({ children, xstyle, tone = 'neutral', ...props }: StatusPillProps) {
  return (
    <span
      data-slot="status-pill"
      {...stylex.props(styles.statusPill, STATUS_PILL_TONES[tone], xstyle)}
      {...props}
    >
      <i aria-hidden="true" {...stylex.props(styles.statusPillI, STATUS_PILL_DOT_TONES[tone])} />
      {children}
    </span>
  );
}

const styles = stylex.create({
  statusPillDanger: {
    backgroundColor: `color-mix(in oklab, ${colors.destructive} 10%, transparent)`,
    color: colors.destructive,
  },
  statusPillInfo: {
    backgroundColor: colors.info,
    color: colors.infoForeground,
  },
  statusPillNeutral: {
    backgroundColor: colors.muted,
    color: colors.mutedForeground,
  },
  statusPillSuccess: {
    backgroundColor: colors.success,
    color: colors.successForeground,
  },
  statusPillWarning: {
    backgroundColor: colors.warning,
    color: colors.warningForeground,
  },
  statusPillDotDanger: {
    backgroundColor: colors.destructive,
  },
  statusPillDotInfo: {
    backgroundColor: colors.primary,
  },
  statusPillDotNeutral: {
    backgroundColor: `color-mix(in oklab, ${colors.mutedForeground} 60%, transparent)`,
  },
  statusPillDotSuccess: {
    backgroundColor: colors.successForeground,
  },
  statusPillDotWarning: {
    backgroundColor: colors.warningForeground,
  },
  settingsSectionHeader: {
    display: 'flex',
    flexWrap: 'wrap',
    alignItems: 'flex-end',
    justifyContent: 'space-between',
    columnGap: '1rem',
    rowGap: '0.25rem',
  },
  settingsSectionTitle: {
    fontSize: '0.875rem',
    lineHeight: '1.5rem',
    fontWeight: 700,
  },
  settingsSectionTitleIcon: {
    display: 'flex',
    alignItems: 'center',
    gap: '0.5rem',
  },
  settingsSectionTitleIcon2: {
    width: '0.875rem',
    height: '0.875rem',
    flexShrink: 0,
  },
  settingsSectionDescription: {
    width: '100%',
    fontSize: '0.75rem',
    lineHeight: 1.625,
    color: colors.mutedForeground,
  },
  settingsGroup: {
    marginInline: {
      default: '-1rem',
      '@media (min-width: 40rem)': '-1.25rem',
    },
    borderBlockStyle: 'solid',
    borderBlockWidth: '1px',
  },
  settingsRow: {
    display: 'flex',
    flexDirection: 'column',
    gap: '0.75rem',
    paddingInline: {
      default: '1rem',
      '@media (min-width: 40rem)': '1.25rem',
    },
    paddingBlock: '0.875rem',
  },
  settingsRowInline: {
    flexDirection: {
      default: 'column',
      '@media (min-width: 40rem)': 'row',
    },
    alignItems: {
      default: null,
      '@media (min-width: 40rem)': 'center',
    },
    justifyContent: {
      default: null,
      '@media (min-width: 40rem)': 'space-between',
    },
    gap: {
      default: '0.75rem',
      '@media (min-width: 40rem)': '1.5rem',
    },
  },
  settingsRowDiv: {
    display: 'flex',
    minWidth: '0px',
    alignItems: 'center',
    gap: '0.75rem',
  },
  settingsRowDiv2: {
    minWidth: '0px',
  },
  settingsRowLabelElement: {
    display: 'block',
    fontSize: '0.875rem',
    lineHeight: '1.5rem',
    fontWeight: 700,
  },
  settingsRowDiv3: {
    fontSize: '0.75rem',
    lineHeight: 1.625,
    color: colors.mutedForeground,
  },
  settingsRowDiv4: {
    display: 'flex',
    minWidth: '0px',
    flexWrap: 'wrap',
    alignItems: 'center',
    gap: '0.5rem',
  },
  settingsRowDivStacked: {
    width: '100%',
  },
  settingsRowDivNotStacked: {
    flexShrink: {
      default: null,
      '@media (min-width: 40rem)': 0,
    },
    justifyContent: {
      default: null,
      '@media (min-width: 40rem)': 'flex-end',
    },
  },
  settingsGroupBody: {
    paddingInline: {
      default: '1rem',
      '@media (min-width: 40rem)': '1.25rem',
    },
    paddingBlock: '1rem',
  },
  statusPill: {
    display: 'inline-flex',
    height: '1.5rem',
    flexShrink: 0,
    alignItems: 'center',
    gap: '0.375rem',
    borderRadius: 'calc(infinity * 1px)',
    paddingInline: '0.625rem',
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    fontWeight: 700,
    whiteSpace: 'nowrap',
  },
  statusPillI: {
    width: '0.375rem',
    height: '0.375rem',
    borderRadius: 'calc(infinity * 1px)',
  },
});

const STATUS_PILL_TONES: Record<NonNullable<StatusPillProps['tone']>, stylex.StyleXStyles> = {
  danger: styles.statusPillDanger,
  info: styles.statusPillInfo,
  neutral: styles.statusPillNeutral,
  success: styles.statusPillSuccess,
  warning: styles.statusPillWarning,
};

const STATUS_PILL_DOT_TONES: Record<NonNullable<StatusPillProps['tone']>, stylex.StyleXStyles> = {
  danger: styles.statusPillDotDanger,
  info: styles.statusPillDotInfo,
  neutral: styles.statusPillDotNeutral,
  success: styles.statusPillDotSuccess,
  warning: styles.statusPillDotWarning,
};
