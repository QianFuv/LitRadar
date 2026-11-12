/**
 * Stable, accessible per-folder favorite controls and removal confirmation.
 */

import { useRef, useState, type MouseEvent } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { Star } from '@/components/ui/animated-icons';

import { useAuth } from '@/lib/auth-context';
import {
  FAVORITE_CHECK_BATCH_QUERY_PREFIX,
  favoriteCheckQuery,
  folderArticlePagesQuery,
  addFavorite,
  removeFavorite,
  foldersQuery,
  type ArticleId,
  type FavoriteCheck,
  type Folder,
} from '@/lib/api';
import { Button } from '@/components/ui/button';
import { ConfirmDialog } from '@/components/ui/confirm-dialog';
import { HoverHighlightGroup, hoverHighlightItemProps } from '@/components/ui/hover-highlight';
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover';
import { RevealItem } from '@/components/ui/reveal';
import { readFreshFavoriteCheck } from '@/components/feature/use-favorite-checks';
import { LoadingText } from '@/components/ui/loading-text';
import * as stylex from '@stylexjs/stylex';
import { colors, lineHeights, shadows } from '@/lib/theme.stylex';

/**
 * Render per-folder favorite membership controls for an authenticated user.
 *
 * @param props - Article identity, source database, and optional cached folder ids.
 * @returns Favorite popover and destructive removal confirmation.
 */
export function FavoriteButton(props: FavoriteButtonProps) {
  const queryState = useFavoriteButtonState(props);
  if (!queryState.user) return null;
  const state = { ...queryState, ...getFavoriteButtonPresentation(queryState) };
  if (props.toggleFolderId !== undefined) {
    return renderFavoriteToggle(state, props.toggleFolderId);
  }
  return renderFavoriteButton(state);
}

type FavoriteButtonProps = {
  articleId: ArticleId;
  dbName: string;
  initialFolderIds?: number[];
  isFavoriteStateUnavailable?: boolean;
  /**
   * Toggle membership in this folder with one click instead of opening the folder picker.
   * The folder's article list is left in place so an accidental removal can be undone.
   */
  toggleFolderId?: number;
};
/** Compact label for an unknown favorite state, contained in its accessible name. */
const UNKNOWN_SHORT_LABEL = '状态未知';

/**
 * Render the visible favorite label; the unknown state shortens itself in a compact toolbar.
 *
 * @param label - Full favorite label, also used as the accessible name.
 * @param isFavoriteUnknown - Whether the membership lookup failed.
 * @returns Visible label content.
 */
function renderFavoriteLabel(label: string, isFavoriteUnknown: boolean) {
  if (!isFavoriteUnknown) {
    return label;
  }
  return (
    <>
      <span {...stylex.props(styles.unknownFullLabel)}>{label}</span>
      <span {...stylex.props(styles.unknownShortLabel)}>{UNKNOWN_SHORT_LABEL}</span>
    </>
  );
}

