/**
 * Shared multiline text input primitive.
 */

import * as React from 'react';
import * as stylex from '@stylexjs/stylex';
import { colors, lineHeights, shadows } from '@/lib/theme.stylex';

/**
 * Render a multiline input with the same states and responsive text sizing as Input.
 *
 * @param props - Native textarea props and optional styles.
 * @returns Styled textarea element.
 */
function Textarea({
  xstyle,
  ...props
}: Omit<React.ComponentProps<'textarea'>, 'className'> & { xstyle?: stylex.StyleXStyles }) {
  return <textarea data-slot="textarea" {...stylex.props(styles.textarea, xstyle)} {...props} />;
}

export { Textarea };

const styles = stylex.create({
  textarea: {
    minHeight: '5rem',
    width: '100%',
    minWidth: '0px',
    resize: 'vertical',
    borderRadius: '6px',
    backgroundColor: {
      default: colors.card,
      ':hover': {
        default: null,
        '@media (hover: hover)': `color-mix(in oklab, ${colors.accent} 50%, transparent)`,
      },
    },
    paddingInline: '0.75rem',
    paddingBlock: '0.5rem',
    fontSize: {
      default: '1rem',
      '@media (min-width: 48rem)': '0.875rem',
    },
    lineHeight: {
      default: 1.5,
      '@media (min-width: 48rem)': lineHeights.textSm,
    },
    boxShadow: {
      default: shadows.vercelRing,
      ':focus-visible': {
        default: `0 0 0 3px color-mix(in oklab, ${colors.ring} 50%, transparent), ${shadows.vercelRing}`,
        ':is([aria-invalid="true"])': `0 0 0 3px ${colors.destructiveRing}, ${shadows.vercelRing}`,
        ':hover': {
          default: null,
          '@media (hover: hover)': {
            default: `0 0 0 3px color-mix(in oklab, ${colors.ring} 50%, transparent), 0 0 0 1px ${colors.inputHover}`,
            ':is([aria-invalid="true"])': `0 0 0 3px ${colors.destructiveRing}, 0 0 0 1px ${colors.inputHover}`,
          },
        },
      },
      ':hover': {
        default: null,
        '@media (hover: hover)': `0 0 0 1px ${colors.inputHover}`,
      },
    },
    transitionProperty: 'background-color, color, box-shadow',
    transitionTimingFunction: 'cubic-bezier(0.25, 0.1, 0.25, 1)',
    transitionDuration: '200ms',
    outlineStyle: 'none',
    pointerEvents: {
      default: null,
      ':disabled': 'none',
    },
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
    '::placeholder': {
      color: colors.mutedForeground,
    },
    '::selection': {
      backgroundColor: colors.primary,
      color: colors.primaryForeground,
    },
  },
});
