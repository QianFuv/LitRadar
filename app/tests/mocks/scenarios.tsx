/**
 * Typed builders for stable cross-stack API scenarios.
 */

import type { components } from '@/lib/generated/api-schema';

import articlePageJson from '../../../testdata/scenarios/api/article-page.json';
import errorJson from '../../../testdata/scenarios/api/error.json';
import loginJson from '../../../testdata/scenarios/api/login.json';
import maskedNotificationSettingsJson from '../../../testdata/scenarios/api/masked-notification-settings.json';
import weeklyArticlePageJson from '../../../testdata/scenarios/api/weekly-article-page.json';
import weeklyUpdateSummaryJson from '../../../testdata/scenarios/api/weekly-update-summary.json';
import weeklyUpdatesJson from '../../../testdata/scenarios/api/weekly-updates.json';

type ApiSchemas = components['schemas'];
type DatePrecision = ApiSchemas['DatePrecision'];

export type LoginScenario = ApiSchemas['LoginResponse'];
export type ArticlePageScenario = ApiSchemas['ArticlePage'];
export type WeeklyUpdatesScenario = ApiSchemas['WeeklyUpdatesResponse'];
export type WeeklyUpdateSummaryScenario = ApiSchemas['WeeklyUpdatesSummaryResponse'];
export type WeeklyArticlePageScenario = ApiSchemas['WeeklyArticlePage'];
export type MaskedNotificationSettingsScenario = ApiSchemas['NotificationSettingsResponse'];
export type ErrorScenario = ApiSchemas['ErrorEnvelope'];

const LOGIN_SCENARIO: LoginScenario = loginJson satisfies LoginScenario;
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
