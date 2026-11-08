/**
 * Project-styled Radix progress bar whose fill springs to new values (Animate UI).
 */

import * as React from 'react';
import * as ProgressPrimitive from '@animate-ui/react/primitives/radix-ui/progress';

import { cn } from '@/lib/utils';

type ProgressProps = Omit<React.ComponentProps<typeof ProgressPrimitive.Root>, 'value'> & {
  /** Filled share from 0 to 100; values outside the range are clamped. */
  value: number;
};

/**
 * Render a thin progress track with an animated fill.
 *
 * @param props - Radix progress properties and the filled percentage.
 * @returns Progress bar.
 */
function Progress({ className, value, ...props }: ProgressProps) {
  const percent = Number.isFinite(value) ? Math.min(100, Math.max(0, value)) : 0;
  return (
    <ProgressPrimitive.Root
      data-slot="progress"
      value={percent}
      className={cn('relative block h-1.5 overflow-hidden rounded-full bg-muted', className)}
      {...props}
    >
      <ProgressPrimitive.Indicator
        data-slot="progress-indicator"
        className="block h-full w-full rounded-full bg-primary"
        style={{ transform: `translateX(-${100 - percent}%)` }}
      />
    </ProgressPrimitive.Root>
  );
}

export { Progress };
