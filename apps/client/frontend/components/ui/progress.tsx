/**
 * Project-styled Radix progress bar whose fill springs to new values (Animate UI).
 */

import * as React from 'react';
import * as ProgressPrimitive from '@animate-ui/react/primitives/radix-ui/progress';
import * as stylex from '@stylexjs/stylex';
import { colors } from '@/lib/theme.stylex';

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
function Progress({
  xstyle,
  value,
  ...props
}: Omit<ProgressProps, 'className'> & { xstyle?: stylex.StyleXStyles }) {
  const percent = Number.isFinite(value) ? Math.min(100, Math.max(0, value)) : 0;
  return (
    <ProgressPrimitive.Root
      data-slot="progress"
      value={percent}
      {...stylex.props(styles.progress, xstyle)}
      {...props}
    >
      <ProgressPrimitive.Indicator
        data-slot="progress-indicator"
        {...stylex.props(styles.progressIndicator, styles.fill(`translateX(-${100 - percent}%)`))}
      />
    </ProgressPrimitive.Root>
  );
}

export { Progress };

const styles = stylex.create({
  fill: (transform: string) => ({
    transform,
  }),
  progress: {
    position: 'relative',
    display: 'block',
    height: '0.375rem',
    overflow: 'hidden',
    borderRadius: 'calc(infinity * 1px)',
    backgroundColor: colors.muted,
  },
  progressIndicator: {
    display: 'block',
    height: '100%',
    width: '100%',
    borderRadius: 'calc(infinity * 1px)',
    backgroundColor: colors.primary,
  },
});
