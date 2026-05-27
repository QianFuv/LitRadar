'use client';

/**
 * Desktop favorites workspace.
 */

import { useInfiniteQuery, useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Download, FolderPlus, Pencil, Radar, Star, Trash2 } from 'lucide-react';
import { useMemo, useState } from 'react';
import { ArticleCard } from '@/components/desktop/article-tools';
import { ShellConfigurator } from '@/components/desktop/shell';
import {
  Badge,
  Button,
  EmptyState,
  Field,
  IconButton,
  Notice,
  Panel,
  SelectInput,
  Skeleton,
  TextInput,
} from '@/components/desktop/ui';
import {
  bulkMoveFavorites,
  bulkRemoveFavorites,
  createFolder,
  deleteFolder,
  getExportUrl,
  getFolderArticles,
  getFolders,
  removeFavorite,
  renameFolder,
  setTrackingFolder,
  type CitationFormat,
  type FavoriteArticleItem,
  type FavoriteArticleRef,
} from '@/lib/client-api';
import { useAuthSession } from '@/lib/auth-session';

const PAGE_SIZE = 100;

/**
 * Build a stable selection key for a favorite item.
 *
 * @param item - Favorite article.
 * @returns Selection key.
 */
function getSelectionKey(item: FavoriteArticleItem): string {
  return `${item.folder_id}:${item.article_id}:${item.db_name}`;
}

/**
 * Convert a favorite article into a backend article ref.
 *
 * @param item - Favorite article.
 * @returns Article reference.
 */
function toArticleRef(item: FavoriteArticleItem): FavoriteArticleRef {
  return { article_id: item.article_id, db_name: item.db_name };
}

/**
 * Render the favorites workspace.
 *
 * @returns Favorites workspace.
 */
