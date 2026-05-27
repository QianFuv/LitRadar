'use client';

/**
 * Article display, detail, and favorite controls for the desktop frontend.
 */

import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import {
  ChevronLeft,
  ChevronRight,
  Check,
  Copy,
  ExternalLink,
  FolderPlus,
  Star,
  Trash2,
  X,
  type LucideIcon,
} from 'lucide-react';
import { useEffect, useMemo, useState, type ReactNode } from 'react';
import {
  addFavorite,
  checkFavorite,
  createFolder,
  getFolders,
  getFullTextUrl,
  removeFavorite,
  type Article,
  type ArticleId,
  type FavoriteCheck,
} from '@/lib/client-api';
import { useAuthSession } from '@/lib/auth-session';
import { buildArticleClipboardText, getArticleTitle, getArticleVenue } from '@/lib/format';
import {
  Badge,
  Button,
  EmptyState,
  Field,
  IconButton,
  Modal,
  Notice,
  Panel,
  TextInput,
  joinClassNames,
} from '@/components/desktop/ui';

interface ArticleAction {
  icon: LucideIcon;
  label: string;
  tone?: 'danger' | 'secondary';
  onClick: () => void;
}

interface ArticleCardProps {
  article: Article;
  dbName: string;
  favoriteFolderIds?: number[];
  favoritePending?: boolean;
  leading?: ReactNode;
  actions?: ArticleAction[];
  title?: ReactNode;
  preview?: ReactNode;
  selected?: boolean;
  onSelect?: (article: Article) => void;
}

interface FavoritePickerProps {
  articleId: ArticleId;
  dbName: string;
  initialFolderIds?: number[];
  onClose: () => void;
  onSaved: (message: string) => void;
}

interface ArticleDetailModalProps {
  article: Article;
  dbName: string;
  initialFolderIds?: number[];
  favoritePending?: boolean;
  actions?: ArticleAction[];
  open: boolean;
  onClose: () => void;
}

/**
 * Check whether an article has a possible full-text target.
 *
 * @param article - Article record.
 * @returns Whether full text can be opened.
 */
function hasFullTextTarget(article: Article): boolean {
  return Boolean(article.full_text_file || article.permalink || article.doi || article.platform_id);
}

/**
 * Get folder ids from favorite checks.
 *
 * @param checks - Favorite checks.
 * @returns Folder ids.
 */
function getFavoriteFolderIds(checks?: FavoriteCheck[]): number[] {
  return checks?.map((item) => item.folder_id) ?? [];
}

/**
 * Copy text to the clipboard.
 *
 * @param text - Text to copy.
 */
async function copyText(text: string): Promise<void> {
  await navigator.clipboard.writeText(text);
}

/**
 * Render article status badges.
 *
 * @param article - Article record.
 * @returns Badge list.
 */
function ArticleBadges({ article }: { article: Article }) {
  return (
    <>
      {article.open_access ? <Badge tone="teal">Open Access</Badge> : null}
      {article.in_press ? <Badge tone="violet">In Press</Badge> : null}
      {article.doi ? <Badge tone="neutral">DOI</Badge> : null}
    </>
  );
}

/**
 * Render a modal for folder favorite selection.
 *
 * @param props - Favorite picker props.
 * @returns Favorite picker modal.
 */
