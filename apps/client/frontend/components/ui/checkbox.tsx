/** Checkbox colors and feedback shared by filters and configuration forms. */

import * as React from 'react';
import { Checkbox as CheckboxPrimitive } from '@animate-ui/react/primitives/base-ui/checkbox';
import { CheckIcon } from 'lucide-react';
import * as stylex from '@stylexjs/stylex';
import { colors } from '@/lib/theme.stylex';

/**
 * Render a checkbox as a native button whose borders and focus feedback stay inside the control,
 * so paint-contained lists cannot clip them.
 *
 * @param props - Base UI checkbox properties and style overrides.
 * @returns Styled checkbox.
 */
function Checkbox({
  xstyle,
  ...props
}: Omit<React.ComponentProps<typeof CheckboxPrimitive.Root>, 'className'> & {
  xstyle?: stylex.StyleXStyles;
}) {
  return (
    <CheckboxPrimitive.Root
      nativeButton
      render={(props) => <button {...props} type="button" />}
      data-slot="checkbox"
      {...stylex.props(styles.checkbox, stylex.defaultMarker(), xstyle)}
      {...props}
    >
      <CheckboxPrimitive.Indicator
        data-slot="checkbox-indicator"
        {...stylex.props(styles.checkboxIndicator)}
      >
        <CheckIcon {...stylex.props(styles.checkboxCheckIcon)} />
      </CheckboxPrimitive.Indicator>
    </CheckboxPrimitive.Root>
  );
}

export { Checkbox };

const styles = stylex.create({
  checkbox: {
    width: '1rem',
    height: '1rem',
    flexShrink: 0,
    borderRadius: '4px',
    backgroundColor: {
      default: colors.card,
      ':is([data-checked])': colors.primary,
      ':hover': {
        default: null,
        ':is([data-checked])': null,
        '@media (hover: hover)': {
          default: null,
          ':is([data-checked])': colors.primaryHover,
          ':is([data-unchecked])': colors.accent,
        },
        ':is([data-unchecked])': null,
      },
      ':is([data-unchecked])': null,
    },
    boxShadow: {
      default: `inset 0 0 0 1px ${colors.input}`,
      ':focus-visible': {
        default: `inset 0 0 0 3px color-mix(in oklab, ${colors.ring} 50%, transparent), inset 0 0 0 1px ${colors.input}`,
        ':is([aria-invalid="true"])': `inset 0 0 0 3px ${colors.destructiveRing}, inset 0 0 0 1px ${colors.input}`,
      },
    },
    transitionProperty: 'background-color, color, box-shadow',
    transitionTimingFunction: 'cubic-bezier(0.25, 0.1, 0.25, 1)',
    transitionDuration: '200ms',
    outlineStyle: 'none',
    cursor: {
      default: null,
      ':disabled': 'not-allowed',
    },
    opacity: {
      default: null,
      ':disabled': '50%',
    },
    borderColor: {
      default: null,
      ':is([aria-invalid="true"])': colors.destructive,
    },
    color: {
      default: null,
      ':is([data-checked])': colors.primaryForeground,
    },
  },
  checkboxIndicator: {
    display: 'grid',
    placeContent: 'center',
    color: 'currentcolor',
    transitionProperty: 'none',
  },
  checkboxCheckIcon: {
    width: '0.875rem',
    height: '0.875rem',
  },
});
