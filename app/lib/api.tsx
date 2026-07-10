/**
 * Typed browser API client for the Paper Scanner backend.
 */

import {
  readLocalStorageValue,
  removeLocalStorageValue,
  writeLocalStorageValue,
} from '@/lib/browser-storage';
import {
  parseAuthUser,
  parseInviteRequirement,
  parseLoginResponse,
  parseManualPushStatus,
  parseNotificationSettings,
  parseNullableNotificationSettings,
  parseRuntimeSettingList,
  parseScheduledTaskInfo,
  parseScheduledTaskList,
  parseTrackingStatus,
  type AuthUser,
  type ContractParser,
  type InviteRequirement,
  type LoginResponse,
  type ManualPushStatus,
  type NotificationSettings,
  type NotificationSettingsUpdate,
  type RuntimeSettingInfo,
  type RuntimeSettingsUpdate,
  type ScheduledTaskCreate,
  type ScheduledTaskInfo,
  type ScheduledTaskUpdate,
  type TrackingStatus,
} from '@/lib/api-contract';

export type {
  AuthUser,
  InviteRequirement,
  LoginResponse,
  ManualPushStatus,
  NotificationSettings,
  NotificationSettingsUpdate,
  RuntimeSettingInfo,
  RuntimeSettingsUpdate,
  ScheduledJobSpec,
  ScheduledTaskCreate,
  ScheduledTaskInfo,
  ScheduledTaskUpdate,
  TrackingStatus,
} from '@/lib/api-contract';

export type ArticleId = string;

export type JournalId = string;

export interface PageMeta {
  total: number | null;
  limit: number;
  offset: number;
  next_cursor?: string | null;
  has_more?: boolean | null;
}

export interface Article {
  article_id: ArticleId;
  journal_id?: JournalId | null;
  issue_id?: number | null;
  title?: string | null;
  date?: string | null;
  authors?: string | null;
  abstract?: string | null;
  doi?: string | null;
  platform_id?: string | null;
  permalink?: string | null;
  journal_title?: string | null;
  open_access?: number | boolean | null;
  in_press?: number | boolean | null;
  volume?: string | null;
  number?: string | null;
  full_text_file?: string | null;
}

export interface ArticlePage {
  items: Article[];
  page: PageMeta;
}

export interface ArticleAccessAction {
  available: boolean;
  label: string;
  provider?: string | null;
  url?: string | null;
  requires_login: boolean;
  message?: string | null;
}

export interface ArticleAccessResponse {
  detail: ArticleAccessAction;
  fulltext: ArticleAccessAction;
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
  journal_id: JournalId;
  title?: string;
}

export type WeeklyArticle = Article;

export interface WeeklyJournalUpdate {
  journal_id: JournalId;
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

export interface AnnouncementInfo {
  id: number;
  title: string;
  message: string;
  priority: 'high' | 'normal' | 'low';
  enabled: boolean;
  created_at: number;
  updated_at: number;
}

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
  article_id: ArticleId;
  db_name: string;
  note: string;
  created_at: number;
}

export interface FavoriteArticleItem extends FavoriteItem {
  journal_id?: JournalId | null;
  issue_id?: number | null;
  title?: string | null;
  date?: string | null;
  authors?: string | null;
  abstract?: string | null;
  doi?: string | null;
  platform_id?: string | null;
  permalink?: string | null;
  journal_title?: string | null;
  open_access?: number | boolean | null;
  in_press?: number | boolean | null;
  volume?: string | null;
  number?: string | null;
  issn?: string | null;
  eissn?: string | null;
  full_text_file?: string | null;
}

export interface FavoriteCheck {
  folder_id: number;
  folder_name: string;
}

export interface FavoriteBatchCheckItem {
  article_id: ArticleId;
  folders: FavoriteCheck[];
}

export interface FavoriteArticleRef {
  article_id: ArticleId;
  db_name: string;
}

export type CitationFormat = 'bibtex' | 'ris' | 'endnote';

export interface AccessToken {
  id: number;
  name: string;
  expires_at: number;
  created_at: number;
}

export interface CnkiSessionStatus {
  configured: boolean;
  status: 'empty' | 'waiting_scan' | 'active' | 'expired' | string;
  has_bff_user_token: boolean;
  expires_at?: number | null;
  seconds_remaining?: number | null;
  cookie_names: string[];
  updated_at?: number | null;
  last_used_at?: number | null;
}

export interface CnkiLoginStartResponse {
  uuid: string;
  status: string;
  qr_code: string;
  session: CnkiSessionStatus;
}

export interface CnkiLoginPollResponse {
  status: string;
  session: CnkiSessionStatus;
}

export interface ApiErrorInfo {
  code: string | null;
  message: string;
  phase: string | null;
}

/**
 * Error raised for non-2xx API responses.
 */
export class ApiError extends Error {
  readonly code: string | null;
  readonly phase: string | null;
  readonly status: number;

