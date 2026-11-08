/**
 * Loading message that shimmers while it waits, in the theme's text colors.
 */

import { ShimmeringText } from '@animate-ui/react/primitives/texts/shimmering';

/**
 * Render a loading message whose characters shimmer from the muted text color to the foreground.
 * Screen readers read the message once as plain text.
 *
 * @param props - Message text and optional classes.
 * @returns Shimmering loading message.
 */
export function LoadingText({ text, className }: { text: string; className?: string }) {
  return (
    <ShimmeringText
      text={text}
      className={className}
      color="var(--muted-foreground)"
      shimmeringColor="var(--foreground)"
    />
  );
}
