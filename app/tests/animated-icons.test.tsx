/**
 * Animated icons stay plain lucide drawings until their control is first hovered or tapped.
 */

import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, test } from 'vitest';

import { IconTriggerProvider, Search, useIconTrigger } from '@/components/ui/animated-icons';
import { Button } from '@/components/ui/button';

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
      <Search className="size-4" aria-hidden="true" />
    </Button>,
  );
  expect(searchIcon()).toHaveClass('lucide-search', 'size-4');
  expect(searchIcon()).toHaveAttribute('aria-hidden', 'true');
}

/** Swap in the animated drawing once a mouse enters the button, keeping its styling. */
function swapsInOnMouseHover(): void {
  render(
    <Button aria-label="搜索">
      <Search className="size-4" aria-hidden="true" />
    </Button>,
  );
  fireEvent.pointerEnter(screen.getByRole('button', { name: '搜索' }), { pointerType: 'mouse' });
  expect(searchIcon()).not.toHaveClass('lucide-search');
  expect(searchIcon()).toHaveClass('size-4');
  fireEvent.pointerLeave(screen.getByRole('button', { name: '搜索' }), { pointerType: 'mouse' });
  expect(searchIcon()).not.toHaveClass('lucide-search');
}

/** Treat a touch press, not a touch hover, as the trigger and keep the button's own handler. */
function swapsInOnTouchPress(): void {
  let pressCount = 0;
  render(
    <Button aria-label="搜索" onPointerDown={() => (pressCount += 1)}>
      <Search className="size-4" aria-hidden="true" />
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
  const { container } = render(<Search animateOnView className="size-4" />);
  expect(container.querySelector('svg')).not.toHaveClass('lucide-search');
}

/**
 * Render a navigation item that drives its icon through the trigger hook.
 *
 * @param props - Whether the item is the current one.
 * @returns Navigation item with an animated icon.
 */
function NavigationItem({ isCurrent }: { isCurrent: boolean }) {
  const { isTriggered, triggerHandlers } = useIconTrigger<HTMLAnchorElement>({}, isCurrent);
  return (
    <IconTriggerProvider isTriggered={isTriggered}>
      <a href="#search" aria-label="检索" {...triggerHandlers}>
        <Search className="size-5" aria-hidden="true" />
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

describe('animated icons', () => {
  test('render still until their control is triggered', rendersStillUntilTriggered);
  test('swap in on mouse hover', swapsInOnMouseHover);
  test('swap in on a touch press', swapsInOnTouchPress);
  test('animate on their own request', animatesOnItsOwnRequest);
  test('play when a navigation item becomes current', playsWhenBecomingCurrent);
});