function FavoritePicker({
  articleId,
  dbName,
  initialFolderIds = [],
  onClose,
  onSaved,
}: FavoritePickerProps) {
  const queryClient = useQueryClient();
  const { token, user } = useAuthSession();
  const [newFolderName, setNewFolderName] = useState('');
  const [hasChanged, setHasChanged] = useState(false);
  const [didAddFavorite, setDidAddFavorite] = useState(false);
  const favoriteQueryKey = ['favorite-check', user?.id, dbName, articleId] as const;

  const { data: folders = [], isPending: foldersPending } = useQuery({
    queryKey: ['folders', user?.id],
    queryFn: () => getFolders(token!),
    enabled: Boolean(token && user),
  });

  const { data: checks } = useQuery({
    queryKey: favoriteQueryKey,
    queryFn: () => checkFavorite(token!, articleId, dbName),
    enabled: Boolean(token && user),
    initialData: initialFolderIds.map((folderId) => ({ folder_id: folderId, folder_name: '' })),
  });

  const selectedFolderIds = new Set(getFavoriteFolderIds(checks));

  const addMutation = useMutation({
    mutationFn: (folderId: number) => addFavorite(token!, folderId, articleId, dbName),
    onSuccess: (_, folderId) => {
      queryClient.setQueryData<FavoriteCheck[]>(favoriteQueryKey, (current = []) => {
        if (current.some((item) => item.folder_id === folderId)) {
          return current;
        }
        const folderName = folders.find((folder) => folder.id === folderId)?.name ?? '';
        return [...current, { folder_id: folderId, folder_name: folderName }];
      });
      queryClient.invalidateQueries({ queryKey: ['favorite-batch', user?.id, dbName] });
      queryClient.invalidateQueries({ queryKey: ['folders'] });
      setHasChanged(true);
      setDidAddFavorite(true);
    },
  });

  const removeMutation = useMutation({
    mutationFn: (folderId: number) => removeFavorite(token!, folderId, articleId, dbName),
    onSuccess: (_, folderId) => {
      queryClient.setQueryData<FavoriteCheck[]>(favoriteQueryKey, (current = []) =>
        current.filter((item) => item.folder_id !== folderId),
      );
      queryClient.invalidateQueries({ queryKey: ['favorite-batch', user?.id, dbName] });
      queryClient.invalidateQueries({ queryKey: ['folders'] });
      queryClient.invalidateQueries({ queryKey: ['folder-articles'] });
      setHasChanged(true);
    },
  });

  const createMutation = useMutation({
    mutationFn: (name: string) => createFolder(token!, name),
    onSuccess: (folder) => {
      setNewFolderName('');
      queryClient.invalidateQueries({ queryKey: ['folders'] });
      addMutation.mutate(folder.id);
    },
  });

  const isSaving = addMutation.isPending || removeMutation.isPending || createMutation.isPending;
  const finish = () => {
    if (didAddFavorite) {
      onSaved('已收藏');
    } else if (hasChanged) {
      onSaved('收藏已更新');
    }
    onClose();
  };

  return (
    <Modal
      narrow
      open
      title="选择收藏夹"
      description={`文章 ${articleId}`}
      onClose={onClose}
      footer={
        <Button disabled={isSaving} onClick={finish}>
          完成
        </Button>
      }
    >
      <div className="form-grid">
        <div className="form-grid form-grid--two">
          <Field label="新建收藏夹">
            <TextInput
              value={newFolderName}
              onChange={(event) => setNewFolderName(event.target.value)}
              placeholder="例如：待读文献"
            />
          </Field>
          <div className="field">
            <span className="field__label">操作</span>
            <Button
              icon={<FolderPlus size={15} />}
              disabled={!newFolderName.trim() || createMutation.isPending}
              onClick={() => createMutation.mutate(newFolderName.trim())}
            >
              创建并收藏
            </Button>
          </div>
        </div>
        {foldersPending ? (
          <Notice>正在加载收藏夹...</Notice>
        ) : folders.length === 0 ? (
          <EmptyState>暂无收藏夹，可先在上方创建。</EmptyState>
        ) : (
          <div className="list-stack">
            {folders.map((folder) => {
              const selected = selectedFolderIds.has(folder.id);
              return (
                <button
                  key={folder.id}
                  className="article-row"
                  type="button"
                  onClick={() => {
                    if (selected) {
                      removeMutation.mutate(folder.id);
                      return;
                    }
                    addMutation.mutate(folder.id);
                  }}
                >
                  <div className="toolbar">
                    <Star
                      size={17}
                      fill={selected ? 'currentColor' : 'none'}
                      color={selected ? 'var(--amber)' : 'var(--muted)'}
                    />
                    <strong>{folder.name}</strong>
                    {folder.is_tracking ? <Badge tone="violet">追踪</Badge> : null}
                    <span className="panel__meta">{folder.article_count} 篇</span>
                  </div>
                </button>
              );
            })}
          </div>
        )}
      </div>
    </Modal>
  );
}

/**
 * Render a transient page-level toast for favorite actions.
 *
 * @param props - Toast props.
 * @returns Toast element or null.
 */
function FavoriteToast({ message, onClose }: { message: string | null; onClose: () => void }) {
  useEffect(() => {
    if (!message) {
      return;
    }
    const timeoutId = window.setTimeout(onClose, 2200);
    return () => window.clearTimeout(timeoutId);
  }, [message, onClose]);

  if (!message) {
    return null;
  }

  return (
    <div className="toast-stack" role="status" aria-live="polite">
      <div className="toast">
        <Check size={16} />
        <span>{message}</span>
      </div>
    </div>
  );
}

