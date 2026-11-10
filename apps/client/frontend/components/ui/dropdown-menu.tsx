/**
 * Project-styled Radix dropdown menu primitives.
 */

import * as DropdownMenuPrimitive from '@animate-ui/react/primitives/radix-ui/dropdown-menu';
import * as React from 'react';

import { IconTriggerProvider, useIconTrigger } from '@/components/ui/animated-icons';
import { RevealScope } from '@/components/ui/reveal';
import * as stylex from '@stylexjs/stylex';
import { colors, lineHeights, shadows } from '@/lib/theme.stylex';

/**
 * Render the dropdown menu state root.
 *
 * @param props - Radix root properties.
 * @returns Dropdown menu root.
 */
function DropdownMenu(props: React.ComponentProps<typeof DropdownMenuPrimitive.Root>) {
  return <DropdownMenuPrimitive.Root data-slot="dropdown-menu" {...props} />;
}

/**
 * Render the control that toggles the menu.
 *
 * @param props - Radix trigger properties.
 * @returns Dropdown menu trigger.
 */
function DropdownMenuTrigger(props: React.ComponentProps<typeof DropdownMenuPrimitive.Trigger>) {
  return <DropdownMenuPrimitive.Trigger data-slot="dropdown-menu-trigger" {...props} />;
}

/**
 * Leave close focus to Radix, which restores the trigger after the pointer's own focus change.
 * Pass it to Animate UI menu content rendered outside `DropdownMenuContent`.
 */
export function keepRadixCloseFocus(): void {}

/**
 * Render positioned menu content in a portal.
 *
 * Animate UI returns focus to the trigger as soon as the menu starts closing, before the outside
 * pointer press moves focus to the page, and then blocks Radix's own restore; passing a close
 * focus handler opts out of that early restore so Radix returns focus once the exit finishes.
 *
 * @param props - Radix content properties.
 * @returns Portaled menu content.
 */
function DropdownMenuContent({
  xstyle,
  sideOffset = 6,
  onCloseAutoFocus = keepRadixCloseFocus,
  ...props
}: Omit<React.ComponentProps<typeof DropdownMenuPrimitive.Content>, 'className'> & {
  xstyle?: stylex.StyleXStyles;
}) {
  return (
    <DropdownMenuPrimitive.Portal>
      <RevealScope>
        <DropdownMenuPrimitive.Content
          data-slot="dropdown-menu-content"
          sideOffset={sideOffset}
          collisionPadding={8}
          onCloseAutoFocus={onCloseAutoFocus}
          {...stylex.props(styles.dropdownMenuContent, xstyle)}
          {...props}
        />
      </RevealScope>
    </DropdownMenuPrimitive.Portal>
  );
}

/**
 * Render one selectable menu item.
 *
 * @param props - Radix item properties and destructive tone.
 * @returns Menu item that plays its animated icons while hovered or pressed.
 */
function DropdownMenuItem({
  xstyle,
  isDestructive = false,
  ...props
}: Omit<
  React.ComponentProps<typeof DropdownMenuPrimitive.Item> & { isDestructive?: boolean },
  'className'
> & { xstyle?: stylex.StyleXStyles }) {
  const { isTriggered, triggerHandlers } = useIconTrigger(props);
  return (
    <IconTriggerProvider isTriggered={isTriggered}>
      <DropdownMenuPrimitive.Item
        data-slot="dropdown-menu-item"
        data-variant={isDestructive ? 'destructive' : undefined}
        {...stylex.props(
          styles.dropdownMenuItem,
          isDestructive && styles.dropdownMenuItemDestructive,
          xstyle,
        )}
        {...props}
        {...triggerHandlers}
      />
    </IconTriggerProvider>
  );
}

/**
 * Render a non-interactive menu label.
 *
 * @param props - Radix label properties.
 * @returns Menu label.
 */
