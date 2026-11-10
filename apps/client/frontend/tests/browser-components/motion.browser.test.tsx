/**
 * Real Chromium motion lifecycle and drawer focus coverage.
 */

import '@/src/globals.css';

import { act, render, screen, waitFor } from '@testing-library/react';
import { Circle } from 'lucide-react';
import { useRef, useState } from 'react';
import { page, userEvent } from 'vite-plus/test/browser';
import { describe, expect, test } from 'vite-plus/test';

import {
  SectionedDialogFrame,
  type SectionedDialogSectionDefinition,
} from '@/components/feature/sectioned-dialog';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
  DialogTrigger,
} from '@/components/ui/dialog';
import { MotionDiv, MotionPresence, MotionProvider, REVEAL_VARIANTS } from '@/components/ui/motion';

const SECTIONED_BROWSER_SECTIONS = [
  {
    description: '第一分类说明',
    icon: Circle,
    id: 'first',
    label: '第一分类',
  },
  {
    description: '第二分类说明',
    icon: Circle,
    id: 'second',
    label: '第二分类',
  },
] satisfies readonly [
  SectionedDialogSectionDefinition<'first' | 'second'>,
  ...SectionedDialogSectionDefinition<'first' | 'second'>[],
];

/**
 * Render one removable element.
 *
 * @returns Animated presence controls.
 */
function MotionLifecycleContent() {
  const [isVisible, setIsVisible] = useState(true);

  return (
    <main>
      <button type="button" onClick={() => setIsVisible((current) => !current)}>
        切换动效内容
      </button>
      <MotionPresence>
        {isVisible && (
          <MotionDiv
            key="motion-item"
            data-testid="motion-item"
            variants={REVEAL_VARIANTS}
            initial="hidden"
            animate="visible"
            exit="exit"
          >
            动效内容
          </MotionDiv>
        )}
      </MotionPresence>
    </main>
  );
}

/**
 * Wrap the lifecycle harness in the application motion provider.
 *
 * @returns Configured motion lifecycle harness.
 */
function MotionLifecycleHarness() {
  return (
    <MotionProvider>
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
 * Render a real sectioned dialog with controlled category and focus state.
 *
 * @returns Sectioned dialog trigger and lifecycle harness.
 */
function SectionedDialogHarness() {
  const [activeSection, setActiveSection] = useState<'first' | 'second'>('first');
  const [isOpen, setIsOpen] = useState(false);
  const returnFocusRef = useRef<HTMLButtonElement | null>(null);

  return (
    <>
      <button ref={returnFocusRef} type="button" onClick={() => setIsOpen(true)}>
        打开分区对话框
      </button>
      <SectionedDialogFrame
        activeSection={activeSection}
        centerIcon={Circle}
        centerSubtitle="浏览器生命周期测试"
        centerTitle="分区测试"
        contentLabelSuffix="测试内容"
        dialogDescription="验证分类标题切换与关闭焦点。"
        navigationLabel="测试分类"
        open={isOpen}
        onOpenChange={setIsOpen}
        onSelectSection={setActiveSection}
        onSessionClosed={() => {}}
        returnFocusRef={returnFocusRef}
        sections={SECTIONED_BROWSER_SECTIONS}
      >
        <div data-testid="sectioned-active-content">{activeSection}</div>
      </SectionedDialogFrame>
    </>
  );
}

/**
 * Verify exits retain their node until their animation finishes.
 */
async function preservesExitLifecycles(): Promise<void> {
  render(<MotionLifecycleHarness />);
  const normalTrigger = page.getByRole('button', { name: '切换动效内容' });

  await act(async () => normalTrigger.click());
  expect(screen.getByTestId('motion-item')).toBeInTheDocument();

  await act(async () => normalTrigger.click());
  await waitFor(() => expect(screen.getAllByTestId('motion-item')).toHaveLength(1));

  await act(async () => normalTrigger.click());
  expect(screen.getByTestId('motion-item')).toBeInTheDocument();
  await waitFor(() => expect(screen.queryByTestId('motion-item')).toBeNull());
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
  expect(dialog).toHaveAttribute('data-side', 'left');
  expect(window.getComputedStyle(dialog).animationName).toBe('none');

  await act(async () => userEvent.keyboard('{Escape}'));
  await waitFor(() => expect(screen.queryByRole('dialog', { name: '移动侧栏' })).toBeNull());
  await expect.element(trigger).toHaveFocus();
}

/** Verify section headers retain a real exit and closing restores the persistent trigger. */
async function transitionsSectionHeaderAndRestoresFocus(): Promise<void> {
  render(
    <MotionProvider>
      <SectionedDialogHarness />
    </MotionProvider>,
  );
  const trigger = page.getByRole('button', { name: '打开分区对话框' });

  await act(async () => trigger.click());
  const dialog = screen.getByRole('dialog', { name: '分区测试' });
  expect(dialog.querySelector('[data-mobile-overflow-cue="true"]')).not.toBeNull();
  expect(dialog.querySelector('[data-motion-section-header="first"]')).not.toBeNull();

  await act(async () => page.getByRole('button', { name: '第二分类' }).first().click());
  expect(dialog.querySelector('[data-motion-section-header="first"]')).not.toBeNull();
  await waitFor(() =>
    expect(dialog.querySelector('[data-motion-section-header="second"]')).not.toBeNull(),
  );
  expect(dialog.querySelector('[data-motion-section-header="first"]')).toBeNull();
  expect(screen.getByTestId('sectioned-active-content')).toHaveTextContent('second');
  expect(
    screen
      .getAllByRole('button', { name: '第二分类' })
      .every((button) => button.getAttribute('data-section-active') === 'true'),
  ).toBe(true);

  await act(async () => userEvent.keyboard('{Escape}'));
  await waitFor(() => expect(screen.queryByRole('dialog', { name: '分区测试' })).toBeNull());
  await expect.element(trigger).toHaveFocus();
}

describe('application motion in Chromium', () => {
  test('preserves exit lifecycles', preservesExitLifecycles);
  test('restores drawer focus after the exit transition', restoresDrawerFocusAfterExit);
  test(
    'transitions section headers and restores dialog focus',
    transitionsSectionHeaderAndRestoresFocus,
  );
});