/** Own the unchanged cache, optimistic state, queries, mutations and confirmation ref. */
function useFavoriteButtonState({
  articleId,
  dbName,
  initialFolderIds = [],
  isFavoriteStateUnavailable = false,
  toggleFolderId,
}: FavoriteButtonProps) {
  const { user } = useAuth();
  const queryClient = useQueryClient();
  const db = dbName;
  const [open, setOpen] = useState(false);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const [folderToRemove, setFolderToRemove] = useState<Folder | null>(null);
  const checkQuery = favoriteCheckQuery(articleId, db);
  const queryKey = checkQuery.queryKey;
  const initialFolderIdsValue = Array.from(new Set(initialFolderIds)).sort((a, b) => a - b);
  const [optimisticFolderIds, setOptimisticFolderIds] = useState<number[] | null>(null);
  const cachedFolderIds =
    queryClient.getQueryData<FavoriteCheck[]>(queryKey)?.map((item) => item.folder_id) ?? null;

  const {
    data: checks,
    error: checksError,
    isFetching: isCheckingFavorite,
    refetch: refetchChecks,
  } = useQuery({
    ...checkQuery,
    enabled: !!user && open,
    staleTime: 5 * 60 * 1000,
  });

  const {
    data: folders = [],
    isPending: isFoldersPending,
    error: foldersError,
    refetch: refetchFolders,
  } = useQuery({
    ...foldersQuery(),
    enabled: !!user && open,
  });

  const addMut = useMutation({
    mutationFn: (folderId: number) => addFavorite(folderId, articleId, db),
    onSuccess: async (_, folderId) => {
      const batchKey = FAVORITE_CHECK_BATCH_QUERY_PREFIX;
      await Promise.all([
        queryClient.cancelQueries({ queryKey, exact: true }),
        queryClient.cancelQueries({ queryKey: batchKey }),
      ]);
      const folderName = folders.find((folder) => folder.id === folderId)?.name ?? '';
      setOptimisticFolderIds((current) => {
        const baseFolderIds =
          current ??
          checks?.map((item) => item.folder_id) ??
          cachedFolderIds ??
          initialFolderIdsValue;
        return baseFolderIds.includes(folderId) ? baseFolderIds : [...baseFolderIds, folderId];
      });
      queryClient.setQueryData(queryKey, (current: FavoriteCheck[] = []) => {
        if (current.some((item) => item.folder_id === folderId)) {
          return current;
        }
        return [...current, { folder_id: folderId, folder_name: folderName }];
      });
      queryClient.removeQueries({ queryKey: batchKey, type: 'inactive' });
      await queryClient.invalidateQueries({ queryKey: batchKey });
      void queryClient.invalidateQueries({ queryKey: foldersQuery().queryKey });
      if (folderId !== toggleFolderId) {
        void queryClient.invalidateQueries({
          queryKey: folderArticlePagesQuery(folderId).queryKey,
        });
      }
    },
  });

  const removeMut = useMutation({
    mutationFn: (folderId: number) => removeFavorite(folderId, articleId, db),
    onSuccess: async (_, folderId) => {
      const batchKey = FAVORITE_CHECK_BATCH_QUERY_PREFIX;
      await Promise.all([
        queryClient.cancelQueries({ queryKey, exact: true }),
        queryClient.cancelQueries({ queryKey: batchKey }),
      ]);
      setOptimisticFolderIds((current) => {
        const baseFolderIds =
          current ??
          checks?.map((item) => item.folder_id) ??
          cachedFolderIds ??
          initialFolderIdsValue;
        return baseFolderIds.filter((id) => id !== folderId);
      });
      queryClient.setQueryData(queryKey, (current: FavoriteCheck[] = []) =>
        current.filter((item) => item.folder_id !== folderId),
      );
      queryClient.removeQueries({ queryKey: batchKey, type: 'inactive' });
      await queryClient.invalidateQueries({ queryKey: batchKey });
      void queryClient.invalidateQueries({ queryKey: foldersQuery().queryKey });
      if (folderId !== toggleFolderId) {
        void queryClient.invalidateQueries({
          queryKey: folderArticlePagesQuery(folderId).queryKey,
        });
      }
      setFolderToRemove((current) => (current?.id === folderId ? null : current));
    },
  });

  return {
    user,
    queryClient,
    open,
    setOpen,
    triggerRef,
    folderToRemove,
    setFolderToRemove,
    queryKey,
    initialFolderIdsValue,
    optimisticFolderIds,
    cachedFolderIds,
    checks,
    checksError,
    isCheckingFavorite,
    refetchChecks,
    folders,
    isFoldersPending,
    foldersError,
    refetchFolders,
    addMut,
    removeMut,
    isFavoriteStateUnavailable,
  };
}

type FavoriteButtonQueryState = ReturnType<typeof useFavoriteButtonState>;
type FavoriteButtonViewState = FavoriteButtonQueryState &
  ReturnType<typeof getFavoriteButtonPresentation>;