export function FavoritesWorkspace() {
  const { token, user } = useAuthSession();
  const queryClient = useQueryClient();
  const [selectedFolderId, setSelectedFolderId] = useState<number | null>(null);
  const [newFolderName, setNewFolderName] = useState('');
  const [renamingFolderId, setRenamingFolderId] = useState<number | null>(null);
  const [renameDraft, setRenameDraft] = useState('');
  const [exportFormat, setExportFormat] = useState<CitationFormat>('bibtex');
  const [selectedKeys, setSelectedKeys] = useState<string[]>([]);
  const [moveTargetId, setMoveTargetId] = useState('');
  const [feedback, setFeedback] = useState<string | null>(null);

  const foldersQuery = useQuery({
    queryKey: ['folders', user?.id],
    queryFn: () => getFolders(token!),
    enabled: Boolean(token),
  });
  const folders = foldersQuery.data ?? [];
  const activeFolder =
    folders.find((folder) => folder.id === selectedFolderId) ??
    folders.find((folder) => folder.is_tracking) ??
    folders[0] ??
    null;

  const articlesQuery = useInfiniteQuery({
    queryKey: ['folder-articles', activeFolder?.id],
    queryFn: ({ pageParam }) => getFolderArticles(token!, activeFolder!.id, PAGE_SIZE, pageParam),
    initialPageParam: 0,
    getNextPageParam: (lastPage, allPages) =>
      lastPage.length === PAGE_SIZE ? allPages.flat().length : undefined,
    enabled: Boolean(token && activeFolder),
  });

  const articles = articlesQuery.data?.pages.flat() ?? [];
  const selectedKeySet = useMemo(() => new Set(selectedKeys), [selectedKeys]);
  const selectedArticles = articles.filter((article) =>
    selectedKeySet.has(getSelectionKey(article)),
  );
  const targetFolders = folders.filter((folder) => folder.id !== activeFolder?.id);
  const validMoveTarget = targetFolders.some((folder) => String(folder.id) === moveTargetId)
    ? moveTargetId
    : '';

  const createMutation = useMutation({
    mutationFn: (name: string) => createFolder(token!, name),
    onSuccess: (folder) => {
      setNewFolderName('');
      setSelectedFolderId(folder.id);
      queryClient.invalidateQueries({ queryKey: ['folders'] });
    },
  });

  const renameMutation = useMutation({
    mutationFn: ({ folderId, name }: { folderId: number; name: string }) =>
      renameFolder(token!, folderId, name),
    onSuccess: () => {
      setRenamingFolderId(null);
      setRenameDraft('');
      queryClient.invalidateQueries({ queryKey: ['folders'] });
    },
  });

  const deleteMutation = useMutation({
    mutationFn: (folderId: number) => deleteFolder(token!, folderId),
    onSuccess: () => {
      setSelectedFolderId(null);
      setSelectedKeys([]);
      queryClient.invalidateQueries({ queryKey: ['folders'] });
    },
  });

  const trackMutation = useMutation({
    mutationFn: (folderId: number) => setTrackingFolder(token!, folderId),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['folders'] }),
  });

  const removeMutation = useMutation({
    mutationFn: (item: FavoriteArticleItem) =>
      removeFavorite(token!, item.folder_id, item.article_id, item.db_name),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['folder-articles'] });
      queryClient.invalidateQueries({ queryKey: ['folders'] });
    },
  });

  const bulkRemoveMutation = useMutation({
    mutationFn: (items: FavoriteArticleRef[]) =>
      bulkRemoveFavorites(token!, activeFolder!.id, items),
    onSuccess: (count) => {
      setSelectedKeys([]);
      setFeedback(`已移除 ${count} 篇文章。`);
      queryClient.invalidateQueries({ queryKey: ['folder-articles'] });
      queryClient.invalidateQueries({ queryKey: ['folders'] });
    },
  });

  const bulkMoveMutation = useMutation({
    mutationFn: ({
      refs,
      targetFolderId,
    }: {
      refs: FavoriteArticleRef[];
      targetFolderId: number;
    }) => bulkMoveFavorites(token!, activeFolder!.id, targetFolderId, refs),
    onSuccess: (count) => {
      setSelectedKeys([]);
      setMoveTargetId('');
      setFeedback(`已移动 ${count} 篇文章。`);
      queryClient.invalidateQueries({ queryKey: ['folder-articles'] });
      queryClient.invalidateQueries({ queryKey: ['folders'] });
    },
  });

  const toggleSelection = (article: FavoriteArticleItem, checked: boolean) => {
    const key = getSelectionKey(article);
    setSelectedKeys((current) => {
      if (checked) {
        return current.includes(key) ? current : [...current, key];
      }
      return current.filter((item) => item !== key);
    });
    setFeedback(null);
  };

  const toggleAll = (checked: boolean) => {
    setSelectedKeys(checked ? articles.map(getSelectionKey) : []);
    setFeedback(null);
  };

  return (
    <>
      <ShellConfigurator
        kicker="Library"
        title="我的收藏"
        actions={
          activeFolder ? (
            <>
              <Badge tone="violet">{activeFolder.name}</Badge>
              <Badge tone="teal">{activeFolder.article_count} 篇</Badge>
            </>
          ) : null
        }
      />
      <div className="workspace-grid workspace-grid--search">
        <Panel title="收藏夹" meta="创建、重命名、追踪">
          <div className="form-grid">
            <div className="form-grid form-grid--two">
              <Field label="新建收藏夹">
                <TextInput
                  value={newFolderName}
                  onChange={(event) => setNewFolderName(event.target.value)}
                  placeholder="文件夹名称"
                />
              </Field>
              <div className="field">
                <span className="field__label">操作</span>
                <Button
                  icon={<FolderPlus size={15} />}
                  disabled={!newFolderName.trim() || createMutation.isPending}
                  onClick={() => createMutation.mutate(newFolderName.trim())}
                >
                  创建
                </Button>
              </div>
            </div>
            {foldersQuery.isPending ? (
              <Skeleton className="h-32" />
            ) : folders.length === 0 ? (
              <EmptyState>暂无收藏夹。</EmptyState>
            ) : (
              <div className="list-stack">
                {folders.map((folder) => {
                  const active = activeFolder?.id === folder.id;
                  const renaming = renamingFolderId === folder.id;
                  return (
                    <div
                      key={folder.id}
                      className="article-row"
                      style={{
                        borderColor: active ? 'var(--teal)' : undefined,
                        background: active ? 'var(--teal-soft)' : undefined,
                      }}
                    >
                      {renaming ? (
                        <div className="toolbar">
                          <TextInput
                            value={renameDraft}
                            onChange={(event) => setRenameDraft(event.target.value)}
                          />
                          <Button
                            size="small"
                            onClick={() =>
                              renameMutation.mutate({
                                folderId: folder.id,
                                name: renameDraft.trim(),
                              })
                            }
                          >
                            保存
                          </Button>
                        </div>
                      ) : (
                        <button
                          className="article-row__button"
                          type="button"
                          onClick={() => {
                            setSelectedFolderId(folder.id);
                            setSelectedKeys([]);
                            setFeedback(null);
                          }}
                        >
                          <div className="toolbar toolbar--wrap">
                            <Star size={16} />
                            <strong>{folder.name}</strong>
                            {folder.is_tracking ? <Badge tone="violet">追踪</Badge> : null}
                            <span className="panel__meta">{folder.article_count} 篇</span>
                          </div>
                        </button>
                      )}
                      <div className="toolbar">
                        <IconButton
                          aria-label="设为追踪"
                          title="设为追踪"
                          onClick={() => trackMutation.mutate(folder.id)}
                        >
                          <Radar size={15} />
                        </IconButton>
                        <IconButton
                          aria-label="重命名"
                          title="重命名"
                          onClick={() => {
                            setRenamingFolderId(folder.id);
                            setRenameDraft(folder.name);
                          }}
                        >
                          <Pencil size={15} />
                        </IconButton>
                        <IconButton
                          danger
                          aria-label="删除"
                          title="删除"
                          onClick={() => deleteMutation.mutate(folder.id)}
                        >
                          <Trash2 size={15} />
                        </IconButton>
                      </div>
                    </div>
                  );
                })}
              </div>
            )}
          </div>
        </Panel>

        <Panel
          title={activeFolder?.name ?? '收藏文章'}
          meta={
            selectedArticles.length > 0
              ? `已选 ${selectedArticles.length} 篇`
              : activeFolder
                ? `${activeFolder.article_count} 篇文章`
                : '请选择收藏夹'
          }
          actions={
            activeFolder ? (
              <div className="toolbar toolbar--wrap">
                <label className="chip">
                  <input
                    type="checkbox"
                    checked={articles.length > 0 && selectedArticles.length === articles.length}
                    onChange={(event) => toggleAll(event.currentTarget.checked)}
                  />
                  全选当前列表
                </label>
                <SelectInput
                  value={validMoveTarget}
                  onChange={(event) => setMoveTargetId(event.target.value)}
                  style={{ width: 170 }}
                >
                  <option value="">移动到...</option>
                  {targetFolders.map((folder) => (
                    <option key={folder.id} value={folder.id}>
                      {folder.name}
                    </option>
                  ))}
                </SelectInput>
                <Button
                  size="small"
                  variant="secondary"
                  disabled={!validMoveTarget || selectedArticles.length === 0}
                  onClick={() =>
                    bulkMoveMutation.mutate({
                      targetFolderId: Number(validMoveTarget),
                      refs: selectedArticles.map(toArticleRef),
                    })
                  }
                >
                  移动所选
                </Button>
                <Button
                  size="small"
                  variant="danger"
                  disabled={selectedArticles.length === 0}
                  onClick={() => bulkRemoveMutation.mutate(selectedArticles.map(toArticleRef))}
                >
                  删除所选
                </Button>
              </div>
            ) : null
          }
        >
          {feedback ? <Notice>{feedback}</Notice> : null}
          {!activeFolder ? (
            <EmptyState>请选择或创建收藏夹。</EmptyState>
          ) : articlesQuery.isPending ? (
            <div className="list-stack">
              <Skeleton className="h-28" />
              <Skeleton className="h-28" />
            </div>
          ) : articles.length === 0 ? (
            <EmptyState>此收藏夹为空。</EmptyState>
          ) : (
            <div className="list-stack scroll-region scroll-region--results">
              {articles.map((article) => (
                <ArticleCard
                  key={article.id}
                  article={article}
                  dbName={article.db_name}
                  favoriteFolderIds={[article.folder_id]}
                  leading={
                    <input
                      type="checkbox"
                      checked={selectedKeySet.has(getSelectionKey(article))}
                      onChange={(event) => toggleSelection(article, event.currentTarget.checked)}
                    />
                  }
                  actions={[
                    {
                      icon: Trash2,
                      label: '移除收藏',
                      tone: 'danger',
                      onClick: () => removeMutation.mutate(article),
                    },
                  ]}
                />
              ))}
              {articlesQuery.hasNextPage ? (
                <Button
                  variant="secondary"
                  disabled={articlesQuery.isFetchingNextPage}
                  onClick={() => void articlesQuery.fetchNextPage()}
                >
                  {articlesQuery.isFetchingNextPage ? '加载中...' : '继续加载'}
                </Button>
              ) : null}
            </div>
          )}
        </Panel>

        <Panel title="导出与队列操作" meta="引用格式和收藏夹状态">
          {activeFolder ? (
            <div className="form-grid">
              <Field label="导出格式">
                <SelectInput
                  value={exportFormat}
                  onChange={(event) => setExportFormat(event.target.value as CitationFormat)}
                >
                  <option value="bibtex">BibTeX</option>
                  <option value="ris">RIS</option>
                  <option value="endnote">EndNote XML</option>
                </SelectInput>
              </Field>
              <a href={getExportUrl(token!, activeFolder.id, exportFormat)} download>
                <Button icon={<Download size={15} />} wide>
                  导出引用
                </Button>
              </a>
              <Notice>批量移动和删除只作用于当前已加载列表中的选中文章。</Notice>
              {activeFolder.is_tracking ? (
                <Badge tone="violet">当前为追踪文件夹</Badge>
              ) : (
                <Button
                  icon={<Radar size={15} />}
                  variant="secondary"
                  onClick={() => trackMutation.mutate(activeFolder.id)}
                >
                  设为追踪文件夹
                </Button>
              )}
            </div>
          ) : (
            <EmptyState>选择收藏夹后可导出引用。</EmptyState>
          )}
        </Panel>
      </div>
    </>
  );
}
