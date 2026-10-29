/**
 * Grouped-list primitives used inside the settings and administration centers.
 *
 * A section is a captioned stack; each group is a rounded surface whose rows place
 * the explanation on the left and the control on the right.
 */

import type { ComponentProps, ReactNode } from 'react';

import { cn } from '@/lib/utils';

/**
 * Render one captioned settings section.
 *
 * @param props - Native section properties.
 * @returns Settings section container.
 */
export function SettingsSection({ className, ...props }: ComponentProps<'section'>) {
  return (
    <section
      data-slot="settings-section"
      className={cn('space-y-3 [&+&]:mt-10', className)}
      {...props}
    />
  );
}

/**
 * Render the caption block above one settings section.
 *
 * @param props - Native header properties.
 * @returns Settings section caption container.
 */
export function SettingsSectionHeader({ className, ...props }: ComponentProps<'header'>) {
  return (
    <header
      data-slot="settings-section-header"
      className={cn('flex flex-wrap items-end justify-between gap-x-4 gap-y-1', className)}
      {...props}
    />
  );
}

/**
 * Render an accessible caption for one settings section.
 *
 * @param props - Native heading properties.
 * @returns Settings section title.
 */
export function SettingsSectionTitle({ className, ...props }: ComponentProps<'h3'>) {
  return (
    <h3
      data-slot="settings-section-title"
      className={cn('text-sm leading-6 font-bold', className)}
      {...props}
    />
  );
}

/**
 * Render supporting text for one settings section.
 *
 * @param props - Native paragraph properties.
 * @returns Settings section description.
 */
export function SettingsSectionDescription({ className, ...props }: ComponentProps<'p'>) {
  return (
    <p
      data-slot="settings-section-description"
      className={cn('w-full text-xs leading-relaxed text-muted-foreground', className)}
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
export function SettingsSectionContent({ className, ...props }: ComponentProps<'div'>) {
  return <div data-slot="settings-section-content" className={cn(className)} {...props} />;
}

/**
 * Render a rounded surface whose children are separated by hairlines.
 *
 * @param props - Native container properties.
 * @returns Grouped settings surface.
 */
export function SettingsGroup({ className, ...props }: ComponentProps<'div'>) {
  return (
    <div
      data-slot="settings-group"
      className={cn('-mx-4 divide-y border-y sm:-mx-5', className)}
      {...props}
    />
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
  className,
  description,
  htmlFor,
  isStacked = false,
  leading,
  title,
  ...props
}: SettingsRowProps) {
  const LabelElement = htmlFor ? 'label' : 'div';
  return (
    <div
      data-slot="settings-row"
      className={cn(
        'flex flex-col gap-3 px-4 py-3.5 sm:px-5',
        !isStacked && 'sm:flex-row sm:items-center sm:justify-between sm:gap-6',
        className,
      )}
      {...props}
    >
      <div className="flex min-w-0 items-center gap-3">
        {leading}
        <div className="min-w-0">
          <LabelElement
            {...(htmlFor ? { htmlFor } : {})}
            className="block text-sm leading-6 font-bold"
          >
            {title}
          </LabelElement>
          {description && (
            <div className="text-xs leading-relaxed text-muted-foreground">{description}</div>
          )}
        </div>
      </div>
      {children !== undefined && children !== null && (
        <div
          className={cn(
            'flex min-w-0 flex-wrap items-center gap-2',
            isStacked ? 'w-full [&>*]:w-full' : 'sm:shrink-0 sm:justify-end',
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
export function SettingsGroupBody({ className, ...props }: ComponentProps<'div'>) {
  return (
    <div
      data-slot="settings-group-body"
      className={cn('px-4 py-4 sm:px-5', className)}
      {...props}
    />
  );
}

type StatusPillProps = ComponentProps<'span'> & {
  /** Visual tone of the status. */
  tone?: 'neutral' | 'success' | 'warning' | 'danger' | 'info';
};

const STATUS_PILL_TONES: Record<NonNullable<StatusPillProps['tone']>, string> = {
  danger: 'bg-destructive/10 text-destructive [&>i]:bg-destructive',
  info: 'bg-info text-info-foreground [&>i]:bg-primary',
  neutral: 'bg-muted text-muted-foreground [&>i]:bg-muted-foreground/60',
  success: 'bg-success text-success-foreground [&>i]:bg-success-foreground',
  warning: 'bg-warning text-warning-foreground [&>i]:bg-warning-foreground',
};

/**
 * Render a compact status label with a leading dot.
 *
 * @param props - Tone and label content.
 * @returns Status pill.
 */
export function StatusPill({ children, className, tone = 'neutral', ...props }: StatusPillProps) {
  return (
    <span
      data-slot="status-pill"
      className={cn(
        'inline-flex h-6 shrink-0 items-center gap-1.5 rounded-full px-2.5 text-xs font-bold whitespace-nowrap',
        STATUS_PILL_TONES[tone],
        className,
      )}
      {...props}
    >
      <i aria-hidden="true" className="size-1.5 rounded-full" />
      {children}
    </span>
  );
}
