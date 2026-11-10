/**
 * Initial-letter account avatar shared by the rail, tab bar, and account menu.
 */

import * as stylex from '@stylexjs/stylex';
import { colors, lineHeights } from '@/lib/theme.stylex';

type UserAvatarProps = {
  xstyle?: stylex.StyleXStyles;
  size?: 'sm' | 'md' | 'lg';
  username: string;
};

/**
 * Render a decorative circular avatar containing the first username character.
 *
 * @param props - Username, visual size, and optional style overrides.
 * @returns Decorative avatar.
 */
export function UserAvatar({ xstyle, size = 'md', username }: UserAvatarProps) {
  const initial = Array.from(username.trim())[0]?.toUpperCase() ?? '?';

  return (
    <span
      aria-hidden="true"
      {...stylex.props(styles.userAvatarSpan, AVATAR_SIZE_STYLES[size], xstyle)}
    >
      {initial}
    </span>
  );
}

const styles = stylex.create({
  sizeSm: {
    width: '1.5rem',
    height: '1.5rem',
    fontSize: '11px',
  },
  sizeMd: {
    width: '2.25rem',
    height: '2.25rem',
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
  },
  sizeLg: {
    width: '2.5rem',
    height: '2.5rem',
    fontSize: '1rem',
    lineHeight: 1.5,
  },
  userAvatarSpan: {
    display: 'inline-flex',
    flexShrink: 0,
    alignItems: 'center',
    justifyContent: 'center',
    borderRadius: 'calc(infinity * 1px)',
    backgroundColor: colors.primary,
    fontWeight: 700,
    color: colors.primaryForeground,
    userSelect: 'none',
  },
});

const AVATAR_SIZE_STYLES: Readonly<
  Record<NonNullable<UserAvatarProps['size']>, stylex.StyleXStyles>
> = {
  sm: styles.sizeSm,
  md: styles.sizeMd,
  lg: styles.sizeLg,
};
