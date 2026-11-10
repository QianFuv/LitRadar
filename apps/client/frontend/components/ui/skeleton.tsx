import * as stylex from '@stylexjs/stylex';
import { colors } from '@/lib/theme.stylex';

function Skeleton({
  xstyle,
  ...props
}: Omit<React.ComponentProps<'div'>, 'className'> & { xstyle?: stylex.StyleXStyles }) {
  return <div data-slot="skeleton" {...stylex.props(styles.skeleton, xstyle)} {...props} />;
}

export { Skeleton };

const pulseKeyframes = stylex.keyframes({ '50%': { opacity: 0.5 } });

const styles = stylex.create({
  skeleton: {
    animationName: pulseKeyframes,
    animationDuration: '2s',
    animationTimingFunction: 'cubic-bezier(0.4, 0, 0.6, 1)',
    animationIterationCount: 'infinite',
    borderRadius: '6px',
    backgroundColor: colors.accent,
  },
});
