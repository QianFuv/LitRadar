/**
 * Minimal React error boundary that renders a recoverable fallback component.
 *
 * Reporting stays in each fallback so every boundary emits its own safe event once.
 */

import { Component, type ComponentType, type ReactNode } from 'react';

/** Properties passed to an error boundary fallback. */
export type ErrorFallbackProps = {
  /** Error captured from the failed subtree. */
  error: Error & { digest?: string };
  /** Clear the captured error and render the subtree again. */
  reset: () => void;
};

type ErrorBoundaryProps = {
  /** Subtree protected by the boundary. */
  children: ReactNode;
  /** Component rendered while an error is captured. */
  fallback: ComponentType<ErrorFallbackProps>;
  /** Value whose change clears a captured error, such as the current route. */
  resetKey?: string;
};

type ErrorBoundaryState = {
  error: (Error & { digest?: string }) | null;
  resetKey: string | undefined;
};

/**
 * Normalize a thrown value into an Error instance without reading its payload.
 *
 * @param value - Value thrown during rendering.
 * @returns Error suitable for fallback rendering.
 */
function toError(value: unknown): Error {
  return value instanceof Error ? value : new Error('Non-error value thrown during rendering');
}

/**
 * Capture render failures below it and render a fallback with a reset action.
 */
export class ErrorBoundary extends Component<ErrorBoundaryProps, ErrorBoundaryState> {
  state: ErrorBoundaryState = { error: null, resetKey: this.props.resetKey };

  /**
   * Store the captured error so the next render shows the fallback.
   *
   * @param error - Value thrown by a descendant.
   * @returns State update containing the normalized error.
   */
  static getDerivedStateFromError(error: unknown): Partial<ErrorBoundaryState> {
    return { error: toError(error) };
  }

  /**
   * Clear a captured error when the reset key changes, such as after navigation.
   *
   * @param props - Next boundary properties.
   * @param state - Current boundary state.
   * @returns State update for a changed reset key, or null.
   */
  static getDerivedStateFromProps(
    props: ErrorBoundaryProps,
    state: ErrorBoundaryState,
  ): Partial<ErrorBoundaryState> | null {
    if (props.resetKey === state.resetKey) {
      return null;
    }
    return { error: null, resetKey: props.resetKey };
  }

  /** Clear the captured error and retry rendering the protected subtree. */
  reset = (): void => {
    this.setState({ error: null });
  };

  /**
   * Render the protected subtree or the fallback for a captured error.
   *
   * @returns Children or fallback content.
   */
  render(): ReactNode {
    const { error } = this.state;
    if (error) {
      const Fallback = this.props.fallback;
      return <Fallback error={error} reset={this.reset} />;
    }
    return this.props.children;
  }
}