  /**
   * Create an API error with optional backend classification.
   *
   * @param message - Displayable error message.
   * @param status - HTTP status code.
   * @param code - Stable backend error code.
   * @param phase - Backend workflow phase that failed.
   */
  constructor(message: string, status: number, code: string | null, phase: string | null) {
    super(message);
    this.name = 'ApiError';
    this.status = status;
    this.code = code;
    this.phase = phase;
    Object.setPrototypeOf(this, ApiError.prototype);
  }
}

export interface InviteCode {
  id: number;
  code: string;
  used: boolean;
  created_at: number;
}

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
    scheduled_tasks: number;
    active_announcements: number;
  };
  index: {
    databases: IndexDbStats[];
    total_articles: number;
    total_journals: number;
  };
  push: PushDbStats[];
}

export interface AnnouncementCreate {
  title: string;
  message: string;
  priority: 'high' | 'normal' | 'low';
  enabled: boolean;
}

export interface AnnouncementUpdate {
  title?: string;
  message?: string;
  priority?: 'high' | 'normal' | 'low';
  enabled?: boolean;
}

export const DEFAULT_DATABASE = 'ccf_computer_journals.sqlite';
export const DEFAULT_DB = DEFAULT_DATABASE;
export const SELECTED_DATABASE_KEY = 'ps:v1:selected_database';
const LEGACY_SELECTED_DATABASE_KEY = 'selected_database';

const API_BASE_URL = process.env.NEXT_PUBLIC_API_URL || '';

/**
 * Resolve the backend base URL for client or server-side rendering.
 *
 * @returns Absolute backend URL.
 */
export function resolveApiBase(): string {
  if (API_BASE_URL) {
    return API_BASE_URL;
  }
  if (typeof window !== 'undefined') {
    return window.location.origin;
  }
  return 'http://localhost:8000';
}

/**
 * Read the selected index database from local storage.
 *
 * @returns Selected database name.
 */
export function readSelectedDatabase(): string {
  if (typeof window === 'undefined') {
    return DEFAULT_DATABASE;
  }
  const selectedDatabase = readLocalStorageValue(SELECTED_DATABASE_KEY);
  if (selectedDatabase) {
    return selectedDatabase;
  }
  const legacySelectedDatabase = readLocalStorageValue(LEGACY_SELECTED_DATABASE_KEY);
  if (legacySelectedDatabase) {
    storeSelectedDatabase(legacySelectedDatabase);
    removeLocalStorageValue(LEGACY_SELECTED_DATABASE_KEY);
    return legacySelectedDatabase;
  }
  return DEFAULT_DATABASE;
}

/**
 * Store the selected index database in local storage.
 *
 * @param dbName - Database file name.
 */
export function storeSelectedDatabase(dbName: string): void {
  if (typeof window !== 'undefined') {
    writeLocalStorageValue(SELECTED_DATABASE_KEY, dbName);
    removeLocalStorageValue(LEGACY_SELECTED_DATABASE_KEY);
  }
}

/**
 * Store the active database for restored pre-desktop UI modules.
 *
 * @param dbName - Database file name.
 */
export function setDatabase(dbName: string): void {
  storeSelectedDatabase(dbName);
}

/**
 * Read the active database for restored pre-desktop UI modules.
 *
 * @returns Selected database name.
 */
export function getCurrentDatabase(): string {
  return readSelectedDatabase();
}

/**
 * Build an absolute URL from a backend path.
 *
 * @param path - API path.
 * @param params - Query parameters to append.
 * @returns Absolute URL.
 */
export function buildApiUrl(path: string, params?: URLSearchParams): string {
  const url = new URL(path, resolveApiBase());
  params?.forEach((value, key) => {
    url.searchParams.append(key, value);
  });
  return url.toString();
}

/**
 * Build an absolute URL for a database-backed API path.
 *
 * @param path - API path.
 * @param dbName - Database name.
 * @param params - Query parameters to append.
 * @returns Absolute URL.
 */
export function buildDatabaseUrl(path: string, dbName: string, params?: URLSearchParams): string {
  const url = new URL(buildApiUrl(path, params));
  if (!url.searchParams.has('db')) {
    url.searchParams.set('db', dbName);
  }
  return url.toString();
}

/**
 * Check whether an unknown value is a string-keyed object.
 *
 * @param value - Value to inspect.
 * @returns Whether the value is a record.
 */
function isRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value && typeof value === 'object');
}

/**
 * Convert an unknown backend error payload into structured error info.
 *
 * @param payload - Parsed backend error payload.
 * @param fallback - Fallback message.
 * @returns Structured API error info.
 */
