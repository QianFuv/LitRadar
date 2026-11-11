import { copyTextToClipboard } from '@/lib/clipboard';
import { useTransientState } from '@/lib/use-transient-state';

/** Time copy feedback stays visible after the last copy. */
const COPY_FEEDBACK_MS = 3000;

export type SettingsCopyScope = 'cnkiQr' | 'invite' | 'token';
export type SettingsCopyFeedback = {
  message: string;
  scope: SettingsCopyScope;
  tone: 'error' | 'success';
};

/**
 * Preserve one shared copy-feedback channel across settings cards.
 *
 * @returns Current feedback and a scoped copy action.
 */
export function useSettingsCopy() {
  const [copyFeedback, setCopyFeedback] = useTransientState<SettingsCopyFeedback | null>(
    null,
    COPY_FEEDBACK_MS,
  );

  /**
   * Copy a settings value and publish scoped feedback.
   *
   * @param value - Text to copy.
   * @param successMessage - Message shown after a successful copy.
   * @param scope - Settings card that owns the feedback.
   */
  const handleCopy = async (
    value: string,
    successMessage: string,
    scope: SettingsCopyScope,
  ): Promise<void> => {
    try {
      await copyTextToClipboard(value);
      setCopyFeedback({ message: successMessage, scope, tone: 'success' });
    } catch {
      setCopyFeedback({ message: '复制失败，请手动选择文本复制。', scope, tone: 'error' });
    }
  };

  return { copyFeedback, handleCopy };
}