/** Derive authenticated membership presentation using the original fallback and unknown-state precedence. */
function getFavoriteButtonPresentation(state: FavoriteButtonQueryState) {
  const {
    queryClient,
    queryKey,
    initialFolderIdsValue,
    optimisticFolderIds,
    cachedFolderIds,
    checks,
    checksError,
    foldersError,
    isFavoriteStateUnavailable,
  } = state;
  const resolvedFolderIds = getResolvedFavoriteFolderIds(
    checks,
    optimisticFolderIds,
    cachedFolderIds,
    initialFolderIdsValue,
  );
  const isFavoriteUnknown =
    Boolean(checksError) ||
    (isFavoriteStateUnavailable && readFreshFavoriteCheck(queryClient, queryKey) === undefined);
  const isFav = !isFavoriteUnknown && resolvedFolderIds.length > 0;
  const favoriteLabel = isFavoriteUnknown ? '收藏状态未知' : isFav ? '已收藏' : '收藏';
  const lookupError = checksError ?? foldersError;
  const favFolderIds = new Set(resolvedFolderIds);

  return { isFavoriteUnknown, isFav, favoriteLabel, lookupError, favFolderIds };
}
/** Render the unchanged trigger, popover and captured confirmation as their original siblings. */
function renderFavoriteButton(state: FavoriteButtonViewState) {
  const {
    open,
    setOpen,
    triggerRef,
    folderToRemove,
    setFolderToRemove,
    removeMut,
    isFavoriteUnknown,
    isFav,
    favoriteLabel,
  } = state;
  return (
    <>
      <Popover open={open} onOpenChange={setOpen}>
        <PopoverTrigger
          render={
            <Button
              ref={triggerRef}
              variant="outline"
              size="sm"
              static
              {...hoverHighlightItemProps}
              xstyle={[
                styles.renderFavoriteButtonButton,
                isFav && styles.renderFavoriteButtonButtonFav,
              ]}
              aria-label={favoriteLabel}
              title={favoriteLabel}
              onClick={(event: MouseEvent<HTMLButtonElement>) => event.stopPropagation()}
            />
          }
        >
          <span
            data-favorite-state={isFavoriteUnknown ? 'unknown' : isFav ? 'active' : 'inactive'}
            {...stylex.props(styles.renderFavoriteButtonSpan)}
            aria-hidden="true"
          >
            <Star
              xstyle={[
                styles.renderFavoriteButtonStar,
                isFav && styles.renderFavoriteButtonStarFav,
              ]}
            />
            <span {...stylex.props(styles.renderFavoriteButtonSpan2)}>
              <span {...stylex.props(styles.renderFavoriteButtonSpan3)}>已收藏</span>
              <span {...stylex.props(styles.renderFavoriteButtonSpan4)}>
                {renderFavoriteLabel(favoriteLabel, isFavoriteUnknown)}
              </span>
            </span>
          </span>
        </PopoverTrigger>
        <PopoverContent
          xstyle={styles.renderFavoriteButtonPopoverContent}
          align="start"
          onClick={(event: MouseEvent<HTMLDivElement>) => event.stopPropagation()}
        >
          <div {...stylex.props(styles.renderFavoriteButtonDiv)}>
            <div {...stylex.props(styles.renderFavoriteButtonDiv2)}>选择收藏夹</div>
            <FavoriteLookup state={state} />
          </div>
        </PopoverContent>
      </Popover>
      <ConfirmDialog
        open={folderToRemove !== null}
        onOpenChange={(nextOpen) => {
          if (!nextOpen && !removeMut.isPending) {
            setFolderToRemove(null);
          }
        }}
        title="移除收藏？"
        description={`确认从“${folderToRemove?.name ?? ''}”移除收藏？`}
        actionLabel="确认移除"
        pendingLabel="移除中…"
        isPending={removeMut.isPending}
        error={removeMut.error instanceof Error ? removeMut.error.message : null}
        focusReturnRef={triggerRef}
        onConfirm={() => {
          if (folderToRemove) {
            removeMut.mutate(folderToRemove.id);
          }
        }}
      />
    </>
  );
}
/**
 * Render a one-click toggle for membership in a single folder.
 *
 * @param state - Favorite button state.
 * @param folderId - Folder whose membership is toggled.
 * @returns Toggle button with inline failure feedback.
 */