function extractErrorInfo(payload: unknown, fallback: string): ApiErrorInfo {
  if (isRecord(payload) && 'detail' in payload) {
    const detail = payload.detail;
    if (typeof detail === 'string') {
      return { code: null, message: detail, phase: null };
    }
    if (isRecord(detail)) {
      const code = typeof detail.code === 'string' ? detail.code : null;
      const message = typeof detail.message === 'string' ? detail.message : fallback;
      const phase = typeof detail.phase === 'string' ? detail.phase : null;
      return { code, message, phase };
    }
  }
  return { code: null, message: fallback, phase: null };
}

/**
 * Parse a fetch response as JSON and raise a typed error on failure.
 *
 * @param response - Fetch response.
 * @param fallback - Fallback error message.
 * @param parser - Optional runtime contract parser for control-plane responses.
 * @returns Parsed response body.
 */
async function parseJson<T>(
  response: Response,
  fallback: string,
  parser?: ContractParser<T>,
): Promise<T> {
  if (response.ok) {
    const payload: unknown = await response.json();
    return parser ? parser(payload) : (payload as T);
  }
  const payload = await response.json().catch(() => null);
  const errorInfo = extractErrorInfo(payload, fallback);
  throw new ApiError(errorInfo.message, response.status, errorInfo.code, errorInfo.phase);
}

/**
 * Fetch JSON from an endpoint using browser cookies and optional bearer auth.
 *
 * @param url - Absolute endpoint URL.
 * @param token - Optional explicit bearer access token.
 * @param init - Fetch options.
 * @param fallback - Fallback error message.
 * @param parser - Optional runtime contract parser for control-plane responses.
 * @returns Parsed response body.
 */
async function requestJson<T>(
  url: string,
  token?: string | null,
  init?: RequestInit,
  fallback = '请求失败',
  parser?: ContractParser<T>,
): Promise<T> {
  const hasBody = typeof init?.body !== 'undefined';
  const headers: Record<string, string> = {
    ...(hasBody ? { 'Content-Type': 'application/json' } : {}),
    ...(token ? { Authorization: `Bearer ${token}` } : {}),
    ...(init?.headers as Record<string, string> | undefined),
  };
  const response = await fetch(url, { ...init, credentials: 'include', headers });
  return parseJson<T>(response, fallback, parser);
}

/**
 * Get the current authenticated user.
 *
 * @param token - Optional explicit bearer access token.
 * @returns Current user.
 */
export function getCurrentUser(token?: string | null): Promise<AuthUser> {
  return requestJson<AuthUser>(
    buildApiUrl('/api/auth/me'),
    token,
    undefined,
    '获取用户失败',
    parseAuthUser,
  );
}

/**
 * Authenticate a user with username and password.
 *
 * @param username - Username.
 * @param password - Password.
 * @returns Login response.
 */
export function loginUser(username: string, password: string): Promise<LoginResponse> {
  return requestJson<LoginResponse>(
    buildApiUrl('/api/auth/login'),
    null,
    {
      method: 'POST',
      body: JSON.stringify({ username, password }),
    },
    '登录失败',
    parseLoginResponse,
  );
}

/**
 * Register a user with a required invite code.
 *
 * @param username - Username.
 * @param password - Password.
 * @param inviteCode - Invite code.
 * @returns Empty promise when registration succeeds.
 */
export async function registerUser(
  username: string,
  password: string,
  inviteCode: string,
): Promise<void> {
  await requestJson<unknown>(
    buildApiUrl('/api/auth/register'),
    null,
    {
      method: 'POST',
      body: JSON.stringify({ username, password, invite_code: inviteCode }),
    },
    '注册失败',
  );
}

/**
 * Revoke the active login token.
 *
 * @param token - Optional explicit bearer access token.
 */
export async function logoutUser(token?: string | null): Promise<void> {
  await fetch(buildApiUrl('/api/auth/logout'), {
    method: 'POST',
    credentials: 'include',
    headers: token ? { Authorization: `Bearer ${token}` } : undefined,
  }).catch(() => undefined);
}

/**
 * Check whether registration currently requires an invite code.
 *
 * @returns Invite requirement.
 */
export function getInviteRequirement(): Promise<InviteRequirement> {
  return requestJson<InviteRequirement>(
    buildApiUrl('/api/auth/invite-required'),
    null,
    undefined,
    '获取邀请码状态失败',
    parseInviteRequirement,
  );
}

/**
 * List available index databases.
 *
 * @returns Database names.
 */
export async function getDatabases(): Promise<string[]> {
  try {
    return await requestJson<string[]>(
      buildApiUrl('/api/meta/databases'),
      null,
      undefined,
      '获取数据库失败',
    );
  } catch {
    return [DEFAULT_DATABASE];
  }
}

/**
 * Fetch metadata areas for a database.
 *
 * @param dbName - Database name. Defaults to the selected database.
 * @returns Area counts.
 */
export function getAreas(dbName = readSelectedDatabase()): Promise<ValueCount[]> {
  return requestJson<ValueCount[]>(
    buildDatabaseUrl('/api/meta/areas', dbName),
    null,
    undefined,
    '获取领域失败',
  );
}

