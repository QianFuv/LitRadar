/**
 * Real Chromium keyboard coverage for the shared Base UI overlays and composite controls: focus
 * management, Escape dismissal, and arrow-key navigation.
 */

import '@/src/globals.css';

import { act, render } from '@testing-library/react';
import { useState } from 'react';
import { page, userEvent } from 'vite-plus/test/browser';
import { describe, expect, test } from 'vite-plus/test';

import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
  DialogTrigger,
} from '@/components/ui/dialog';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu';
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover';
import { RadioGroup, RadioGroupItem } from '@/components/ui/radio-group';
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select';
import { ToggleGroup, ToggleGroupItem } from '@/components/ui/toggle-group';

const FIELD_ITEMS = [
  { value: 'title', label: '标题' },
  { value: 'abstract', label: '摘要' },
  { value: 'author', label: '作者' },
] as const;

type FieldValue = (typeof FIELD_ITEMS)[number]['value'];

const PERIODS = ['7 天', '30 天', '90 天'] as const;

/**
 * Press keys with real Chromium keyboard input.
 *
 * @param keys - `userEvent.keyboard` key sequence.
 */
async function press(keys: string): Promise<void> {
  await act(async () => userEvent.keyboard(keys));
}

/**
 * Report whether keyboard focus is inside an element.
 *
 * @param container - Element that should contain the focused element.
 * @returns Whether the active element is the container or one of its descendants.
 */
function hasFocusWithin(container: Element): boolean {
  return container.contains(document.activeElement);
}

/** Render a dialog with a text field next to its trigger. */
function DialogHarness() {
  return (
    <Dialog>
      <DialogTrigger>打开对话框</DialogTrigger>
      <DialogContent>
        <DialogTitle>键盘对话框</DialogTitle>
        <DialogDescription>焦点留在对话框内。</DialogDescription>
        <input aria-label="名称" />
      </DialogContent>
    </Dialog>
  );
}

/** Verify a keyboard-opened dialog traps Tab focus and Escape returns focus to its trigger. */
async function trapsDialogFocus(): Promise<void> {
  render(<DialogHarness />);
  const trigger = page.getByRole('button', { name: '打开对话框' });

  await press('{Tab}');
  await expect.element(trigger).toHaveFocus();
  await press('{Enter}');
  const dialog = page.getByRole('dialog', { name: '键盘对话框' });
  await expect.element(dialog).toBeVisible();
  await expect.poll(() => hasFocusWithin(dialog.element())).toBe(true);

  for (const key of ['{Tab}', '{Tab}', '{Tab}', '{Shift>}{Tab}{/Shift}']) {
    await press(key);
    expect(hasFocusWithin(dialog.element())).toBe(true);
  }

  await press('{Escape}');
  await expect.element(page.getByRole('dialog')).not.toBeInTheDocument();
  await expect.element(trigger).toHaveFocus();
}

/** Render a menu whose activated item is shown next to it. */
function MenuHarness() {
  const [lastAction, setLastAction] = useState('none');
  return (
    <>
      <DropdownMenu>
        <DropdownMenuTrigger>更多操作</DropdownMenuTrigger>
        <DropdownMenuContent>
          {['重命名', '复制', '删除'].map((label) => (
            <DropdownMenuItem key={label} onClick={() => setLastAction(label)}>
              {label}
            </DropdownMenuItem>
          ))}
        </DropdownMenuContent>
      </DropdownMenu>
      <output aria-label="最后操作">{lastAction}</output>
    </>
  );
}

