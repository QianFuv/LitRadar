/**
 * Compact semantic state messages for workspace loading outcomes and feedback.
 */

import { AlertCircle, AlertTriangle, CheckCircle2, SearchX, type LucideIcon } from 'lucide-react';
import type { ReactNode } from 'react';

import { cn } from '@/lib/utils';

type StateMessageTone = 'danger' | 'neutral' | 'success' | 'warning';

type StateMessageProps = {
  action?: ReactNode;
  className?: string;
  description?: ReactNode;
  isLive?: boolean;
  role?: 'alert' | 'status';
  title: ReactNode;
  tone?: StateMessageTone;
};

const STATE_MESSAGE_ICONS: Readonly<Record<StateMessageTone, LucideIcon>> = {
  danger: AlertCircle,
  neutral: SearchX,
  success: CheckCircle2,
  warning: AlertTriangle,
};

const STATE_MESSAGE_TONE_CLASSES: Readonly<Record<StateMessageTone, string>> = {
  danger: 'text-destructive',
  neutral: 'text-muted-foreground',
  success: 'text-green-700 dark:text-green-400',
  warning: 'text-amber-700 dark:text-amber-400',
};

/**
 * Render one visually restrained state surface with explicit live-region semantics.
 *
 * @param props - Message content, semantic tone, optional action, and role override.
 * @returns Accessible workspace state message.
 */
export function StateMessage({
  action,
  className,
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
      className={cn(
        'flex min-h-40 flex-col items-center justify-center rounded-lg bg-muted/30 px-5 py-8 text-center shadow-vercel-ring',
        className,
      )}
    >
      <span
        className={cn(
          'mb-3 inline-flex size-9 items-center justify-center rounded-full bg-background shadow-vercel-ring',
          STATE_MESSAGE_TONE_CLASSES[tone],
        )}
      >
        <Icon className="size-4.5" aria-hidden="true" />
      </span>
      <h2 className="text-sm font-semibold text-foreground">{title}</h2>
      {description && (
        <p className="mt-1 max-w-xl text-xs leading-relaxed text-muted-foreground">{description}</p>
      )}
      {action && <div className="mt-4">{action}</div>}
    </section>
  );
}
