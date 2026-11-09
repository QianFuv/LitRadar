/**
 * Lazily loaded React components that can also be fetched ahead of rendering.
 *
 * @packageDocumentation
 */

import { lazy, type ComponentType, type LazyExoticComponent } from 'react';

/** Lazy component paired with an idempotent, error-tolerant preload trigger. */
export type PreloadableComponent<Props extends object> = {
  /** Suspense-aware component that renders the loaded module export. */
  Component: LazyExoticComponent<ComponentType<Props>>;
  /** Start fetching the component module without rendering it. */
  preload: () => void;
};

/**
 * Create a lazy component whose module request is shared between preloading and rendering.
 *
 * A failed request is forgotten so a later intent or render retries instead of replaying the
 * cached rejection; the rejection still reaches the nearest error boundary when rendering.
 *
 * @param loadComponent - Dynamic import that resolves the component export.
 * @returns Lazy component and its preload trigger.
 */
export function createPreloadableComponent<Props extends object>(
  loadComponent: () => Promise<ComponentType<Props>>,
): PreloadableComponent<Props> {
  let pendingModule: Promise<{ default: ComponentType<Props> }> | null = null;

  /**
   * Return the in-flight or completed module request, starting it on first use.
   *
   * @returns Promise for the lazy module record.
   */
  function loadModule(): Promise<{ default: ComponentType<Props> }> {
    pendingModule ??= loadComponent().then(
      (component) => ({ default: component }),
      (error: unknown) => {
        pendingModule = null;
        throw error;
      },
    );
    return pendingModule;
  }

  /** Fetch the module in the background and leave failures to the eventual render. */
  function preload(): void {
    loadModule().catch(() => undefined);
  }

  return { Component: lazy(loadModule), preload };
}
