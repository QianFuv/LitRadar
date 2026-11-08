/**
 * Typed builders for stable cross-stack API scenarios.
 */

import type { components } from '@/lib/generated/api-schema';

import articlePageJson from '../../../../../tests/data/scenarios/api/article-page.json';
import errorJson from '../../../../../tests/data/scenarios/api/error.json';
import loginJson from '../../../../../tests/data/scenarios/api/login.json';
import maskedNotificationSettingsJson from '../../../../../tests/data/scenarios/api/masked-notification-settings.json';
import weeklyArticlePageJson from '../../../../../tests/data/scenarios/api/weekly-article-page.json';
import weeklyUpdateSummaryJson from '../../../../../tests/data/scenarios/api/weekly-update-summary.json';
import weeklyUpdatesJson from '../../../../../tests/data/scenarios/api/weekly-updates.json';

type ApiSchemas = components['schemas'];
type DatePrecision = ApiSchemas['DatePrecision'];
type AuthKind = ApiSchemas['UserResponse']['auth_kind'];

export type LoginScenario = ApiSchemas['LoginResponse'];
export type AuthUserScenario = ApiSchemas['UserResponse'];
export type AuthProvidersScenario = ApiSchemas['AuthProvidersResponse'];
export type ArticlePageScenario = ApiSchemas['ArticlePage'];
export type WeeklyUpdatesScenario = ApiSchemas['WeeklyUpdatesResponse'];
export type WeeklyUpdateSummaryScenario = ApiSchemas['WeeklyUpdatesSummaryResponse'];
export type WeeklyArticlePageScenario = ApiSchemas['WeeklyArticlePage'];
export type MaskedNotificationSettingsScenario = ApiSchemas['NotificationSettingsResponse'];
export type ErrorScenario = ApiSchemas['ErrorEnvelope'];

/**
 * Narrow a JSON-imported account kind to the generated enum.
 *
 * @param value - JSON scenario value.
 * @returns Valid generated account kind.
 */
function parseAuthKind(value: string): AuthKind {
  switch (value) {
    case 'local':
    case 'oidc':
      return value;
    default:
      throw new Error(`Invalid account kind fixture: ${value}`);
  }
}

const LOGIN_SCENARIO: LoginScenario = {
  ...loginJson,
  user: { ...loginJson.user, auth_kind: parseAuthKind(loginJson.user.auth_kind) },
} satisfies LoginScenario;
/**
 * Narrow a JSON-imported date precision to the generated enum.
 *
 * @param value - JSON scenario value.
 * @returns Valid generated date precision.
 */
function parseDatePrecision(value: string): DatePrecision {
  switch (value) {
    case 'year':
    case 'month':
    case 'day':
      return value;
    default:
      throw new Error(`Invalid date precision fixture: ${value}`);
  }
}

const ARTICLE_PAGE_SCENARIO: ArticlePageScenario = {
  ...articlePageJson,
  items: articlePageJson.items.map((article) => ({
    ...article,
    date_precision: parseDatePrecision(article.date_precision),
  })),
} satisfies ArticlePageScenario;
const WEEKLY_UPDATES_SCENARIO: WeeklyUpdatesScenario = {
  ...weeklyUpdatesJson,
  databases: weeklyUpdatesJson.databases.map((database) => ({
    ...database,
    journals: database.journals.map((journal) => ({
      ...journal,
      articles: journal.articles.map((article) => ({
        ...article,
        date_precision: parseDatePrecision(article.date_precision),
      })),
    })),
  })),
} satisfies WeeklyUpdatesScenario;
const WEEKLY_UPDATE_SUMMARY_SCENARIO: WeeklyUpdateSummaryScenario =
  weeklyUpdateSummaryJson satisfies WeeklyUpdateSummaryScenario;
const WEEKLY_ARTICLE_PAGE_SCENARIO: WeeklyArticlePageScenario = {
  ...weeklyArticlePageJson,
  items: weeklyArticlePageJson.items.map((article) => ({
    ...article,
    date_precision: parseDatePrecision(article.date_precision),
  })),
} satisfies WeeklyArticlePageScenario;
const MASKED_NOTIFICATION_SETTINGS_SCENARIO: MaskedNotificationSettingsScenario =
  maskedNotificationSettingsJson satisfies MaskedNotificationSettingsScenario;
