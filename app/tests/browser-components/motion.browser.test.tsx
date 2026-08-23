/**
 * Real Chromium motion lifecycle, reduced-mode, and drawer focus coverage.
 */

import '@/app/globals.css';

import { act, render, screen, waitFor } from '@testing-library/react';
import { useState } from 'react';
import { page, userEvent } from 'vitest/browser';
import { describe, expect, test } from 'vitest';

import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
  DialogTrigger,
} from '@/components/ui/dialog';
import {
  FADE_UP_VARIANTS,
  MOTION_DURATION_SECONDS,
  MotionDiv,
  MotionPresence,
  MotionProvider,
  useMotionTransition,
} from '@/components/ui/motion';

type MotionLifecycleHarnessProps = {
  reducedMotion: 'always' | 'never';
};

/**
 * Render one removable element and expose its resolved transition duration.
 *
 * @returns Animated presence controls and duration probe.
 */
function MotionLifecycleContent() {
  const [isVisible, setIsVisible] = useState(true);
  const transition = useMotionTransition(MOTION_DURATION_SECONDS.fast, 'exit');

  return (
    <main>
      <button type="button" onClick={() => setIsVisible((current) => !current)}>
        切换动效内容
      </button>
      <output aria-label="动效时长">{String(transition.duration)}</output>
      <MotionPresence>
        {isVisible && (
          <MotionDiv
            key="motion-item"
            data-testid="motion-item"
            variants={FADE_UP_VARIANTS}
            initial="hidden"
            animate="visible"
            exit="exit"
            transition={transition}
          >
            动效内容
          </MotionDiv>
        )}
      </MotionPresence>
    </main>
  );
}

/**
 * Apply one explicit motion preference around the lifecycle harness.
 *
 * @param props - Deterministic reduced-motion preference.
 * @returns Configured motion lifecycle harness.
 */
function MotionLifecycleHarness({ reducedMotion }: MotionLifecycleHarnessProps) {
  return (
    <MotionProvider reducedMotion={reducedMotion}>
      <MotionLifecycleContent />
    </MotionProvider>
  );
}

/**
 * Render a mobile drawer using the production placement contract.
 *
 * @returns Dialog trigger and left-placed content.
 */
function DrawerFocusHarness() {
  return (
    <Dialog>
      <DialogTrigger asChild>
        <button type="button">打开移动侧栏</button>
      </DialogTrigger>
      <DialogContent placement="left">
        <DialogTitle>移动侧栏</DialogTitle>
        <DialogDescription>测试关闭后的焦点归还。</DialogDescription>
        <button type="button">侧栏操作</button>
      </DialogContent>
    </Dialog>
  );
}

/**
 * Verify normal exits retain their node and reduced motion removes the delay.
 */
async function respectsPresenceAndReducedMotion(): Promise<void> {
  const normalRender = render(<MotionLifecycleHarness reducedMotion="never" />);
  const normalTrigger = page.getByRole('button', { name: '切换动效内容' });

  await act(async () => normalTrigger.click());
  expect(screen.getByTestId('motion-item')).toBeInTheDocument();

  await act(async () => normalTrigger.click());
  await waitFor(() => expect(screen.getAllByTestId('motion-item')).toHaveLength(1));

  await act(async () => normalTrigger.click());
  expect(screen.getByTestId('motion-item')).toBeInTheDocument();
  await waitFor(() => expect(screen.queryByTestId('motion-item')).toBeNull());

  normalRender.unmount();
  render(<MotionLifecycleHarness reducedMotion="always" />);
  await expect.element(page.getByLabelText('动效时长')).toHaveTextContent('0');
  await act(async () => page.getByRole('button', { name: '切换动效内容' }).click());
  expect(screen.queryByTestId('motion-item')).toBeNull();
}

/**
 * Verify the left drawer closes after Escape and restores its persistent trigger focus.
 */
async function restoresDrawerFocusAfterExit(): Promise<void> {
  render(<DrawerFocusHarness />);
  const trigger = page.getByRole('button', { name: '打开移动侧栏' });

  await act(async () => trigger.click());
  const dialog = screen.getByRole('dialog', { name: '移动侧栏' });
  expect(dialog).toHaveAttribute('data-motion-placement', 'left');
  expect(dialog).toHaveClass('motion-drawer');
  expect(window.getComputedStyle(dialog).animationName).toBe('motion-drawer-enter');

  await act(async () => userEvent.keyboard('{Escape}'));
  await waitFor(() => expect(screen.queryByRole('dialog', { name: '移动侧栏' })).toBeNull());
  await expect.element(trigger).toHaveFocus();
}

describe('application motion in Chromium', () => {
  test(
    'preserves exit lifecycles and removes reduced-motion delay',
    respectsPresenceAndReducedMotion,
  );
  test('restores drawer focus after the exit transition', restoresDrawerFocusAfterExit);
});
