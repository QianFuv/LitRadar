/**
 * Shared data-plane and control-plane API value types.
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
  publication_year?: number | null;
  date?: string | null;
  authors?: string[] | null;
  abstract?: string | null;
  doi?: string | null;
  pmid?: string | null;
  start_page?: string | null;
  end_page?: string | null;
  retraction_dois?: string[];
  journal_title?: string | null;
  open_access?: boolean | null;
  in_press?: boolean | null;
  volume?: string | null;
  number?: string | null;
}

export interface ArticlePage {
  items: Article[];
  page: PageMeta;
}

export interface ArticleAccessAction {
  available: boolean;
  label: string;
  requires_login: boolean;
  message?: string | null;
}

export interface ArticleAccessResponse {
  detail: ArticleAccessAction;
  abstract_page: ArticleAccessAction;
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
  publication_year?: number | null;
  date?: string | null;
  authors?: string[] | null;
  abstract?: string | null;
  doi?: string | null;
  journal_title?: string | null;
  open_access?: boolean | null;
  in_press?: boolean | null;
  volume?: string | null;
  number?: string | null;
  issn?: string | null;
  eissn?: string | null;
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
