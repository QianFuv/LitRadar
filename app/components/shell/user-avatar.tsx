/**
 * Initial-letter account avatar shared by the rail, tab bar, and account menu.
 */

import { cn } from '@/lib/utils';

type UserAvatarProps = {
  className?: string;
  size?: 'sm' | 'md' | 'lg';
  username: string;
};

const AVATAR_SIZE_CLASSES: Readonly<Record<NonNullable<UserAvatarProps['size']>, string>> = {
  sm: 'size-6 text-[11px]',
  md: 'size-9 text-sm',
  lg: 'size-10 text-base',
};

/**
 * Render a decorative circular avatar containing the first username character.
 *
 * @param props - Username, visual size, and optional classes.
 * @returns Decorative avatar.
 */
export function UserAvatar({ className, size = 'md', username }: UserAvatarProps) {
  const initial = Array.from(username.trim())[0]?.toUpperCase() ?? '?';

  return (
    <span
      aria-hidden="true"
      className={cn(
        'inline-flex shrink-0 select-none items-center justify-center rounded-full bg-primary font-bold text-primary-foreground',
        AVATAR_SIZE_CLASSES[size],
        className,
      )}
    >
      {initial}
    </span>
  );
}
