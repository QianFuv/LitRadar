/**
 * Real Chromium Clipboard API and settings feedback coverage.
 */

import { act, render } from '@testing-library/react';
import { page } from 'vitest/browser';
import { describe, expect, test } from 'vitest';

import { useSettingsCopy } from '@/components/settings/use-settings-copy';

/**
 * Render the production settings copy hook through an accessible action and feedback channel.
 *
 * @returns Copy action and scoped feedback.
 */
function BrowserClipboardHarness() {
  const { copyFeedback, handleCopy } = useSettingsCopy();

  return (
    <>
      <button
        type="button"
        onClick={() => void handleCopy('browser-invite-code', '邀请码已复制。', 'invite')}
      >
        复制邀请码
      </button>
      {copyFeedback && (
        <p role={copyFeedback.tone === 'error' ? 'alert' : 'status'}>{copyFeedback.message}</p>
      )}
    </>
  );
}

/**
 * Verify the real Clipboard API succeeds and the unavailable branch remains actionable.
 */
async function reportsClipboardSuccessAndUnavailability(): Promise<void> {
  render(<BrowserClipboardHarness />);
  const copyButton = page.getByRole('button', { name: '复制邀请码' });

  await act(async () => copyButton.click());
  await expect.element(page.getByRole('status')).toHaveTextContent('邀请码已复制。');
  expect(await navigator.clipboard.readText()).toBe('browser-invite-code');

  Object.defineProperty(navigator, 'clipboard', {
    configurable: true,
    value: undefined,
  });
  try {
    await act(async () => copyButton.click());
    await expect
      .element(page.getByRole('alert'))
      .toHaveTextContent('复制失败，请手动选择文本复制。');
  } finally {
    Reflect.deleteProperty(navigator, 'clipboard');
  }
}

describe('settings clipboard in Chromium', () => {
  test(
    'uses the real Clipboard API and reports unavailable access',
    reportsClipboardSuccessAndUnavailability,
  );
});