function renderFavoriteToggle(state: FavoriteButtonViewState, folderId: number) {
  const { triggerRef, addMut, removeMut, isFavoriteUnknown, favFolderIds } = state;
  const isInFolder = !isFavoriteUnknown && favFolderIds.has(folderId);
  const label = isFavoriteUnknown ? '收藏状态未知' : isInFolder ? '已收藏' : '收藏';
  const failure = addMut.error ?? removeMut.error;
  return (
    <span {...stylex.props(styles.renderFavoriteToggleSpan)}>
      <Button
        ref={triggerRef}
        variant="outline"
        size="sm"
        static
        aria-pressed={isInFolder}
        aria-label={label}
        title={isInFolder ? '取消收藏' : '收藏到此收藏夹'}
        {...hoverHighlightItemProps}
        xstyle={[
          styles.renderFavoriteButtonButton,
          isInFolder && styles.renderFavoriteButtonButtonFav,
        ]}
        disabled={addMut.isPending || removeMut.isPending || isFavoriteUnknown}
        onClick={(event: MouseEvent<HTMLButtonElement>) => {
          event.stopPropagation();
          addMut.reset();
          removeMut.reset();
          if (isInFolder) {
            removeMut.mutate(folderId);
          } else {
            addMut.mutate(folderId);
          }
        }}
      >
        <span
          data-favorite-state={isFavoriteUnknown ? 'unknown' : isInFolder ? 'active' : 'inactive'}
          {...stylex.props(styles.renderFavoriteButtonSpan)}
          aria-hidden="true"
        >
          <Star
            xstyle={[
              styles.renderFavoriteButtonStar,
              isInFolder && styles.renderFavoriteButtonStarFav,
            ]}
          />
          <span {...stylex.props(styles.renderFavoriteButtonSpan2)}>
            <span {...stylex.props(styles.renderFavoriteButtonSpan3)}>已收藏</span>
            <span {...stylex.props(styles.renderFavoriteButtonSpan4)}>
              {renderFavoriteLabel(label, isFavoriteUnknown)}
            </span>
          </span>
        </span>
      </Button>
      {failure && (
        <span role="alert" {...stylex.props(styles.renderFavoriteToggleSpan2)}>
          {failure instanceof Error ? failure.message : '收藏操作失败'}
        </span>
      )}
    </span>
  );
}

/** Retain each folder button, pending guards and reset-before-target event order. */
function renderFavoriteFolderButton(state: FavoriteButtonViewState, folder: Folder) {
  const {
    setFolderToRemove,
    isCheckingFavorite,
    addMut,
    removeMut,
    isFavoriteUnknown,
    favFolderIds,
  } = state;

  const isInFolder = favFolderIds.has(folder.id);
  return (
    <button
      key={folder.id}
      type="button"
      aria-pressed={isInFolder}
      {...hoverHighlightItemProps}
      {...stylex.props(
        styles.renderFavoriteFolderButtonButton,
        isInFolder
          ? styles.renderFavoriteFolderButtonButtonInFolder
          : styles.renderFavoriteFolderButtonButtonNotInFolder,
      )}
      disabled={addMut.isPending || removeMut.isPending || isCheckingFavorite || isFavoriteUnknown}
      onClick={() => {
        if (isInFolder) {
          removeMut.reset();
          setFolderToRemove(folder);
        } else {
          addMut.mutate(folder.id);
        }
      }}
    >
      <Star
        xstyle={[
          styles.renderFavoriteFolderButtonStar,
          isInFolder && styles.renderFavoriteButtonStarFav,
        ]}
        strokeWidth={1.5}
        aria-hidden="true"
      />
      <span {...stylex.props(styles.renderFavoriteFolderButtonSpan)}>{folder.name}</span>
    </button>
  );
}

