/**
 * Shared Node request interception server for Vitest.
 */

import { setupServer } from 'msw/node';

export const server = setupServer();
