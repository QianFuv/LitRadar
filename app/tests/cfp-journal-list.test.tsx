/** Deferred CFP journal groups retain visited row identity and independent expansion state. */

import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, test, vi } from 'vitest';

import { CfpJournalList, groupCfpJournals } from '@/components/cfp/cfp-journal-list';
import type { CfpJournalSummary } from '@/lib/api';

/** Build an adapted or unadapted journal without network-owned state. */
function journal(
  catalogId: string,
  coverage: 'adapted' | 'unadapted',
  currentCount = 0,
): CfpJournalSummary {
  return {
    catalogId,
    title: catalogId,
    coverage,
    currentCount,
    noticeCount: currentCount,
    catalogAliases: [],
    titleAliases: [],
    allIssns: [],
    stateCounts: {},
    canRefresh: false,
    refreshStatus: 'unadapted',
  };
}

/** Verify unopened groups skip row construction while visited groups preserve their DOM. */
async function defersUnvisitedGroups(): Promise<void> {
  const groups = groupCfpJournals([
    journal('Current', 'adapted', 1),
    ...Array.from({ length: 100 }, (_, index) => journal(`Inactive ${index}`, 'adapted')),
    ...Array.from({ length: 100 }, (_, index) => journal(`Unadapted ${index}`, 'unadapted')),
  ]);
  const inactiveRows = vi.spyOn(groups[1].journals, 'map');
  const unadaptedRows = vi.spyOn(groups[2].journals, 'map');
  const onSelect = vi.fn();
  render(<CfpJournalList groups={groups} isSearching={false} onSelect={onSelect} />);
  expect(inactiveRows).not.toHaveBeenCalled();
  expect(unadaptedRows).not.toHaveBeenCalled();
  const inactivePanel = screen.getByLabelText('当前未征稿期刊');
  expect(within(inactivePanel).queryAllByRole('button', { hidden: true })).toHaveLength(0);
  const user = userEvent.setup();
  const toggle = screen.getByRole('button', { name: /^当前未征稿/ });
  await user.click(toggle);
  const firstRow = within(inactivePanel).getByTitle('Inactive 0');
  inactivePanel.scrollTop = 120;
  await user.click(firstRow);
  expect(onSelect).toHaveBeenCalledWith('Inactive 0');
  await user.click(toggle);
  expect(inactivePanel).toHaveAttribute('hidden');
  expect(inactivePanel).toContainElement(firstRow);
  await user.click(toggle);
  expect(screen.getByRole('region', { name: '当前未征稿期刊' })).toBe(inactivePanel);
  expect(within(inactivePanel).getByTitle('Inactive 0')).toBe(firstRow);
  expect(inactivePanel.scrollTop).toBe(120);
  expect(unadaptedRows).not.toHaveBeenCalled();
}

describe('CFP journal groups', () => {
  test('defers unvisited groups and retains visited rows on collapse', defersUnvisitedGroups);
});
