/**
 * Real Chromium pointer, Escape, and focus-return coverage for destructive dialogs.
 */

import { act, render } from '@testing-library/react';
import { useState } from 'react';
import { page, userEvent } from 'vitest/browser';
import { describe, expect, test } from 'vitest';

import { ConfirmDialog } from '@/components/ui/confirm-dialog';

/**
 * Render a controlled confirmation dialog and expose the last real pointer type.
 *
 * @returns Confirmation trigger, pointer probe, and production dialog.
 */
function BrowserConfirmHarness() {
  const [isOpen, setIsOpen] = useState(false);
  const [pointerType, setPointerType] = useState('none');

  return (
    <>
      <button
        type="button"
        onPointerDown={(event) => setPointerType(event.pointerType)}
        onClick={() => setIsOpen(true)}
      >
        删除项目
      </button>
      <output aria-label="最后指针类型">{pointerType}</output>
      <ConfirmDialog
        open={isOpen}
        onOpenChange={setIsOpen}
        title="删除项目？"
        description="此操作无法撤销。"
        actionLabel="确认删除"
        onConfirm={() => setIsOpen(false)}
      />
    </>
  );
}

/**
 * Verify Playwright pointer input opens the dialog and both close paths restore focus.
 */
async function restoresFocusAfterRealBrowserInput(): Promise<void> {
  render(<BrowserConfirmHarness />);
  const trigger = page.getByRole('button', { name: '删除项目' });

  await act(async () => trigger.click());
  await expect.element(page.getByRole('alertdialog', { name: '删除项目？' })).toBeVisible();
  await expect.element(page.getByLabelText('最后指针类型')).toHaveTextContent('mouse');

  await act(async () => page.getByRole('button', { name: '取消' }).click());
  await expect.element(page.getByRole('alertdialog')).not.toBeInTheDocument();
  await expect.element(trigger).toHaveFocus();

  await act(async () => trigger.click());
  await expect.element(page.getByRole('alertdialog', { name: '删除项目？' })).toBeVisible();
  await act(async () => userEvent.keyboard('{Escape}'));
  await expect.element(page.getByRole('alertdialog')).not.toBeInTheDocument();
  await expect.element(trigger).toHaveFocus();
}

describe('ConfirmDialog in Chromium', () => {
  test('restores focus after real pointer and Escape input', restoresFocusAfterRealBrowserInput);
});
