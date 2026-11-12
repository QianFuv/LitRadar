/**
 * Project-styled Animate UI Select primitives on Base UI with shared surface motion.
 */

import * as React from 'react';
import { Select as SelectPrimitive } from '@animate-ui/react/primitives/base-ui/select';
import { CheckIcon, ChevronDownIcon, ChevronUpIcon } from 'lucide-react';

import { IconStyleProvider } from '@/components/ui/animated-icons';
import { controlIconStyle } from '@/components/ui/button';
import { menuItemIconStyle } from '@/components/ui/dropdown-menu';
import { RevealScope } from '@/components/ui/reveal';
import { popupLayerStyle } from '@/components/ui/popup-layer';
import * as stylex from '@stylexjs/stylex';
import { colors, lineHeights, shadows } from '@/lib/theme.stylex';

type SelectProps<Value extends string> = Omit<
  React.ComponentProps<typeof SelectPrimitive.Root<Value>>,
  'value' | 'defaultValue' | 'onValueChange' | 'multiple'
> & {
  /** Selected value; an empty string shows the placeholder. */
  value: Value | '';
  /** Called with the newly selected value. */
  onValueChange: (value: Value) => void;
};

/**
 * Render the single-value Select state root. Pass `items` so the trigger shows the selected
 * item's label rather than its value.
 *
 * @param props - Selected value, change handler, and Base UI Select root properties.
 * @returns Select root.
 */
function Select<Value extends string>({ value, onValueChange, ...props }: SelectProps<Value>) {
  return (
    <SelectPrimitive.Root<Value>
      value={value === '' ? null : value}
      onValueChange={(next) => {
        if (next !== null) onValueChange(next);
      }}
      {...props}
    />
  );
}

/**
 * Render the selected value or placeholder.
 *
 * @param props - Base UI Select value properties and style overrides.
 * @returns Select value.
 */
function SelectValue({
  xstyle,
  ...props
}: Omit<React.ComponentProps<typeof SelectPrimitive.Value>, 'className'> & {
  xstyle?: stylex.StyleXStyles;
}) {
  return (
    <SelectPrimitive.Value
      data-slot="select-value"
      {...stylex.props(styles.selectValue, xstyle)}
      {...props}
    />
  );
}

/**
 * Render the keyboard-accessible Select trigger.
 *
 * @param props - Base UI Select trigger properties, control size, and style overrides for the
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
      <IconStyleProvider xstyle={[controlIconStyle, menuItemIconStyle]}>
        {children}
      </IconStyleProvider>
      <ChevronDownIcon
        {...stylex.props(styles.selectTriggerChevronDownIcon, iconXstyle)}
        aria-hidden="true"
      />
    </SelectPrimitive.Trigger>
  );
}

type SelectContentProps = Omit<React.ComponentProps<typeof SelectPrimitive.Popup>, 'className'> & {
  /** Alignment of the popup against the trigger. */
  align?: 'start' | 'center' | 'end';
  /** Extra styles for the scrolling option list. */
  listXstyle?: stylex.StyleXStyles;
  xstyle?: stylex.StyleXStyles;
};

/**
 * Render the Select options below the trigger in a portal.
 *
 * @param props - Base UI Select popup properties, alignment, and style overrides.
 * @returns Portaled Select options.
 */
