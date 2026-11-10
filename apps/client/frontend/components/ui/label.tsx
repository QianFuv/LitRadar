/**
 * Form label with the shared label typography.
 */

import * as React from 'react';
import * as stylex from '@stylexjs/stylex';

/**
 * Render a native label.
 *
 * @param props - Native label properties and style overrides.
 * @returns Styled label.
 */
function Label({
  xstyle,
  htmlFor,
  ...props
}: Omit<React.ComponentProps<'label'>, 'className'> & {
  xstyle?: stylex.StyleXStyles;
}) {
  return (
    <label data-slot="label" htmlFor={htmlFor} {...stylex.props(styles.label, xstyle)} {...props} />
  );
}

export { Label };

const styles = stylex.create({
  label: {
    display: 'flex',
    alignItems: 'center',
    gap: '0.5rem',
    fontSize: '0.875rem',
    lineHeight: 1,
    fontWeight: 500,
    userSelect: 'none',
    pointerEvents: {
      default: null,
      [stylex.when.ancestor('[data-disabled="true"]')]: 'none',
    },
    opacity: {
      default: null,
      [stylex.when.ancestor('[data-disabled="true"]')]: '50%',
      [stylex.when.siblingBefore(':disabled')]: '50%',
    },
    cursor: {
      default: null,
      [stylex.when.siblingBefore(':disabled')]: 'not-allowed',
    },
  },
});
