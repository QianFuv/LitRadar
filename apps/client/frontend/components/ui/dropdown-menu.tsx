/**
 * Project-styled Animate UI menu primitives on Base UI.
 */

import { Menu as DropdownMenuPrimitive } from '@animate-ui/react/primitives/base-ui/menu';
import * as React from 'react';

import {
  IconStyleProvider,
  IconTriggerProvider,
  useIconTrigger,
} from '@/components/ui/animated-icons';
import { controlIconStyle } from '@/components/ui/button';
import { RevealScope } from '@/components/ui/reveal';
import { popupLayerStyle } from '@/components/ui/popup-layer';
import * as stylex from '@stylexjs/stylex';
import { colors, lineHeights, shadows } from '@/lib/theme.stylex';

/**
 * Render the dropdown menu state root.
 *
 * @param props - Base UI menu root properties.
 * @returns Dropdown menu root.
 */
function DropdownMenu(props: React.ComponentProps<typeof DropdownMenuPrimitive.Root>) {
  return <DropdownMenuPrimitive.Root {...props} />;
}

/**
 * Render the control that toggles the menu.
 *
 * @param props - Base UI menu trigger properties.
 * @returns Dropdown menu trigger.
 */
function DropdownMenuTrigger(props: React.ComponentProps<typeof DropdownMenuPrimitive.Trigger>) {
  return <DropdownMenuPrimitive.Trigger data-slot="dropdown-menu-trigger" {...props} />;
}

type DropdownMenuContentProps = Omit<
  React.ComponentProps<typeof DropdownMenuPrimitive.Popup>,
  'className'
> &
  Pick<
    React.ComponentProps<typeof DropdownMenuPrimitive.Positioner>,
    'align' | 'side' | 'sideOffset'
  > & {
    xstyle?: stylex.StyleXStyles;
  };

/**
 * Render positioned menu content in a portal.
 *
 * @param props - Base UI menu popup properties, positioning, and style overrides.
 * @returns Portaled menu content.
 */
function DropdownMenuContent({
  xstyle,
  align = 'center',
  side = 'bottom',
  sideOffset = 6,
  ...props
}: DropdownMenuContentProps) {
  return (
    <DropdownMenuPrimitive.Portal {...stylex.props(popupLayerStyle)}>
      <DropdownMenuPrimitive.Positioner
        align={align}
        side={side}
        sideOffset={sideOffset}
        collisionPadding={8}
        collisionAvoidance={{ align: 'shift', fallbackAxisSide: 'none' }}
        {...stylex.props(styles.dropdownMenuPositioner)}
      >
        <RevealScope>
          <DropdownMenuPrimitive.Popup
            data-slot="dropdown-menu-content"
            {...stylex.props(styles.dropdownMenuContent, xstyle)}
            {...props}
          />
        </RevealScope>
      </DropdownMenuPrimitive.Positioner>
    </DropdownMenuPrimitive.Portal>
  );
}

/**
 * Render one selectable menu item.
 *
 * @param props - Base UI menu item properties, destructive tone, and style overrides.
 * @returns Menu item that plays its animated icons while hovered or pressed.
 */
function DropdownMenuItem({
  xstyle,
  isDestructive = false,
  ...props
}: Omit<React.ComponentProps<typeof DropdownMenuPrimitive.Item>, 'className'> & {
  isDestructive?: boolean;
  xstyle?: stylex.StyleXStyles;
}) {
  const { isTriggered, triggerHandlers } = useIconTrigger(props);
  return (
    <IconTriggerProvider isTriggered={isTriggered}>
      <IconStyleProvider
        xstyle={[controlIconStyle, isDestructive ? styles.itemIconDestructive : menuItemIconStyle]}
      >
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
      </IconStyleProvider>
    </IconTriggerProvider>
  );
}

/**
 * Render a non-interactive caption at the top of a menu.
 *
 * @param props - Native container properties and style overrides.
 * @returns Menu caption.
 */
function DropdownMenuLabel({
  xstyle,
  ...props
}: Omit<React.ComponentProps<'div'>, 'className'> & {
  xstyle?: stylex.StyleXStyles;
}) {
  return (
    <div
      data-slot="dropdown-menu-label"
      {...stylex.props(styles.dropdownMenuLabel, xstyle)}
      {...props}
    />
  );
}

/**
 * Render a visual separator between menu groups.
 *
 * @param props - Base UI separator properties and style overrides.
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
  menuItemIconStyle,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
};

const styles = stylex.create({
  itemIcon: {
    color: colors.mutedForeground,
  },
  itemIconDestructive: {
    color: colors.destructive,
  },
  dropdownMenuPositioner: {
    zIndex: 50,
  },
  dropdownMenuContent: {
    minWidth: '12rem',
    transformOrigin: 'var(--transform-origin)',
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
    transitionProperty: 'color',
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

/** Muted color of icons inside menu items, combined with the shared control icon size. */
const menuItemIconStyle = styles.itemIcon;
