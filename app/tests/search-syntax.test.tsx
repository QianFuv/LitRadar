/**
 * Advanced search: syntax detection, rewrites, the visual builder, and the search-bar assistant.
 */

import { useQueryState, parseAsString } from 'nuqs';
import { NuqsTestingAdapter } from 'nuqs/adapters/testing';
import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, test } from 'vitest';

import { SearchBar } from '@/components/feature/search-bar';
import {
  buildConditionExpression,
  compileSearchConditions,
  createSearchCondition,
} from '@/lib/search-builder';
import {
  buildRewriteSuggestions,
  describeSearchError,
  getSearchSyntaxMode,
  isAdvancedSearchQuery,
} from '@/lib/search-syntax';
import { renderWithQuery } from '@/tests/render';

/**
 * Expose one query parameter for assertions.
 *
 * @param props - Query parameter name and test id.
 * @returns Query value output.
 */
function QueryProbe({ parameter, testId }: { parameter: string; testId: string }) {
  const [value] = useQueryState(parameter, parseAsString);
  return <output data-testid={testId}>{value ?? ''}</output>;
}

/**
 * Render the article search bar with its assistant and a query probe.
 *
 * @param searchParams - Initial URL search string.
 */
function renderAssistedSearchBar(searchParams = ''): void {
  renderWithQuery(
    <NuqsTestingAdapter searchParams={searchParams} hasMemory>
      <SearchBar hasSyntaxAssist />
      <QueryProbe parameter="q" testId="query-value" />
    </NuqsTestingAdapter>,
  );
}

/**
 * Choose a Radix select option from the keyboard, which jsdom supports without pointer capture.
 *
 * @param user - User-event instance.
 * @param trigger - Select trigger.
 * @param label - Option label to choose.
 */
async function chooseOption(
  user: ReturnType<typeof userEvent.setup>,
  trigger: HTMLElement,
  label: string,
): Promise<void> {
  trigger.focus();
  await user.keyboard('{Enter}');
  await screen.findByRole('option', { name: label });
  const index = screen.getAllByRole('option').findIndex((option) => option.textContent === label);
  await user.keyboard(`{Home}${'{ArrowDown}'.repeat(index)}{Enter}`);
  await waitFor(() => expect(trigger).toHaveTextContent(label));
}

/** Verify only FTS5 syntax switches article requests to advanced mode. */
function detectsAdvancedSyntax(): void {
  for (const query of [
    'graph AND neural',
    'graph OR tree',
    'graph NOT tree',
    '"graph neural network"',
    'title:diabetes',
    'journal_title:"Nature"',
    '{title abstract_text}:imaging',
    'bio*',
    'NEAR(gene therapy, 5)',
    '^introduction',
  ]) {
    expect(isAdvancedSearchQuery(query), query).toBe(true);
    expect(getSearchSyntaxMode(query)).toBe('advanced');
  }
  for (const query of [
    '',
    'graph neural network',
    'rock and roll',
    'Not only',
    'C++ templates',
    'mail@example.org',
    '数字 经济',
    'note: draft',
  ]) {
    expect(isAdvancedSearchQuery(query), query).toBe(false);
    expect(getSearchSyntaxMode(query)).toBe('simple');
  }
}

/** Verify rewrites only offer changes that apply to the current draft. */
function buildsRewrites(): void {
  const multiWord = buildRewriteSuggestions('graph neural');
  expect(multiWord.map((item) => item.value)).toEqual([
    '"graph neural"',
    'graph AND neural',
    'graph OR neural',
    'title:"graph neural"',
    'authors:"graph neural"',
    'journal_title:"graph neural"',
    'graph neural NOT ',
  ]);
  expect(buildRewriteSuggestions('graph').map((item) => item.value)).toContain('graph*');
  expect(buildRewriteSuggestions('title:graph').some((item) => item.id.startsWith('field-'))).toBe(
    false,
  );
  expect(buildRewriteSuggestions('  ')).toEqual([]);
}

/** Verify the backend's opaque invalid-expression error becomes actionable guidance. */
function describesSyntaxErrors(): void {
  expect(describeSearchError('Invalid search expression')).toContain('检索语法有误');
  expect(describeSearchError('获取文章失败')).toBe('获取文章失败');
}