function SelectContent({
  xstyle,
  children,
  align = 'start',
  listXstyle,
  ...props
}: SelectContentProps) {
  return (
    <SelectPrimitive.Portal {...stylex.props(popupLayerStyle)}>
      <SelectPrimitive.Positioner
        align={align}
        sideOffset={4}
        collisionPadding={10}
        collisionAvoidance={{ align: 'shift', fallbackAxisSide: 'none' }}
        alignItemWithTrigger={false}
        {...stylex.props(styles.selectPositioner)}
      >
        <RevealScope>
          <SelectPrimitive.Popup
            data-slot="select-content"
            {...stylex.props(styles.selectContent, xstyle)}
            {...props}
          >
            <SelectPrimitive.ScrollUpArrow
              data-slot="select-scroll-up-button"
              {...stylex.props(styles.selectScrollArrow, styles.selectScrollUpArrow)}
            >
              <ChevronUpIcon {...stylex.props(styles.selectItemCheckIcon)} />
            </SelectPrimitive.ScrollUpArrow>
            <SelectPrimitive.List
              data-slot="select-list"
              {...stylex.props(styles.selectViewport, listXstyle)}
            >
              {children}
            </SelectPrimitive.List>
            <SelectPrimitive.ScrollDownArrow
              data-slot="select-scroll-down-button"
              {...stylex.props(styles.selectScrollArrow, styles.selectScrollDownArrow)}
            >
              <ChevronDownIcon {...stylex.props(styles.selectItemCheckIcon)} />
            </SelectPrimitive.ScrollDownArrow>
          </SelectPrimitive.Popup>
        </RevealScope>
      </SelectPrimitive.Positioner>
    </SelectPrimitive.Portal>
  );
}

/**
 * Render one selectable option and its checked indicator.
 *
 * @param props - Base UI Select item properties, visible label, optional trailing detail, and
 * style overrides.
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
          <CheckIcon
            {...stylex.props(controlIconStyle, menuItemIconStyle, styles.selectItemCheckIcon)}
          />
        </SelectPrimitive.ItemIndicator>
      </span>
      <SelectPrimitive.ItemText
        {...stylex.props(detail === undefined && styles.selectItemLastSpan)}
      >
        {children}
      </SelectPrimitive.ItemText>
      {detail !== undefined && (
        <span {...stylex.props(styles.selectItemSpan, styles.selectItemLastSpan)}>
          <span {...stylex.props(styles.selectItemSpan2)}> · </span>
          {detail}
        </span>
      )}
    </SelectPrimitive.Item>
  );
}

export { Select, SelectContent, SelectItem, SelectTrigger, SelectValue };

const styles = stylex.create({
  selectViewport: {
    minHeight: 0,
    overflowY: 'auto',
    padding: '0.25rem',
    scrollMarginBlock: '0.25rem',
    scrollPaddingBlock: '1.5rem',
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
        '@media (hover: hover)': {
          default: colors.accent,
          ':is([data-popup-open])': colors.card,
        },
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
    pointerEvents: 'none',
    flexShrink: 0,
    width: '1rem',
    height: '1rem',
    color: colors.mutedForeground,
    opacity: '50%',
    transitionProperty: 'transform, translate, scale, rotate',
    transitionTimingFunction: 'cubic-bezier(0.215, 0.61, 0.355, 1)',
    transitionDuration: '220ms',
    rotate: {
      default: null,
      [stylex.when.ancestor('[data-popup-open]')]: '180deg',
    },
  },
  selectPositioner: {
    zIndex: 50,
  },
  selectValue: {
    display: 'flex',
    alignItems: 'center',
    gap: '0.5rem',
    overflow: 'hidden',
    WebkitBoxOrient: 'vertical',
    WebkitLineClamp: 1,
  },
  selectItemLastSpan: {
    display: 'flex',
    alignItems: 'center',
    gap: '0.5rem',
  },
  selectContent: {
    position: 'relative',
    display: 'flex',
    flexDirection: 'column',
    maxHeight: 'var(--available-height)',
    minWidth: 'max(8rem, var(--anchor-width))',
    transformOrigin: 'var(--transform-origin)',
    overflow: 'hidden',
    borderRadius: '12px',
    backgroundColor: colors.popover,
    color: colors.popoverForeground,
    boxShadow: shadows.overlay,
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
  selectScrollArrow: {
    insetInline: 0,
    zIndex: 1,
    display: 'flex',
    cursor: 'default',
    alignItems: 'center',
    justifyContent: 'center',
    paddingBlock: '0.25rem',
    backgroundColor: colors.popover,
  },
  selectScrollUpArrow: {
    top: 0,
  },
  selectScrollDownArrow: {
    bottom: 0,
  },
});
