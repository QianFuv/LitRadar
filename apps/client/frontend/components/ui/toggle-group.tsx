/**
 * Project-styled Base UI toggle group whose selected surface slides between items (Animate UI).
 */

import * as React from 'react';
import { ToggleGroup as ToggleGroupPrimitive } from '@animate-ui/react/primitives/base-ui/toggle-group';
import { Toggle } from '@animate-ui/react/primitives/base-ui/toggle';

import { PresenceBoundary } from '@/components/ui/motion';
import * as stylex from '@stylexjs/stylex';
import { colors, lineHeights } from '@/lib/theme.stylex';

type ToggleGroupProps = Omit<
  React.ComponentProps<typeof ToggleGroupPrimitive>,
  'className' | 'value' | 'defaultValue' | 'onValueChange' | 'multiple'
> & {
  /** Value of the pressed segment. */
  value: string;
  /** Called with the newly pressed segment, or an empty string when the pressed one is released. */
  onValueChange: (value: string) => void;
  xstyle?: stylex.StyleXStyles;
};

/**
 * Render a segmented control where exactly one segment is pressed at a time.
 *
 * @param props - Pressed value, change handler, Base UI toggle group properties, and style
 * overrides.
 * @returns Segmented control root.
 */
function ToggleGroup({ xstyle, value, onValueChange, ...props }: ToggleGroupProps) {
  return (
    <ToggleGroupPrimitive
      data-slot="toggle-group"
      value={[value]}
      onValueChange={(values) => onValueChange(values[0] ?? '')}
      {...stylex.props(styles.toggleGroup, xstyle)}
      {...props}
    />
  );
}

/**
 * Render one segment; the selected segment's background becomes the sliding surface.
 *
 * @param props - Base UI toggle properties and style overrides.
 * @returns Segment button.
 */
function ToggleGroupItem({
  xstyle,
  ...props
}: Omit<React.ComponentProps<typeof Toggle>, 'className'> & {
  xstyle?: stylex.StyleXStyles;
}) {
  return (
    <PresenceBoundary>
      <Toggle
        data-slot="toggle-group-item"
        {...stylex.props(styles.toggleGroupItem, xstyle)}
        {...props}
      />
    </PresenceBoundary>
  );
}

export { ToggleGroup, ToggleGroupItem };

const styles = stylex.create({
  toggleGroup: {
    display: 'flex',
    width: 'fit-content',
    alignItems: 'center',
    borderRadius: '8px',
    borderStyle: 'solid',
    borderWidth: '1px',
    padding: '0.125rem',
  },
  toggleGroupItem: {
    display: 'flex',
    height: {
      default: '2.5rem',
      '@media (min-width: 48rem)': '1.75rem',
    },
    alignItems: 'center',
    gap: '0.25rem',
    borderRadius: '6px',
    paddingInline: {
      default: '0.75rem',
      '@media (min-width: 48rem)': '0.625rem',
    },
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    color: {
      default: colors.mutedForeground,
      ':is([data-pressed])': colors.background,
      ':hover': {
        default: null,
        ':is([data-pressed])': null,
        '@media (hover: hover)': {
          default: colors.foreground,
          ':is([data-pressed])': colors.background,
        },
      },
    },
    transitionProperty:
      'color, background-color, border-color, outline-color, text-decoration-color, fill, stroke',
    transitionTimingFunction: 'cubic-bezier(0.25, 0.1, 0.25, 1)',
    transitionDuration: '200ms',
    outlineStyle: 'none',
    boxShadow: {
      default: null,
      ':focus-visible': `0 0 0 2px ${colors.ring}`,
    },
    backgroundColor: {
      default: null,
      ':is([data-pressed])': colors.foreground,
    },
  },
});