/** Verify each condition compiles to quoted, parenthesized, column-scoped FTS5. */
function compilesConditions(): void {
  expect(buildConditionExpression(createSearchCondition({ value: ' graph   neural ' }))).toBe(
    '("graph" AND "neural")',
  );
  expect(
    buildConditionExpression(
      createSearchCondition({ field: 'title', match: 'phrase', value: 'graph neural' }),
    ),
  ).toBe('title : ("graph neural")');
  expect(
    buildConditionExpression(
      createSearchCondition({ field: 'authors', match: 'any', value: 'Smith Lee' }),
    ),
  ).toBe('authors : ("Smith" OR "Lee")');
  expect(buildConditionExpression(createSearchCondition({ match: 'prefix', value: 'bio' }))).toBe(
    '"bio" *',
  );
  expect(buildConditionExpression(createSearchCondition({ value: 'say "hi" AND (x' }))).toBe(
    '("say" AND """hi""" AND "AND" AND "(x")',
  );
  expect(buildConditionExpression(createSearchCondition({ value: '   ' }))).toBeNull();

  const compiled = compileSearchConditions([
    createSearchCondition({ field: 'title', match: 'phrase', value: 'graph neural' }),
    createSearchCondition(),
    createSearchCondition({ connector: 'OR', field: 'abstract_text', value: 'gnn' }),
    createSearchCondition({ connector: 'NOT', field: 'authors', value: 'Smith' }),
  ]);
  expect(compiled).toEqual({
    query: '(title : ("graph neural") OR abstract_text : ("gnn")) NOT authors : ("Smith")',
    activeCount: 3,
    error: null,
  });
  expect(isAdvancedSearchQuery(compiled.query)).toBe(true);

  expect(compileSearchConditions([createSearchCondition()]).error).toBe('请至少填写一个检索词。');
  expect(
    compileSearchConditions([
      createSearchCondition(),
      createSearchCondition({ connector: 'NOT', value: 'survey' }),
    ]).error,
  ).toContain('不含');
}

/** Verify the builder composes conditions visually and searches with a valid expression. */
async function searchesWithBuilder(): Promise<void> {
  const expected = 'title : ("graph neural") NOT "survey"';
  const user = userEvent.setup();
  renderAssistedSearchBar('?q=graph%20neural');

  await user.click(screen.getByRole('button', { name: '高级检索' }));
  const dialog = await screen.findByRole('dialog', { name: '高级检索' });
  expect(within(dialog).getByRole('textbox', { name: '第 1 个条件的检索词' })).toHaveValue(
    'graph neural',
  );

  await chooseOption(
    user,
    within(dialog).getByRole('combobox', { name: '第 1 个条件的字段' }),
    '标题',
  );
  await chooseOption(
    user,
    within(dialog).getByRole('combobox', { name: '第 1 个条件的匹配方式' }),
    '精确短语',
  );
  await user.click(
    within(within(dialog).getByRole('radiogroup', { name: '第 2 个条件的连接方式' })).getByRole(
      'radio',
      { name: '不含' },
    ),
  );
  await user.type(within(dialog).getByRole('textbox', { name: '第 2 个条件的检索词' }), 'survey');

  await user.click(within(dialog).getByRole('button', { name: '检索' }));
  await waitFor(() => expect(screen.getByTestId('query-value')).toHaveTextContent(expected));
  await waitFor(() =>
    expect(screen.queryByRole('dialog', { name: '高级检索' })).not.toBeInTheDocument(),
  );

  const applied = screen.getByRole('list', { name: '已应用的检索条件' });
  expect(within(applied).getByText('标题含短语「graph neural」')).toBeInTheDocument();
  expect(within(applied).getByText('排除')).toBeInTheDocument();
  expect(
    within(applied).getByRole('button', { name: '移除条件：标题含短语「graph neural」' }),
  ).toBeDisabled();

  await user.click(within(applied).getByRole('button', { name: '移除条件：任意字段含「survey」' }));
  await waitFor(() =>
    expect(screen.getByTestId('query-value')).toHaveTextContent('title : ("graph neural")'),
  );
  expect(screen.queryByText('排除')).not.toBeInTheDocument();
}

