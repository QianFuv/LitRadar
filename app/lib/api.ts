export interface PageMeta {
  total: number | null;
  limit: number;
  offset: number;
  next_cursor?: string | null;
  has_more?: boolean | null;
}

export interface Article {
  article_id: number;
  journal_id: number;
  issue_id?: number;
  title?: string;
  date?: string;
  authors?: string;
  abstract?: string;
  doi?: string;
  platform_id?: string;
  journal_title?: string;
  open_access?: number;
  in_press?: number;
  volume?: string;
  number?: string;
  full_text_file?: string;
}

export interface ArticlePage {
  items: Article[];
  page: PageMeta;
}

export interface ValueCount {
  value: string;
  count: number;
}

export interface YearSummary {
  year: number;
  issue_count: number;
  journal_count: number;
}

export interface JournalOption {
  journal_id: number;
  title?: string;
}

export interface WeeklyArticle {
  article_id: number;
  journal_id: number;
  issue_id?: number;
  title?: string;
  date?: string;
  doi?: string;
  journal_title?: string;
  open_access?: number;
  in_press?: number;
}

export interface WeeklyJournalUpdate {
  journal_id: number;
  journal_title?: string;
  new_article_count: number;
  articles: WeeklyArticle[];
}

export interface WeeklyDatabaseUpdate {
  db_name: string;
  run_id?: string;
  generated_at: string;
  new_article_count: number;
  journals: WeeklyJournalUpdate[];
}

export interface WeeklyUpdatesResponse {
  generated_at: string;
  window_start: string;
  window_end: string;
  databases: WeeklyDatabaseUpdate[];
}

const API_BASE_URL = process.env.NEXT_PUBLIC_API_URL || '';

function resolveBase(): string {
    if (API_BASE_URL) return API_BASE_URL;
    if (typeof window !== 'undefined') return window.location.origin;
    return 'http://localhost:8000';
}
export const DEFAULT_DB = 'utd24.sqlite';
const DB_STORAGE_KEY = 'selected_database';

function getStoredDatabase(): string {
    if (typeof window !== 'undefined') {
        return localStorage.getItem(DB_STORAGE_KEY) || DEFAULT_DB;
    }
    return DEFAULT_DB;
}

let currentDb = getStoredDatabase();

export function setDatabase(db: string) {
    currentDb = db;
    if (typeof window !== 'undefined') {
        localStorage.setItem(DB_STORAGE_KEY, db);
    }
}

export function getCurrentDatabase() {
    return currentDb;
}

export function getFullTextUrl(articleId: number): string {
    return withDb(`/api/articles/${articleId}/fulltext`);
}

export function getFullTextUrlForDatabase(articleId: number, dbName: string): string {
    const url = new URL(`/api/articles/${articleId}/fulltext`, resolveBase());
    url.searchParams.set('db', dbName);
    return url.toString();
}

function withDb(url: string, params?: URLSearchParams): string {
    const urlObj = new URL(url, resolveBase());
    const p = urlObj.searchParams;
    
    // Merge provided params
    if (params) {
        params.forEach((value, key) => {
            p.append(key, value);
        });
    }

    // Set DB if not present
    if (!p.has('db')) {
        p.set('db', currentDb);
    }
    return urlObj.toString();
}

export async function getDatabases(): Promise<string[]> {
    const res = await fetch(`${resolveBase()}/api/meta/databases`);
    if (!res.ok) {
        return [DEFAULT_DB];
    }
    return res.json();
}

export async function getArticles(
  params: URLSearchParams,
  pageParam: string | number | null = null,
  includeTotal: boolean = false,
): Promise<ArticlePage> {
  const newParams = new URLSearchParams(params);
  const shouldIncludeTotal = includeTotal && (pageParam === null || pageParam === 0);

  if (typeof pageParam === 'string' && pageParam.length > 0) {
    newParams.set('cursor', pageParam);
    newParams.delete('offset');
  } else if (typeof pageParam === 'number') {
    newParams.set('offset', pageParam.toString());
  }
  newParams.set('include_total', shouldIncludeTotal ? '1' : '0');

  const res = await fetch(withDb('/api/articles', newParams));
  if (!res.ok) {
    throw new Error('获取文章失败');
  }
  return res.json();
}

