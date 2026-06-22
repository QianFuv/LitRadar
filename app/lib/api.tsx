/**
 * Typed browser API client for the Paper Scanner backend.
 */

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

export interface AuthUser {
  id: number;
  username: string;
  is_admin?: boolean;
}

export interface LoginResponse {
  access_token: string;
  expires_at: number;
  user: AuthUser;
}

export interface InviteRequirement {
  required: boolean;
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

export interface InviteCode {
  id: number;
  code: string;
  used: boolean;
  created_at: number;
}

export interface TrackingStatus {
  tracking_folder: { id: number; name: string } | null;
  total_folders: number;
  weekly_articles_available: number;
  notification_configured: boolean;
}

export interface ManualPushStatus {
  job_id: string | null;
  status: 'idle' | 'running' | 'completed' | 'failed';
  message: string;
  started_at: number | null;
  finished_at: number | null;
  pushed: number;
  selected: number;
  total_candidates?: number | null;
  summary: string;
  folder_id?: number | null;
  folder_name?: string | null;
}

export interface NotificationSettings {
  id: number;
  user_id: number;
  keywords: string[];
  directions: string[];
  selected_databases: string[];
  delivery_method: 'folder' | 'pushplus';
  pushplus_token: string;
  pushplus_template: string;
  pushplus_topic: string;
  pushplus_channel: string;
  sync_to_tracking_folder: boolean;
  ai_base_url: string;
  ai_api_key: string;
  ai_model: string;
  ai_system_prompt: string;
  ai_backup_base_url: string;
  ai_backup_api_key: string;
  ai_backup_model: string;
  ai_backup_system_prompt: string;
  ai_retry_attempts: number;
  enabled: boolean;
  created_at: number;
  updated_at: number;
}

export interface NotificationSettingsUpdate {
  keywords: string[];
  directions: string[];
  selected_databases: string[];
  delivery_method: 'folder' | 'pushplus';
  pushplus_token: string;
  pushplus_template: string;
  pushplus_topic: string;
  pushplus_channel: string;
  sync_to_tracking_folder: boolean;
  ai_base_url: string;
  ai_api_key: string;
  ai_model: string;
  ai_system_prompt: string;
  ai_backup_base_url: string;
  ai_backup_api_key: string;
  ai_backup_model: string;
  ai_backup_system_prompt: string;
  ai_retry_attempts: number;
  enabled: boolean;
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

export interface RuntimeSettingInfo {
  field: string;
  key: string;
  label: string;
  description: string;
  input_type: 'text' | 'password' | 'email' | 'boolean';
  is_secret: boolean;
  value: string;
  source: 'database' | 'environment' | 'default';
  updated_at: number | null;
}

export interface RuntimeSettingsUpdate {
  values: Record<string, string>;
}

export interface ScheduledTaskInfo {
  id: number;
  name: string;
  command: string;
  cron: string;
  enabled: boolean;
  last_run_at: number | null;
  last_status: string;
  created_at: number;
  updated_at: number;
}

export interface ScheduledTaskCreate {
  name: string;
  command: string;
  cron: string;
  enabled: boolean;
}

export interface ScheduledTaskUpdate {
  name?: string;
  command?: string;
  cron?: string;
  enabled?: boolean;
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
 * Read a localStorage value without assuming browser storage is available.
 *
 * @param key - Storage key.
 * @returns Stored value or null.
 */
function readLocalStorageValue(key: string): string | null {
  if (typeof window === 'undefined') {
    return null;
  }
  try {
    return window.localStorage.getItem(key);
  } catch {
    return null;
  }
}

/**
 * Write a localStorage value without surfacing quota or privacy-mode errors.
 *
 * @param key - Storage key.
 * @param value - Value to store.
 */
function writeLocalStorageValue(key: string, value: string): void {
  if (typeof window === 'undefined') {
    return;
  }
  try {
    window.localStorage.setItem(key, value);
  } catch {}
}

/**
 * Remove a localStorage value without assuming browser storage is available.
 *
 * @param key - Storage key.
 */
function removeLocalStorageValue(key: string): void {
  if (typeof window === 'undefined') {
    return;
  }
  try {
    window.localStorage.removeItem(key);
  } catch {}
}

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
 * Convert an unknown backend error payload into a display message.
 *
 * @param payload - Parsed backend error payload.
 * @param fallback - Fallback message.
 * @returns Error message.
 */
function extractErrorMessage(payload: unknown, fallback: string): string {
  if (payload && typeof payload === 'object' && 'detail' in payload) {
    const detail = (payload as { detail?: unknown }).detail;
    if (typeof detail === 'string') {
      return detail;
    }
  }
  return fallback;
}

/**
 * Parse a fetch response as JSON and raise a typed error on failure.
 *
 * @param response - Fetch response.
 * @param fallback - Fallback error message.
 * @returns Parsed response body.
 */
async function parseJson<T>(response: Response, fallback: string): Promise<T> {
  if (response.ok) {
    return response.json() as Promise<T>;
  }
  const payload = await response.json().catch(() => null);
  throw new Error(extractErrorMessage(payload, fallback));
}

/**
 * Fetch JSON from an endpoint with an optional bearer token.
 *
 * @param url - Absolute endpoint URL.
 * @param token - Bearer access token.
 * @param init - Fetch options.
 * @param fallback - Fallback error message.
 * @returns Parsed response body.
 */
async function requestJson<T>(
  url: string,
  token?: string | null,
  init?: RequestInit,
  fallback = '请求失败',
): Promise<T> {
  const hasBody = typeof init?.body !== 'undefined';
  const headers: Record<string, string> = {
    ...(hasBody ? { 'Content-Type': 'application/json' } : {}),
    ...(token ? { Authorization: `Bearer ${token}` } : {}),
    ...(init?.headers as Record<string, string> | undefined),
  };
  const response = await fetch(url, { ...init, headers });
  return parseJson<T>(response, fallback);
}

/**
 * Get the current authenticated user.
 *
 * @param token - Bearer access token.
 * @returns Current user.
 */
export function getCurrentUser(token: string): Promise<AuthUser> {
  return requestJson<AuthUser>(buildApiUrl('/api/auth/me'), token, undefined, '获取用户失败');
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
  );
}

/**
 * Register a user with an optional invite code.
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
 * @param token - Bearer access token.
 */
export async function logoutUser(token: string): Promise<void> {
  await fetch(buildApiUrl('/api/auth/logout'), {
    method: 'POST',
    headers: { Authorization: `Bearer ${token}` },
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
  );
}

/**
 * List available index databases.
 *
 * @param token - Bearer access token.
 * @returns Database names.
 */
export async function getDatabases(token: string): Promise<string[]> {
  try {
    return await requestJson<string[]>(
      buildApiUrl('/api/meta/databases'),
      token,
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
 * @param token - Bearer access token.
 * @param dbName - Database name. Defaults to the selected database.
 * @returns Area counts.
 */
export function getAreas(token: string, dbName = readSelectedDatabase()): Promise<ValueCount[]> {
  return requestJson<ValueCount[]>(
    buildDatabaseUrl('/api/meta/areas', dbName),
    token,
    undefined,
    '获取领域失败',
  );
}

/**
 * Fetch indexed year summaries for a database.
 *
 * @param token - Bearer access token.
 * @param dbName - Database name. Defaults to the selected database.
 * @returns Year summaries.
 */
export function getYears(token: string, dbName = readSelectedDatabase()): Promise<YearSummary[]> {
  return requestJson<YearSummary[]>(
    buildDatabaseUrl('/api/years', dbName),
    token,
    undefined,
    '获取年份失败',
  );
}

/**
 * Fetch journal filter options for a database.
 *
 * @param token - Bearer access token.
 * @param dbName - Database name. Defaults to the selected database.
 * @returns Journal options.
 */
export function getJournalOptions(
  token: string,
  dbName = readSelectedDatabase(),
): Promise<JournalOption[]> {
  return requestJson<JournalOption[]>(
    buildDatabaseUrl('/api/meta/journals', dbName),
    token,
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
 * @param token - Bearer access token.
 * @param dbName - Database name. Defaults to the selected database.
 * @returns Article page.
 */
export function getArticles(
  params: URLSearchParams,
  pageParam: string | number | null = null,
  includeTotal = false,
  token?: string,
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
    token,
    undefined,
    '获取文章失败',
  );
}

/**
 * Fetch weekly update data.
 *
 * @param token - Bearer access token.
 * @returns Weekly update response.
 */
export function getWeeklyUpdates(token: string): Promise<WeeklyUpdatesResponse> {
  return requestJson<WeeklyUpdatesResponse>(
    buildApiUrl('/api/weekly-updates'),
    token,
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
 * @param token - Optional access token.
 * @returns Full-text URL.
 */
export function getFullTextUrl(articleId: ArticleId, token?: string): string {
  return getFullTextUrlForDatabase(articleId, readSelectedDatabase(), token);
}

/**
 * Build the full-text redirect URL for a specific database.
 *
 * @param articleId - Article id.
 * @param dbName - Database name.
 * @param token - Optional access token.
 * @returns Full-text URL.
 */
export function getFullTextUrlForDatabase(
  articleId: ArticleId,
  dbName: string,
  token?: string,
): string {
  const url = new URL(`/api/articles/${articleId}/fulltext`, resolveApiBase());
  url.searchParams.set('db', dbName);
  if (token) {
    url.searchParams.set('access_token', token);
  }
  return url.toString();
}

/**
 * Fetch article detail and full-text access capabilities.
 *
 * @param articleId - Article id.
 * @param dbName - Database name.
 * @param token - Bearer access token.
 * @returns Article access capabilities.
 */
export function getArticleAccess(
  articleId: ArticleId,
  dbName: string,
  token: string,
): Promise<ArticleAccessResponse> {
  return requestJson<ArticleAccessResponse>(
    buildDatabaseUrl(`/api/articles/${articleId}/access`, dbName),
    token,
    undefined,
    '获取文章访问状态失败',
  );
}

/**
 * Fetch one article by id from a database.
 *
 * @param articleId - Article id.
 * @param dbName - Database name.
 * @param token - Optional bearer access token.
 * @returns Article record.
 */
export function getArticleById(
  articleId: ArticleId,
  dbName: string,
  token?: string,
): Promise<Article> {
  return requestJson<Article>(
    buildDatabaseUrl(`/api/articles/${articleId}`, dbName),
    token,
    undefined,
    '获取文章详情失败',
  );
}

/**
 * Fetch all folders for the current user.
 *
 * @param token - Bearer access token.
 * @returns Folders.
 */
export function getFolders(token: string): Promise<Folder[]> {
  return requestJson<Folder[]>(
    buildApiUrl('/api/favorites/folders'),
    token,
    undefined,
    '获取收藏夹失败',
  );
}

/**
 * Create a favorite folder.
 *
 * @param token - Bearer access token.
 * @param name - Folder name.
 * @param isTracking - Whether the folder is the tracking folder.
 * @returns Created folder.
 */
export function createFolder(token: string, name: string, isTracking = false): Promise<Folder> {
  return requestJson<Folder>(
    buildApiUrl('/api/favorites/folders'),
    token,
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
 * @param token - Bearer access token.
 * @param folderId - Folder id.
 * @param name - New name.
 */
export async function renameFolder(token: string, folderId: number, name: string): Promise<void> {
  await requestJson<unknown>(
    buildApiUrl(`/api/favorites/folders/${folderId}`),
    token,
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
 * @param token - Bearer access token.
 * @param folderId - Folder id.
 */
export async function deleteFolder(token: string, folderId: number): Promise<void> {
  await requestJson<unknown>(
    buildApiUrl(`/api/favorites/folders/${folderId}`),
    token,
    { method: 'DELETE' },
    '删除收藏夹失败',
  );
}

/**
 * Fetch articles in a folder.
 *
 * @param token - Bearer access token.
 * @param folderId - Folder id.
 * @param limit - Page size.
 * @param offset - Page offset.
 * @returns Favorite articles.
 */
export function getFolderArticles(
  token: string,
  folderId: number,
  limit: number,
  offset: number,
): Promise<FavoriteArticleItem[]> {
  const params = new URLSearchParams({ limit: String(limit), offset: String(offset) });
  return requestJson<FavoriteArticleItem[]>(
    buildApiUrl(`/api/favorites/folders/${folderId}/articles`, params),
    token,
    undefined,
    '获取收藏文章失败',
  );
}

/**
 * Add an article to a folder.
 *
 * @param token - Bearer access token.
 * @param folderId - Folder id.
 * @param articleId - Article id.
 * @param dbName - Database name.
 * @returns Favorite item.
 */
export function addFavorite(
  token: string,
  folderId: number,
  articleId: ArticleId,
  dbName: string,
): Promise<FavoriteItem> {
  return requestJson<FavoriteItem>(
    buildApiUrl(`/api/favorites/folders/${folderId}/articles`),
    token,
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
 * @param token - Bearer access token.
 * @param folderId - Folder id.
 * @param articleId - Article id.
 * @param dbName - Database name.
 */
export async function removeFavorite(
  token: string,
  folderId: number,
  articleId: ArticleId,
  dbName: string,
): Promise<void> {
  const params = new URLSearchParams({ db_name: dbName });
  await requestJson<unknown>(
    buildApiUrl(`/api/favorites/folders/${folderId}/articles/${articleId}`, params),
    token,
    { method: 'DELETE' },
    '移除收藏失败',
  );
}

/**
 * Bulk remove favorite articles from a folder.
 *
 * @param token - Bearer access token.
 * @param folderId - Folder id.
 * @param articles - Article references.
 * @returns Removed count.
 */
export async function bulkRemoveFavorites(
  token: string,
  folderId: number,
  articles: FavoriteArticleRef[],
): Promise<number> {
  const data = await requestJson<{ count: number }>(
    buildApiUrl(`/api/favorites/folders/${folderId}/articles/bulk-remove`),
    token,
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
 * @param token - Bearer access token.
 * @param folderId - Source folder id.
 * @param targetFolderId - Target folder id.
 * @param articles - Article references.
 * @returns Moved count.
 */
export async function bulkMoveFavorites(
  token: string,
  folderId: number,
  targetFolderId: number,
  articles: FavoriteArticleRef[],
): Promise<number> {
  const data = await requestJson<{ count: number }>(
    buildApiUrl(`/api/favorites/folders/${folderId}/articles/bulk-move`),
    token,
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
 * @param token - Access token.
 * @param folderId - Folder id.
 * @param format - Citation format.
 * @returns Export URL.
 */
export function getExportUrl(token: string, folderId: number, format: CitationFormat): string {
  const url = new URL(`/api/favorites/folders/${folderId}/export`, resolveApiBase());
  url.searchParams.set('format', format);
  url.searchParams.set('access_token', token);
  return url.toString();
}

/**
 * Check which folders contain an article.
 *
 * @param token - Bearer access token.
 * @param articleId - Article id.
 * @param dbName - Database name.
 * @returns Favorite checks.
 */
export async function checkFavorite(
  token: string,
  articleId: ArticleId,
  dbName: string,
): Promise<FavoriteCheck[]> {
  const params = new URLSearchParams({ article_id: articleId, db_name: dbName });
  try {
    return await requestJson<FavoriteCheck[]>(
      buildApiUrl('/api/favorites/check', params),
      token,
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
 * @param token - Bearer access token.
 * @param articleIds - Article ids.
 * @param dbName - Database name.
 * @returns Favorite checks keyed by article id.
 */
export async function checkFavoritesBatch(
  token: string,
  articleIds: ArticleId[],
  dbName: string,
): Promise<Record<ArticleId, FavoriteCheck[]>> {
  if (articleIds.length === 0) {
    return {};
  }
  try {
    const data = await requestJson<FavoriteBatchCheckItem[]>(
      buildApiUrl('/api/favorites/check/batch'),
      token,
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
 * @param token - Bearer access token.
 * @param folderId - Folder id.
 */
export async function setTrackingFolder(token: string, folderId: number): Promise<void> {
  await requestJson<unknown>(
    buildApiUrl('/api/favorites/tracking'),
    token,
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
 * @param token - Bearer access token.
 * @returns Tracking status.
 */
export function getTrackingStatus(token: string): Promise<TrackingStatus> {
  return requestJson<TrackingStatus>(
    buildApiUrl('/api/tracking/status'),
    token,
    undefined,
    '获取追踪状态失败',
  );
}

/**
 * Start weekly article push.
 *
 * @param token - Bearer access token.
 * @returns Push status.
 */
export function pushWeeklyToTracking(token: string): Promise<ManualPushStatus> {
  return requestJson<ManualPushStatus>(
    buildApiUrl('/api/tracking/push-weekly'),
    token,
    { method: 'POST' },
    '推送每周文章失败',
  );
}

/**
 * Fetch weekly push status.
 *
 * @param token - Bearer access token.
 * @returns Push status.
 */
export function getPushWeeklyStatus(token: string): Promise<ManualPushStatus> {
  return requestJson<ManualPushStatus>(
    buildApiUrl('/api/tracking/push-weekly/status'),
    token,
    undefined,
    '获取推送状态失败',
  );
}

/**
 * Fetch notification settings.
 *
 * @param token - Bearer access token.
 * @returns Notification settings or null.
 */
export function getNotificationSettings(token: string): Promise<NotificationSettings | null> {
  return requestJson<NotificationSettings | null>(
    buildApiUrl('/api/tracking/notification-settings'),
    token,
    undefined,
    '获取通知设置失败',
  );
}

/**
 * Update notification settings.
 *
 * @param token - Bearer access token.
 * @param settings - Settings payload.
 * @returns Saved settings.
 */
export function updateNotificationSettings(
  token: string,
  settings: NotificationSettingsUpdate,
): Promise<NotificationSettings> {
  return requestJson<NotificationSettings>(
    buildApiUrl('/api/tracking/notification-settings'),
    token,
    {
      method: 'PUT',
      body: JSON.stringify(settings),
    },
    '更新通知设置失败',
  );
}

/**
 * Fetch current user access tokens.
 *
 * @param token - Bearer access token.
 * @returns Access tokens.
 */
export function getAccessTokens(token: string): Promise<AccessToken[]> {
  return requestJson<AccessToken[]>(
    buildApiUrl('/api/auth/tokens'),
    token,
    undefined,
    '获取访问令牌失败',
  );
}

/**
 * Fetch current user's Zhejiang Library CNKI session status.
 *
 * @param token - Bearer access token.
 * @returns Safe CNKI session status.
 */
export function getCnkiSession(token: string): Promise<CnkiSessionStatus> {
  return requestJson<CnkiSessionStatus>(
    buildApiUrl('/api/cnki/session'),
    token,
    undefined,
    '获取知网登录状态失败',
  );
}

/**
 * Start Zhejiang Library CNKI QR login for the current user.
 *
 * @param token - Bearer access token.
 * @returns QR login challenge.
 */
export function startCnkiLogin(token: string): Promise<CnkiLoginStartResponse> {
  return requestJson<CnkiLoginStartResponse>(
    buildApiUrl('/api/cnki/login/start'),
    token,
    { method: 'POST' },
    '启动知网登录失败',
  );
}

/**
 * Poll Zhejiang Library CNKI QR login for the current user.
 *
 * @param token - Bearer access token.
 * @param timeoutSeconds - Maximum polling duration.
 * @param intervalSeconds - Polling interval.
 * @returns QR login poll result.
 */
export function pollCnkiLogin(
  token: string,
  timeoutSeconds: number,
  intervalSeconds: number,
): Promise<CnkiLoginPollResponse> {
  return requestJson<CnkiLoginPollResponse>(
    buildApiUrl('/api/cnki/login/poll'),
    token,
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
 * @param token - Bearer access token.
 * @returns Safe empty CNKI session status.
 */
export function clearCnkiSession(token: string): Promise<CnkiSessionStatus> {
  return requestJson<CnkiSessionStatus>(
    buildApiUrl('/api/cnki/session'),
    token,
    { method: 'DELETE' },
    '清除知网登录失败',
  );
}

/**
 * Create an access token.
 *
 * @param token - Bearer access token.
 * @param name - Token name.
 * @param ttl - Time to live in seconds.
 * @returns Created token response.
 */
export function createAccessToken(
  token: string,
  name: string,
  ttl: number,
): Promise<{ id: number; token: string; name: string; expires_at: number }> {
  return requestJson<{ id: number; token: string; name: string; expires_at: number }>(
    buildApiUrl('/api/auth/tokens'),
    token,
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
 * @param token - Bearer access token.
 * @param tokenId - Access token id.
 */
export async function revokeAccessToken(token: string, tokenId: number): Promise<void> {
  await requestJson<unknown>(
    buildApiUrl(`/api/auth/tokens/${tokenId}`),
    token,
    { method: 'DELETE' },
    '撤销访问令牌失败',
  );
}

/**
 * Change the active user's password.
 *
 * @param token - Bearer access token.
 * @param oldPassword - Current password.
 * @param newPassword - New password.
 */
export async function changePassword(
  token: string,
  oldPassword: string,
  newPassword: string,
): Promise<void> {
  await requestJson<unknown>(
    buildApiUrl('/api/auth/change-password'),
    token,
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
 * @param token - Bearer access token.
 * @returns Invite code or null.
 */
export function getInviteCode(token: string): Promise<InviteCode | null> {
  return requestJson<InviteCode | null>(
    buildApiUrl('/api/auth/invite-code'),
    token,
    undefined,
    '获取邀请码失败',
  );
}

/**
 * Generate the current user's invite code.
 *
 * @param token - Bearer access token.
 * @returns Generated invite code.
 */
export function generateInviteCode(token: string): Promise<InviteCode> {
  return requestJson<InviteCode>(
    buildApiUrl('/api/auth/invite-code'),
    token,
    { method: 'POST' },
    '生成邀请码失败',
  );
}

/**
 * Fetch admin stats.
 *
 * @param token - Bearer access token.
 * @returns Admin stats.
 */
export function adminGetStats(token: string): Promise<AdminStats> {
  return requestJson<AdminStats>(
    buildApiUrl('/api/admin/stats'),
    token,
    undefined,
    '获取统计信息失败',
  );
}

/**
 * Fetch admin user list.
 *
 * @param token - Bearer access token.
 * @returns Users.
 */
export function adminGetUsers(token: string): Promise<AdminUserInfo[]> {
  return requestJson<AdminUserInfo[]>(
    buildApiUrl('/api/admin/users'),
    token,
    undefined,
    '获取用户列表失败',
  );
}

/**
 * Grant or revoke admin access.
 *
 * @param token - Bearer access token.
 * @param userId - User id.
 * @param isAdmin - Whether the user should be admin.
 */
export async function adminSetAdmin(
  token: string,
  userId: number,
  isAdmin: boolean,
): Promise<void> {
  await requestJson<unknown>(
    buildApiUrl(`/api/admin/users/${userId}/admin`),
    token,
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
 * @param token - Bearer access token.
 * @param userId - User id.
 * @param newPassword - New password.
 */
export async function adminResetPassword(
  token: string,
  userId: number,
  newPassword: string,
): Promise<void> {
  await requestJson<unknown>(
    buildApiUrl(`/api/admin/users/${userId}/reset-password`),
    token,
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
 * @param token - Bearer access token.
 * @param userId - User id.
 */
export async function adminDeleteUser(token: string, userId: number): Promise<void> {
  await requestJson<unknown>(
    buildApiUrl(`/api/admin/users/${userId}`),
    token,
    { method: 'DELETE' },
    '删除用户失败',
  );
}

/**
 * Fetch admin invite codes.
 *
 * @param token - Bearer access token.
 * @returns Invite codes.
 */
export function adminGetInviteCodes(token: string): Promise<AdminInviteCode[]> {
  return requestJson<AdminInviteCode[]>(
    buildApiUrl('/api/admin/invite-codes'),
    token,
    undefined,
    '获取邀请码列表失败',
  );
}

/**
 * Create an admin invite code.
 *
 * @param token - Bearer access token.
 * @returns Created invite code summary.
 */
export function adminCreateInviteCode(token: string): Promise<{ id: number; code: string }> {
  return requestJson<{ id: number; code: string }>(
    buildApiUrl('/api/admin/invite-codes'),
    token,
    { method: 'POST' },
    '创建邀请码失败',
  );
}

/**
 * Delete an unused admin invite code.
 *
 * @param token - Bearer access token.
 * @param codeId - Invite code id.
 */
export async function adminDeleteInviteCode(token: string, codeId: number): Promise<void> {
  await requestJson<unknown>(
    buildApiUrl(`/api/admin/invite-codes/${codeId}`),
    token,
    { method: 'DELETE' },
    '删除邀请码失败',
  );
}

/**
 * Fetch runtime settings.
 *
 * @param token - Bearer access token.
 * @returns Runtime settings.
 */
export function adminGetRuntimeSettings(token: string): Promise<RuntimeSettingInfo[]> {
  return requestJson<RuntimeSettingInfo[]>(
    buildApiUrl('/api/admin/runtime-settings'),
    token,
    undefined,
    '获取运行配置失败',
  );
}

/**
 * Update runtime settings.
 *
 * @param token - Bearer access token.
 * @param payload - Runtime settings payload.
 * @returns Updated runtime settings.
 */
export function adminUpdateRuntimeSettings(
  token: string,
  payload: RuntimeSettingsUpdate,
): Promise<RuntimeSettingInfo[]> {
  return requestJson<RuntimeSettingInfo[]>(
    buildApiUrl('/api/admin/runtime-settings'),
    token,
    {
      method: 'PUT',
      body: JSON.stringify(payload),
    },
    '更新运行配置失败',
  );
}

/**
 * Fetch scheduled tasks.
 *
 * @param token - Bearer access token.
 * @returns Scheduled tasks.
 */
export function adminGetScheduledTasks(token: string): Promise<ScheduledTaskInfo[]> {
  return requestJson<ScheduledTaskInfo[]>(
    buildApiUrl('/api/admin/scheduled-tasks'),
    token,
    undefined,
    '获取定时任务失败',
  );
}

/**
 * Create a scheduled task.
 *
 * @param token - Bearer access token.
 * @param payload - Scheduled task payload.
 * @returns Created scheduled task.
 */
export function adminCreateScheduledTask(
  token: string,
  payload: ScheduledTaskCreate,
): Promise<ScheduledTaskInfo> {
  return requestJson<ScheduledTaskInfo>(
    buildApiUrl('/api/admin/scheduled-tasks'),
    token,
    {
      method: 'POST',
      body: JSON.stringify(payload),
    },
    '创建定时任务失败',
  );
}

/**
 * Update a scheduled task.
 *
 * @param token - Bearer access token.
 * @param taskId - Task id.
 * @param payload - Scheduled task patch.
 * @returns Updated scheduled task.
 */
export function adminUpdateScheduledTask(
  token: string,
  taskId: number,
  payload: ScheduledTaskUpdate,
): Promise<ScheduledTaskInfo> {
  return requestJson<ScheduledTaskInfo>(
    buildApiUrl(`/api/admin/scheduled-tasks/${taskId}`),
    token,
    {
      method: 'PUT',
      body: JSON.stringify(payload),
    },
    '更新定时任务失败',
  );
}

/**
 * Delete a scheduled task.
 *
 * @param token - Bearer access token.
 * @param taskId - Task id.
 */
export async function adminDeleteScheduledTask(token: string, taskId: number): Promise<void> {
  await requestJson<unknown>(
    buildApiUrl(`/api/admin/scheduled-tasks/${taskId}`),
    token,
    { method: 'DELETE' },
    '删除定时任务失败',
  );
}

/**
 * Fetch admin announcement list.
 *
 * @param token - Bearer access token.
 * @returns Announcements.
 */
export function adminGetAnnouncements(token: string): Promise<AnnouncementInfo[]> {
  return requestJson<AnnouncementInfo[]>(
    buildApiUrl('/api/admin/announcements'),
    token,
    undefined,
    '获取公告列表失败',
  );
}

/**
 * Create an announcement.
 *
 * @param token - Bearer access token.
 * @param payload - Announcement payload.
 * @returns Created announcement.
 */
export function adminCreateAnnouncement(
  token: string,
  payload: AnnouncementCreate,
): Promise<AnnouncementInfo> {
  return requestJson<AnnouncementInfo>(
    buildApiUrl('/api/admin/announcements'),
    token,
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
 * @param token - Bearer access token.
 * @param announcementId - Announcement id.
 * @param payload - Announcement patch.
 * @returns Updated announcement.
 */
export function adminUpdateAnnouncement(
  token: string,
  announcementId: number,
  payload: AnnouncementUpdate,
): Promise<AnnouncementInfo> {
  return requestJson<AnnouncementInfo>(
    buildApiUrl(`/api/admin/announcements/${announcementId}`),
    token,
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
 * @param token - Bearer access token.
 * @param announcementId - Announcement id.
 */
export async function adminDeleteAnnouncement(
  token: string,
  announcementId: number,
): Promise<void> {
  await requestJson<unknown>(
    buildApiUrl(`/api/admin/announcements/${announcementId}`),
    token,
    { method: 'DELETE' },
    '删除公告失败',
  );
}
