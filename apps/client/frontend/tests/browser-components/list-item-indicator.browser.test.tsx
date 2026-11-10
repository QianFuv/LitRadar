/**
 * Real Chromium coverage for option check marks in multi-value lists.
 */

import '@/src/globals.css';

import { render, screen, waitFor } from '@testing-library/react';
import { userEvent } from 'vite-plus/test/browser';
import { describe, expect, test } from 'vite-plus/test';

import {
  Combobox,
  ComboboxContent,
  ComboboxItem,
  ComboboxList,
  ComboboxTrigger,
} from '@/components/ui/combobox';
import {
  MultiSelect,
  MultiSelectContent,
  MultiSelectItem,
  MultiSelectList,
  MultiSelectTrigger,
} from '@/components/ui/multi-select';

const OPTIONS = ['alpha', 'beta', 'gamma'] as const;

/** Time for indicator and icon animations to settle, in milliseconds. */
const SETTLE_MS = 1200;

/**
 * Report whether an option's check mark is painted.
 *
 * @param option - Option element.
 * @returns Whether the check mark is visible.
 */
function isCheckPainted(option: HTMLElement): boolean {
  const path = option.querySelector('svg path');
  if (!path) return false;
  const svg = path.closest('svg') as SVGElement;
  const isTransparent = [path, svg, svg.parentElement as Element].some(
    (element) => Number(getComputedStyle(element).opacity) === 0,
  );
  const dash = path.getAttribute('stroke-dasharray');
  const isUndrawn = dash !== null && Number.parseFloat(dash) === 0;
  return !isTransparent && !isUndrawn;
}

/**
 * Wait for the list animations, then read which options show a check mark.
 *
 * @returns Option labels whose check mark is painted.
 */
async function readCheckedOptions(): Promise<string[]> {
  await new Promise((resolve) => setTimeout(resolve, SETTLE_MS));
  return screen
    .getAllByRole('option')
    .filter(isCheckPainted)
    .map((option) => option.textContent ?? '');
}

describe('list item check marks', () => {
  test('multi-select marks only the selected option', async () => {
    render(
      <MultiSelect multiple defaultValue={['beta']}>
        <MultiSelectTrigger>open</MultiSelectTrigger>
        <MultiSelectContent>
          <MultiSelectList>
            {OPTIONS.map((option) => (
              <MultiSelectItem key={option} value={option}>
                {option}
              </MultiSelectItem>
            ))}
          </MultiSelectList>
        </MultiSelectContent>
      </MultiSelect>,
    );

    await userEvent.click(screen.getByRole('combobox'));
    await waitFor(() => expect(screen.getAllByRole('option')).toHaveLength(OPTIONS.length));

    expect(await readCheckedOptions()).toEqual(['beta']);
  });

  test('combobox marks only the selected option', async () => {
    render(
      <Combobox multiple items={[...OPTIONS]} defaultValue={['beta']}>
        <ComboboxTrigger>open</ComboboxTrigger>
        <ComboboxContent>
          <ComboboxList>
            {(option: string) => (
              <ComboboxItem key={option} value={option}>
                {option}
              </ComboboxItem>
            )}
          </ComboboxList>
        </ComboboxContent>
      </Combobox>,
    );

    await userEvent.click(screen.getByText('open'));
    await waitFor(() => expect(screen.getAllByRole('option')).toHaveLength(OPTIONS.length));

    expect(await readCheckedOptions()).toEqual(['beta']);
  });
});
