/** Shared single-line input with theme-aware pointer and keyboard feedback. */

import { Input as BaseInput } from '@base-ui/react/input';
import * as stylex from '@stylexjs/stylex';
import { colors, lineHeights, shadows } from '@/lib/theme.stylex';

/**
 * Render a Base UI input, which joins an enclosing `Field` for labelling and validity, while
 * preserving native attributes, sizing, and invalid states.
 *
 * @param props - Base UI input props and optional styles.
 * @returns Styled input element.
 */
function Input({
  xstyle,
  ...props
}: Omit<BaseInput.Props, 'className' | 'style'> & { xstyle?: stylex.StyleXStyles }) {
  return <BaseInput data-slot="input" {...stylex.props(styles.input, xstyle)} {...props} />;
}

export { Input };

const styles = stylex.create({
  input: {
    height: '2.25rem',
    width: '100%',
    minWidth: '0px',
    borderRadius: '8px',
    backgroundColor: {
      default: colors.card,
      ':hover': {
        default: null,
        '@media (hover: hover)': `color-mix(in oklab, ${colors.accent} 50%, transparent)`,
      },
    },
    paddingInline: '0.75rem',
    paddingBlock: '0.25rem',
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
    '::file-selector-button': {
      display: 'inline-flex',
      height: '1.75rem',
      borderStyle: 'solid',
      borderWidth: '0px',
      backgroundColor: colors.card,
      fontSize: '0.875rem',
      lineHeight: lineHeights.textSm,
      fontWeight: 500,
      color: colors.foreground,
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

/** Input styles shared with number fields so every text control matches. */
export const inputStyle = styles.input;