/**
 * Render the article detail modal.
 *
 * @param props - Article detail props.
 * @returns Article detail modal.
 */
function ArticleDetailModal({
  actions = [],
  article,
  dbName,
  favoritePending = false,
  initialFolderIds = [],
  onClose,
  open,
}: ArticleDetailModalProps) {
  const { token } = useAuthSession();
  const [copyState, setCopyState] = useState<'title' | 'info' | null>(null);
  const [favoriteOpen, setFavoriteOpen] = useState(false);
  const [toastMessage, setToastMessage] = useState<string | null>(null);
  const hasRemoveFavoriteAction = actions.some((action) => action.label === '移除收藏');
  const fullTextUrl = hasFullTextTarget(article)
    ? getFullTextUrl(article.article_id, dbName, token ?? undefined)
    : null;

  const runCopy = async (kind: 'title' | 'info', text: string) => {
    await copyText(text);
    setCopyState(kind);
    window.setTimeout(() => setCopyState(null), 1800);
  };
  const articleTitle = getArticleTitle(article);

  return (
    <>
      <Modal
        open={open}
        title={
          <button
            aria-label={copyState === 'title' ? '题名已复制' : '点击复制题名'}
            className={joinClassNames(
              'article-detail-title-copy',
              copyState === 'title' && 'article-detail-title-copy--copied',
            )}
            title={copyState === 'title' ? '题名已复制' : '点击复制题名'}
            type="button"
            onClick={() => void runCopy('title', articleTitle)}
          >
            {articleTitle}
          </button>
        }
        description={getArticleVenue(article)}
        onClose={onClose}
        footer={
          <>
            <Button
              icon={copyState === 'info' ? <Check size={15} /> : <Copy size={15} />}
              onClick={() => void runCopy('info', buildArticleClipboardText(article))}
            >
              {copyState === 'info' ? '已复制' : '复制信息'}
            </Button>
            {fullTextUrl ? (
              <a href={fullTextUrl} rel="noreferrer" target="_blank">
                <Button icon={<ExternalLink size={15} />} variant="violet">
                  查看全文
                </Button>
              </a>
            ) : null}
            {hasRemoveFavoriteAction ? null : (
              <Button
                icon={<Star size={15} />}
                variant="secondary"
                disabled={favoritePending}
                onClick={() => setFavoriteOpen(true)}
              >
                收藏
              </Button>
            )}
            {actions.map((action) => {
              const Icon = action.icon;
              return (
                <Button
                  key={action.label}
                  icon={<Icon size={15} />}
                  variant={action.tone === 'danger' ? 'danger' : 'secondary'}
                  onClick={action.onClick}
                >
                  {action.label}
                </Button>
              );
            })}
          </>
        }
      >
        <div className="form-grid">
          <div className="toolbar toolbar--wrap">
            <ArticleBadges article={article} />
            {article.doi ? <Badge tone="neutral">{article.doi}</Badge> : null}
          </div>
          {article.authors ? (
            <PanelSection title="作者">
              <p>{article.authors}</p>
            </PanelSection>
          ) : null}
          <PanelSection title="摘要">
            <p>{article.abstract || '暂无摘要。'}</p>
          </PanelSection>
          <PanelSection title="记录">
            <div className="form-grid form-grid--two">
              <Notice>数据库：{dbName}</Notice>
              <Notice>文章 ID：{article.article_id}</Notice>
              <Notice>期刊 ID：{article.journal_id || '未知'}</Notice>
              <Notice>平台 ID：{article.platform_id || '未知'}</Notice>
            </div>
          </PanelSection>
        </div>
      </Modal>
      {favoriteOpen ? (
        <FavoritePicker
          articleId={article.article_id}
          dbName={dbName}
          initialFolderIds={initialFolderIds}
          onClose={() => setFavoriteOpen(false)}
          onSaved={setToastMessage}
        />
      ) : null}
      <FavoriteToast message={toastMessage} onClose={() => setToastMessage(null)} />
    </>
  );
}

/**
 * Render a named section inside the article detail modal.
 *
 * @param props - Section props.
 * @returns Section element.
 */