export async function getAreas(): Promise<ValueCount[]> {
  const res = await fetch(withDb('/api/meta/areas'));
  if (!res.ok) {
    throw new Error('获取领域失败');
  }
  return res.json();
}

export async function getYears(): Promise<YearSummary[]> {
    const res = await fetch(withDb('/api/years'));
    if (!res.ok) {
      throw new Error('获取年份失败');
    }
    return res.json();
  }

export async function getJournalOptions(): Promise<JournalOption[]> {
  const res = await fetch(withDb('/api/meta/journals'));
  if (!res.ok) {
    throw new Error('获取期刊失败');
  }
  return res.json();
}

export async function getWeeklyUpdates(windowDays: number = 7): Promise<WeeklyUpdatesResponse> {
  const params = new URLSearchParams();
  params.set('window_days', String(windowDays));
  const url = new URL('/api/weekly-updates', resolveBase());
  url.search = params.toString();
  const res = await fetch(url.toString());
  if (!res.ok) {
    throw new Error('获取每周更新失败');
  }
  return res.json();
}

export async function getArticleById(articleId: number, dbName: string): Promise<Article> {
  const url = new URL(`/api/articles/${articleId}`, resolveBase());
  url.searchParams.set('db', dbName);
  const res = await fetch(url.toString());
  if (!res.ok) {
    throw new Error('获取文章详情失败');
  }
  return res.json();
}

// ── Auth helpers ─────────────────────────────────────────────────

function authFetch(url: string, token: string, init?: RequestInit): Promise<Response> {
  return fetch(url, {
    ...init,
    headers: {
      'Content-Type': 'application/json',
      Authorization: `Bearer ${token}`,
      ...(init?.headers || {}),
    },
  });
}

// ── Folder types ─────────────────────────────────────────────────

export interface Folder {
  id: number;
  name: string;
  is_tracking: boolean;
  article_count: number;
  created_at: number;
}

export interface FavoriteItem {
  id: number;
  folder_id: number;
  article_id: number;
  db_name: string;
  note: string;
  created_at: number;
}

export interface FavoriteArticleItem extends FavoriteItem {
  journal_id?: number | null;
  issue_id?: number | null;
  title?: string;
  date?: string;
  authors?: string;
  abstract?: string;
  doi?: string;
  platform_id?: string;
  journal_title?: string;
  open_access?: number;
  in_press?: number;
  volume?: string;
  number?: string;
  issn?: string | null;
  eissn?: string | null;
  full_text_file?: string;
}

export type CitationFormat = 'bibtex' | 'ris' | 'endnote';

export interface FavoriteCheck {
  folder_id: number;
  folder_name: string;
}

export interface FavoriteBatchCheckItem {
  article_id: number;
  folders: FavoriteCheck[];
}

export interface AccessToken {
  id: number;
  name: string;
  expires_at: number;
  created_at: number;
}

export interface TrackingStatus {
  tracking_folder: { id: number; name: string } | null;
  total_folders: number;
  weekly_articles_available: number;
  notification_configured: boolean;
}

export interface NotificationSettings {
  id: number;
  user_id: number;
  keywords: string[];
  directions: string[];
  delivery_method: 'folder' | 'pushplus';
  pushplus_token: string;
  pushplus_template: string;
  pushplus_topic: string;
  pushplus_to: string;
  ai_base_url: string;
  ai_api_key: string;
  ai_model: string;
  ai_system_prompt: string;
  enabled: boolean;
  created_at: number;
  updated_at: number;
}

export interface NotificationSettingsUpdate {
  keywords: string[];
  directions: string[];
  delivery_method: 'folder' | 'pushplus';
  pushplus_token: string;
  pushplus_template: string;
  pushplus_topic: string;
  pushplus_to: string;
  ai_base_url: string;
  ai_api_key: string;
  ai_model: string;
  ai_system_prompt: string;
  enabled: boolean;
}

