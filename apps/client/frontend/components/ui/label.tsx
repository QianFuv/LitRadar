import * as React from 'react';
import * as LabelPrimitive from '@radix-ui/react-label';
import * as stylex from '@stylexjs/stylex';

function Label({
  xstyle,
  ...props
}: Omit<React.ComponentProps<typeof LabelPrimitive.Root>, 'className'> & {
  xstyle?: stylex.StyleXStyles;
}) {
  return (
    <LabelPrimitive.Root data-slot="label" {...stylex.props(styles.label, xstyle)} {...props} />
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
