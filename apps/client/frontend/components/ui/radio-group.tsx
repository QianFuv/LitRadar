/**
 * Project-styled Base UI radio group with Animate UI indicator motion.
 */

import * as React from 'react';
import { RadioGroup as RadioGroupPrimitive } from '@animate-ui/react/primitives/base-ui/radio-group';
import type { RadioGroupProps } from '@base-ui/react/radio-group';
import { Radio } from '@animate-ui/react/primitives/base-ui/radio';
import { Check } from 'lucide-react';

import { PresenceBoundary } from '@/components/ui/motion';
import * as stylex from '@stylexjs/stylex';
import { colors } from '@/lib/theme.stylex';

/**
 * Render the radio group root.
 *
 * @param props - Base UI radio group properties and style overrides.
 * @returns Radio group root.
 */
function RadioGroup<Value extends string>({
  xstyle,
  ...props
}: Omit<RadioGroupProps<Value>, 'className'> & {
  xstyle?: stylex.StyleXStyles;
}) {
  return (
    <RadioGroupPrimitive<Value> data-slot="radio-group" {...stylex.props(xstyle)} {...props} />
  );
}

/**
 * Render one card-style choice whose animated check marks the selection.
 *
 * @param props - Base UI radio properties, the visible choice content, and style overrides.
 * @returns Selectable choice.
 */
function RadioGroupCard({
  xstyle,
  children,
  ...props
}: Omit<React.ComponentProps<typeof Radio.Root>, 'className'> & {
  xstyle?: stylex.StyleXStyles;
}) {
  return (
    <PresenceBoundary>
      <Radio.Root
        nativeButton
        render={(props) => <button {...props} type="button" />}
        data-slot="radio-group-card"
        {...stylex.props(styles.radioGroupCard, xstyle)}
        {...props}
      >
        {children}
        <Radio.Indicator {...stylex.props(styles.radioGroupCardRadioGroupPrimitiveIndicator)}>
          <Check {...stylex.props(styles.radioGroupCardCheck)} aria-hidden="true" />
        </Radio.Indicator>
      </Radio.Root>
    </PresenceBoundary>
  );
}

/**
 * Render one choice with no styling of its own, for choices that compose their own layout.
 *
 * @param props - Base UI radio properties.
 * @returns Selectable choice.
 */
function RadioGroupItem(props: React.ComponentProps<typeof Radio.Root>) {
  return (
    <PresenceBoundary>
      <Radio.Root
        nativeButton
        render={(props) => <button {...props} type="button" />}
        data-slot="radio-group-item"
        {...props}
      />
    </PresenceBoundary>
  );
}

/**
 * Render the animated mark shown inside the selected choice.
 *
 * @param props - Base UI radio indicator properties.
 * @returns Selection indicator.
 */
function RadioGroupIndicator(props: React.ComponentProps<typeof Radio.Indicator>) {
  return <Radio.Indicator data-slot="radio-group-indicator" {...props} />;
}

export { RadioGroup, RadioGroupCard, RadioGroupIndicator, RadioGroupItem };

const styles = stylex.create({
  radioGroupCard: {
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'space-between',
    gap: '0.5rem',
    borderRadius: '8px',
    paddingInline: '0.75rem',
    textAlign: 'left',
    fontSize: '13px',
    boxShadow: {
      default: `inset 0 0 0 1px ${colors.border}`,
      ':focus-visible': {
        default: `0 0 0 2px ${colors.ring}, inset 0 0 0 1px ${colors.border}`,
        ':is([data-checked])': `0 0 0 2px ${colors.ring}, inset 0 0 0 1px ${colors.primary}`,
      },
      ':is([data-checked])': `inset 0 0 0 1px ${colors.primary}`,
    },
    transitionProperty: 'background-color, box-shadow',
    transitionTimingFunction: 'cubic-bezier(0.25, 0.1, 0.25, 1)',
    transitionDuration: '200ms',
    outlineStyle: 'none',
    backgroundColor: {
      default: null,
      ':is([data-checked])': colors.info,
      ':hover': {
        default: null,
        ':is([data-checked])': null,
        '@media (hover: hover)': {
          default: colors.accent,
          ':is([data-checked])': colors.info,
        },
      },
    },
    cursor: {
      default: null,
      ':disabled': 'not-allowed',
    },
    opacity: {
      default: null,
      ':disabled': '50%',
    },
    fontWeight: {
      default: null,
      ':is([data-checked])': 700,
    },
    color: {
      default: null,
      ':is([data-checked])': colors.infoForeground,
    },
  },
  radioGroupCardRadioGroupPrimitiveIndicator: {
    display: 'flex',
    width: '0.875rem',
    height: '0.875rem',
    flexShrink: 0,
    alignItems: 'center',
    justifyContent: 'center',
  },
  radioGroupCardCheck: {
    width: '0.875rem',
    height: '0.875rem',
  },
});