/**
 * Fetch indexed year summaries for a database.
 *
 * @param dbName - Database name. Defaults to the selected database.
 * @returns Year summaries.
 */
export function getYears(dbName = readSelectedDatabase()): Promise<YearSummary[]> {
  return requestJson<YearSummary[]>(
    buildDatabaseUrl('/api/years', dbName),
    null,
    undefined,
    '获取年份失败',
  );
}

/**
 * Fetch journal filter options for a database.
 *
 * @param dbName - Database name. Defaults to the selected database.
 * @returns Journal options.
 */
export function getJournalOptions(dbName = readSelectedDatabase()): Promise<JournalOption[]> {
  return requestJson<JournalOption[]>(
    buildDatabaseUrl('/api/meta/journals', dbName),
    null,
    undefined,
    '获取期刊失败',
  );
}

/**
 * Fetch a cursor-paginated article page.
 *
 * @param params - Article query parameters.
 * @param pageParam - Cursor or offset page parameter.
 * @param includeTotal - Whether to include total on the first page.
 * @param dbName - Database name. Defaults to the selected database.
 * @returns Article page.
 */
export function getArticles(
  params: URLSearchParams,
  pageParam: string | number | null = null,
  includeTotal = false,
  dbName = readSelectedDatabase(),
): Promise<ArticlePage> {
  const nextParams = new URLSearchParams(params);
  if (typeof pageParam === 'string' && pageParam.length > 0) {
    nextParams.set('cursor', pageParam);
    nextParams.delete('offset');
  }
  if (typeof pageParam === 'number') {
    nextParams.set('offset', String(pageParam));
  }
  nextParams.set('include_total', includeTotal ? '1' : '0');
  return requestJson<ArticlePage>(
    buildDatabaseUrl('/api/articles', dbName, nextParams),
    null,
    undefined,
    '获取文章失败',
  );
}

/**
 * Fetch weekly update data.
 *
 * @returns Weekly update response.
 */
export function getWeeklyUpdates(): Promise<WeeklyUpdatesResponse> {
  return requestJson<WeeklyUpdatesResponse>(
    buildApiUrl('/api/weekly-updates'),
    null,
    undefined,
    '获取每周更新失败',
  );
}

/**
 * Fetch enabled announcements.
 *
 * @returns Announcements.
 */
export function getAnnouncements(): Promise<AnnouncementInfo[]> {
  return requestJson<AnnouncementInfo[]>(
    buildApiUrl('/api/announcements'),
    null,
    undefined,
    '获取公告失败',
  );
}

/**
 * Build the full-text redirect URL for an article.
 *
 * @param articleId - Article id.
 * @returns Full-text URL.
 */
export function getFullTextUrl(articleId: ArticleId): string {
  return getFullTextUrlForDatabase(articleId, readSelectedDatabase());
}

/**
 * Build the full-text redirect URL for a specific database.
 *
 * @param articleId - Article id.
 * @param dbName - Database name.
 * @returns Full-text URL.
 */
export function getFullTextUrlForDatabase(articleId: ArticleId, dbName: string): string {
  const url = new URL(`/api/articles/${articleId}/fulltext`, resolveApiBase());
  url.searchParams.set('db', dbName);
  return url.toString();
}

/**
 * Fetch article detail and full-text access capabilities.
 *
 * @param articleId - Article id.
 * @param dbName - Database name.
 * @returns Article access capabilities.
 */
export function getArticleAccess(
  articleId: ArticleId,
  dbName: string,
): Promise<ArticleAccessResponse> {
  return requestJson<ArticleAccessResponse>(
    buildDatabaseUrl(`/api/articles/${articleId}/access`, dbName),
    null,
    undefined,
    '获取文章访问状态失败',
  );
}

/**
 * Fetch one article by id from a database.
 *
 * @param articleId - Article id.
 * @param dbName - Database name.
 * @returns Article record.
 */
export function getArticleById(articleId: ArticleId, dbName: string): Promise<Article> {
  return requestJson<Article>(
    buildDatabaseUrl(`/api/articles/${articleId}`, dbName),
    null,
    undefined,
    '获取文章详情失败',
  );
}

/**
 * Fetch all folders for the current user.
 *
 * @returns Folders.
 */
export function getFolders(): Promise<Folder[]> {
  return requestJson<Folder[]>(
    buildApiUrl('/api/favorites/folders'),
    null,
    undefined,
    '获取收藏夹失败',
  );
}

/**
 * Create a favorite folder.
 *
 * @param name - Folder name.
 * @param isTracking - Whether the folder is the tracking folder.
 * @returns Created folder.
 */
export function createFolder(name: string, isTracking = false): Promise<Folder> {
  return requestJson<Folder>(
    buildApiUrl('/api/favorites/folders'),
    null,
    {
      method: 'POST',
      body: JSON.stringify({ name, is_tracking: isTracking }),
    },
    '创建收藏夹失败',
  );
}

