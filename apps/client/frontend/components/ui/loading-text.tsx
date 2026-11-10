/**
 * Loading message that shimmers while it waits, in the theme's text colors.
 */

import { ShimmeringText } from '@animate-ui/react/primitives/texts/shimmering';
import * as stylex from '@stylexjs/stylex';
import { colors } from '@/lib/theme.stylex';

/**
 * Render a loading message whose characters shimmer from the muted text color to the foreground.
 * Screen readers read the message once as plain text.
 *
 * @param props - Message text and optional styles.
 * @returns Shimmering loading message.
 */
export function LoadingText({ text, xstyle }: { text: string; xstyle?: stylex.StyleXStyles }) {
  return (
    <ShimmeringText
      text={text}
      {...stylex.props(xstyle)}
      color={colors.mutedForeground}
      shimmeringColor={colors.foreground}
    />
  );
}
