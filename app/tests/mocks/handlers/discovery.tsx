/**
 * Explicit discovery and weekly-update scenario handlers.
 */

import { http, HttpResponse, type RequestHandler } from 'msw';

import {
  createArticlePageScenario,
  createWeeklyUpdatesScenario,
  type ArticlePageScenario,
  type WeeklyUpdatesScenario,
} from '@/tests/mocks/scenarios';

const API_URL = 'http://localhost/api';

/** Overrides supported by discovery scenario handlers. */
export type DiscoveryScenarioOverrides = {
  articles?: Partial<ArticlePageScenario>;
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
  ];
}

/** Stable discovery happy-path handlers. */
export const DISCOVERY_SCENARIO_HANDLERS = createDiscoveryScenarioHandlers();
