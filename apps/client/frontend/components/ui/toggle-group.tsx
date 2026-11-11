/**
 * Project-styled single-choice segmented control: a Base UI radio group whose selected surface
 * slides between segments (Animate UI Highlight).
 */

import * as React from 'react';
import { RadioGroup as RadioGroupPrimitive } from '@base-ui/react/radio-group';
import { Radio } from '@animate-ui/react/primitives/base-ui/radio';
import { Highlight, HighlightItem } from '@animate-ui/react/primitives/effects/highlight';

import { PresenceBoundary } from '@/components/ui/motion';
import * as stylex from '@stylexjs/stylex';
import { colors, lineHeights } from '@/lib/theme.stylex';

/** Spring that moves the selected surface between segments. */
const SELECTION_SPRING = { type: 'spring', stiffness: 300, damping: 30 } as const;

type ToggleGroupProps<Value extends string> = Omit<
  React.ComponentProps<typeof RadioGroupPrimitive<Value>>,
  'className' | 'value' | 'defaultValue' | 'onValueChange'
> & {
  /** Value of the selected segment. */
  value: Value;
  /** Called with the newly selected segment. */
  onValueChange: (value: Value) => void;
  xstyle?: stylex.StyleXStyles;
};

/**
 * Render a segmented control where exactly one segment is selected. It is a `radiogroup` of
 * `radio` segments: arrow keys move the selection, and Space or a click selects.
 *
 * @param props - Selected value, change handler, Base UI radio group properties, and style
 * overrides.
 * @returns Segmented control root.
 */
function ToggleGroup<Value extends string>({
  xstyle,
  value,
  onValueChange,
  children,
  ...props
}: ToggleGroupProps<Value>) {
  return (
    <RadioGroupPrimitive<Value>
      data-slot="toggle-group"
      value={value}
      onValueChange={onValueChange}
      {...stylex.props(styles.toggleGroup, xstyle)}
      {...props}
    >
      <Highlight
        controlledItems
        value={value}
        click={false}
        transition={SELECTION_SPRING}
        {...stylex.props(styles.selectedSurface)}
      >
        {children}
      </Highlight>
    </RadioGroupPrimitive>
  );
}

/**
 * Render one segment as a `radio` button; the selected segment sits on the sliding surface.
 *
 * @param props - Segment value, Base UI radio properties, and style overrides.
 * @returns Segment button.
 */
function ToggleGroupItem({
  value,
  xstyle,
  ...props
}: Omit<React.ComponentProps<typeof Radio.Root>, 'className' | 'value' | 'nativeButton'> & {
  value: string;
  xstyle?: stylex.StyleXStyles;
}) {
  return (
    <PresenceBoundary>
      <HighlightItem value={value}>
        <Radio.Root
          nativeButton
          data-slot="toggle-group-item"
          value={value}
          {...stylex.props(styles.toggleGroupItem, xstyle)}
          {...props}
        />
      </HighlightItem>
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
  selectedSurface: {
    inset: 0,
    borderRadius: '6px',
    backgroundColor: colors.foreground,
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
      ':is([data-checked])': colors.background,
      ':hover': {
        default: null,
        ':is([data-checked])': null,
        '@media (hover: hover)': {
          default: colors.foreground,
          ':is([data-checked])': colors.background,
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
  },
});
