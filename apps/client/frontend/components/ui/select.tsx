/**
 * Project-styled Radix Select primitives with shared surface motion.
 */

import * as React from 'react';
import * as SelectPrimitive from '@animate-ui/react/primitives/radix-ui/select';
import { CheckIcon, ChevronDownIcon, ChevronUpIcon } from 'lucide-react';

import { RevealScope } from '@/components/ui/reveal';
import * as stylex from '@stylexjs/stylex';
import { colors, lineHeights, shadows } from '@/lib/theme.stylex';

/**
 * Render the Select state root.
 *
 * @param props - Radix Select root properties.
 * @returns Select root.
 */
function Select({ ...props }: React.ComponentProps<typeof SelectPrimitive.Root>) {
  return <SelectPrimitive.Root data-slot="select" {...props} />;
}

/**
 * Render the selected value or placeholder.
 *
 * @param props - Radix Select value properties.
 * @returns Select value.
 */
function SelectValue({ ...props }: React.ComponentProps<typeof SelectPrimitive.Value>) {
  return <SelectPrimitive.Value data-slot="select-value" {...props} />;
}

/**
 * Render the keyboard-accessible Select trigger.
 *
 * @param props - Radix Select trigger properties, control size, and style overrides for the
 * trigger and its chevron.
 * @returns Styled Select trigger.
 */
function SelectTrigger({
  xstyle,
  iconXstyle,
  size = 'default',
  children,
  ...props
}: Omit<
  React.ComponentProps<typeof SelectPrimitive.Trigger> & {
    size?: 'sm' | 'default';
  },
  'className'
> & { xstyle?: stylex.StyleXStyles; iconXstyle?: stylex.StyleXStyles }) {
  return (
    <SelectPrimitive.Trigger
      data-slot="select-trigger"
      data-size={size}
      {...stylex.props(styles.selectTrigger, stylex.defaultMarker(), xstyle)}
      {...props}
    >
      {children}
      <SelectPrimitive.Icon asChild>
        <ChevronDownIcon
          {...stylex.props(styles.selectTriggerChevronDownIcon, iconXstyle)}
          aria-hidden="true"
        />
      </SelectPrimitive.Icon>
    </SelectPrimitive.Trigger>
  );
}

/**
 * Render positioned Select options in a portal.
 *
 * @param props - Radix Select content properties and positioning defaults.
 * @returns Portaled Select options.
 */
function SelectContent({
  xstyle,
  children,
  position = 'popper',
  align = 'start',
  viewportXstyle,
  ...props
}: Omit<
  React.ComponentProps<typeof SelectPrimitive.Content> & {
    /** Extra styles for the scrolling option viewport. */
    viewportXstyle?: stylex.StyleXStyles;
  },
  'className'
> & { xstyle?: stylex.StyleXStyles }) {
  return (
    <SelectPrimitive.Portal>
      <RevealScope>
        <SelectPrimitive.Content
          data-slot="select-content"
          {...stylex.props(
            styles.selectContent,
            position === 'popper' && styles.selectContentPopper,
            xstyle,
          )}
          position={position}
          align={align}
          {...props}
        >
          <SelectScrollUpButton />
          <SelectPrimitive.Viewport
            {...stylex.props(
              styles.selectViewport,
              viewportXstyle,
              position === 'popper' && styles.selectViewportPopper,
            )}
          >
            {children}
          </SelectPrimitive.Viewport>
          <SelectScrollDownButton />
        </SelectPrimitive.Content>
      </RevealScope>
    </SelectPrimitive.Portal>
  );
}

/**
 * Render one selectable option and its checked indicator.
 *
 * @param props - Radix Select item properties, visible label, and optional trailing detail.
 * @returns Styled Select item.
 */
function SelectItem({
  xstyle,
  children,
  detail,
  ...props
}: Omit<
  React.ComponentProps<typeof SelectPrimitive.Item> & {
    /** Trailing detail such as a count, kept out of the selected value text. */
    detail?: React.ReactNode;
  },
  'className'
> & { xstyle?: stylex.StyleXStyles }) {
  return (
    <SelectPrimitive.Item
      data-slot="select-item"
      {...stylex.props(styles.selectItem, xstyle)}
      {...props}
    >
      <span data-slot="select-item-indicator" {...stylex.props(styles.selectItemIndicator)}>
        <SelectPrimitive.ItemIndicator>
          <CheckIcon {...stylex.props(styles.selectItemCheckIcon)} />
        </SelectPrimitive.ItemIndicator>
      </span>
      <SelectPrimitive.ItemText>{children}</SelectPrimitive.ItemText>
      {detail !== undefined && (
        <span {...stylex.props(styles.selectItemSpan)}>
          <span {...stylex.props(styles.selectItemSpan2)}> · </span>
          {detail}
        </span>
      )}
    </SelectPrimitive.Item>
  );
}

/**
 * Render the upward overflow affordance.
 *
 * @param props - Radix Select scroll-up properties.
 * @returns Styled scroll-up control.
 */
function SelectScrollUpButton({
  xstyle,
  ...props
}: Omit<React.ComponentProps<typeof SelectPrimitive.ScrollUpButton>, 'className'> & {
  xstyle?: stylex.StyleXStyles;
}) {
  return (
    <SelectPrimitive.ScrollUpButton
      data-slot="select-scroll-up-button"
      {...stylex.props(styles.selectScrollUpButton, xstyle)}
      {...props}
    >
      <ChevronUpIcon {...stylex.props(styles.selectItemCheckIcon)} />
    </SelectPrimitive.ScrollUpButton>
  );
}

