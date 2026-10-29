'use client';

/**
 * Visual advanced-search builder: add, edit, and remove conditions without writing syntax.
 */

import { ChevronRight, Plus, Trash2 } from 'lucide-react';
import { useState, type FormEvent } from 'react';

import { Button } from '@/components/ui/button';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import { Input } from '@/components/ui/input';
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select';
import {
  compileSearchConditions,
  createSearchCondition,
  readStoredSearchBuilder,
  SEARCH_BUILDER_CONNECTORS,
  SEARCH_BUILDER_FIELDS,
  SEARCH_BUILDER_MATCHES,
  SEARCH_BUILDER_MAX_CONDITIONS,
  type SearchBuilderCondition,
} from '@/lib/search-builder';
import { isAdvancedSearchQuery } from '@/lib/search-syntax';
import { cn } from '@/lib/utils';

type SearchBuilderDialogProps = {
  /** Query currently applied in the URL. */
  currentQuery: string;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** Run a compiled query and remember the rows that produced it. */
  onSearch: (query: string, conditions: SearchBuilderCondition[]) => void;
};

/**
 * Choose the rows shown when the builder opens.
 *
 * @param currentQuery - Applied query.
 * @returns Initial rows and whether a handwritten expression will be replaced.
 */
function getInitialConditions(currentQuery: string): {
  conditions: SearchBuilderCondition[];
  replacesHandwritten: boolean;
} {
  const query = currentQuery.trim();
  const stored = readStoredSearchBuilder();
  if (stored && query && stored.query === query) {
    return { conditions: stored.conditions, replacesHandwritten: false };
  }
  if (query && !isAdvancedSearchQuery(query)) {
    return {
      conditions: [createSearchCondition({ value: query }), createSearchCondition()],
      replacesHandwritten: false,
    };
  }
  return {
    conditions: [createSearchCondition(), createSearchCondition()],
    replacesHandwritten: Boolean(query),
  };
}

/**
 * Render the advanced-search builder dialog.
 *
 * @param props - Applied query, open state, and search callback.
 * @returns Builder dialog.
 */
export function SearchBuilderDialog({
  currentQuery,
  open,
  onOpenChange,
  onSearch,
}: SearchBuilderDialogProps) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="flex max-h-[90dvh] flex-col gap-0 p-0 md:max-w-3xl">
        <DialogHeader className="border-b px-5 pt-5 pb-4 text-left sm:px-6">
          <DialogTitle>高级检索</DialogTitle>
          <DialogDescription>
            按条件组合检索，只需填写检索词，系统会自动生成正确的检索式。
          </DialogDescription>
        </DialogHeader>
        {open && (
          <SearchBuilderBody
            currentQuery={currentQuery}
            onCancel={() => onOpenChange(false)}
            onSearch={onSearch}
          />
        )}
      </DialogContent>
    </Dialog>
  );
}

/**
 * Render builder rows, the generated expression, and actions.
 *
 * @param props - Applied query, cancel action, and search callback.
 * @returns Builder form.
 */