/**
 * Rename a folder.
 *
 * @param folderId - Folder id.
 * @param name - New name.
 */
export async function renameFolder(folderId: number, name: string): Promise<void> {
  await requestJson<unknown>(
    buildApiUrl(`/api/favorites/folders/${folderId}`),
    null,
    {
      method: 'PUT',
      body: JSON.stringify({ name }),
    },
    '重命名收藏夹失败',
  );
}

/**
 * Delete a folder.
 *
 * @param folderId - Folder id.
 */
export async function deleteFolder(folderId: number): Promise<void> {
  await requestJson<unknown>(
    buildApiUrl(`/api/favorites/folders/${folderId}`),
    null,
    { method: 'DELETE' },
    '删除收藏夹失败',
  );
}

/**
 * Fetch articles in a folder.
 *
 * @param folderId - Folder id.
 * @param limit - Page size.
 * @param offset - Page offset.
 * @returns Favorite articles.
 */
export function getFolderArticles(
  folderId: number,
  limit: number,
  offset: number,
): Promise<FavoriteArticleItem[]> {
  const params = new URLSearchParams({ limit: String(limit), offset: String(offset) });
  return requestJson<FavoriteArticleItem[]>(
    buildApiUrl(`/api/favorites/folders/${folderId}/articles`, params),
    null,
    undefined,
    '获取收藏文章失败',
  );
}

/**
 * Add an article to a folder.
 *
 * @param folderId - Folder id.
 * @param articleId - Article id.
 * @param dbName - Database name.
 * @returns Favorite item.
 */
export function addFavorite(
  folderId: number,
  articleId: ArticleId,
  dbName: string,
): Promise<FavoriteItem> {
  return requestJson<FavoriteItem>(
    buildApiUrl(`/api/favorites/folders/${folderId}/articles`),
    null,
    {
      method: 'POST',
      body: JSON.stringify({ article_id: articleId, db_name: dbName, note: '' }),
    },
    '添加收藏失败',
  );
}

/**
 * Remove an article from a folder.
 *
 * @param folderId - Folder id.
 * @param articleId - Article id.
 * @param dbName - Database name.
 */
export async function removeFavorite(
  folderId: number,
  articleId: ArticleId,
  dbName: string,
): Promise<void> {
  const params = new URLSearchParams({ db_name: dbName });
  await requestJson<unknown>(
    buildApiUrl(`/api/favorites/folders/${folderId}/articles/${articleId}`, params),
    null,
    { method: 'DELETE' },
    '移除收藏失败',
  );
}

/**
 * Bulk remove favorite articles from a folder.
 *
 * @param folderId - Folder id.
 * @param articles - Article references.
 * @returns Removed count.
 */
export async function bulkRemoveFavorites(
  folderId: number,
  articles: FavoriteArticleRef[],
): Promise<number> {
  const data = await requestJson<{ count: number }>(
    buildApiUrl(`/api/favorites/folders/${folderId}/articles/bulk-remove`),
    null,
    {
      method: 'POST',
      body: JSON.stringify({ articles }),
    },
    '批量移除收藏失败',
  );
  return data.count;
}

/**
 * Bulk move favorite articles between folders.
 *
 * @param folderId - Source folder id.
 * @param targetFolderId - Target folder id.
 * @param articles - Article references.
 * @returns Moved count.
 */
export async function bulkMoveFavorites(
  folderId: number,
  targetFolderId: number,
  articles: FavoriteArticleRef[],
): Promise<number> {
  const data = await requestJson<{ count: number }>(
    buildApiUrl(`/api/favorites/folders/${folderId}/articles/bulk-move`),
    null,
    {
      method: 'POST',
      body: JSON.stringify({ target_folder_id: targetFolderId, articles }),
    },
    '批量移动收藏失败',
  );
  return data.count;
}

/**
 * Build a folder export URL.
 *
 * @param folderId - Folder id.
 * @param format - Citation format.
 * @returns Export URL.
 */
export function getExportUrl(folderId: number, format: CitationFormat): string {
  const url = new URL(`/api/favorites/folders/${folderId}/export`, resolveApiBase());
  url.searchParams.set('format', format);
  return url.toString();
}

/**
 * Check which folders contain an article.
 *
 * @param articleId - Article id.
 * @param dbName - Database name.
 * @returns Favorite checks.
 */
export async function checkFavorite(
  articleId: ArticleId,
  dbName: string,
): Promise<FavoriteCheck[]> {
  const params = new URLSearchParams({ article_id: articleId, db_name: dbName });
  try {
    return await requestJson<FavoriteCheck[]>(
      buildApiUrl('/api/favorites/check', params),
      null,
      undefined,
      '获取收藏状态失败',
    );
  } catch {
    return [];
  }
}

/**
 * Check favorite state for many articles.
 *
 * @param articleIds - Article ids.
 * @param dbName - Database name.
 * @returns Favorite checks keyed by article id.
 */