const ERROR_SCENARIO: ErrorScenario = errorJson satisfies ErrorScenario;

/**
 * Clone a JSON scenario and apply top-level overrides without mutating shared data.
 *
 * @typeParam Scenario - Generated API schema represented by the scenario.
 * @param scenario - Stable checked-in scenario.
 * @param overrides - Optional top-level response overrides.
 * @returns Independent typed scenario data.
 */
function buildScenario<Scenario>(scenario: Scenario, overrides: Partial<Scenario>): Scenario {
  const clone = JSON.parse(JSON.stringify(scenario)) as Scenario;
  return { ...clone, ...overrides };
}

/**
 * Build the stable login response scenario.
 *
 * @param overrides - Optional top-level response overrides.
 * @returns Independent login response data.
 */
export function createLoginScenario(overrides: Partial<LoginScenario> = {}): LoginScenario {
  return buildScenario(LOGIN_SCENARIO, overrides);
}

/**
 * Build an authenticated local user projection.
 *
 * @param overrides - Optional user field overrides.
 * @returns Independent user response data.
 */
export function createAuthUserScenario(
  overrides: Partial<AuthUserScenario> = {},
): AuthUserScenario {
  return buildScenario(LOGIN_SCENARIO.user, overrides);
}

/**
 * Build the sign-in provider summary with single sign-on enabled.
 *
 * @param overrides - Optional provider field overrides.
 * @returns Independent provider summary data.
 */
export function createAuthProvidersScenario(
  overrides: Partial<AuthProvidersScenario> = {},
): AuthProvidersScenario {
  return { oidc_enabled: true, oidc_display_name: 'authentik', ...overrides };
}

/**
 * Build the stable article page scenario.
 *
 * @param overrides - Optional top-level response overrides.
 * @returns Independent article page data.
 */
export function createArticlePageScenario(
  overrides: Partial<ArticlePageScenario> = {},
): ArticlePageScenario {
  return buildScenario(ARTICLE_PAGE_SCENARIO, overrides);
}

/**
 * Build the stable weekly update scenario.
 *
 * @param overrides - Optional top-level response overrides.
 * @returns Independent weekly update data.
 */
export function createWeeklyUpdatesScenario(
  overrides: Partial<WeeklyUpdatesScenario> = {},
): WeeklyUpdatesScenario {
  return buildScenario(WEEKLY_UPDATES_SCENARIO, overrides);
}

/**
 * Build the stable weekly update summary scenario.
 *
 * @param overrides - Optional top-level response overrides.
 * @returns Independent weekly summary data.
 */
export function createWeeklyUpdateSummaryScenario(
  overrides: Partial<WeeklyUpdateSummaryScenario> = {},
): WeeklyUpdateSummaryScenario {
  return buildScenario(WEEKLY_UPDATE_SUMMARY_SCENARIO, overrides);
}

/**
 * Build the stable weekly article page scenario.
 *
 * @param overrides - Optional top-level response overrides.
 * @returns Independent weekly article page data.
 */
export function createWeeklyArticlePageScenario(
  overrides: Partial<WeeklyArticlePageScenario> = {},
): WeeklyArticlePageScenario {
  return buildScenario(WEEKLY_ARTICLE_PAGE_SCENARIO, overrides);
}

/**
 * Build the stable masked notification settings scenario.
 *
 * @param overrides - Optional top-level response overrides.
 * @returns Independent notification settings data.
 */
export function createMaskedNotificationSettingsScenario(
  overrides: Partial<MaskedNotificationSettingsScenario> = {},
): MaskedNotificationSettingsScenario {
  return buildScenario(MASKED_NOTIFICATION_SETTINGS_SCENARIO, overrides);
}

/**
 * Build the stable API error scenario.
 *
 * @param overrides - Optional top-level response overrides.
 * @returns Independent error envelope data.
 */
export function createErrorScenario(overrides: Partial<ErrorScenario> = {}): ErrorScenario {
  return buildScenario(ERROR_SCENARIO, overrides);
}