function PanelSection({ children, title }: { children: ReactNode; title: string }) {
  return (
    <section className="notice">
      <h3 className="panel__title">{title}</h3>
      <div className="modal__description">{children}</div>
    </section>
  );
}

/**
 * Render an interactive article row with detail modal.
 *
 * @param props - Article card props.
 * @returns Article card.
 */
export function ArticleCard({
  actions,
  article,
  dbName,
  favoriteFolderIds = [],
  favoritePending = false,
  leading,
  onSelect,
  preview,
  selected = false,
  title,
}: ArticleCardProps) {
  const [detailOpen, setDetailOpen] = useState(false);
  const [favoriteOpen, setFavoriteOpen] = useState(false);
  const [toastMessage, setToastMessage] = useState<string | null>(null);

  const isFavorite = favoriteFolderIds.length > 0;
  const resolvedPreview = preview ?? article.abstract;
  const resolvedTitle = title ?? getArticleTitle(article);
  const handleSelect = () => {
    if (onSelect) {
      onSelect(article);
      return;
    }
    setDetailOpen(true);
  };

  return (
    <>
      <div className={joinClassNames('article-row', selected && 'article-row--selected')}>
        <div className="toolbar" style={{ alignItems: 'flex-start' }}>
          {leading}
          <button className="article-row__button" type="button" onClick={handleSelect}>
            <div className="toolbar toolbar--wrap">
              <ArticleBadges article={article} />
              {isFavorite ? <Badge tone="amber">已收藏</Badge> : null}
            </div>
            <h3 className="article-row__title">{resolvedTitle}</h3>
            <div className="article-row__meta">
              <span>{getArticleVenue(article)}</span>
            </div>
            {resolvedPreview ? <p className="article-row__abstract">{resolvedPreview}</p> : null}
          </button>
          <IconButton
            aria-label="收藏"
            title="收藏"
            disabled={favoritePending}
            onClick={(e) => {
              e.stopPropagation();
              setFavoriteOpen(true);
            }}
          >
            <Star
              size={16}
              fill={isFavorite ? 'currentColor' : 'none'}
              color={isFavorite ? 'var(--amber)' : 'currentColor'}
            />
          </IconButton>
          {actions?.map((action) => {
            const Icon = action.icon;
            return (
              <IconButton
                key={action.label}
                aria-label={action.label}
                danger={action.tone === 'danger'}
                title={action.label}
                onClick={action.onClick}
              >
                {action.icon === Trash2 ? <Trash2 size={16} /> : <Icon size={16} />}
              </IconButton>
            );
          })}
        </div>
      </div>
      <ArticleDetailModal
        actions={actions}
        article={article}
        dbName={dbName}
        favoritePending={favoritePending}
        initialFolderIds={favoriteFolderIds}
        open={detailOpen}
        onClose={() => setDetailOpen(false)}
      />
      {favoriteOpen ? (
        <FavoritePicker
          articleId={article.article_id}
          dbName={dbName}
          initialFolderIds={favoriteFolderIds}
          onClose={() => setFavoriteOpen(false)}
          onSaved={setToastMessage}
        />
      ) : null}
      <FavoriteToast message={toastMessage} onClose={() => setToastMessage(null)} />
    </>
  );
}

interface ArticleDetailPanelProps {
  article: Article | null;
  dbName: string;
  favoriteFolderIds?: number[];
  favoritePending?: boolean;
  nextDisabled?: boolean;
  previousDisabled?: boolean;
  onClose?: () => void;
  onNext?: () => void;
  onPrevious?: () => void;
}

/**
 * Render a persistent right-side article detail panel.
 *
 * @param props - Detail panel props.
 * @returns Detail panel.
 */
