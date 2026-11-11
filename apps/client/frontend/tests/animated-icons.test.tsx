/**
 * Animated icons stay plain lucide drawings until their control is first hovered or tapped.
 */

import type { PointerEvent } from 'react';
import { fireEvent, render, screen } from '@testing-library/react';
import * as stylex from '@stylexjs/stylex';
import { Circle } from 'lucide-react';
import { describe, expect, test } from 'vite-plus/test';

import {
  IconTriggerProvider,
  Search,
  useIconTrigger,
  type AppIcon,
} from '@/components/ui/animated-icons';
import { Button } from '@/components/ui/button';

const styles = stylex.create({ icon: { width: '1.25rem' } });

/** Atomic class of the icon's own width, which must survive the button's default icon styles. */
const ICON_CLASS = stylex.props(styles.icon).className ?? '';

/**
 * Read the icon drawn inside the labelled button.
 *
 * @returns Icon SVG element.
 */
function searchIcon(): SVGElement {
  const icon = screen.getByRole('button', { name: '搜索' }).querySelector('svg');
  expect(icon).not.toBeNull();
  return icon!;
}

/** Keep idle icons as the still lucide drawing. */
function rendersStillUntilTriggered(): void {
  render(
    <Button aria-label="搜索">
      <Search xstyle={styles.icon} aria-hidden="true" />
    </Button>,
  );
  expect(searchIcon()).toHaveClass('lucide-search', ICON_CLASS);
  expect(searchIcon()).toHaveAttribute('aria-hidden', 'true');
}

/** Swap in the animated drawing once a mouse enters the button, keeping its styling. */
function swapsInOnMouseHover(): void {
  render(
    <Button aria-label="搜索">
      <Search xstyle={styles.icon} aria-hidden="true" />
    </Button>,
  );
  fireEvent.pointerEnter(screen.getByRole('button', { name: '搜索' }), { pointerType: 'mouse' });
  expect(searchIcon()).not.toHaveClass('lucide-search');
  expect(searchIcon()).toHaveClass(ICON_CLASS);
  fireEvent.pointerLeave(screen.getByRole('button', { name: '搜索' }), { pointerType: 'mouse' });
  expect(searchIcon()).not.toHaveClass('lucide-search');
}

/** Treat a touch press, not a touch hover, as the trigger and keep the button's own handler. */
function swapsInOnTouchPress(): void {
  let pressCount = 0;
  render(
    <Button aria-label="搜索" onPointerDown={() => (pressCount += 1)}>
      <Search xstyle={styles.icon} aria-hidden="true" />
    </Button>,
  );
  const button = screen.getByRole('button', { name: '搜索' });
  fireEvent.pointerEnter(button, { pointerType: 'touch' });
  expect(searchIcon()).toHaveClass('lucide-search');
  fireEvent.pointerDown(button, { pointerType: 'touch' });
  expect(searchIcon()).not.toHaveClass('lucide-search');
  expect(pressCount).toBe(1);
}

/** Play right away when the icon asks for its own animation, even outside a control. */
function animatesOnItsOwnRequest(): void {
  const { container } = render(<Search animateOnView xstyle={styles.icon} />);
  expect(container.querySelector('svg')).not.toHaveClass('lucide-search');
}

/**
 * Render a navigation item that drives its icon through the trigger hook.
 *
 * @param props - Whether the item is the current one.
 * @returns Navigation item with an animated icon.
 */
function NavigationItem({ isCurrent }: { isCurrent: boolean }) {
  const { isTriggered, triggerHandlers } = useIconTrigger<PointerEvent<HTMLAnchorElement>>(
    {},
    isCurrent,
  );
  return (
    <IconTriggerProvider isTriggered={isTriggered}>
      <a href="#search" aria-label="检索" {...triggerHandlers}>
        <Search xstyle={styles.icon} aria-hidden="true" />
      </a>
    </IconTriggerProvider>
  );
}

/** Stay still for the item that is current on first render, then play when another becomes current. */
function playsWhenBecomingCurrent(): void {
  const { container, rerender } = render(<NavigationItem isCurrent />);
  expect(container.querySelector('svg')).toHaveClass('lucide-search');
  rerender(<NavigationItem isCurrent={false} />);
  expect(container.querySelector('svg')).toHaveClass('lucide-search');
  rerender(<NavigationItem isCurrent />);
  expect(container.querySelector('svg')).not.toHaveClass('lucide-search');
}

/** Keep raw lucide-react icons, which ignore `xstyle`, out of application icon slots. */
function rejectsRawLucideIcons(): void {
  // @ts-expect-error A raw lucide-react icon lacks the styled icon marker.
  const rawIcon: AppIcon = Circle;
  const styledIcon: AppIcon = Search;
  expect(styledIcon.canApplyXstyle).toBe(true);
  expect('canApplyXstyle' in rawIcon).toBe(false);
}

describe('animated icons', () => {
  test('reject raw lucide-react icons in icon slots', rejectsRawLucideIcons);
  test('render still until their control is triggered', rendersStillUntilTriggered);
  test('swap in on mouse hover', swapsInOnMouseHover);
  test('swap in on a touch press', swapsInOnTouchPress);
  test('animate on their own request', animatesOnItsOwnRequest);
  test('play when a navigation item becomes current', playsWhenBecomingCurrent);
});
