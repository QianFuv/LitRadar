/**
 * Generated API value types and explicit sparse metadata accepted by the browser UI.
 */

import type { components } from '@/lib/generated/api-schema';

type ApiSchemas = components['schemas'];

export type ArticleId = ApiSchemas['ArticleId'];
export type JournalId = ApiSchemas['JournalId'];
export type ArticleSearchMode = ApiSchemas['ArticleSearchMode'];
export type { DatePrecision, PushStatsState } from '@/lib/api-contract';

export type PageMeta = Omit<ApiSchemas['PageMeta'], 'total'> &
  Required<Pick<ApiSchemas['PageMeta'], 'total'>>;

/** Article cards also accept sparse metadata from unavailable favorite records. */
export type Article = Pick<ApiSchemas['ArticleRecord'], 'article_id'> &
  Partial<
    Omit<
      ApiSchemas['ArticleRecord'],
      'article_id' | 'authors' | 'journal_id' | 'journal_title' | 'title'
    >
  > & {
    [Field in 'authors' | 'journal_id' | 'journal_title' | 'title']?:
      | ApiSchemas['ArticleRecord'][Field]
      | null;
  };

export type ArticlePage = Omit<ApiSchemas['ArticlePage'], 'items' | 'page'> & {
  items: Article[];
  page: PageMeta;
};

export type ArticleAccessAction = ApiSchemas['ArticleAccessAction'];
export type ArticleAccessResponse = ApiSchemas['ArticleAccessResponse'];
export type ValueCount = ApiSchemas['ValueCount'];
export type YearSummary = ApiSchemas['YearSummary'];
export type JournalOption = Pick<ApiSchemas['JournalOption'], 'journal_id'> &
  Partial<Pick<ApiSchemas['JournalOption'], 'title'>>;
export type JournalRecord = ApiSchemas['JournalRecord'];
export type JournalPage = ApiSchemas['JournalPage'];
export type CfpCatalogResponse = ApiSchemas['CfpCatalogResponse'];
export type CfpJournalSummary = ApiSchemas['CfpJournalSummary'];
export type CfpNoticePage = ApiSchemas['CfpNoticePage'];
export type CfpNoticeView = ApiSchemas['CfpNoticeView'];
export type CfpState = ApiSchemas['CfpState'];
export type CfpKind = ApiSchemas['CfpKind'];
export type CfpDateStage = ApiSchemas['CfpDateStage'];
export type WeeklyArticle = Article;
export type WeeklyJournalSummary = ApiSchemas['WeeklyJournalSummary'];
export type WeeklyDatabaseSummary = ApiSchemas['WeeklyDatabaseSummary'];
export type WeeklyUpdatesSummaryResponse = ApiSchemas['WeeklyUpdatesSummaryResponse'];
export type WeeklyArticlePage = Omit<ApiSchemas['WeeklyArticlePage'], 'items' | 'page'> & {
  items: WeeklyArticle[];
  page: PageMeta;
};

type AnnouncementPriority = 'high' | 'normal' | 'low';

export type AnnouncementInfo = Omit<ApiSchemas['AnnouncementInfo'], 'priority'> & {
  priority: AnnouncementPriority;
};
export type Folder = ApiSchemas['FolderResponse'];
export type FavoriteItem = ApiSchemas['FavoriteResponse'];
export type FavoriteArticleItem = ApiSchemas['FavoriteArticleResponse'];

/** Stable cursor page of favorite article rows. */
export type FavoriteArticlePage = Omit<ApiSchemas['FavoriteArticlePage'], 'page'> & {
  page: PageMeta;
};

export type FavoriteCheck = ApiSchemas['FavoriteCheckResponse'];
export type FavoriteBatchCheckItem = ApiSchemas['FavoriteBatchCheckResponse'];
export type FavoriteArticleRef = Required<ApiSchemas['FavoriteArticleRef']>;
export type CitationFormat = 'bibtex' | 'ris' | 'endnote';
export type AccessToken = ApiSchemas['TokenInfo'];
export type CnkiSessionStatus = ApiSchemas['CnkiSessionStatusResponse'];
export type CnkiLoginStartResponse = ApiSchemas['CnkiLoginStartResponse'];
export type CnkiLoginPollResponse = ApiSchemas['CnkiLoginPollResponse'];
export type {
  AdminInviteCode,
  AdminInviteCodeCreate,
  InviteCode,
  InviteCodeStatus,
} from '@/lib/api-contract';

export type AdminUserInfo = ApiSchemas['AdminUserInfo'];
export type IndexDbStats = Omit<ApiSchemas['IndexDatabaseStats'], 'error'> & {
  error?: Exclude<ApiSchemas['IndexDatabaseStats']['error'], null>;
};
export type PushDbStats = Omit<ApiSchemas['PushStats'], 'delivered_count' | 'user_results'> & {
  [Field in 'delivered_count' | 'user_results']?: NonNullable<ApiSchemas['PushStats'][Field]>;
};
export type AdminStats = Omit<ApiSchemas['AdminStatsResponse'], 'index' | 'push'> & {
  index: Omit<ApiSchemas['IndexStats'], 'databases'> & { databases: IndexDbStats[] };
  push: PushDbStats[];
};
export type AnnouncementCreate = Required<Omit<ApiSchemas['AnnouncementCreate'], 'priority'>> & {
  priority: AnnouncementPriority;
};
export type AnnouncementUpdate = Partial<AnnouncementCreate>;