function DropdownMenuLabel({
  xstyle,
  ...props
}: Omit<React.ComponentProps<typeof DropdownMenuPrimitive.Label>, 'className'> & {
  xstyle?: stylex.StyleXStyles;
}) {
  return (
    <DropdownMenuPrimitive.Label
      data-slot="dropdown-menu-label"
      {...stylex.props(styles.dropdownMenuLabel, xstyle)}
      {...props}
    />
  );
}

/**
 * Render a visual separator between menu groups.
 *
 * @param props - Radix separator properties.
 * @returns Menu separator.
 */
function DropdownMenuSeparator({
  xstyle,
  ...props
}: Omit<React.ComponentProps<typeof DropdownMenuPrimitive.Separator>, 'className'> & {
  xstyle?: stylex.StyleXStyles;
}) {
  return (
    <DropdownMenuPrimitive.Separator
      data-slot="dropdown-menu-separator"
      {...stylex.props(styles.dropdownMenuSeparator, xstyle)}
      {...props}
    />
  );
}

export {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
};

const styles = stylex.create({
  dropdownMenuContent: {
    zIndex: 50,
    minWidth: '12rem',
    transformOrigin: 'var(--radix-dropdown-menu-content-transform-origin)',
    borderRadius: '12px',
    backgroundColor: colors.popover,
    padding: '0.375rem',
    color: colors.popoverForeground,
    boxShadow: shadows.overlay,
    outlineStyle: {
      default: 'none',
      '@media (forced-colors: active)': 'solid',
    },
    outlineWidth: {
      default: null,
      '@media (forced-colors: active)': '2px',
    },
    outlineColor: {
      default: null,
      '@media (forced-colors: active)': 'transparent',
    },
    outlineOffset: {
      default: null,
      '@media (forced-colors: active)': '2px',
    },
  },
  dropdownMenuItem: {
    position: 'relative',
    display: 'flex',
    minHeight: '2.25rem',
    width: '100%',
    cursor: 'default',
    alignItems: 'center',
    gap: '0.625rem',
    borderRadius: '8px',
    paddingInline: '0.625rem',
    paddingBlock: '0.375rem',
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
    outlineStyle: {
      default: 'none',
      '@media (forced-colors: active)': 'solid',
    },
    transitionProperty: 'background-color, color',
    transitionTimingFunction: 'cubic-bezier(0.25, 0.1, 0.25, 1)',
    transitionDuration: '200ms',
    userSelect: 'none',
    outlineWidth: {
      default: null,
      '@media (forced-colors: active)': '2px',
    },
    outlineColor: {
      default: null,
      '@media (forced-colors: active)': 'transparent',
    },
    outlineOffset: {
      default: null,
      '@media (forced-colors: active)': '2px',
    },
    backgroundColor: {
      default: null,
      ':focus': colors.accent,
      ':active': {
        default: colors.accentPressed,
        ':is([data-highlighted])': colors.accent,
      },
      ':is([data-highlighted])': colors.accent,
    },
    color: {
      default: null,
      ':focus': colors.accentForeground,
    },
    pointerEvents: {
      default: null,
      ':is([data-disabled])': 'none',
    },
    opacity: {
      default: null,
      ':is([data-disabled])': '50%',
    },
  },
  dropdownMenuItemDestructive: {
    color: {
      default: colors.destructive,
      ':focus': colors.destructive,
    },
    backgroundColor: {
      default: null,
      ':active': {
        default: colors.accentPressed,
        ':is([data-highlighted])': colors.accent,
      },
      ':focus': `color-mix(in oklab, ${colors.destructive} 10%, transparent)`,
      ':is([data-highlighted])': `color-mix(in oklab, ${colors.destructive} 10%, transparent)`,
    },
  },
  dropdownMenuLabel: {
    paddingInline: '0.625rem',
    paddingBlock: '0.375rem',
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    color: colors.mutedForeground,
  },
  dropdownMenuSeparator: {
    marginInline: '-0.375rem',
    marginBlock: '0.375rem',
    height: '1px',
    backgroundColor: colors.border,
  },
});