export function ArticleDetailPanel({
  article,
  dbName,
  favoriteFolderIds = [],
  favoritePending = false,
  nextDisabled = false,
  onClose,
  onNext,
  onPrevious,
  previousDisabled = false,
}: ArticleDetailPanelProps) {
  const { token, user } = useAuthSession();
  const [copyState, setCopyState] = useState<'info' | 'title' | null>(null);
  const [favoriteOpen, setFavoriteOpen] = useState(false);
  const [toastMessage, setToastMessage] = useState<string | null>(null);

  const favoriteQueryKey = article
    ? (['favorite-check', user?.id, dbName, article.article_id] as const)
    : null;

  const { data: checks } = useQuery({
    queryKey: favoriteQueryKey ?? [],
    queryFn: () => checkFavorite(token!, article!.article_id, dbName),
    enabled: Boolean(token && user && dbName && article && favoriteQueryKey),
    initialData: article
      ? favoriteFolderIds.map((folderId) => ({ folder_id: folderId, folder_name: '' }))
      : undefined,
  });

  const activeFolderIds = checks?.map((item) => item.folder_id) ?? favoriteFolderIds;

  if (!article) {
    return (
      <Panel
        title="文献详情"
        meta="选择一篇文章查看详情"
        actions={
          onClose ? (
            <IconButton aria-label="关闭详情" title="关闭详情" onClick={onClose}>
              <X size={15} />
            </IconButton>
          ) : null
        }
      >
        <EmptyState>从中间列表选择一篇文献。</EmptyState>
      </Panel>
    );
  }

  const fullTextUrl = hasFullTextTarget(article)
    ? getFullTextUrl(article.article_id, dbName, token ?? undefined)
    : null;

  const runCopy = async (kind: 'title' | 'info', text: string) => {
    await copyText(text);
    setCopyState(kind);
    window.setTimeout(() => setCopyState(null), 1800);
  };

  return (
    <>
      <Panel
        title="文献详情"
        meta={getArticleVenue(article)}
        actions={
          <>
            <IconButton
              aria-label="上一条"
              disabled={previousDisabled}
              title="上一条"
              onClick={onPrevious}
            >
              <ChevronLeft size={15} />
            </IconButton>
            <IconButton aria-label="下一条" disabled={nextDisabled} title="下一条" onClick={onNext}>
              <ChevronRight size={15} />
            </IconButton>
            {onClose ? (
              <IconButton aria-label="关闭详情" title="关闭详情" onClick={onClose}>
                <X size={15} />
              </IconButton>
            ) : null}
          </>
        }
      >
        <div className="form-grid">
          <div className="toolbar toolbar--wrap">
            <ArticleBadges article={article} />
            {activeFolderIds.length > 0 ? <Badge tone="amber">已收藏</Badge> : null}
          </div>
          <h2 className="detail-panel__title">{getArticleTitle(article)}</h2>
          {article.authors ? <p className="panel__meta">{article.authors}</p> : null}
          <section>
            <h3 className="panel__title">摘要</h3>
            <p className="detail-panel__abstract">{article.abstract || '暂无摘要。'}</p>
          </section>
          <div className="detail-panel__actions">
            <Button
              icon={copyState === 'info' ? <Check size={15} /> : <Copy size={15} />}
              variant="secondary"
              onClick={() => void runCopy('info', buildArticleClipboardText(article))}
            >
              {copyState === 'info' ? '已复制' : '复制信息'}
            </Button>
            {fullTextUrl ? (
              <a href={fullTextUrl} rel="noreferrer" target="_blank">
                <Button icon={<ExternalLink size={15} />}>查看全文</Button>
              </a>
            ) : null}
            <Button
              disabled={favoritePending}
              icon={<Star size={15} />}
              variant="danger"
              onClick={() => setFavoriteOpen(true)}
            >
              收藏
            </Button>
          </div>
        </div>
      </Panel>
      {favoriteOpen ? (
        <FavoritePicker
          articleId={article.article_id}
          dbName={dbName}
          initialFolderIds={activeFolderIds}
          onClose={() => setFavoriteOpen(false)}
          onSaved={setToastMessage}
        />
      ) : null}
      <FavoriteToast message={toastMessage} onClose={() => setToastMessage(null)} />
    </>
  );
}

interface FavoriteStateOptions {
  articleIds: ArticleId[];
  dbName: string;
}

/**
 * Build a stable key for a batch of article ids.
 *
 * @param articleIds - Article ids.
 * @returns Joined key.
 */
export function buildArticleIdsKey(articleIds: ArticleId[]): string {
  return articleIds.join(',');
}

/**
 * Memoize a mapping of favorite ids for visible articles.
 *
 * @param options - Favorite state options.
 * @returns Favorite query key parts.
 */
export function useFavoriteBatchIdentity({ articleIds, dbName }: FavoriteStateOptions) {
  return useMemo(
    () => ({
      articleIdsKey: buildArticleIdsKey(articleIds),
      queryKey: ['favorite-batch', dbName, buildArticleIdsKey(articleIds)] as const,
    }),
    [articleIds, dbName],
  );
}