// ── Folder API ───────────────────────────────────────────────────

export async function getFolders(token: string): Promise<Folder[]> {
  const res = await authFetch(`${resolveBase()}/api/favorites/folders`, token);
  if (!res.ok) throw new Error('获取收藏夹失败');
  return res.json();
}

export async function createFolder(token: string, name: string, isTracking = false): Promise<Folder> {
  const res = await authFetch(`${resolveBase()}/api/favorites/folders`, token, {
    method: 'POST',
    body: JSON.stringify({ name, is_tracking: isTracking }),
  });
  if (!res.ok) {
    const err = await res.json().catch(() => ({}));
    throw new Error(err.detail || '创建收藏夹失败');
  }
  return res.json();
}

export async function renameFolder(token: string, folderId: number, name: string): Promise<void> {
  const res = await authFetch(`${resolveBase()}/api/favorites/folders/${folderId}`, token, {
    method: 'PUT',
    body: JSON.stringify({ name }),
  });
  if (!res.ok) throw new Error('重命名收藏夹失败');
}

export async function deleteFolder(token: string, folderId: number): Promise<void> {
  const res = await authFetch(`${resolveBase()}/api/favorites/folders/${folderId}`, token, {
    method: 'DELETE',
  });
  if (!res.ok) throw new Error('删除收藏夹失败');
}

// ── Favorites API ────────────────────────────────────────────────

export async function getFolderArticles(
  token: string, folderId: number, limit = 100, offset = 0
): Promise<FavoriteArticleItem[]> {
  const url = new URL(`/api/favorites/folders/${folderId}/articles`, resolveBase());
  url.searchParams.set('limit', String(limit));
  url.searchParams.set('offset', String(offset));
  const res = await authFetch(url.toString(), token);
  if (!res.ok) throw new Error('获取收藏文章失败');
  return res.json();
}

export async function addFavorite(
  token: string, folderId: number, articleId: number, dbName: string, note = ''
): Promise<FavoriteItem> {
  const res = await authFetch(`${resolveBase()}/api/favorites/folders/${folderId}/articles`, token, {
    method: 'POST',
    body: JSON.stringify({ article_id: articleId, db_name: dbName, note }),
  });
  if (!res.ok) throw new Error('添加收藏失败');
  return res.json();
}

export async function removeFavorite(
  token: string, folderId: number, articleId: number, dbName = ''
): Promise<void> {
  const url = new URL(`/api/favorites/folders/${folderId}/articles/${articleId}`, resolveBase());
  url.searchParams.set('db_name', dbName);
  const res = await authFetch(url.toString(), token, { method: 'DELETE' });
  if (!res.ok) throw new Error('移除收藏失败');
}

export function getExportUrl(
  token: string,
  folderId: number,
  format: CitationFormat,
): string {
  const url = new URL(`/api/favorites/folders/${folderId}/export`, resolveBase());
  url.searchParams.set('format', format);
  url.searchParams.set('access_token', token);
  return url.toString();
}

export async function checkFavorite(
  token: string, articleId: number, dbName = ''
): Promise<FavoriteCheck[]> {
  const url = new URL('/api/favorites/check', resolveBase());
  url.searchParams.set('article_id', String(articleId));
  url.searchParams.set('db_name', dbName);
  const res = await authFetch(url.toString(), token);
  if (!res.ok) return [];
  return res.json();
}

export async function checkFavoritesBatch(
  token: string, articleIds: number[], dbName = ''
): Promise<Record<number, FavoriteCheck[]>> {
  if (articleIds.length === 0) {
    return {};
  }
  const res = await authFetch(`${resolveBase()}/api/favorites/check/batch`, token, {
    method: 'POST',
    body: JSON.stringify({ article_ids: articleIds, db_name: dbName }),
  });
  if (!res.ok) return {};
  const data: FavoriteBatchCheckItem[] = await res.json();
  return Object.fromEntries(data.map((item) => [item.article_id, item.folders]));
}