export async function checkFavoritesBatch(
  articleIds: ArticleId[],
  dbName: string,
): Promise<Record<ArticleId, FavoriteCheck[]>> {
  if (articleIds.length === 0) {
    return {};
  }
  try {
    const data = await requestJson<FavoriteBatchCheckItem[]>(
      buildApiUrl('/api/favorites/check/batch'),
      null,
      {
        method: 'POST',
        body: JSON.stringify({ article_ids: articleIds, db_name: dbName }),
      },
      '获取收藏状态失败',
    );
    return Object.fromEntries(data.map((item) => [item.article_id, item.folders]));
  } catch {
    return {};
  }
}

/**
 * Set the tracking folder.
 *
 * @param folderId - Folder id.
 */
export async function setTrackingFolder(folderId: number): Promise<void> {
  await requestJson<unknown>(
    buildApiUrl('/api/favorites/tracking'),
    null,
    {
      method: 'PUT',
      body: JSON.stringify({ folder_id: folderId }),
    },
    '设置追踪文件夹失败',
  );
}

/**
 * Fetch tracking status.
 *
 * @returns Tracking status.
 */
export function getTrackingStatus(): Promise<TrackingStatus> {
  return requestJson<TrackingStatus>(
    buildApiUrl('/api/tracking/status'),
    null,
    undefined,
    '获取追踪状态失败',
    parseTrackingStatus,
  );
}

/**
 * Start weekly article push.
 *
 * @returns Push status.
 */
export function pushWeeklyToTracking(): Promise<ManualPushStatus> {
  return requestJson<ManualPushStatus>(
    buildApiUrl('/api/tracking/push-weekly'),
    null,
    { method: 'POST' },
    '推送每周文章失败',
    parseManualPushStatus,
  );
}

/**
 * Fetch weekly push status.
 *
 * @returns Push status.
 */
export function getPushWeeklyStatus(): Promise<ManualPushStatus> {
  return requestJson<ManualPushStatus>(
    buildApiUrl('/api/tracking/push-weekly/status'),
    null,
    undefined,
    '获取推送状态失败',
    parseManualPushStatus,
  );
}

/**
 * Fetch notification settings.
 *
 * @returns Notification settings or null.
 */
export function getNotificationSettings(): Promise<NotificationSettings | null> {
  return requestJson<NotificationSettings | null>(
    buildApiUrl('/api/tracking/notification-settings'),
    null,
    undefined,
    '获取通知设置失败',
    parseNullableNotificationSettings,
  );
}

/**
 * Update notification settings.
 *
 * @param settings - Settings payload.
 * @returns Saved settings.
 */
export function updateNotificationSettings(
  settings: NotificationSettingsUpdate,
): Promise<NotificationSettings> {
  return requestJson<NotificationSettings>(
    buildApiUrl('/api/tracking/notification-settings'),
    null,
    {
      method: 'PUT',
      body: JSON.stringify(settings),
    },
    '更新通知设置失败',
    parseNotificationSettings,
  );
}

/**
 * Fetch current user access tokens.
 *
 * @returns Access tokens.
 */
export function getAccessTokens(): Promise<AccessToken[]> {
  return requestJson<AccessToken[]>(
    buildApiUrl('/api/auth/tokens'),
    null,
    undefined,
    '获取访问令牌失败',
  );
}

/**
 * Fetch current user's Zhejiang Library CNKI session status.
 *
 * @returns Safe CNKI session status.
 */
export function getCnkiSession(): Promise<CnkiSessionStatus> {
  return requestJson<CnkiSessionStatus>(
    buildApiUrl('/api/cnki/session'),
    null,
    undefined,
    '获取知网登录状态失败',
  );
}

/**
 * Start Zhejiang Library CNKI QR login for the current user.
 *
 * @returns QR login challenge.
 */
export function startCnkiLogin(): Promise<CnkiLoginStartResponse> {
  return requestJson<CnkiLoginStartResponse>(
    buildApiUrl('/api/cnki/login/start'),
    null,
    { method: 'POST' },
    '启动知网登录失败',
  );
}

/**
 * Poll Zhejiang Library CNKI QR login for the current user.
 *
 * @param timeoutSeconds - Maximum polling duration.
 * @param intervalSeconds - Polling interval.
 * @returns QR login poll result.
 */
export function pollCnkiLogin(
  timeoutSeconds: number,
  intervalSeconds: number,
): Promise<CnkiLoginPollResponse> {
  return requestJson<CnkiLoginPollResponse>(
    buildApiUrl('/api/cnki/login/poll'),
    null,
    {
      method: 'POST',
      body: JSON.stringify({
        timeout_seconds: timeoutSeconds,
        interval_seconds: intervalSeconds,
      }),
    },
    '确认知网登录失败',
  );
}

/**
 * Clear current user's Zhejiang Library CNKI session.
 *
 * @returns Safe empty CNKI session status.
 */