/** Render lookup states and folder rows only while the Popover mounts its content. */
function FavoriteLookup({ state }: { state: FavoriteButtonViewState }) {
  const { checks, isCheckingFavorite, isFoldersPending, lookupError } = state;
  const isLookupPending = isFoldersPending || (isCheckingFavorite && checks === undefined);
  return (
    <RevealItem
      isReady={Boolean(lookupError) || !isLookupPending}
      xstyle={styles.renderFavoriteButtonDiv}
      placeholder={
        <output {...stylex.props(styles.statusOutput, styles.favoriteLookupDiv)}>
          <LoadingText text="加载中…" />
        </output>
      }
    >
      <FavoriteLookupContent state={state} />
    </RevealItem>
  );
}

/** Render lookup failures, the empty state, or folder rows once the lookup has settled. */
function FavoriteLookupContent({ state }: { state: FavoriteButtonViewState }) {
  const { refetchChecks, folders, refetchFolders, lookupError } = state;
  if (lookupError)
    return (
      <div role="alert" {...stylex.props(styles.favoriteLookupContentDiv)}>
        <p {...stylex.props(styles.favoriteLookupContentDivItem)}>
          {lookupError instanceof Error ? lookupError.message : '无法读取收藏状态'}
        </p>
        <Button
          xstyle={styles.favoriteLookupContentDivItem}
          type="button"
          variant="outline"
          size="sm"
          onClick={() => {
            void Promise.all([refetchChecks(), refetchFolders()]);
          }}
        >
          重试收藏状态
        </Button>
      </div>
    );
  if (folders.length === 0)
    return (
      <div {...stylex.props(styles.favoriteLookupDiv)}>暂无收藏夹，请先在「我的收藏」中创建</div>
    );
  return (
    <HoverHighlightGroup xstyle={styles.renderFavoriteButtonDiv}>
      {folders.map((folder) => renderFavoriteFolderButton(state, folder))}
    </HoverHighlightGroup>
  );
}

/** Resolve visible membership from checks before optimistic, cached and initial values. */
function getResolvedFavoriteFolderIds(
  checks: FavoriteCheck[] | undefined,
  optimistic: number[] | null,
  cached: number[] | null,
  initial: number[],
): number[] {
  return checks?.map((item) => item.folder_id) ?? optimistic ?? cached ?? initial;
}

