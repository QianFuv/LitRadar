import * as React from 'react';
import * as ScrollAreaPrimitive from '@radix-ui/react-scroll-area';
import * as stylex from '@stylexjs/stylex';
import { colors } from '@/lib/theme.stylex';

function ScrollArea({
  xstyle,
  children,
  ...props
}: Omit<React.ComponentProps<typeof ScrollAreaPrimitive.Root>, 'className'> & {
  xstyle?: stylex.StyleXStyles;
}) {
  return (
    <ScrollAreaPrimitive.Root
      data-slot="scroll-area"
      {...stylex.props(styles.scrollArea, xstyle)}
      {...props}
    >
      <ScrollAreaPrimitive.Viewport
        data-slot="scroll-area-viewport"
        {...stylex.props(styles.scrollAreaViewport)}
      >
        {children}
      </ScrollAreaPrimitive.Viewport>
      <ScrollBar />
      <ScrollAreaPrimitive.Corner />
    </ScrollAreaPrimitive.Root>
  );
}

function ScrollBar({
  xstyle,
  orientation = 'vertical',
  ...props
}: Omit<React.ComponentProps<typeof ScrollAreaPrimitive.ScrollAreaScrollbar>, 'className'> & {
  xstyle?: stylex.StyleXStyles;
}) {
  return (
    <ScrollAreaPrimitive.ScrollAreaScrollbar
      data-slot="scroll-area-scrollbar"
      orientation={orientation}
      {...stylex.props(
        styles.scrollAreaScrollbar,
        orientation === 'vertical' && styles.scrollAreaScrollbarVertical,
        orientation === 'horizontal' && styles.scrollAreaScrollbarHorizontal,
        xstyle,
      )}
      {...props}
    >
      <ScrollAreaPrimitive.ScrollAreaThumb
        data-slot="scroll-area-thumb"
        {...stylex.props(styles.scrollAreaThumb)}
      />
    </ScrollAreaPrimitive.ScrollAreaScrollbar>
  );
}

export { ScrollArea, ScrollBar };

const styles = stylex.create({
  scrollArea: {
    position: 'relative',
  },
  scrollAreaViewport: {
    width: '100%',
    height: '100%',
    borderRadius: 'inherit',
    transitionProperty: 'color, box-shadow',
    transitionTimingFunction: 'cubic-bezier(0.4, 0, 0.2, 1)',
    transitionDuration: '150ms',
    outlineStyle: {
      default: 'none',
      ':focus-visible': 'none',
    },
    boxShadow: {
      default: null,
      ':focus-visible': `0 0 0 3px color-mix(in oklab, ${colors.ring} 50%, transparent)`,
    },
    outlineWidth: {
      default: null,
      ':focus-visible': '1px',
    },
  },
  scrollAreaScrollbar: {
    display: 'flex',
    touchAction: 'none',
    padding: '1px',
    transitionProperty:
      'color, background-color, border-color, outline-color, text-decoration-color, fill, stroke',
    transitionTimingFunction: 'cubic-bezier(0.4, 0, 0.2, 1)',
    transitionDuration: '150ms',
    userSelect: 'none',
  },
  scrollAreaScrollbarVertical: {
    height: '100%',
    width: '0.625rem',
    borderLeftStyle: 'solid',
    borderLeftWidth: '1px',
    borderLeftColor: 'transparent',
  },
  scrollAreaScrollbarHorizontal: {
    height: '0.625rem',
    flexDirection: 'column',
    borderTopStyle: 'solid',
    borderTopWidth: '1px',
    borderTopColor: 'transparent',
  },
  scrollAreaThumb: {
    position: 'relative',
    flex: '1',
    borderRadius: 'calc(infinity * 1px)',
    backgroundColor: colors.border,
  },
});