// ── Tracking API ─────────────────────────────────────────────────

export async function getTrackingStatus(token: string): Promise<TrackingStatus> {
  const res = await authFetch(`${resolveBase()}/api/tracking/status`, token);
  if (!res.ok) throw new Error('获取追踪状态失败');
  return res.json();
}

export async function setTrackingFolder(token: string, folderId: number): Promise<void> {
  const res = await authFetch(`${resolveBase()}/api/favorites/tracking`, token, {
    method: 'PUT',
    body: JSON.stringify({ folder_id: folderId }),
  });
  if (!res.ok) throw new Error('设置追踪文件夹失败');
}

export async function pushWeeklyToTracking(
  token: string,
): Promise<{ pushed: number; message?: string }> {
  const res = await authFetch(`${resolveBase()}/api/tracking/push-weekly`, token, {
    method: 'POST',
  });
  if (!res.ok) {
    const err = await res.json().catch(() => ({}));
    throw new Error(err.detail || '推送每周文章失败');
  }
  return res.json();
}

export async function getNotificationSettings(token: string): Promise<NotificationSettings | null> {
  const res = await authFetch(`${resolveBase()}/api/tracking/notification-settings`, token);
  if (!res.ok) throw new Error('获取通知设置失败');
  const data = await res.json();
  return data || null;
}

export async function updateNotificationSettings(
  token: string,
  settings: NotificationSettingsUpdate,
): Promise<NotificationSettings> {
  const res = await authFetch(`${resolveBase()}/api/tracking/notification-settings`, token, {
    method: 'PUT',
    body: JSON.stringify(settings),
  });
  if (!res.ok) {
    const err = await res.json().catch(() => ({}));
    throw new Error(err.detail || '更新通知设置失败');
  }
  return res.json();
}

// ── Access Token API ─────────────────────────────────────────────

export async function getAccessTokens(token: string): Promise<AccessToken[]> {
  const res = await authFetch(`${resolveBase()}/api/auth/tokens`, token);
  if (!res.ok) throw new Error('获取访问令牌失败');
  return res.json();
}

export async function createAccessToken(
  token: string, name: string, ttl: number
): Promise<{ id: number; token: string; name: string; expires_at: number }> {
  const res = await authFetch(`${resolveBase()}/api/auth/tokens`, token, {
    method: 'POST',
    body: JSON.stringify({ name, ttl }),
  });
  if (!res.ok) throw new Error('创建访问令牌失败');
  return res.json();
}

export async function revokeAccessToken(token: string, tokenId: number): Promise<void> {
  const res = await authFetch(`${resolveBase()}/api/auth/tokens/${tokenId}`, token, {
    method: 'DELETE',
  });
  if (!res.ok) throw new Error('撤销访问令牌失败');
}

export async function changePassword(
  token: string, oldPassword: string, newPassword: string
): Promise<void> {
  const res = await authFetch(`${resolveBase()}/api/auth/change-password`, token, {
    method: 'POST',
    body: JSON.stringify({ old_password: oldPassword, new_password: newPassword }),
  });
  if (!res.ok) {
    const err = await res.json().catch(() => ({}));
    throw new Error(err.detail || '修改密码失败');
  }
}

// ── Invite Code API ──────────────────────────────────────────────

export interface InviteCode {
  id: number;
  code: string;
  used: boolean;
  created_at: number;
}

export async function getInviteCode(token: string): Promise<InviteCode | null> {
  const res = await authFetch(`${resolveBase()}/api/auth/invite-code`, token);
  if (!res.ok) throw new Error('获取邀请码失败');
  const data = await res.json();
  return data || null;
}

export async function generateInviteCode(token: string): Promise<InviteCode> {
  const res = await authFetch(`${resolveBase()}/api/auth/invite-code`, token, {
    method: 'POST',
  });
  if (!res.ok) {
    const err = await res.json().catch(() => ({}));
    throw new Error(err.detail || '生成邀请码失败');
  }
  return res.json();
}