/** Verify the builder blocks an exclusion without a preceding inclusion. */
async function blocksLeadingExclusion(): Promise<void> {
  const user = userEvent.setup();
  renderAssistedSearchBar();

  await user.click(screen.getByRole('button', { name: '高级检索' }));
  const dialog = await screen.findByRole('dialog', { name: '高级检索' });
  expect(within(dialog).getByRole('button', { name: '检索' })).toBeDisabled();
  await user.click(
    within(within(dialog).getByRole('radiogroup', { name: '第 2 个条件的连接方式' })).getByRole(
      'radio',
      { name: '不含' },
    ),
  );
  await user.type(within(dialog).getByRole('textbox', { name: '第 2 个条件的检索词' }), 'survey');
  expect(within(dialog).getByRole('alert')).toHaveTextContent('不含');
  expect(within(dialog).getByRole('button', { name: '检索' })).toBeDisabled();

  await user.click(within(dialog).getByRole('button', { name: '删除第 1 个条件' }));
  expect(within(dialog).getByRole('alert')).toHaveTextContent('不含');
  expect(within(dialog).getByRole('button', { name: '检索' })).toBeDisabled();
  await user.click(
    within(within(dialog).getByRole('radiogroup', { name: '第 1 个条件的连接方式' })).getByRole(
      'radio',
      { name: '且' },
    ),
  );
  expect(within(dialog).getByRole('button', { name: '检索' })).toBeEnabled();
  expect(within(dialog).queryByRole('alert')).not.toBeInTheDocument();
  await user.click(within(dialog).getByRole('button', { name: '添加条件' }));
  expect(within(dialog).getAllByRole('textbox')).toHaveLength(2);
}

/** Verify typing offers rewrites, keyboard selection applies one, and Enter then searches. */
async function rewritesDraftFromKeyboard(): Promise<void> {
  const user = userEvent.setup();
  renderAssistedSearchBar();
  const input = screen.getByRole('combobox', { name: '搜索文章' });

  await user.type(input, 'graph neural');
  const rewrites = await screen.findByRole('listbox', { name: '改写当前输入' });
  expect(within(rewrites).getByRole('option', { name: /^"graph neural"/ })).toBeInTheDocument();

  await user.keyboard('{ArrowDown}');
  expect(input).toHaveAttribute('aria-activedescendant', 'search-assist-option-rewrite-phrase');
  await user.keyboard('{Enter}');
  expect(input).toHaveValue('"graph neural"');
  expect(screen.getByTestId('query-value')).toHaveTextContent('');
  expect(screen.getByText('将按高级语法检索')).toBeInTheDocument();

  await user.keyboard('{Enter}');
  await waitFor(() =>
    expect(screen.getByTestId('query-value')).toHaveTextContent('"graph neural"'),
  );
}

/** Verify history stays beside the builder entry and the entry opens the builder. */
async function keepsHistoryBesideBuilder(): Promise<void> {
  window.localStorage.setItem(
    'litradar:v1:search_history',
    JSON.stringify(['graph theory', 'protein folding']),
  );
  const user = userEvent.setup();
  renderAssistedSearchBar();
  const input = screen.getByRole('combobox', { name: '搜索文章' });

  await user.click(input);
  const history = await screen.findByRole('listbox', { name: '最近搜索' });
  expect(within(history).getAllByRole('option')).toHaveLength(2);
  expect(screen.getByRole('listbox', { name: '高级检索构建器' })).toBeInTheDocument();

  await user.type(input, 'gra', { skipClick: true });
  await waitFor(() =>
    expect(
      within(screen.getByRole('listbox', { name: '最近搜索' })).getAllByRole('option'),
    ).toHaveLength(1),
  );
  await user.click(screen.getByRole('option', { name: 'graph theory' }));
  await waitFor(() => expect(screen.getByTestId('query-value')).toHaveTextContent('graph theory'));

  await user.click(screen.getByRole('button', { name: '清空搜索输入' }));
  await user.click(input);
  await user.click(screen.getByRole('button', { name: '清空' }));
  expect(window.localStorage.getItem('litradar:v1:search_history')).toBeNull();
  expect(screen.queryByRole('listbox', { name: '最近搜索' })).not.toBeInTheDocument();

  await user.click(screen.getByRole('option', { name: /使用高级检索构建器/ }));
  expect(await screen.findByRole('dialog', { name: '高级检索' })).toBeInTheDocument();
}

beforeEach(() => {
  window.localStorage.clear();
  window.sessionStorage.clear();
  Object.defineProperty(HTMLElement.prototype, 'scrollIntoView', {
    configurable: true,
    value: () => undefined,
  });
});

describe('advanced search', () => {
  test('detects FTS5 syntax and keeps ordinary text simple', detectsAdvancedSyntax);
  test('builds draft rewrites', buildsRewrites);
  test('describes invalid expressions', describesSyntaxErrors);
  test('compiles builder conditions into valid FTS5', compilesConditions);
  test(
    'searches with the visual builder and removes applied conditions',
    searchesWithBuilder,
    15000,
  );
  test('blocks an exclusion without a preceding inclusion', blocksLeadingExclusion, 15000);
  test('rewrites a draft from the keyboard before searching', rewritesDraftFromKeyboard);
  test('keeps search history beside the builder entry', keepsHistoryBesideBuilder);
});