export function clearCnkiSession(): Promise<CnkiSessionStatus> {
  return requestJson<CnkiSessionStatus>(
    buildApiUrl('/api/cnki/session'),
    null,
    { method: 'DELETE' },
    '清除知网登录失败',
  );
}

/**
 * Create an access token.
 *
 * @param name - Token name.
 * @param ttl - Time to live in seconds.
 * @returns Created token response.
 */
export function createAccessToken(
  name: string,
  ttl: number,
): Promise<{ id: number; token: string; name: string; expires_at: number }> {
  return requestJson<{ id: number; token: string; name: string; expires_at: number }>(
    buildApiUrl('/api/auth/tokens'),
    null,
    {
      method: 'POST',
      body: JSON.stringify({ name, ttl }),
    },
    '创建访问令牌失败',
  );
}

/**
 * Revoke an access token.
 *
 * @param tokenId - Access token id.
 */
export async function revokeAccessToken(tokenId: number): Promise<void> {
  await requestJson<unknown>(
    buildApiUrl(`/api/auth/tokens/${tokenId}`),
    null,
    { method: 'DELETE' },
    '撤销访问令牌失败',
  );
}

/**
 * Change the active user's password.
 *
 * @param oldPassword - Current password.
 * @param newPassword - New password.
 */
export async function changePassword(oldPassword: string, newPassword: string): Promise<void> {
  await requestJson<unknown>(
    buildApiUrl('/api/auth/change-password'),
    null,
    {
      method: 'POST',
      body: JSON.stringify({ old_password: oldPassword, new_password: newPassword }),
    },
    '修改密码失败',
  );
}

/**
 * Fetch the current user's invite code.
 *
 * @returns Invite code or null.
 */
export function getInviteCode(): Promise<InviteCode | null> {
  return requestJson<InviteCode | null>(
    buildApiUrl('/api/auth/invite-code'),
    null,
    undefined,
    '获取邀请码失败',
  );
}

/**
 * Generate the current user's invite code.
 *
 * @returns Generated invite code.
 */
export function generateInviteCode(): Promise<InviteCode> {
  return requestJson<InviteCode>(
    buildApiUrl('/api/auth/invite-code'),
    null,
    { method: 'POST' },
    '生成邀请码失败',
  );
}

/**
 * Fetch admin stats.
 *
 * @returns Admin stats.
 */
export function adminGetStats(): Promise<AdminStats> {
  return requestJson<AdminStats>(
    buildApiUrl('/api/admin/stats'),
    null,
    undefined,
    '获取统计信息失败',
  );
}

/**
 * Fetch admin user list.
 *
 * @returns Users.
 */
export function adminGetUsers(): Promise<AdminUserInfo[]> {
  return requestJson<AdminUserInfo[]>(
    buildApiUrl('/api/admin/users'),
    null,
    undefined,
    '获取用户列表失败',
  );
}

/**
 * Grant or revoke admin access.
 *
 * @param userId - User id.
 * @param isAdmin - Whether the user should be admin.
 */
export async function adminSetAdmin(userId: number, isAdmin: boolean): Promise<void> {
  await requestJson<unknown>(
    buildApiUrl(`/api/admin/users/${userId}/admin`),
    null,
    {
      method: 'PUT',
      body: JSON.stringify({ is_admin: isAdmin }),
    },
    '更新管理员状态失败',
  );
}

/**
 * Reset a user's password.
 *
 * @param userId - User id.
 * @param newPassword - New password.
 */
export async function adminResetPassword(userId: number, newPassword: string): Promise<void> {
  await requestJson<unknown>(
    buildApiUrl(`/api/admin/users/${userId}/reset-password`),
    null,
    {
      method: 'POST',
      body: JSON.stringify({ new_password: newPassword }),
    },
    '重置密码失败',
  );
}

/**
 * Delete a user.
 *
 * @param userId - User id.
 */
export async function adminDeleteUser(userId: number): Promise<void> {
  await requestJson<unknown>(
    buildApiUrl(`/api/admin/users/${userId}`),
    null,
    { method: 'DELETE' },
    '删除用户失败',
  );
}

/**
 * Fetch admin invite codes.
 *
 * @returns Invite codes.
 */
export function adminGetInviteCodes(): Promise<AdminInviteCode[]> {
  return requestJson<AdminInviteCode[]>(
    buildApiUrl('/api/admin/invite-codes'),
    null,
    undefined,
    '获取邀请码列表失败',
  );
}

/**
 * Create an admin invite code.
 *
 * @returns Created invite code summary.
 */
export function adminCreateInviteCode(): Promise<{ id: number; code: string }> {
  return requestJson<{ id: number; code: string }>(
    buildApiUrl('/api/admin/invite-codes'),
    null,
    { method: 'POST' },
    '创建邀请码失败',
  );
}

/**
 * Delete an unused admin invite code.
 *
 * @param codeId - Invite code id.
 */