function SearchBuilderBody({
  currentQuery,
  onCancel,
  onSearch,
}: {
  currentQuery: string;
  onCancel: () => void;
  onSearch: (query: string, conditions: SearchBuilderCondition[]) => void;
}) {
  const [initialState] = useState(() => getInitialConditions(currentQuery));
  const [conditions, setConditions] = useState(initialState.conditions);
  const result = compileSearchConditions(conditions);
  const activeConditions = conditions.filter((condition) => condition.value.trim());

  /**
   * Update one row.
   *
   * @param id - Row id.
   * @param patch - Changed fields.
   */
  const updateCondition = (id: string, patch: Partial<SearchBuilderCondition>) => {
    setConditions((current) =>
      current.map((condition) => (condition.id === id ? { ...condition, ...patch } : condition)),
    );
  };

  /**
   * Remove one row, keeping at least one editable row.
   *
   * @param id - Row id.
   */
  const removeCondition = (id: string) => {
    setConditions((current) => {
      const next = current.filter((condition) => condition.id !== id);
      return next.length > 0 ? next : [createSearchCondition()];
    });
  };

  /**
   * Run the compiled query.
   *
   * @param event - Form submission.
   */
  const handleSubmit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (result.error) return;
    onSearch(
      result.query,
      conditions.filter((condition) => condition.value.trim()),
    );
  };

  return (
    <form onSubmit={handleSubmit} className="flex min-h-0 flex-1 flex-col">
      <div className="min-h-0 flex-1 overflow-y-auto px-5 py-4 sm:px-6">
        {initialState.replacesHandwritten && (
          <p className="mb-3 rounded-lg bg-muted px-3 py-2 text-xs leading-5 text-muted-foreground">
            当前检索式是手写语法，构建器会生成新的检索式替换它。
          </p>
        )}
        <ol className="divide-y" aria-label="检索条件">
          {conditions.map((condition, index) => {
            const position = index + 1;
            return (
              <li
                key={condition.id}
                data-search-condition={condition.id}
                className="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-2 py-3 sm:grid-cols-[8rem_7.5rem_8.75rem_minmax(0,1fr)_auto]"
              >
                {index === 0 && condition.connector !== 'NOT' ? (
                  <span className="flex h-9 items-center text-xs font-bold text-muted-foreground">
                    检索
                  </span>
                ) : (
                  <div
                    role="radiogroup"
                    aria-label={`第 ${position} 个条件的连接方式`}
                    className="inline-flex h-9 w-fit items-center gap-0.5 rounded-lg bg-muted p-0.5"
                  >
                    {SEARCH_BUILDER_CONNECTORS.map((connector) => {
                      const isSelected = condition.connector === connector.value;
                      return (
                        <button
                          key={connector.value}
                          type="button"
                          role="radio"
                          aria-checked={isSelected}
                          className={cn(
                            'motion-control h-8 rounded-md px-2.5 text-[13px] whitespace-nowrap outline-none transition-[background-color,color,box-shadow] focus-visible:ring-2 focus-visible:ring-ring',
                            isSelected
                              ? 'bg-background font-bold shadow-[0_0_0_1px_var(--border)]'
                              : 'text-muted-foreground hover:text-foreground',
                            isSelected && connector.value === 'NOT' && 'text-destructive',
                          )}
                          onClick={() =>
                            updateCondition(condition.id, { connector: connector.value })
                          }
                        >
                          {connector.label}
                        </button>
                      );
                    })}
                  </div>
                )}
                <Button
                  type="button"
                  variant="ghost"
                  size="icon"
                  className="size-11 justify-self-end text-muted-foreground hover:bg-destructive/10 hover:text-destructive sm:order-last sm:size-9"
                  aria-label={`删除第 ${position} 个条件`}
                  onClick={() => removeCondition(condition.id)}
                >
                  <Trash2 className="size-4" aria-hidden="true" />
                </Button>
                <div className="col-span-2 grid grid-cols-2 gap-2 sm:contents">
                  <Select
                    value={condition.field}
                    onValueChange={(value) =>
                      updateCondition(condition.id, {
                        field: value as SearchBuilderCondition['field'],
                      })
                    }
                  >
                    <SelectTrigger
                      size="sm"
                      aria-label={`第 ${position} 个条件的字段`}
                      className="w-full"
                    >
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {SEARCH_BUILDER_FIELDS.map((field) => (
                        <SelectItem key={field.value} value={field.value}>
                          {field.label}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                  <Select
                    value={condition.match}
                    onValueChange={(value) =>
                      updateCondition(condition.id, {
                        match: value as SearchBuilderCondition['match'],
                      })
                    }
                  >
                    <SelectTrigger
                      size="sm"
                      aria-label={`第 ${position} 个条件的匹配方式`}
                      className="w-full"
                    >
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {SEARCH_BUILDER_MATCHES.map((match) => (
                        <SelectItem key={match.value} value={match.value}>
                          {match.label}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </div>
                <Input
                  aria-label={`第 ${position} 个条件的检索词`}
                  autoComplete="off"
                  spellCheck={false}
                  placeholder={index === 0 ? '例如：graph neural network' : '输入检索词'}
                  value={condition.value}
                  autoFocus={index === 0 && !condition.value}
                  onChange={(event) => updateCondition(condition.id, { value: event.target.value })}
                  className="col-span-2 h-11 sm:col-span-1 sm:h-9"
                />
              </li>
            );
          })}
        </ol>
        <Button
          type="button"
          variant="ghost"
          size="sm"
          className="mt-1 -ml-2 text-primary hover:text-primary"
          disabled={conditions.length >= SEARCH_BUILDER_MAX_CONDITIONS}
          onClick={() => setConditions((current) => [...current, createSearchCondition()])}
        >
          <Plus className="size-4" aria-hidden="true" />
          添加条件
        </Button>

        {result.error && activeConditions.length > 0 && (
          <p role="alert" className="mt-2 text-xs text-destructive">
            {result.error}
          </p>
        )}

        <details className="group mt-4 border-t pt-3 text-xs text-muted-foreground">
          <summary className="flex w-fit cursor-pointer list-none items-center gap-1 font-bold text-foreground/80 hover:text-foreground [&::-webkit-details-marker]:hidden">
            <ChevronRight
              className="size-3.5 transition-transform group-open:rotate-90"
              aria-hidden="true"
            />
            <span className="group-open:hidden">点击展开手写检索语法</span>
            <span className="hidden group-open:inline">收起手写检索语法</span>
          </summary>
          <p className="mt-2">在搜索框中直接输入以下语法同样有效：</p>
          <ul className="mt-2 grid gap-1 sm:grid-cols-2">
            <li>
              <code>a AND b</code> 同时包含
            </li>
            <li>
              <code>a OR b</code> 任一包含
            </li>
            <li>
              <code>a NOT b</code> 排除 b
            </li>
            <li>
              <code>&quot;exact phrase&quot;</code> 精确短语
            </li>
            <li>
              <code>bio*</code> 前缀匹配
            </li>
            <li>
              <code>NEAR(gene therapy, 5)</code> 相距 5 词内
            </li>
            <li>
              <code>title:diabetes</code> 标题
            </li>
            <li>
              <code>authors:&quot;Smith&quot;</code> 作者
            </li>
            <li>
              <code>journal_title:&quot;Nature&quot;</code> 期刊
            </li>
            <li>
              <code>abstract_text:imaging</code> 摘要
            </li>
          </ul>
          <p className="mt-2">运算符 AND / OR / NOT / NEAR 需要大写。</p>
        </details>
      </div>
      <div className="flex items-center gap-2 border-t px-5 py-3 sm:px-6">
        <Button
          type="button"
          variant="ghost"
          size="sm"
          className="mr-auto text-muted-foreground"
          onClick={() => setConditions([createSearchCondition(), createSearchCondition()])}
        >
          清空条件
        </Button>
        <Button type="button" variant="outline" onClick={onCancel}>
          取消
        </Button>
        <Button
          type="submit"
          disabled={Boolean(result.error)}
          className="bg-foreground text-background hover:bg-foreground/85"
        >
          检索
        </Button>
      </div>
    </form>
  );
}
