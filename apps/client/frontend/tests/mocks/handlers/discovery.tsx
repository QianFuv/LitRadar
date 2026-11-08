/**
 * Explicit discovery and weekly-update scenario handlers.
 */

import { http, HttpResponse, type RequestHandler } from 'msw';

import {
  createArticlePageScenario,
  createWeeklyArticlePageScenario,
  createWeeklyUpdateSummaryScenario,
  createWeeklyUpdatesScenario,
  type ArticlePageScenario,
  type WeeklyArticlePageScenario,
  type WeeklyUpdateSummaryScenario,
  type WeeklyUpdatesScenario,
} from '@/tests/mocks/scenarios';

const API_URL = 'http://localhost/api';

/** Overrides supported by discovery scenario handlers. */
export type DiscoveryScenarioOverrides = {
  articles?: Partial<ArticlePageScenario>;
  weeklyArticles?: Partial<WeeklyArticlePageScenario>;
  weeklySummary?: Partial<WeeklyUpdateSummaryScenario>;
  weeklyUpdates?: Partial<WeeklyUpdatesScenario>;
};

/**
 * Create discovery handlers backed by typed shared scenarios.
 *
 * @param overrides - Optional article and weekly response overrides.
 * @returns Discovery request handlers.
 */
export function createDiscoveryScenarioHandlers(
  overrides: DiscoveryScenarioOverrides = {},
): RequestHandler[] {
  return [
    http.get(`${API_URL}/articles`, () =>
      HttpResponse.json(createArticlePageScenario(overrides.articles)),
    ),
    http.get(`${API_URL}/weekly-updates`, () =>
      HttpResponse.json(createWeeklyUpdatesScenario(overrides.weeklyUpdates)),
    ),
    http.get(`${API_URL}/weekly-updates/summary`, () =>
      HttpResponse.json(createWeeklyUpdateSummaryScenario(overrides.weeklySummary)),
    ),
    http.get(`${API_URL}/weekly-updates/articles`, () =>
      HttpResponse.json(createWeeklyArticlePageScenario(overrides.weeklyArticles)),
    ),
  ];
}

/** Stable discovery happy-path handlers. */
export const DISCOVERY_SCENARIO_HANDLERS = createDiscoveryScenarioHandlers();