/** Verify arrow keys move through menu items, Enter activates one, and Escape restores focus. */
async function navigatesMenuWithArrowKeys(): Promise<void> {
  render(<MenuHarness />);
  const trigger = page.getByRole('button', { name: '更多操作' });

  await press('{Tab}');
  await press('{Enter}');
  await expect.element(page.getByRole('menu')).toBeVisible();
  await expect.element(page.getByRole('menuitem', { name: '重命名' })).toHaveFocus();
  await press('{ArrowDown}');
  await expect.element(page.getByRole('menuitem', { name: '复制' })).toHaveFocus();
  await press('{ArrowDown}');
  await expect.element(page.getByRole('menuitem', { name: '删除' })).toHaveFocus();
  await press('{ArrowUp}');
  await expect.element(page.getByRole('menuitem', { name: '复制' })).toHaveFocus();
  await press('{Enter}');
  await expect.element(page.getByLabelText('最后操作')).toHaveTextContent('复制');
  await expect.element(page.getByRole('menu')).not.toBeInTheDocument();
  await expect.element(trigger).toHaveFocus();

  await press('{ArrowDown}');
  await expect.element(page.getByRole('menu')).toBeVisible();
  await press('{Escape}');
  await expect.element(page.getByRole('menu')).not.toBeInTheDocument();
  await expect.element(trigger).toHaveFocus();
  await expect.element(page.getByLabelText('最后操作')).toHaveTextContent('复制');
}

