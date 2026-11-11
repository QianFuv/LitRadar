/**
 * State that falls back to an idle value a fixed time after its last change.
 */

import { useDebouncedCallback } from '@tanstack/react-pacer';
import { useState } from 'react';

/**
 * Hold a value that returns to its idle value once `durationMs` passes without a newer value.
 *
 * Each new value restarts the countdown, so a quick second change is never cleared by the
 * first one's timer, and the pending reset is cancelled on unmount.
 *
 * @param idleValue - Value held before the first change and after each reset.
 * @param durationMs - Time a value stays visible after its last change.
 * @returns Current value and a setter that starts or restarts the countdown.
 */
export function useTransientState<T>(idleValue: T, durationMs: number): [T, (value: T) => void] {
  const [value, setValue] = useState<T>(idleValue);
  const scheduleReset = useDebouncedCallback(() => setValue(idleValue), { wait: durationMs });

  /**
   * Show a value and restart the reset countdown.
   *
   * @param nextValue - Value to show.
   */
  function showValue(nextValue: T): void {
    setValue(nextValue);
    scheduleReset();
  }

  return [value, showValue];
}
