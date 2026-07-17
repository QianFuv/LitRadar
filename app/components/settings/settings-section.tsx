/**
 * Flat section primitives used inside the aggregated settings center.
 */

import type { ComponentProps } from 'react';

import { cn } from '@/lib/utils';

/**
 * Render one settings group separated from adjacent groups by a thin rule.
 *
 * @param props - Native section properties.
 * @returns Flat settings section container.
 */
export function SettingsSection({ className, ...props }: ComponentProps<'section'>) {
  return (
    <section
      data-slot="settings-section"
      className={cn('border-b py-6 first:pt-0 last:border-b-0 last:pb-0', className)}
      {...props}
    />
  );
}

/**
 * Render the heading block for one settings group.
 *
 * @param props - Native header properties.
 * @returns Settings section heading container.
 */
export function SettingsSectionHeader({ className, ...props }: ComponentProps<'header'>) {
  return (
    <header
      data-slot="settings-section-header"
      className={cn('mb-4 flex flex-col gap-1', className)}
      {...props}
    />
  );
}

/**
 * Render an accessible heading for one settings group.
 *
 * @param props - Native heading properties.
 * @returns Settings section title.
 */
export function SettingsSectionTitle({ className, ...props }: ComponentProps<'h3'>) {
  return (
    <h3
      data-slot="settings-section-title"
      className={cn('text-base font-semibold leading-none', className)}
      {...props}
    />
  );
}

/**
 * Render supporting text for one settings group.
 *
 * @param props - Native paragraph properties.
 * @returns Settings section description.
 */
export function SettingsSectionDescription({ className, ...props }: ComponentProps<'p'>) {
  return (
    <p
      data-slot="settings-section-description"
      className={cn('text-sm leading-relaxed text-muted-foreground', className)}
      {...props}
    />
  );
}

/**
 * Render the controls and status content for one settings group.
 *
 * @param props - Native container properties.
 * @returns Settings section content container.
 */
export function SettingsSectionContent({ className, ...props }: ComponentProps<'div'>) {
  return <div data-slot="settings-section-content" className={cn(className)} {...props} />;
}