const styles = stylex.create({
  statusOutput: {
    display: 'block',
  },
  renderFavoriteButtonButton: {
    backgroundColor: {
      default: 'transparent',
      ':hover': {
        default: null,
        '@media (hover: hover)': colors.accent,
      },
    },
    boxShadow: {
      default: 'none',
      ':focus-visible': {
        default: `0 0 0 3px color-mix(in oklab, ${colors.ring} 50%, transparent)`,
        ':is([aria-invalid="true"])': `0 0 0 3px ${colors.destructiveRing}`,
      },
    },
    width: {
      default: '2.75rem',
      '@media (min-width: 48rem)': 'auto',
    },
    height: {
      default: '2.75rem',
      '@media (min-width: 48rem)': '2.5rem',
    },
    padding: '0px',
    paddingInline: {
      default: null,
      ':has( > svg)': '0.625rem',
      '@media (min-width: 48rem)': {
        default: '0.75rem',
        ':has( > svg)': '0.625rem',
      },
    },
  },
  renderFavoriteButtonButtonFav: {
    color: {
      default: colors.warningForeground,
      ':hover': {
        default: null,
        '@media (hover: hover)': colors.warningForeground,
      },
    },
  },
  renderFavoriteButtonSpan: {
    display: 'flex',
    alignItems: 'center',
    gap: '0.25rem',
  },
  renderFavoriteButtonStar: {
    width: '1rem',
    height: '1rem',
    transitionProperty: 'fill',
    transitionTimingFunction: 'cubic-bezier(0.25, 0.1, 0.25, 1)',
    transitionDuration: '200ms',
  },
  renderFavoriteButtonStarFav: {
    fill: 'currentcolor',
  },
  renderFavoriteButtonSpan2: {
    display: {
      default: 'none',
      '@media (min-width: 48rem)': 'grid',
    },
  },
  renderFavoriteButtonSpan3: {
    visibility: 'hidden',
    gridColumnStart: 1,
    gridRowStart: 1,
  },
  renderFavoriteButtonSpan4: {
    gridColumnStart: 1,
    gridRowStart: 1,
  },
  unknownFullLabel: {
    display: {
      default: 'inline',
      '@container (width < 43rem)': 'none',
    },
  },
  unknownShortLabel: {
    display: {
      default: 'none',
      '@container (width < 43rem)': 'inline',
    },
  },
  renderFavoriteButtonPopoverContent: {
    width: '14rem',
    borderRadius: '12px',
    borderStyle: 'solid',
    borderWidth: '0px',
    padding: '0.5rem',
    boxShadow: shadows.vercelCard,
  },
  renderFavoriteButtonDiv: {
    display: 'flex',
    flexDirection: 'column',
    rowGap: '0.25rem',
  },
  renderFavoriteButtonDiv2: {
    paddingInline: '0.5rem',
    paddingBlock: '0.25rem',
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    fontWeight: 500,
    color: colors.mutedForeground,
  },
  renderFavoriteToggleSpan: {
    display: 'inline-flex',
    alignItems: 'center',
    gap: '0.5rem',
  },
  renderFavoriteToggleSpan2: {
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    color: colors.destructive,
  },
  renderFavoriteFolderButtonButton: {
    display: 'flex',
    minHeight: {
      default: '2.75rem',
      '@media (min-width: 48rem)': '2.5rem',
    },
    width: '100%',
    alignItems: 'center',
    gap: '0.5rem',
    borderRadius: '4px',
    paddingInline: '0.5rem',
    paddingBlock: '0.375rem',
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
    transitionProperty: 'color, box-shadow',
    transitionTimingFunction: 'cubic-bezier(0.25, 0.1, 0.25, 1)',
    transitionDuration: '200ms',
    outlineStyle: 'none',
    boxShadow: {
      default: null,
      ':focus-visible': `0 0 0 3px color-mix(in oklab, ${colors.ring} 50%, transparent)`,
    },
    pointerEvents: {
      default: null,
      ':disabled': 'none',
    },
    opacity: {
      default: null,
      ':disabled': '50%',
    },
  },
  renderFavoriteFolderButtonButtonInFolder: {
    backgroundColor: {
      default: colors.warning,
      ':active': {
        default: `color-mix(in oklab, ${colors.warning} 50%, transparent)`,
        ':hover': {
          default: null,
          '@media (hover: hover)': `color-mix(in oklab, ${colors.warning} 50%, transparent)`,
        },
      },
      ':hover': {
        default: null,
        '@media (hover: hover)': `color-mix(in oklab, ${colors.warning} 70%, transparent)`,
      },
    },
    color: colors.warningForeground,
  },
  renderFavoriteFolderButtonButtonNotInFolder: {
    backgroundColor: {
      default: null,
      ':active': {
        default: colors.accentPressed,
        ':hover': {
          default: null,
          '@media (hover: hover)': colors.accentPressed,
        },
      },
      ':hover': {
        default: null,
        '@media (hover: hover)': colors.accent,
      },
    },
  },
  renderFavoriteFolderButtonStar: {
    width: '1rem',
    height: '1rem',
    flexShrink: 0,
    transitionProperty: 'fill',
    transitionTimingFunction: 'cubic-bezier(0.25, 0.1, 0.25, 1)',
    transitionDuration: '200ms',
  },
  renderFavoriteFolderButtonSpan: {
    overflow: 'hidden',
    textOverflow: 'ellipsis',
    whiteSpace: 'nowrap',
  },
  favoriteLookupDiv: {
    paddingInline: '0.5rem',
    paddingBlock: '0.5rem',
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    color: colors.mutedForeground,
  },
  favoriteLookupContentDivItem: {
    marginBottom: {
      default: null,
      ':not(:last-child)': '0.5rem',
    },
  },
  favoriteLookupContentDiv: {
    paddingInline: '0.5rem',
    paddingBlock: '0.5rem',
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    color: colors.destructive,
  },
});