/**
 * Render the downward overflow affordance.
 *
 * @param props - Radix Select scroll-down properties.
 * @returns Styled scroll-down control.
 */
function SelectScrollDownButton({
  xstyle,
  ...props
}: Omit<React.ComponentProps<typeof SelectPrimitive.ScrollDownButton>, 'className'> & {
  xstyle?: stylex.StyleXStyles;
}) {
  return (
    <SelectPrimitive.ScrollDownButton
      data-slot="select-scroll-down-button"
      {...stylex.props(styles.selectScrollUpButton, xstyle)}
      {...props}
    >
      <ChevronDownIcon {...stylex.props(styles.selectItemCheckIcon)} />
    </SelectPrimitive.ScrollDownButton>
  );
}

export {
  Select,
  SelectContent,
  SelectItem,
  SelectScrollDownButton,
  SelectScrollUpButton,
  SelectTrigger,
  SelectValue,
};

const styles = stylex.create({
  selectViewport: {
    padding: '0.25rem',
  },
  selectViewportPopper: {
    height: 'var(--radix-select-trigger-height)',
    width: '100%',
    minWidth: 'var(--radix-select-trigger-width)',
    scrollMarginBlock: '0.25rem',
  },
  selectTrigger: {
    display: 'flex',
    width: 'fit-content',
    alignItems: 'center',
    justifyContent: 'space-between',
    gap: '0.5rem',
    borderRadius: '8px',
    backgroundColor: {
      default: colors.card,
      ':active': {
        default: colors.accentPressed,
        ':hover': {
          default: null,
          '@media (hover: hover)': colors.accentPressed,
        },
      },
      ':hover': {
        default: null,
        '@media (hover: hover)': colors.accent,
      },
    },
    paddingInline: '0.75rem',
    paddingBlock: '0.5rem',
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
    whiteSpace: 'nowrap',
    boxShadow: {
      default: shadows.vercelRing,
      ':focus-visible': {
        default: `0 0 0 3px color-mix(in oklab, ${colors.ring} 50%, transparent), ${shadows.vercelRing}`,
        ':is([aria-invalid="true"])': `0 0 0 3px ${colors.destructiveRing}, ${shadows.vercelRing}`,
      },
    },
    transitionProperty: 'background-color, color, box-shadow',
    transitionTimingFunction: 'cubic-bezier(0.25, 0.1, 0.25, 1)',
    transitionDuration: '200ms',
    outlineStyle: 'none',
    cursor: {
      default: null,
      ':disabled': 'not-allowed',
    },
    opacity: {
      default: null,
      ':disabled': '50%',
    },
    borderColor: {
      default: null,
      ':is([aria-invalid="true"])': colors.destructive,
    },
    color: {
      default: null,
      ':is([data-placeholder])': colors.mutedForeground,
    },
    height: {
      default: null,
      ':is([data-size="default"])': '2.25rem',
      ':is([data-size="sm"])': '2rem',
    },
  },
  selectTriggerChevronDownIcon: {
    width: '1rem',
    height: '1rem',
    opacity: '50%',
    transitionProperty: 'transform, translate, scale, rotate',
    transitionTimingFunction: 'cubic-bezier(0.215, 0.61, 0.355, 1)',
    transitionDuration: '220ms',
    rotate: {
      default: null,
      [stylex.when.ancestor('[data-state="open"]')]: '180deg',
    },
  },
  selectContent: {
    position: 'relative',
    zIndex: 50,
    maxHeight: 'var(--radix-select-content-available-height)',
    minWidth: '8rem',
    transformOrigin: 'var(--radix-select-content-transform-origin)',
    overflowX: 'hidden',
    overflowY: 'auto',
    borderRadius: '12px',
    backgroundColor: colors.popover,
    color: colors.popoverForeground,
    boxShadow: shadows.overlay,
  },
  selectContentPopper: {
    translate: {
      default: null,
      ':is([data-side="bottom"])': '0 0.25rem',
      ':is([data-side="left"])': '-0.25rem 0',
      ':is([data-side="right"])': '0.25rem 0',
      ':is([data-side="top"])': '0 -0.25rem',
    },
  },
  selectItem: {
    position: 'relative',
    display: 'flex',
    width: '100%',
    cursor: 'default',
    alignItems: 'center',
    gap: '0.5rem',
    borderRadius: '4px',
    paddingBlock: '0.375rem',
    paddingRight: '2rem',
    paddingLeft: '0.5rem',
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
        ':is([data-highlighted])': colors.accentPressed,
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
  selectItemIndicator: {
    position: 'absolute',
    right: '0.5rem',
    display: 'flex',
    width: '0.875rem',
    height: '0.875rem',
    alignItems: 'center',
    justifyContent: 'center',
  },
  selectItemCheckIcon: {
    width: '1rem',
    height: '1rem',
  },
  selectItemSpan: {
    marginLeft: 'auto',
    flexShrink: 0,
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    color: colors.mutedForeground,
    fontVariantNumeric: 'tabular-nums',
  },
  selectItemSpan2: {
    position: 'absolute',
    width: '1px',
    height: '1px',
    padding: 0,
    margin: '-1px',
    overflow: 'hidden',
    clipPath: 'inset(50%)',
    whiteSpace: 'nowrap',
    borderWidth: 0,
  },
  selectScrollUpButton: {
    display: 'flex',
    cursor: 'default',
    alignItems: 'center',
    justifyContent: 'center',
    paddingBlock: '0.25rem',
  },
});
