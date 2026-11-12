/**
 * Sliding hover highlight for groups of sibling controls outside popups.
 */

import * as React from 'react';
import { Highlight } from '@animate-ui/react/primitives/sidebar';

import * as stylex from '@stylexjs/stylex';

/**
 * Props that enroll a control in the nearest {@link HoverHighlightGroup}. The group finds its
 * controls by this attribute, the contract of Animate UI's sidebar highlight.
 */
export const hoverHighlightItemProps = { 'data-sidebar': 'menu-button' } as const;

/**
 * {@link hoverHighlightItemProps} for a control that is also the child of an Animate UI selection
 * `HighlightItem` (the rail's current link, the favorites folder tabs). `HighlightItem` stamps
 * `data-disabled="false"` on its child unless the child sets the attribute itself, and the hover
 * highlight treats any `data-disabled` as disabled; `null` keeps the attribute off.
 */
export const selectionHighlightChildProps = {
  ...hoverHighlightItemProps,
  'data-disabled': null,
} as const;

type HoverHighlightGroupProps = React.ComponentProps<'div'> & {
  xstyle?: stylex.StyleXStyles;
};

/**
 * Render a group whose enrolled controls share one sliding hover surface: the same Animate UI
 * item highlight that popup lists (Select, Combobox, DropdownMenu) use. Each control keeps
 * painting its own `:hover` background in CSS; the highlight reads that surface, clears it, and
 * glides one copy of it between controls with the shared highlight spring, so color and radius
 * follow each control.
 *
 * The copy is placed in the control's parent at `z-index: -1` inside the group's isolated
 * stacking context, beneath every label, which holds only while those parents do not create
 * stacking contexts of their own. The group is positioned so the copy is laid out, scrolled, and
 * clipped with the controls even when it is a scroll container. Reduced motion turns the glide
 * off, and the CSS `(hover: hover)` guard keeps touch taps from leaving a surface behind.
 *
 * Pass it as the `render` element of a Base UI part (a tab list, a radio group) to make that part
 * the group; the part's own `className` and `style` are kept when no `xstyle` is given.
 *
 * @param props - Group element properties, children with {@link hoverHighlightItemProps}, and
 * style overrides; the group renders as a `div`.
 * @returns Highlight group.
 */
export function HoverHighlightGroup({
  xstyle,
  className,
  style,
  ...props
}: HoverHighlightGroupProps) {
  const styled = xstyle === undefined ? { className, style } : stylex.props(xstyle);
  return (
    <Highlight
      {...props}
      className={styled.className}
      style={{ position: 'relative', ...styled.style }}
    />
  );
}