/** Render a controlled select. */
function SelectHarness() {
  const [field, setField] = useState<FieldValue>('title');
  return (
    <Select value={field} items={FIELD_ITEMS} onValueChange={setField}>
      <SelectTrigger aria-label="检索字段">
        <SelectValue />
      </SelectTrigger>
      <SelectContent>
        {FIELD_ITEMS.map((item) => (
          <SelectItem key={item.value} value={item.value}>
            {item.label}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  );
}

/** Verify a select opens from the keyboard, follows arrow keys, and commits with Enter. */
async function choosesSelectOptionWithKeyboard(): Promise<void> {
  render(<SelectHarness />);
  const trigger = page.getByRole('combobox', { name: '检索字段' });

  await press('{Tab}');
  await expect.element(trigger).toHaveFocus();
  await press('{ArrowDown}');
  await expect.element(page.getByRole('listbox')).toBeVisible();
  await expect.element(page.getByRole('option', { name: '标题' })).toHaveFocus();
  await press('{ArrowDown}');
  await expect.element(page.getByRole('option', { name: '摘要' })).toHaveFocus();
  await press('{ArrowDown}');
  await expect.element(page.getByRole('option', { name: '作者' })).toHaveFocus();
  await press('{Enter}');
  await expect.element(page.getByRole('listbox')).not.toBeInTheDocument();
  await expect.element(trigger).toHaveTextContent('作者');
  await expect.element(trigger).toHaveFocus();

  await press('{ArrowDown}');
  await expect.element(page.getByRole('listbox')).toBeVisible();
  await press('{ArrowUp}');
  await press('{Escape}');
  await expect.element(page.getByRole('listbox')).not.toBeInTheDocument();
  await expect.element(trigger).toHaveTextContent('作者');
  await expect.element(trigger).toHaveFocus();
}

/** Render a select inside a dialog. */
function NestedSelectHarness() {
  return (
    <Dialog>
      <DialogTrigger>打开筛选对话框</DialogTrigger>
      <DialogContent>
        <DialogTitle>筛选对话框</DialogTitle>
        <DialogDescription>内层选择框先关闭。</DialogDescription>
        <SelectHarness />
      </DialogContent>
    </Dialog>
  );
}

/** Verify Escape closes only the innermost layer: the select first, then its dialog. */
async function closesInnermostLayerWithEscape(): Promise<void> {
  render(<NestedSelectHarness />);
  const dialogTrigger = page.getByRole('button', { name: '打开筛选对话框' });

  await press('{Tab}');
  await press('{Enter}');
  const dialog = page.getByRole('dialog', { name: '筛选对话框' });
  await expect.element(dialog).toBeVisible();
  const selectTrigger = page.getByRole('combobox', { name: '检索字段' });
  await act(async () => selectTrigger.element().focus());
  await press('{ArrowDown}');
  await expect.element(page.getByRole('listbox')).toBeVisible();

  await press('{Escape}');
  await expect.element(page.getByRole('listbox')).not.toBeInTheDocument();
  await expect.element(dialog).toBeVisible();
  await expect.element(selectTrigger).toHaveFocus();

  await press('{Escape}');
  await expect.element(page.getByRole('dialog')).not.toBeInTheDocument();
  await expect.element(dialogTrigger).toHaveFocus();
}

/** Render a popover containing one action. */
function PopoverHarness() {
  return (
    <Popover>
      <PopoverTrigger>筛选</PopoverTrigger>
      <PopoverContent aria-label="筛选面板">
        <button type="button">清空筛选</button>
      </PopoverContent>
    </Popover>
  );
}

/** Verify a keyboard-opened popover takes focus and Escape returns it to the trigger. */
async function dismissesPopoverWithEscape(): Promise<void> {
  render(<PopoverHarness />);
  const trigger = page.getByRole('button', { name: '筛选', exact: true });

  await press('{Tab}');
  await press('{Enter}');
  const popover = page.getByRole('dialog', { name: '筛选面板' });
  await expect.element(popover).toBeVisible();
  await expect.poll(() => hasFocusWithin(popover.element())).toBe(true);
  await press('{Escape}');
  await expect.element(page.getByRole('dialog')).not.toBeInTheDocument();
  await expect.element(trigger).toHaveFocus();
}

/** Render a segmented period switch and a radio group. */
function CompositeHarness() {
  const [period, setPeriod] = useState<string>(PERIODS[0]);
  const [method, setMethod] = useState('folder');
  return (
    <>
      <ToggleGroup aria-label="统计周期" value={period} onValueChange={setPeriod}>
        {PERIODS.map((label) => (
          <ToggleGroupItem key={label} value={label}>
            {label}
          </ToggleGroupItem>
        ))}
      </ToggleGroup>
      <RadioGroup aria-label="推送方式" value={method} onValueChange={setMethod}>
        <RadioGroupItem value="folder" aria-label="追踪文件夹推送" />
        <RadioGroupItem value="pushplus" aria-label="PushPlus 外部推送" />
      </RadioGroup>
    </>
  );
}

/** Verify arrow keys rove focus in a toggle group and move the checked radio in a radio group. */
async function rovesCompositeControlsWithArrowKeys(): Promise<void> {
  render(<CompositeHarness />);
  const firstPeriod = page.getByRole('button', { name: '7 天' });
  const secondPeriod = page.getByRole('button', { name: '30 天' });

  await press('{Tab}');
  await expect.element(firstPeriod).toHaveFocus();
  await press('{ArrowRight}');
  await expect.element(secondPeriod).toHaveFocus();
  await expect.element(firstPeriod).toHaveAttribute('aria-pressed', 'true');
  await press(' ');
  await expect.element(secondPeriod).toHaveAttribute('aria-pressed', 'true');
  await expect.element(firstPeriod).toHaveAttribute('aria-pressed', 'false');

  await press('{Tab}');
  const folderRadio = page.getByRole('radio', { name: '追踪文件夹推送' });
  const pushPlusRadio = page.getByRole('radio', { name: 'PushPlus 外部推送' });
  await expect.element(folderRadio).toHaveFocus();
  await press('{ArrowDown}');
  await expect.element(pushPlusRadio).toHaveFocus();
  await expect.element(pushPlusRadio).toHaveAttribute('aria-checked', 'true');
  await expect.element(folderRadio).toHaveAttribute('aria-checked', 'false');
}

describe('keyboard access in Chromium', () => {
  test('traps dialog focus and closes with Escape', trapsDialogFocus);
  test('navigates menu items with arrow keys', navigatesMenuWithArrowKeys);
  test('chooses select options with the keyboard', choosesSelectOptionWithKeyboard);
  test('dismisses popovers with Escape', dismissesPopoverWithEscape);
  test('closes the innermost layer with Escape', closesInnermostLayerWithEscape);
  test('roves composite controls with arrow keys', rovesCompositeControlsWithArrowKeys);
});
