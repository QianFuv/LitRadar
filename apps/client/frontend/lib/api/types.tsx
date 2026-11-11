/**
 * Generated API value types and explicit sparse metadata accepted by the browser UI.
 */

import type { components, operations } from '@/lib/generated/api-schema';

type ApiSchemas = components['schemas'];

export type ArticleId = ApiSchemas['Article']['article_id'];
export type JournalId = ApiSchemas['Article']['journal_id'];
export type ArticleSearchMode = NonNullable<
  NonNullable<operations['list_articles']['parameters']['query']>['search_mode']
>;

type PageMeta = ApiSchemas['PageMeta'];

/** Article cards also accept sparse metadata from unavailable favorite records. */
export type Article = Pick<ApiSchemas['Article'], 'article_id'> &
  Partial<
    Omit<ApiSchemas['Article'], 'article_id' | 'authors' | 'journal_id' | 'journal_title' | 'title'>
  > & {
    [Field in 'authors' | 'journal_id' | 'journal_title' | 'title']?:
      | ApiSchemas['Article'][Field]
      | null;
  };

export type ArticlePage = Omit<ApiSchemas['PageArticle'], 'items' | 'page'> & {
  items: Article[];
  page: PageMeta;
};

export type ArticleAccessResponse = ApiSchemas['ArticleAccess'];
export type JournalRecord = ApiSchemas['Journal'];
export type CfpCatalogResponse = ApiSchemas['CfpCatalogResponse'];
export type CfpJournalSummary = ApiSchemas['CfpJournalSummary'];
export type CfpNoticePage = ApiSchemas['CfpNoticePage'];
export type CfpNoticeView = ApiSchemas['CfpNoticeView'];
export type CfpState = CfpNoticeView['state'];
export type CfpKind = CfpNoticeView['kind'];
export type CfpDateStage = ApiSchemas['Date']['stage'];
export type WeeklyArticle = Article;
export type WeeklyJournalSummary = ApiSchemas['WeeklyJournalSummary'];
export type WeeklyDatabaseSummary = ApiSchemas['WeeklyDatabaseWeeklyJournalSummary'];
export type WeeklyUpdatesSummaryResponse = ApiSchemas['WeeklyResponseWeeklyJournalSummary'];
export type WeeklyArticlePage = Omit<ApiSchemas['PageWeeklyArticle'], 'items' | 'page'> & {
  items: WeeklyArticle[];
  page: PageMeta;
};

export type { AnnouncementInfo } from '@/lib/api-contract';
type AnnouncementPriority = NonNullable<ApiSchemas['AnnouncementCreate']['priority']>;
export type Folder = ApiSchemas['FavoriteFolder'];
export type FavoriteItem = ApiSchemas['FavoriteFavorite'];
export type FavoriteArticleItem = ApiSchemas['FavoriteArticle'];

/** Stable cursor page of favorite article rows. */
export type FavoriteArticlePage = Omit<ApiSchemas['FavoritePageArticle'], 'page'> & {
  page: PageMeta;
};

export type FavoriteCheck = ApiSchemas['FavoriteMembership'];
export type FavoriteArticleRef = Required<ApiSchemas['FavoriteArticleRef']>;
export type RelationGraphFavoriteSummary = ApiSchemas['FavoriteGraphSummary'];
export type RelationGraphFavorite = ApiSchemas['FavoriteGraph'];
export type RelationGraphFavoritePage = ApiSchemas['FavoriteGraphPage'];
export type RelationGraphFavoriteCreate = ApiSchemas['RelationGraphFavoriteCreate'];
export type RelationGraphFavoriteUpdate = ApiSchemas['RelationGraphFavoriteUpdate'];
export type CitationFormat = 'bibtex' | 'ris' | 'endnote';
export type AccessToken = ApiSchemas['TokenInfo'];
export type CreatedAccessToken = ApiSchemas['IssuedToken'];
export type CnkiSessionStatus = ApiSchemas['CnkiStatus'];
export type CnkiLoginStartResponse = ApiSchemas['CnkiStartResponse'];
export type CnkiLoginPollResponse = ApiSchemas['CnkiPollResponse'];
export type {
  AdminInviteCode,
  AdminInviteCodeCreate,
  InviteCode,
  InviteCodeStatus,
} from '@/lib/api-contract';

export type AdminUserInfo = ApiSchemas['AdminUserResponse'];
export type AdminStats = ApiSchemas['AdminStats'];
export type AnnouncementCreate = Required<Omit<ApiSchemas['AnnouncementCreate'], 'priority'>> & {
  priority: AnnouncementPriority;
};
export type AnnouncementUpdate = Partial<AnnouncementCreate>;

type TelemetryQuery = NonNullable<operations['telemetry']['parameters']['query']>;

export type TelemetryResponse = ApiSchemas['Report'];
export type TelemetrySeries = ApiSchemas['Summary'];
export type TelemetryHour = ApiSchemas['Hour'];
export type TelemetrySearchShape = ApiSchemas['Distribution'];
export type TelemetryPeriod = NonNullable<TelemetryQuery['period']>;
export type TelemetryKind = NonNullable<TelemetryQuery['kind']>;
