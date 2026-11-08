/**
 * Shared destructive confirmation pending and failure coverage.
 */

import { useState } from 'react';
import { act, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, test, vi } from 'vitest';

import { ConfirmDialog } from '@/components/ui/confirm-dialog';

type ConfirmHarnessProps = {
  action: () => Promise<void>;
};

/**
 * Render a caller-controlled asynchronous destructive action.
 *
 * @param props - Asynchronous action invoked only after confirmation.
 * @returns Confirmation trigger and dialog.
 */
function ConfirmHarness({ action }: ConfirmHarnessProps) {
  const [error, setError] = useState<string | null>(null);
  const [isOpen, setIsOpen] = useState(false);
  const [isPending, setIsPending] = useState(false);

  /**
   * Run the external action while the caller owns pending, error, and open state.
   */
  async function handleConfirm(): Promise<void> {
    setError(null);
    setIsPending(true);
    try {
      await action();
      setIsOpen(false);
    } catch (actionError) {
      setError(actionError instanceof Error ? actionError.message : '操作失败');
    } finally {
      setIsPending(false);
    }
  }

  return (
    <>
      <button type="button" onClick={() => setIsOpen(true)}>
        删除项目
      </button>
      <ConfirmDialog
        open={isOpen}
        onOpenChange={setIsOpen}
        title="删除项目？"
        description="此操作无法撤销。"
        actionLabel="确认删除"
        pendingLabel="删除中…"
        isPending={isPending}
        error={error}
        onConfirm={() => void handleConfirm()}
      />
    </>
  );
}

/**
 * Verify a pending failure cannot submit twice and remains visible for retry.
 */
async function keepsFailedPendingActionOpen(): Promise<void> {
  let rejectAction: ((reason: Error) => void) | undefined;
  const action = vi.fn(
    () =>
      new Promise<void>((_resolve, reject) => {
        rejectAction = reject;
      }),
  );
  const user = userEvent.setup();
  render(<ConfirmHarness action={action} />);

  await user.click(screen.getByRole('button', { name: '删除项目' }));
  const actionButton = screen.getByRole('button', { name: '确认删除' });
  expect(actionButton).toHaveClass('bg-destructive');
  await user.click(actionButton);

  const pendingButton = screen.getByRole('button', { name: '删除中…' });
  expect(pendingButton).toBeDisabled();
  await user.click(pendingButton);
  expect(action).toHaveBeenCalledTimes(1);

  await act(async () => rejectAction?.(new Error('删除失败')));

  expect(await screen.findByRole('alert')).toHaveTextContent('删除失败');
  expect(screen.getByRole('alertdialog', { name: '删除项目？' })).toBeInTheDocument();
  expect(screen.getByRole('button', { name: '确认删除' })).toBeEnabled();
}

describe('ConfirmDialog', () => {
  test('blocks duplicate pending actions and keeps failures open', keepsFailedPendingActionOpen);
});