export async function adminDeleteInviteCode(codeId: number): Promise<void> {
  await requestJson<unknown>(
    buildApiUrl(`/api/admin/invite-codes/${codeId}`),
    null,
    { method: 'DELETE' },
    '删除邀请码失败',
  );
}

/**
 * Fetch runtime settings.
 *
 * @returns Runtime settings.
 */
export function adminGetRuntimeSettings(): Promise<RuntimeSettingInfo[]> {
  return requestJson<RuntimeSettingInfo[]>(
    buildApiUrl('/api/admin/runtime-settings'),
    null,
    undefined,
    '获取运行配置失败',
    parseRuntimeSettingList,
  );
}

/**
 * Update runtime settings.
 *
 * @param payload - Runtime settings payload.
 * @returns Updated runtime settings.
 */
export function adminUpdateRuntimeSettings(
  payload: RuntimeSettingsUpdate,
): Promise<RuntimeSettingInfo[]> {
  return requestJson<RuntimeSettingInfo[]>(
    buildApiUrl('/api/admin/runtime-settings'),
    null,
    {
      method: 'PUT',
      body: JSON.stringify(payload),
    },
    '更新运行配置失败',
    parseRuntimeSettingList,
  );
}

/**
 * Fetch scheduled tasks.
 *
 * @returns Scheduled tasks.
 */
export function adminGetScheduledTasks(): Promise<ScheduledTaskInfo[]> {
  return requestJson<ScheduledTaskInfo[]>(
    buildApiUrl('/api/admin/scheduled-tasks'),
    null,
    undefined,
    '获取定时任务失败',
    parseScheduledTaskList,
  );
}

/**
 * Create a scheduled task.
 *
 * @param payload - Scheduled task payload.
 * @returns Created scheduled task.
 */
export function adminCreateScheduledTask(payload: ScheduledTaskCreate): Promise<ScheduledTaskInfo> {
  return requestJson<ScheduledTaskInfo>(
    buildApiUrl('/api/admin/scheduled-tasks'),
    null,
    {
      method: 'POST',
      body: JSON.stringify(payload),
    },
    '创建定时任务失败',
    parseScheduledTaskInfo,
  );
}

/**
 * Update a scheduled task.
 *
 * @param taskId - Task id.
 * @param payload - Scheduled task patch.
 * @returns Updated scheduled task.
 */
export function adminUpdateScheduledTask(
  taskId: number,
  payload: ScheduledTaskUpdate,
): Promise<ScheduledTaskInfo> {
  return requestJson<ScheduledTaskInfo>(
    buildApiUrl(`/api/admin/scheduled-tasks/${taskId}`),
    null,
    {
      method: 'PUT',
      body: JSON.stringify(payload),
    },
    '更新定时任务失败',
    parseScheduledTaskInfo,
  );
}

/**
 * Delete a scheduled task.
 *
 * @param taskId - Task id.
 */
export async function adminDeleteScheduledTask(taskId: number): Promise<void> {
  await requestJson<unknown>(
    buildApiUrl(`/api/admin/scheduled-tasks/${taskId}`),
    null,
    { method: 'DELETE' },
    '删除定时任务失败',
  );
}

/**
 * Fetch admin announcement list.
 *
 * @returns Announcements.
 */
export function adminGetAnnouncements(): Promise<AnnouncementInfo[]> {
  return requestJson<AnnouncementInfo[]>(
    buildApiUrl('/api/admin/announcements'),
    null,
    undefined,
    '获取公告列表失败',
  );
}

/**
 * Create an announcement.
 *
 * @param payload - Announcement payload.
 * @returns Created announcement.
 */
export function adminCreateAnnouncement(payload: AnnouncementCreate): Promise<AnnouncementInfo> {
  return requestJson<AnnouncementInfo>(
    buildApiUrl('/api/admin/announcements'),
    null,
    {
      method: 'POST',
      body: JSON.stringify(payload),
    },
    '创建公告失败',
  );
}

/**
 * Update an announcement.
 *
 * @param announcementId - Announcement id.
 * @param payload - Announcement patch.
 * @returns Updated announcement.
 */
export function adminUpdateAnnouncement(
  announcementId: number,
  payload: AnnouncementUpdate,
): Promise<AnnouncementInfo> {
  return requestJson<AnnouncementInfo>(
    buildApiUrl(`/api/admin/announcements/${announcementId}`),
    null,
    {
      method: 'PUT',
      body: JSON.stringify(payload),
    },
    '更新公告失败',
  );
}

/**
 * Delete an announcement.
 *
 * @param announcementId - Announcement id.
 */
export async function adminDeleteAnnouncement(announcementId: number): Promise<void> {
  await requestJson<unknown>(
    buildApiUrl(`/api/admin/announcements/${announcementId}`),
    null,
    { method: 'DELETE' },
    '删除公告失败',
  );
}
