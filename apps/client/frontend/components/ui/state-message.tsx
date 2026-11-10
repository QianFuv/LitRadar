/**
 * Compact semantic state messages for workspace loading outcomes and feedback.
 */

import { SearchX } from 'lucide-react';
import {
  AlertCircle,
  AlertTriangle,
  CheckCircle2,
  type AppIcon,
} from '@/components/ui/animated-icons';
import type { ReactNode } from 'react';

import * as stylex from '@stylexjs/stylex';
import { colors } from '@/lib/theme.stylex';

type StateMessageTone = 'danger' | 'neutral' | 'success' | 'warning';

type StateMessageProps = {
  action?: ReactNode;
  xstyle?: stylex.StyleXStyles;
  description?: ReactNode;
  isLive?: boolean;
  role?: 'alert' | 'status';
  title: ReactNode;
  tone?: StateMessageTone;
};

const STATE_MESSAGE_ICONS: Readonly<Record<StateMessageTone, AppIcon>> = {
  danger: AlertCircle,
  neutral: SearchX,
  success: CheckCircle2,
  warning: AlertTriangle,
};

/**
 * Render one visually restrained state surface with explicit live-region semantics.
 *
 * @param props - Message content, semantic tone, optional action, and role override.
 * @returns Accessible workspace state message.
 */
export function StateMessage({
  action,
  xstyle,
  description,
  isLive = true,
  role,
  title,
  tone = 'neutral',
}: StateMessageProps) {
  const Icon = STATE_MESSAGE_ICONS[tone];
  const resolvedRole = role ?? (tone === 'danger' ? 'alert' : 'status');

  return (
    <section
      data-slot="state-message"
      data-tone={tone}
      role={isLive ? resolvedRole : undefined}
      aria-atomic={isLive ? 'true' : undefined}
      aria-live={isLive ? (resolvedRole === 'alert' ? 'assertive' : 'polite') : undefined}
      {...stylex.props(styles.stateMessage, xstyle)}
    >
      <span {...stylex.props(styles.stateMessageBadge, STATE_MESSAGE_TONE_STYLES[tone])}>
        <Icon {...stylex.props(styles.stateMessageIcon)} aria-hidden="true" />
      </span>
      <h2 {...stylex.props(styles.stateMessageH2)}>{title}</h2>
      {description && <p {...stylex.props(styles.stateMessageP)}>{description}</p>}
      {action && <div {...stylex.props(styles.stateMessageDiv)}>{action}</div>}
    </section>
  );
}

const styles = stylex.create({
  stateMessageBadge: {
    marginBottom: '1rem',
    display: 'inline-flex',
    width: '3rem',
    height: '3rem',
    alignItems: 'center',
    justifyContent: 'center',
    borderRadius: '16px',
    backgroundColor: colors.muted,
  },
  toneDanger: {
    color: colors.destructive,
  },
  toneNeutral: {
    color: colors.mutedForeground,
  },
  toneSuccess: {
    color: colors.successForeground,
  },
  toneWarning: {
    color: colors.warningForeground,
  },
  stateMessage: {
    display: 'flex',
    minHeight: '12rem',
    flexDirection: 'column',
    alignItems: 'center',
    justifyContent: 'center',
    paddingInline: '1.5rem',
    paddingBlock: '2.5rem',
    textAlign: 'center',
  },
  stateMessageIcon: {
    width: '1.25rem',
    height: '1.25rem',
  },
  stateMessageH2: {
    fontSize: '15px',
    fontWeight: 700,
    color: colors.foreground,
  },
  stateMessageP: {
    marginTop: '0.375rem',
    maxWidth: '24rem',
    fontSize: '13px',
    lineHeight: 1.625,
    color: colors.mutedForeground,
  },
  stateMessageDiv: {
    marginTop: '1rem',
  },
});

const STATE_MESSAGE_TONE_STYLES: Readonly<Record<StateMessageTone, stylex.StyleXStyles>> = {
  danger: styles.toneDanger,
  neutral: styles.toneNeutral,
  success: styles.toneSuccess,
  warning: styles.toneWarning,
};