// ── Admin API ────────────────────────────────────────────────────

export interface AdminUserInfo {
  id: number;
  username: string;
  is_admin: boolean;
  created_at: number;
  updated_at: number;
  folder_count: number;
  favorite_count: number;
  notify_enabled: boolean;
}

export interface AdminInviteCode {
  id: number;
  code: string;
  created_by: number | null;
  created_by_name: string | null;
  used_by: number | null;
  used_by_name: string | null;
  used_at: number | null;
  created_at: number;
}

export interface IndexDbStats {
  db_name: string;
  articles: number;
  journals: number;
  issues: number;
  error?: boolean;
}

export interface PushDbStats {
  db_name: string;
  status: string;
  last_completed?: string | null;
  delivered_count?: number;
  user_results?: number;
}

export interface AdminStats {
  auth: {
    total_users: number;
    admin_count: number;
    total_folders: number;
    total_favorites: number;
    total_invite_codes: number;
    used_invite_codes: number;
    unused_invite_codes: number;
    active_tokens: number;
    notification_subscribers: number;
  };
  index: {
    databases: IndexDbStats[];
    total_articles: number;
    total_journals: number;
  };
  push: PushDbStats[];
}

export async function adminGetUsers(token: string): Promise<AdminUserInfo[]> {
  const res = await authFetch(`${resolveBase()}/api/admin/users`, token);
  if (!res.ok) throw new Error('获取用户列表失败');
  return res.json();
}

export async function adminSetAdmin(
  token: string, userId: number, isAdmin: boolean,
): Promise<void> {
  const res = await authFetch(`${resolveBase()}/api/admin/users/${userId}/admin`, token, {
    method: 'PUT',
    body: JSON.stringify({ is_admin: isAdmin }),
  });
  if (!res.ok) {
    const err = await res.json().catch(() => ({}));
    throw new Error(err.detail || '更新管理员状态失败');
  }
}

export async function adminResetPassword(
  token: string, userId: number, newPassword: string,
): Promise<void> {
  const res = await authFetch(`${resolveBase()}/api/admin/users/${userId}/reset-password`, token, {
    method: 'POST',
    body: JSON.stringify({ new_password: newPassword }),
  });
  if (!res.ok) {
    const err = await res.json().catch(() => ({}));
    throw new Error(err.detail || '重置密码失败');
  }
}

export async function adminDeleteUser(token: string, userId: number): Promise<void> {
  const res = await authFetch(`${resolveBase()}/api/admin/users/${userId}`, token, {
    method: 'DELETE',
  });
  if (!res.ok) {
    const err = await res.json().catch(() => ({}));
    throw new Error(err.detail || '删除用户失败');
  }
}

export async function adminGetInviteCodes(token: string): Promise<AdminInviteCode[]> {
  const res = await authFetch(`${resolveBase()}/api/admin/invite-codes`, token);
  if (!res.ok) throw new Error('获取邀请码列表失败');
  return res.json();
}

export async function adminCreateInviteCode(token: string): Promise<{ id: number; code: string }> {
  const res = await authFetch(`${resolveBase()}/api/admin/invite-codes`, token, {
    method: 'POST',
  });
  if (!res.ok) throw new Error('创建邀请码失败');
  return res.json();
}

export async function adminDeleteInviteCode(token: string, codeId: number): Promise<void> {
  const res = await authFetch(`${resolveBase()}/api/admin/invite-codes/${codeId}`, token, {
    method: 'DELETE',
  });
  if (!res.ok) {
    const err = await res.json().catch(() => ({}));
    throw new Error(err.detail || '删除邀请码失败');
  }
}

export async function adminGetStats(token: string): Promise<AdminStats> {
  const res = await authFetch(`${resolveBase()}/api/admin/stats`, token);
  if (!res.ok) throw new Error('获取统计信息失败');
  return res.json();
}
